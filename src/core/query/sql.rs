//! Read-only SQL, run through the `query_readonly` Postgres function.
//!
//! `db exec-sql` reaches the Supabase Management API and so is cloud-only — a self-hosted
//! stack has no project ref. `PostgREST` exposes no SQL endpoint either, so the read path
//! goes through a function called over `/rest/v1/rpc/`, which behaves identically on both
//! deployments.
//!
//! **The refusal that matters is the engine's, not this module's.** Because
//! `query_readonly` is declared `STABLE`, `PostgREST` runs the call in a READ ONLY
//! transaction and Postgres refuses every write — verified against the live deployment,
//! where `SELECT nextval(...)` comes back `25006` and the sequence does not move. The
//! function's own wrapping is a second layer: `q` lands in a scalar subquery, so bare DML
//! is a syntax error and a data-modifying CTE is refused "must be at the top level".
//!
//! Note the mechanism is the transaction, not SPI's `read_only` flag — that flag rejects
//! non-SELECT commands but does not stop a volatile function called inside a SELECT from
//! writing, which is why the same call over psql succeeds.
//!
//! The check in this file turns all of that into a message naming the offending keyword
//! before a round trip. It is a courtesy, not the guarantee, and is deliberately a filter
//! on obvious writes rather than an attempt at a complete SQL parser.

use std::sync::Arc;

use crate::core::database::{DatabaseError, DbClient};
use crate::core::json::{error_json, to_json_pretty};
use serde::Serialize;

/// Name of the backend function. Defined in `docs/database/programs-schema.sql`.
const READONLY_FN: &str = "query_readonly";

/// Rows the backend returns when the caller does not say.
///
/// The result is a single `jsonb` value, so `PGRST_DB_MAX_ROWS` does not cap it — without
/// a bound a broad query returns one very large payload instead of being truncated.
pub const DEFAULT_MAX_ROWS: usize = 1_000;

/// Upper bound on `--max-rows`, to keep one request from pulling the corpus.
pub const MAX_MAX_ROWS: usize = 10_000;

/// Statements a read-only path must never be asked to run.
///
/// Matched as whole words against the statement with strings and comments stripped. The
/// engine refuses these anyway; naming the keyword is friendlier than relaying
/// `"UPDATE is not allowed in a non-volatile function"` from a failed round trip.
const WRITE_KEYWORDS: [&str; 16] = [
    "insert", "update", "delete", "truncate", "drop", "create", "alter", "grant", "revoke",
    "comment", "vacuum", "reindex", "refresh", "call", "do", "copy",
];

/// Why a SQL file was refused before being sent.
#[derive(Debug, PartialEq, Eq)]
pub enum SqlRejection {
    /// The file held no statement.
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
            Self::Empty => "the SQL file is empty".to_string(),
            Self::NotASelect(word) => {
                format!("a read-only query must start with SELECT or WITH, found \"{word}\"")
            }
            Self::WriteKeyword(word) => {
                format!("\"{}\" writes; this path is read-only", word.to_uppercase())
            }
            Self::MultipleStatements => {
                "pass one statement per file — a second statement after a semicolon would \
                 run too"
                    .to_string()
            }
        }
    }
}

/// Strip `--` line comments, `/* */` block comments, and the contents of string literals.
///
/// Keyword matching runs over the result, so a column named `"update_note"` or a literal
/// `'please delete later'` cannot trip the check. Literals collapse to `''` rather than
/// vanish so adjacent tokens do not fuse into a new word.
fn strip_noise(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '-' if chars.peek() == Some(&'-') => {
                for c in chars.by_ref() {
                    if c == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut prev = '\0';
                for c in chars.by_ref() {
                    if prev == '*' && c == '/' {
                        break;
                    }
                    prev = c;
                }
                out.push(' ');
            }
            '\'' | '"' => {
                // Quoted text is data or an identifier, not a keyword. Doubling the quote
                // is how SQL escapes it, which this consumes as an immediate close
                // followed by a fresh open — the net effect is the same.
                for q in chars.by_ref() {
                    if q == c {
                        break;
                    }
                }
                out.push(' ');
            }
            _ => out.push(c),
        }
    }
    out
}

/// Refuse a statement that is obviously not a read-only query.
///
/// Returns `None` when the statement looks like a single `SELECT`/`WITH`. The engine is
/// what actually enforces read-only-ness — see the module docs.
///
/// # Errors
/// Never returns `Err`; the `Option` carries the rejection.
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

/// Response for `db query --sql`.
#[derive(Debug, Serialize)]
struct SqlResponse {
    count: usize,
    /// True when `max_rows` was reached, so the result may be incomplete.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    truncated: bool,
    max_rows: usize,
    rows: serde_json::Value,
}

/// Run `sql` through the backend's read-only function and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, sql: &str, max_rows: Option<usize>) -> String {
    if let Some(rejection) = reject_if_not_read_only(sql) {
        return serde_json::json!({
            "error": rejection.message(),
            "tip": "db query --sql runs one read-only SELECT. Use `db exec-sql` for writes \
                    and DDL (Supabase cloud only).",
        })
        .to_string();
    }

    let max_rows = max_rows.unwrap_or(DEFAULT_MAX_ROWS).clamp(1, MAX_MAX_ROWS);
    let params = serde_json::json!({ "q": sql.trim().trim_end_matches(';'), "max_rows": max_rows });

    let rows = match client.rpc(READONLY_FN, &params).await {
        Ok(v) => v,
        Err(e) => return sql_error_json(e),
    };

    let count = rows.as_array().map_or(0, Vec::len);
    to_json_pretty(&SqlResponse {
        count,
        truncated: count >= max_rows,
        max_rows,
        rows,
    })
}

/// Render a backend failure, naming the missing function when that is the cause.
///
/// A deployment that predates `query_readonly` answers with `PostgREST`'s `PGRST202`
/// ("could not find the function"), which on its own reads like a client bug. Say what to
/// install instead.
fn sql_error_json(e: DatabaseError) -> String {
    let text = e.to_string();
    if text.contains("PGRST202") || text.contains(READONLY_FN) {
        return serde_json::json!({
            "error": format!("the backend has no `{READONLY_FN}` function: {text}"),
            "tip": "Apply docs/database/programs-schema.sql (or `db bootstrap --print`) to \
                    install it, then retry.",
        })
        .to_string();
    }
    error_json(e)
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
    fn strip_noise_leaves_a_separator_so_tokens_do_not_fuse() {
        // 'x''y' collapsing to nothing would join neighbours into a new word.
        let out = strip_noise("select a/* c */b from t");
        assert!(!out.contains("ab"), "tokens fused: {out}");
    }
}
