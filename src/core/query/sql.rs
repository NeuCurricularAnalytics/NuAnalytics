//! Read-only SQL, run through the `query_readonly` Postgres functions.
//!
//! `db exec-sql` reaches the Supabase Management API and so is cloud-only — a self-hosted
//! stack has no project ref. `PostgREST` exposes no SQL endpoint either, so the read path
//! goes through functions called over `/rest/v1/rpc/`, which behave identically on both
//! deployments:
//!
//! - `query_readonly(q, max_rows)` runs an ad-hoc statement — `db query --sql`.
//! - `query_readonly_params(q, params, max_rows)` runs a statement whose inputs are bound
//!   as `$1`, a JSON object, rather than pasted into the text. The curated queries in
//!   [`catalog`](super::catalog) go through it, and so does an ad-hoc query that brings
//!   `params`.
//!
//! **The refusal that matters is the engine's, not this module's.** Both functions are
//! declared `STABLE`, so `PostgREST` runs the call in a READ ONLY transaction and Postgres
//! refuses every write — verified against the live deployment, where
//! `SELECT nextval(...)` comes back `25006` and the sequence does not move. The functions'
//! own wrapping is a second layer: `q` lands in a subquery, so bare DML is a syntax error
//! and a data-modifying CTE is refused "must be at the top level".
//!
//! Note the mechanism is the transaction, not SPI's `read_only` flag — that flag rejects
//! non-SELECT commands but does not stop a volatile function called inside a SELECT from
//! writing, which is why the same call over psql succeeds.
//!
//! The check in this file turns all of that into a message naming the offending keyword
//! before a round trip. It is a courtesy, not the guarantee, and is deliberately a filter
//! on obvious writes rather than an attempt at a complete SQL parser.

use std::fmt;
use std::sync::Arc;

use crate::core::database::codes;
use crate::core::database::functions::{
    INSTALL_STEP, QUERY_READONLY as READONLY_FN, QUERY_READONLY_PARAMS as READONLY_PARAMS_FN,
};
use crate::core::database::{DatabaseError, DbClient};
use crate::core::json::{to_json_pretty, value_kind};
use serde::{Deserialize, Serialize};

/// Rows the CLI returns when the caller does not say.
///
/// The result is a single `jsonb` value, so `PGRST_DB_MAX_ROWS` does not cap it — without
/// a bound a broad query returns one very large payload instead of being truncated.
pub const DEFAULT_MAX_ROWS: usize = 1_000;

/// Upper bound on the CLI's `--max-rows`, to keep one request from pulling the corpus.
pub const MAX_MAX_ROWS: usize = 10_000;

/// Rows an agent gets when it does not say.
///
/// Lower than the CLI's: every row an agent receives is context it has to read, and a
/// question that needs more than this usually wants an aggregate, not more rows.
pub const AGENT_DEFAULT_MAX_ROWS: usize = 200;

/// Upper bound on the rows an agent may ask for.
pub const AGENT_MAX_ROWS: usize = 2_000;

/// How many rows one caller gets by default, and at most.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowLimits {
    /// Used when the request names no `max_rows`.
    pub default: usize,
    /// The most a request may ask for; larger requests are clamped, not refused.
    pub cap: usize,
}

impl RowLimits {
    /// `nuanalytics db query --sql`.
    pub const CLI: Self = Self {
        default: DEFAULT_MAX_ROWS,
        cap: MAX_MAX_ROWS,
    };

    /// An agent, which reads every row it is given as context.
    pub const AGENT: Self = Self {
        default: AGENT_DEFAULT_MAX_ROWS,
        cap: AGENT_MAX_ROWS,
    };

    /// The row bound for a request: its own, or the default, within `1..=cap`.
    #[must_use]
    pub fn resolve(self, requested: Option<usize>) -> usize {
        requested.unwrap_or(self.default).clamp(1, self.cap)
    }
}

/// Statements a read-only path must never be asked to run.
///
/// Matched as whole words against the statement's code, with strings and comments set
/// aside. The engine refuses these anyway; naming the keyword is friendlier than relaying
/// `"UPDATE is not allowed in a non-volatile function"` from a failed round trip.
const WRITE_KEYWORDS: [&str; 16] = [
    "insert", "update", "delete", "truncate", "drop", "create", "alter", "grant", "revoke",
    "comment", "vacuum", "reindex", "refresh", "call", "do", "copy",
];

/// Why a statement was refused before being sent.
#[derive(Debug, PartialEq, Eq)]
pub enum SqlRejection {
    /// There was no statement.
    Empty,
    /// The statement does not begin `SELECT` or `WITH`.
    NotASelect(String),
    /// A write keyword appeared outside a string literal.
    WriteKeyword(String),
    /// More than one statement was present.
    MultipleStatements,
}

impl SqlRejection {
    /// The message shown to the caller.
    #[must_use]
    pub fn message(&self) -> String {
        match self {
            Self::Empty => "the query is empty".to_string(),
            Self::NotASelect(word) => {
                format!("a read-only query must start with SELECT or WITH, found \"{word}\"")
            }
            Self::WriteKeyword(word) => {
                format!("\"{}\" writes; this path is read-only", word.to_uppercase())
            }
            Self::MultipleStatements => {
                "pass one statement — a second statement after a semicolon would run too"
                    .to_string()
            }
        }
    }
}

/// What a stretch of SQL text is, for the purposes of this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Span {
    /// Tokens the server parses: keywords, names, operators, `$1`.
    Code,
    /// `--` to end of line, or `/* */` (which nests in Postgres).
    Comment,
    /// `'…'`, `E'…'`, `"…"` or `$tag$…$tag$` — data or a quoted name, never a keyword.
    Literal,
}

/// Split `sql` into code, comment and literal spans, in order and covering every byte.
///
/// Deliberately small: it knows how each kind of span *ends*, which is all that keyword
/// matching and statement-end detection need. An unterminated literal or comment runs to
/// the end of the text, which is what the server would complain about anyway.
pub(crate) fn lex(sql: &str) -> Vec<(Span, &str)> {
    let bytes = sql.as_bytes();
    let mut spans = Vec::new();
    let mut code_start = 0;
    let mut i = 0;
    while i < bytes.len() {
        let end = match bytes[i] {
            b'-' if bytes.get(i + 1) == Some(&b'-') => Some((
                Span::Comment,
                sql[i..].find(['\n', '\r']).map_or(sql.len(), |n| i + n),
            )),
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                Some((Span::Comment, block_comment_end(bytes, i)))
            }
            b'\'' => {
                let escaped = i > 0
                    && matches!(bytes[i - 1], b'E' | b'e')
                    && (i < 2 || !is_ident_byte(bytes[i - 2]));
                Some((Span::Literal, quoted_end(bytes, i, b'\'', escaped)))
            }
            b'"' => Some((Span::Literal, quoted_end(bytes, i, b'"', false))),
            b'$' => dollar_quote_end(sql, i).map(|end| (Span::Literal, end)),
            _ => None,
        };
        match end {
            Some((kind, end)) => {
                if code_start < i {
                    spans.push((Span::Code, &sql[code_start..i]));
                }
                spans.push((kind, &sql[i..end]));
                i = end;
                code_start = end;
            }
            None => i += 1,
        }
    }
    if code_start < sql.len() {
        spans.push((Span::Code, &sql[code_start..]));
    }
    spans
}

/// Whether `b` can continue an identifier — so the `E` in `typE'…'` is not an escape
/// prefix. Bytes of a non-ASCII letter count, as Postgres lets them into names.
const fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$' || b >= 0x80
}

/// Whether `b` can appear in a dollar-quote tag: an identifier byte other than `$`.
const fn is_tag_byte(b: u8) -> bool {
    b != b'$' && is_ident_byte(b)
}

/// End of the `/* */` comment starting at `start`. Postgres nests block comments.
fn block_comment_end(bytes: &[u8], start: usize) -> usize {
    let mut depth = 0usize;
    let mut i = start;
    while i + 1 < bytes.len() {
        match (bytes[i], bytes[i + 1]) {
            (b'/', b'*') => {
                depth += 1;
                i += 2;
            }
            (b'*', b'/') => {
                depth -= 1;
                i += 2;
                if depth == 0 {
                    return i;
                }
            }
            _ => i += 1,
        }
    }
    bytes.len()
}

/// End of the quoted run starting at `start`, where a doubled quote is an escaped quote
/// and, in an `E'…'` string, so is a backslash-escaped one.
fn quoted_end(bytes: &[u8], start: usize, quote: u8, backslash_escapes: bool) -> usize {
    let mut i = start + 1;
    while i < bytes.len() {
        if backslash_escapes && bytes[i] == b'\\' {
            i += 2;
            continue;
        }
        if bytes[i] == quote {
            if bytes.get(i + 1) == Some(&quote) {
                i += 2;
                continue;
            }
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

/// End of the `$tag$…$tag$` literal starting at `start`, or `None` when the `$` opens no
/// dollar quote — `$1`, the bound parameter, is code.
fn dollar_quote_end(sql: &str, start: usize) -> Option<usize> {
    let rest = &sql[start + 1..];
    let tag_len = rest.find('$')?;
    let tag = &rest[..tag_len];
    let starts_like_a_tag = tag
        .bytes()
        .next()
        .is_none_or(|b| b.is_ascii_alphabetic() || b == b'_' || b >= 0x80);
    if !starts_like_a_tag || !tag.bytes().all(is_tag_byte) {
        return None;
    }
    let delimiter = &sql[start..=start + 1 + tag_len];
    let body_start = start + delimiter.len();
    Some(
        sql[body_start..]
            .find(delimiter)
            .map_or(sql.len(), |n| body_start + n + delimiter.len()),
    )
}

/// The statement's code, with comments and literals each reduced to one space.
///
/// Keyword matching runs over the result, so a column named `"update_note"` or a literal
/// `'please delete later'` cannot trip the check. A space rather than nothing so adjacent
/// tokens do not fuse into a new word.
fn strip_noise(sql: &str) -> String {
    lex(sql)
        .into_iter()
        .map(|(kind, text)| if kind == Span::Code { text } else { " " })
        .collect()
}

/// The text to send for `sql`: its own bytes, less a statement-ending `;`, plus a newline.
///
/// The backend wraps the text as `(… q …)`. A `;` there is a syntax error, and a query
/// whose last line is a `--` comment would comment out the closing parenthesis — so the
/// `;` goes only when it is the last *code* (not one inside `';'` or a comment), and a
/// newline always ends the text. Everything else is sent as written, which is what lets
/// the catalog's queries run byte-for-byte as compiled in.
#[must_use]
pub fn prepare_query_text(sql: &str) -> String {
    let spans = lex(sql);
    let mut offset = 0;
    let mut last_code_end = None;
    for (kind, text) in &spans {
        if *kind == Span::Code {
            let trimmed = text.trim_end();
            if !trimmed.is_empty() {
                last_code_end = Some(offset + trimmed.len());
            }
        } else if *kind == Span::Literal {
            last_code_end = None;
        }
        offset += text.len();
    }
    let mut out = String::with_capacity(sql.len() + 1);
    match last_code_end {
        Some(end) if sql[..end].ends_with(';') => {
            out.push_str(&sql[..end - 1]);
            out.push_str(&sql[end..]);
        }
        _ => out.push_str(sql),
    }
    out.push('\n');
    out
}

/// Refuse a statement that is obviously not a read-only query.
///
/// Returns `None` when the statement looks like a single `SELECT`/`WITH`. The engine is
/// what actually enforces read-only-ness — see the module docs.
#[must_use]
pub fn reject_if_not_read_only(sql: &str) -> Option<SqlRejection> {
    let cleaned = strip_noise(sql);
    let trimmed = cleaned.trim().trim_end_matches(';').trim();
    if trimmed.is_empty() {
        return Some(SqlRejection::Empty);
    }
    if trimmed.contains(';') {
        return Some(SqlRejection::MultipleStatements);
    }

    let lower = trimmed.to_lowercase();
    let first = lower.split_whitespace().next().unwrap_or_default();
    if first != "select" && first != "with" {
        return Some(SqlRejection::NotASelect(first.to_string()));
    }

    // Whole-word match: `updated_year` and `created_at` are ordinary column names.
    let found = lower
        .split(|c: char| !c.is_alphanumeric() && c != '_')
        .find(|word| WRITE_KEYWORDS.contains(word));
    found.map(|w| SqlRejection::WriteKeyword(w.to_string()))
}

/// Why a query failed.
#[derive(Debug)]
pub enum SqlError {
    /// Refused here, before a round trip.
    Rejected(SqlRejection),
    /// The database ran the call and refused the query.
    Backend {
        /// Backend function that was called — named when it turns out not to exist.
        function: &'static str,
        /// Postgres SQLSTATE (`42P01`) or `PostgREST` code (`PGRST202`), when reported.
        code: Option<String>,
        /// The backend's own message.
        message: String,
        /// The backend's hint, when it gave one.
        hint: Option<String>,
        /// The backend's detail, when it gave one.
        detail: Option<String>,
    },
    /// The call did not get as far as the query: not signed in, unreachable, unreadable.
    Database(DatabaseError),
    /// The backend answered, but not in the shape this client expects.
    Shape(String),
}

impl SqlError {
    /// Classify a failed `rpc` call, keeping the backend's error object when it sent one.
    ///
    /// See [`DatabaseError::backend_error`]: anything that is not the backend's own
    /// refusal — transport, auth, a proxy's JSON — stays the [`DatabaseError`] it was.
    #[must_use]
    pub fn from_database(error: DatabaseError, function: &'static str) -> Self {
        match error.backend_error() {
            Some(backend) => Self::Backend {
                function,
                code: Some(backend.code),
                message: backend.message,
                hint: backend.hint,
                detail: backend.detail,
            },
            None => Self::Database(error),
        }
    }

    /// Prefix the message with the catalog query it came from, so a failure names it.
    #[must_use]
    pub fn in_query(self, name: &str) -> Self {
        match self {
            Self::Backend {
                function,
                code,
                message,
                hint,
                detail,
            } => Self::Backend {
                function,
                code,
                message: format!("{name}: {message}"),
                hint,
                detail,
            },
            other => other,
        }
    }

    /// Machine-readable kind: `sql_rejected`, `sql_backend`, `bad_response`, or — when the
    /// call never reached the query — the [`DatabaseError::kind`] of what stopped it.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Rejected(_) => "sql_rejected",
            Self::Backend { .. } => "sql_backend",
            Self::Database(e) => e.kind(),
            Self::Shape(_) => "bad_response",
        }
    }

    /// The next step, when the failure says what it is.
    ///
    /// Surface-neutral on purpose — the same text reaches every surface, so it names no
    /// CLI flag and no tool. `None` for a [`Self::Database`] failure: its remediation is
    /// [`DatabaseError::next_steps`], which needs the endpoint only the surface knows.
    #[must_use]
    pub fn tip(&self) -> Option<String> {
        match self {
            Self::Rejected(_) => Some(
                "Only one read-only SELECT or WITH statement can run on this path.".to_string(),
            ),
            Self::Backend { function, code, .. } => backend_tip(function, code.as_deref()),
            Self::Database(_) | Self::Shape(_) => None,
        }
    }

    /// Render as the JSON object every surface returns.
    #[must_use]
    pub fn to_json_value(&self) -> serde_json::Value {
        let mut out = serde_json::json!({ "error": self.to_string(), "kind": self.kind() });
        if let Self::Backend {
            code, hint, detail, ..
        } = self
        {
            for (key, value) in [("code", code), ("hint", hint), ("detail", detail)] {
                if let Some(v) = value {
                    out[key] = serde_json::Value::String(v.clone());
                }
            }
        }
        if let Some(tip) = self.tip() {
            out["tip"] = serde_json::Value::String(tip);
        }
        out
    }
}

impl fmt::Display for SqlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(r) => f.write_str(&r.message()),
            Self::Backend { message, .. } => f.write_str(message),
            Self::Database(e) => write!(f, "{e}"),
            Self::Shape(what) => write!(f, "unexpected response from the backend: {what}"),
        }
    }
}

/// The next step for a query the backend refused, keyed by its error code.
fn backend_tip(function: &str, code: Option<&str>) -> Option<String> {
    let tip = match code? {
        codes::FUNCTION_NOT_FOUND => {
            return Some(format!(
                "The backend has no `{function}` function: {INSTALL_STEP}."
            ))
        }
        codes::READ_ONLY_TRANSACTION => "The query tried to write; this path only reads.",
        codes::QUERY_CANCELED => {
            "The query ran past the 30-second limit. Filter earlier, aggregate, or add a LIMIT."
        }
        codes::UNDEFINED_TABLE => "No such table. Check the name against the database schema.",
        codes::UNDEFINED_COLUMN => "No such column. Check the name against the database schema.",
        codes::UNDEFINED_FUNCTION => {
            "No such function or operator for these argument types. A value read from the \
             params object is text or jsonb until cast, e.g. ($1->>'year')::int."
        }
        codes::INVALID_TEXT_REPRESENTATION => {
            "A value could not be converted to the type it was cast to."
        }
        codes::SYNTAX_ERROR => "Syntax error. Send a single SELECT or WITH statement.",
        _ => return None,
    };
    Some(tip.to_string())
}

/// Rows from a successful query.
#[derive(Debug, Serialize)]
pub struct SqlRows {
    /// Rows returned.
    pub count: usize,
    /// True when `max_rows` was reached, so the result may be incomplete.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub truncated: bool,
    /// The row bound the query ran with.
    pub max_rows: usize,
    /// The rows, one JSON object each.
    pub rows: serde_json::Value,
}

/// An ad-hoc read-only query.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct QuerySqlRequest {
    /// One SELECT or WITH statement.
    #[schemars(
        description = "One read-only SELECT or WITH statement. Inputs can be passed in `params` and read as $1, a JSON object: ($1->>'unitid')::int"
    )]
    pub sql: String,
    /// Values the statement reads from `$1`.
    #[schemars(
        description = "Optional JSON object bound as $1. Read fields with $1->>'name' (text) and cast: ($1->>'year')::int"
    )]
    #[serde(default)]
    pub params: Option<serde_json::Map<String, serde_json::Value>>,
    /// Row bound; defaults and caps depend on the caller.
    #[schemars(description = "Maximum rows to return (default 200, max 2000)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub max_rows: Option<usize>,
}

/// Send `sql` to the backend and return its rows as JSON, unchecked.
///
/// The one place a query engine's statement reaches `DbClient::rpc` (`db doctor` probes
/// the functions directly). With `params` it goes to the bound function; without, to the
/// plain one, so a backend that predates `query_readonly_params` still runs ad-hoc SQL.
/// Callers are responsible for the read-only pre-check — the catalog's queries are
/// checked by test instead.
///
/// # Errors
/// [`SqlError::Backend`] when the database refuses the query, [`SqlError::Database`] when
/// the call does not reach it.
pub(crate) async fn call(
    client: &DbClient,
    sql: &str,
    params: Option<&serde_json::Value>,
    max_rows: usize,
) -> Result<serde_json::Value, SqlError> {
    let q = prepare_query_text(sql);
    let (function, body) = params.map_or_else(
        || {
            (
                READONLY_FN,
                serde_json::json!({ "q": &q, "max_rows": max_rows }),
            )
        },
        |params| {
            (
                READONLY_PARAMS_FN,
                serde_json::json!({ "q": &q, "params": params, "max_rows": max_rows }),
            )
        },
    );
    client
        .rpc(function, &body)
        .await
        .map_err(|e| SqlError::from_database(e, function))
}

/// Run an ad-hoc query within `limits`.
///
/// # Errors
/// [`SqlError::Rejected`] before any round trip when the statement is not a single
/// SELECT/WITH; otherwise as `call`.
pub async fn execute_request(
    client: &DbClient,
    req: &QuerySqlRequest,
    limits: RowLimits,
) -> Result<SqlRows, SqlError> {
    if let Some(rejection) = reject_if_not_read_only(&req.sql) {
        return Err(SqlError::Rejected(rejection));
    }
    let max_rows = limits.resolve(req.max_rows);
    let params = req.params.clone().map(serde_json::Value::Object);
    let rows = call(client, &req.sql, params.as_ref(), max_rows).await?;
    rows_from(rows, max_rows)
}

/// Wrap the backend's array of rows, or say it was not one.
fn rows_from(rows: serde_json::Value, max_rows: usize) -> Result<SqlRows, SqlError> {
    let Some(count) = rows.as_array().map(Vec::len) else {
        return Err(SqlError::Shape(format!(
            "expected an array of rows, got {}",
            value_kind(&rows)
        )));
    };
    Ok(SqlRows {
        count,
        truncated: count >= max_rows,
        max_rows,
        rows,
    })
}

/// Run `sql` for `db query --sql` and return pretty JSON — rows, or an error object.
pub async fn execute_json(client: &Arc<DbClient>, sql: &str, max_rows: Option<usize>) -> String {
    let req = QuerySqlRequest {
        sql: sql.to_string(),
        params: None,
        max_rows,
    };
    match execute_request(client, &req, RowLimits::CLI).await {
        Ok(rows) => to_json_pretty(&rows),
        Err(e) => e.to_json_value().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_select_is_accepted() {
        assert_eq!(reject_if_not_read_only("SELECT * FROM programs"), None);
        assert_eq!(reject_if_not_read_only("  select 1  "), None);
        assert_eq!(reject_if_not_read_only("SELECT * FROM programs;"), None);
    }

    #[test]
    fn a_cte_is_accepted_because_analysis_queries_need_one() {
        assert_eq!(
            reject_if_not_read_only("WITH t AS (SELECT 1) SELECT * FROM t"),
            None
        );
    }

    #[test]
    fn every_write_keyword_is_refused_as_the_opening_statement() {
        for stmt in [
            "INSERT INTO programs VALUES (1)",
            "UPDATE programs SET name = 'x'",
            "DELETE FROM programs",
            "TRUNCATE programs",
            "DROP TABLE programs",
            "CREATE TABLE t (a int)",
            "ALTER TABLE programs ADD COLUMN c int",
            "GRANT ALL ON programs TO public",
            "COPY programs FROM '/tmp/x'",
            "DO $$ BEGIN END $$",
            "CALL something()",
        ] {
            assert!(
                reject_if_not_read_only(stmt).is_some(),
                "accepted a write: {stmt}"
            );
        }
    }

    #[test]
    fn a_write_smuggled_after_a_semicolon_is_refused() {
        // The engine would refuse it too, but only after a round trip, and the message
        // would name the wrong culprit.
        let r = reject_if_not_read_only("SELECT 1; DELETE FROM programs");
        assert_eq!(r, Some(SqlRejection::MultipleStatements));
    }

    #[test]
    fn a_write_hidden_in_a_subquery_is_still_caught() {
        assert_eq!(
            reject_if_not_read_only("SELECT * FROM (DELETE FROM programs RETURNING *) x"),
            Some(SqlRejection::WriteKeyword("delete".to_string()))
        );
    }

    #[test]
    fn a_column_name_containing_a_keyword_is_not_a_write() {
        // `updated_year` and `created_at` are real columns; a substring match would make
        // the most ordinary queries in this schema unrunnable.
        assert_eq!(
            reject_if_not_read_only("SELECT updated_year, created_at FROM institutions"),
            None
        );
        assert_eq!(
            reject_if_not_read_only("SELECT delete_flag FROM t"),
            None,
            "delete_flag is one word, not the DELETE keyword"
        );
    }

    #[test]
    fn a_keyword_inside_a_string_literal_is_not_a_write() {
        assert_eq!(
            reject_if_not_read_only("SELECT * FROM programs WHERE name = 'delete me'"),
            None
        );
        assert_eq!(
            reject_if_not_read_only(r#"SELECT "drop" FROM t"#),
            None,
            "a quoted identifier is not a keyword"
        );
    }

    #[test]
    fn a_keyword_inside_a_comment_is_not_a_write() {
        assert_eq!(
            reject_if_not_read_only("SELECT 1 -- delete this later\n"),
            None
        );
        assert_eq!(reject_if_not_read_only("SELECT 1 /* drop me */"), None);
        assert_eq!(
            reject_if_not_read_only("-- drop table t\nSELECT 1"),
            None,
            "a leading comment must not decide the first keyword"
        );
    }

    #[test]
    fn an_empty_or_comment_only_file_says_so_rather_than_being_sent() {
        assert_eq!(reject_if_not_read_only(""), Some(SqlRejection::Empty));
        assert_eq!(
            reject_if_not_read_only("   \n\t "),
            Some(SqlRejection::Empty)
        );
        assert_eq!(
            reject_if_not_read_only("-- just a note\n"),
            Some(SqlRejection::Empty)
        );
        assert_eq!(reject_if_not_read_only(";"), Some(SqlRejection::Empty));
    }

    #[test]
    fn a_trailing_semicolon_and_whitespace_do_not_look_like_two_statements() {
        assert_eq!(reject_if_not_read_only("SELECT 1 ;  \n"), None);
    }

    #[test]
    fn rejection_messages_name_the_offending_token() {
        assert!(SqlRejection::WriteKeyword("delete".to_string())
            .message()
            .contains("DELETE"));
        assert!(SqlRejection::NotASelect("explain".to_string())
            .message()
            .contains("explain"));
        assert!(SqlRejection::Empty.message().contains("empty"));
        assert!(SqlRejection::MultipleStatements
            .message()
            .contains("one statement"));
    }

    #[test]
    fn prepare_query_text_ends_every_query_with_a_newline_so_a_comment_cannot_eat_the_wrapper() {
        // The backend wraps the text as `(… q …)`; without the newline, a query ending in
        // a line comment comments out the closing parenthesis.
        for (sql, want) in [
            ("SELECT 1 -- note", "SELECT 1 -- note\n"),
            ("SELECT 1", "SELECT 1\n"),
            ("SELECT 1\n", "SELECT 1\n\n"),
        ] {
            assert_eq!(prepare_query_text(sql), want, "{sql:?}");
        }
    }

    #[test]
    fn prepare_query_text_drops_only_a_statement_ending_semicolon() {
        for (sql, want) in [
            ("SELECT 1;", "SELECT 1\n"),
            ("SELECT 1 ;  ", "SELECT 1   \n"),
            // The semicolon is the last *code*, even with a comment after it.
            ("SELECT 1; -- note", "SELECT 1 -- note\n"),
            ("SELECT 1 /* ; */", "SELECT 1 /* ; */\n"),
            // Inside a literal it is data, and a literal ends the statement's code.
            ("SELECT ';'", "SELECT ';'\n"),
            ("SELECT $$;$$", "SELECT $$;$$\n"),
            // A literal resets the search; the `;` after it has to set it again.
            ("SELECT 'a';", "SELECT 'a'\n"),
            ("SELECT 1; /* c */", "SELECT 1 /* c */\n"),
        ] {
            assert_eq!(prepare_query_text(sql), want, "{sql:?}");
        }
    }

    #[test]
    fn lex_keeps_every_byte_and_tags_each_kind_of_span() {
        let sql =
            "SELECT 'a''b', E'c\\'d', \"x\"\"y\", $$e$$, $t$f$t$, $1 -- g\n/* h /* i */ j */ k";
        let spans = lex(sql);
        let rebuilt: String = spans.iter().map(|(_, t)| *t).collect();
        assert_eq!(rebuilt, sql, "spans must cover the text exactly");
        let literals: Vec<&str> = spans
            .iter()
            .filter(|(k, _)| *k == Span::Literal)
            .map(|(_, t)| *t)
            .collect();
        assert_eq!(
            literals,
            ["'a''b'", "'c\\'d'", "\"x\"\"y\"", "$$e$$", "$t$f$t$"]
        );
        let comments: Vec<&str> = spans
            .iter()
            .filter(|(k, _)| *k == Span::Comment)
            .map(|(_, t)| *t)
            .collect();
        assert_eq!(
            comments,
            ["-- g", "/* h /* i */ j */"],
            "block comments nest"
        );
    }

    #[test]
    fn a_bound_parameter_is_code_not_a_dollar_quote() {
        // `$1` must stay code — the catalog's queries are made of it, and swallowing the
        // rest of the text as a literal would hide every keyword after it.
        assert_eq!(
            strip_noise("SELECT ($1->>'unitid')::int, $2"),
            "SELECT ($1->> )::int, $2"
        );
        assert_eq!(
            reject_if_not_read_only("SELECT ($1->>'unitid')::int FROM institutions"),
            None
        );
    }

    #[test]
    fn a_keyword_inside_a_dollar_quote_or_escaped_string_is_not_a_write() {
        assert_eq!(reject_if_not_read_only("SELECT $$delete$$"), None);
        assert_eq!(reject_if_not_read_only("SELECT $x$drop table$x$"), None);
        // An escaped quote does not end an E'' string early and expose `delete`.
        assert_eq!(reject_if_not_read_only("SELECT E'it\\'s; delete'"), None);
        // But a name ending in E is not an escape prefix: in `typE'a\'` the backslash is
        // data, so the literal ends there and `'delete'` is a literal too. Lexing it as an
        // E-string would run the literal on and expose `delete` as code.
        assert_eq!(
            reject_if_not_read_only("SELECT typE'a\\' AS x, 'delete'"),
            None
        );
    }

    #[test]
    fn row_limits_apply_the_callers_default_and_cap() {
        assert_eq!(RowLimits::CLI.resolve(None), DEFAULT_MAX_ROWS);
        assert_eq!(RowLimits::AGENT.resolve(None), AGENT_DEFAULT_MAX_ROWS);
        assert_eq!(RowLimits::AGENT.resolve(Some(50_000)), AGENT_MAX_ROWS);
        assert_eq!(RowLimits::CLI.resolve(Some(50_000)), MAX_MAX_ROWS);
        assert_eq!(RowLimits::AGENT.resolve(Some(0)), 1, "never zero rows");
    }

    /// The body `PostgREST` sends for a failed query, as `DbClient` carries it.
    fn backend_failure(code: &str, message: &str) -> DatabaseError {
        DatabaseError::QueryError(format!(
            "PostgREST error (400): {}",
            serde_json::json!({ "code": code, "message": message, "details": "d", "hint": null })
        ))
    }

    #[test]
    fn a_backend_refusal_keeps_its_code_message_and_detail() {
        let e = SqlError::from_database(
            backend_failure("42P01", "relation \"x\" does not exist"),
            READONLY_FN,
        );
        let SqlError::Backend {
            code,
            message,
            hint,
            detail,
            ..
        } = &e
        else {
            panic!("expected Backend, got {e:?}");
        };
        assert_eq!(code.as_deref(), Some("42P01"));
        assert_eq!(message, "relation \"x\" does not exist");
        assert_eq!(hint, &None);
        assert_eq!(detail.as_deref(), Some("d"));
        let json = e.to_json_value();
        assert_eq!(json["kind"], "sql_backend");
        assert_eq!(json["code"], "42P01");
        assert!(json["tip"].as_str().unwrap().contains("No such table"));
    }

    #[test]
    fn every_mapped_code_has_a_tip_and_an_unknown_one_asserts_nothing() {
        for code in [
            codes::READ_ONLY_TRANSACTION,
            codes::QUERY_CANCELED,
            codes::UNDEFINED_TABLE,
            codes::UNDEFINED_COLUMN,
            codes::UNDEFINED_FUNCTION,
            codes::INVALID_TEXT_REPRESENTATION,
            codes::SYNTAX_ERROR,
            codes::FUNCTION_NOT_FOUND,
        ] {
            assert!(
                backend_tip(READONLY_FN, Some(code)).is_some(),
                "{code} has no tip"
            );
        }
        assert_eq!(backend_tip(READONLY_FN, Some("XX000")), None);
        assert_eq!(backend_tip(READONLY_FN, None), None);
    }

    #[test]
    fn a_missing_function_is_named_in_the_tip() {
        let e = SqlError::from_database(
            backend_failure("PGRST202", "Could not find the function"),
            READONLY_PARAMS_FN,
        );
        let tip = e.tip().expect("PGRST202 has a tip");
        assert!(tip.contains(READONLY_PARAMS_FN), "{tip}");
        assert!(tip.contains("step 4c"), "{tip}");
    }

    #[test]
    fn a_failure_that_never_reached_the_query_stays_a_database_error() {
        // No tip of its own: `DatabaseError::next_steps` owns that text and needs the
        // endpoint, which only the surface knows.
        let e = SqlError::from_database(
            DatabaseError::ConnectionError("timed out".to_string()),
            READONLY_FN,
        );
        assert!(matches!(e, SqlError::Database(_)), "{e:?}");
        assert_eq!(e.tip(), None);
        assert_eq!(
            e.kind(),
            "unreachable",
            "the database layer's own kind, not a vaguer one"
        );
        // A QueryError whose body is not PostgREST's object is not reinterpreted.
        let e = SqlError::from_database(
            DatabaseError::QueryError("PostgREST error (502): bad gateway".to_string()),
            READONLY_FN,
        );
        assert!(matches!(e, SqlError::Database(_)), "{e:?}");
    }

    #[test]
    fn a_proxy_json_body_is_not_mistaken_for_the_database_refusing_the_query() {
        // Only PostgREST's object carries `code`; a gateway's `{"message": ...}` must not
        // be reported as "the database refused the query".
        let e = SqlError::from_database(
            DatabaseError::QueryError(
                r#"PostgREST error (502): {"message":"An invalid response was received"}"#
                    .to_string(),
            ),
            READONLY_FN,
        );
        assert!(matches!(e, SqlError::Database(_)), "{e:?}");
    }

    #[test]
    fn a_body_without_a_message_keeps_the_raw_text_and_every_kind_renders() {
        let raw = r#"PostgREST error (404): {"code":"PGRST202"}"#;
        let e = SqlError::from_database(DatabaseError::QueryError(raw.into()), READONLY_PARAMS_FN);
        assert!(
            matches!(&e, SqlError::Backend { code: Some(c), message, .. }
                if c == codes::FUNCTION_NOT_FOUND && message == raw),
            "{e:?}"
        );
        let db = SqlError::Database(DatabaseError::ConnectionError("down".into())).to_json_value();
        assert_eq!(db["kind"], "unreachable");
        assert!(db.get("tip").is_none() && db.get("code").is_none(), "{db}");
        let shape = SqlError::Shape("got null".into()).to_json_value();
        assert_eq!(shape["kind"], "bad_response");
        assert!(shape["error"].as_str().unwrap().contains("got null"));
    }

    #[test]
    fn a_catalog_failure_names_its_query() {
        let e = SqlError::from_database(
            backend_failure(codes::UNDEFINED_COLUMN, "no x"),
            READONLY_PARAMS_FN,
        )
        .in_query("institutions_with_programs");
        assert_eq!(e.to_string(), "institutions_with_programs: no x");
        let untouched = SqlError::Shape("s".into()).in_query("q");
        assert!(!untouched.to_string().starts_with("q:"), "{untouched}");
    }

    #[test]
    fn lex_is_byte_safe_on_multibyte_and_unterminated_text() {
        for sql in [
            "SELECT 'café' -- naïve\nFROM \"é\"",
            "SELECT E'\\é' /* ü */",
            "SELECT 'café",
            "SELECT /* ü",
            "SELECT $t$ü",
            "SELECT $é$x$é$",
            "SELECT E'\\",
            "SELECT \"ü",
            "SELECT 1 $",
            "SELECT $",
            "SELECT -- é",
            "SELECT 1 -- c\r\nFROM t",
        ] {
            let rebuilt: String = lex(sql).iter().map(|(_, t)| *t).collect();
            assert_eq!(rebuilt, sql, "{sql:?}");
            let _ = (prepare_query_text(sql), reject_if_not_read_only(sql));
        }
        assert_eq!(
            lex("SELECT 'a; delete").last(),
            Some(&(Span::Literal, "'a; delete"))
        );
        // A non-ASCII dollar tag is a dollar quote, as in Postgres.
        assert_eq!(reject_if_not_read_only("SELECT $é$delete; drop$é$"), None);
        // Fails if a plain '…' string ever honours backslash escapes and swallows the rest.
        assert_eq!(
            reject_if_not_read_only("SELECT 'a\\'; DELETE FROM t"),
            Some(SqlRejection::MultipleStatements)
        );
    }

    #[test]
    fn the_agent_row_limits_in_the_request_description_match_the_constants() {
        // A const cannot go in an attribute, so the description spells the numbers out;
        // this keeps them honest.
        let schema =
            serde_json::to_string(&schemars::schema_for!(QuerySqlRequest)).expect("schema");
        for n in [AGENT_DEFAULT_MAX_ROWS, AGENT_MAX_ROWS] {
            assert!(
                schema.contains(&n.to_string()),
                "description lost {n}: {schema}"
            );
        }
    }

    #[test]
    fn a_rejection_renders_without_a_round_trip_and_names_no_surface() {
        let json = SqlError::Rejected(SqlRejection::MultipleStatements).to_json_value();
        assert_eq!(json["kind"], "sql_rejected");
        let tip = json["tip"].as_str().unwrap();
        assert!(
            !tip.contains("--") && !tip.contains("db "),
            "surface-specific tip: {tip}"
        );
    }

    #[test]
    fn rows_that_are_not_an_array_are_a_shape_error_not_an_empty_result() {
        let err = rows_from(serde_json::json!({"x": 1}), 10).expect_err("an object is not rows");
        assert!(matches!(err, SqlError::Shape(_)));
        let rows = rows_from(serde_json::json!([{"a": 1}, {"a": 2}]), 2).expect("rows");
        assert_eq!(rows.count, 2);
        assert!(rows.truncated, "count reaching max_rows may be incomplete");
    }

    #[test]
    fn the_shipped_example_queries_pass_the_check_and_the_refusal_example_is_refused() {
        // docs/database/examples/ is what a new user runs first; an example the client
        // refuses, or a refusal demo it lets through, would teach the wrong thing.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("docs/database/examples");
        let mut seen = 0;
        for entry in std::fs::read_dir(&dir).expect("examples directory exists") {
            let path = entry.expect("dir entry").path();
            if path.extension().and_then(|e| e.to_str()) != Some("sql") {
                continue;
            }
            seen += 1;
            let sql = std::fs::read_to_string(&path).expect("example reads");
            let name = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default();
            let verdict = reject_if_not_read_only(&sql);
            if name == "refused-write.sql" {
                assert!(verdict.is_some(), "{name} must be refused");
            } else {
                assert_eq!(verdict, None, "{name} is refused");
            }
        }
        assert!(seen >= 2, "expected the shipped examples, found {seen}");
    }

    #[test]
    fn strip_noise_leaves_a_separator_so_tokens_do_not_fuse() {
        // 'x''y' collapsing to nothing would join neighbours into a new word.
        let out = strip_noise("select a/* c */b from t");
        assert!(!out.contains("ab"), "tokens fused: {out}");
    }
}
