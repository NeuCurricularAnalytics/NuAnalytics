//! Curated read-only queries, compiled in from `catalog/*.sql`.
//!
//! The layer between the query engines and the database: an engine never writes SQL, it
//! names a [`NamedQuery`] here and passes a typed params struct. The query's text is the
//! `.sql` file, verbatim — `include_str!`, not assembled — and its inputs travel
//! separately as a JSON object that `query_readonly_params` binds to `$1`. So no value is
//! ever spliced into SQL, and what runs is exactly what the tests below read.
//!
//! **Writing a query.**
//! - The first CTE, `arg`, is the only place `$1` appears: it reads every input once and
//!   casts it — `($1->>'unitid')::int AS unitid`. The rest of the query reads `arg`.
//! - An optional filter is `(arg.x IS NULL OR col = arg.x)`, so a params struct sends
//!   every key and a `null` means "not filtered".
//! - A query returns **one row**, an envelope of scalars and `jsonb_agg` arrays. The
//!   backend's `max_rows` bounds only that row, not the arrays inside it, so an array that
//!   grows with the data carries its own `LIMIT`.
//! - Register it in [`CATALOG`] with a [`ParamSpec`] per key. The tests fail until the
//!   file, the spec and the engine's params struct agree on every key and its cast.

use serde::de::DeserializeOwned;
use serde::Serialize;

use super::sql::{self, SqlError};
use crate::core::database::DbClient;

/// How a query casts one of its inputs. Checked against the file by test.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParamKind {
    /// Read as `($1->>'k')::int`.
    Int,
    /// Read as `$1->>'k'`.
    Text,
    /// Read as `($1->>'k')::boolean`.
    Bool,
    /// Read as a JSON array of strings, `$1->'k'`.
    TextArray,
}

/// One input a query reads from `$1`.
#[derive(Debug)]
pub struct ParamSpec {
    /// Key in the params object.
    pub name: &'static str,
    /// How the query casts it.
    pub kind: ParamKind,
    /// What it filters or sets; `null` always means "not filtered".
    pub doc: &'static str,
}

/// A curated query and the inputs it takes.
#[derive(Debug)]
pub struct NamedQuery {
    /// Stable name; the file is `catalog/{name}.sql`.
    pub name: &'static str,
    /// One line on what it answers.
    pub purpose: &'static str,
    /// The file's text, compiled in.
    pub sql: &'static str,
    /// Every key the query reads from `$1`.
    pub params: &'static [ParamSpec],
}

/// Schools holding stored programs, with those programs.
pub const INSTITUTIONS_WITH_PROGRAMS: NamedQuery = NamedQuery {
    name: "institutions_with_programs",
    purpose: "institutions that hold stored degree programs, each with its programs",
    sql: include_str!("catalog/institutions_with_programs.sql"),
    params: &[
        ParamSpec {
            name: "name",
            kind: ParamKind::Text,
            doc: "institution name substring, case-insensitive; `*`, `%` and `_` are wildcards",
        },
        ParamSpec {
            name: "state",
            kind: ParamKind::Text,
            doc: "two-letter state code",
        },
        ParamSpec {
            name: "carnegie_class",
            kind: ParamKind::Int,
            doc: "Carnegie classification code",
        },
        ParamSpec {
            name: "control",
            kind: ParamKind::Int,
            doc: "control code: 1 public, 2 private non-profit, 3 for-profit",
        },
        ParamSpec {
            name: "hbcu",
            kind: ParamKind::Bool,
            doc: "historically Black colleges and universities",
        },
        ParamSpec {
            name: "tribal",
            kind: ParamKind::Bool,
            doc: "tribal colleges",
        },
        ParamSpec {
            name: "inst_size_min",
            kind: ParamKind::Int,
            doc: "minimum institution size bucket",
        },
        ParamSpec {
            name: "limit",
            kind: ParamKind::Int,
            doc: "schools to return",
        },
    ],
};

/// Shorthand for the completions queries' long parameter lists.
const fn param(name: &'static str, kind: ParamKind, doc: &'static str) -> ParamSpec {
    ParamSpec { name, kind, doc }
}

const UNITID: ParamSpec = param("unitid", ParamKind::Int, "one institution");
const CARNEGIE_CLASS: ParamSpec = param(
    "carnegie_class",
    ParamKind::Int,
    "Carnegie classification code",
);
const CONTROL: ParamSpec = param("control", ParamKind::Int, "control code");
const STATE: ParamSpec = param("state", ParamKind::Text, "two-letter state code");
const HBCU: ParamSpec = param("hbcu", ParamKind::Bool, "HBCUs only, or none of them");
const TRIBAL: ParamSpec = param(
    "tribal",
    ParamKind::Bool,
    "tribal colleges only, or none of them",
);
const INST_SIZE_MIN: ParamSpec = param("inst_size_min", ParamKind::Int, "minimum size bucket");
const CIP_PREFIX: ParamSpec = param("cip_prefix", ParamKind::Text, "CIP code prefix, e.g. `11.`");
const CIP_CODES: ParamSpec = param(
    "cip_codes",
    ParamKind::TextArray,
    "exact CIP codes; the engine sends these or `cip_prefix`, never both",
);
const AWARD_LEVEL: ParamSpec = param("award_level", ParamKind::Int, "IPEDS award level");
const MAJOR_NUM: ParamSpec = param(
    "major_num",
    ParamKind::Int,
    "1 first major, 2 second; null counts both",
);
const YEAR: ParamSpec = param(
    "year",
    ParamKind::Int,
    "reporting year; null picks the latest with data",
);
const WITH_BASELINE: ParamSpec = param(
    "with_baseline",
    ParamKind::Bool,
    "also sum every CIP, for representation ratios",
);

/// Demographics aggregated over every matched institution.
pub const COMPLETIONS_TOTAL: NamedQuery = NamedQuery {
    name: "completions_total",
    purpose: "completion demographics aggregated over the matched institutions",
    sql: include_str!("catalog/completions_total.sql"),
    params: &[
        UNITID,
        CARNEGIE_CLASS,
        CONTROL,
        STATE,
        HBCU,
        TRIBAL,
        INST_SIZE_MIN,
        CIP_PREFIX,
        CIP_CODES,
        AWARD_LEVEL,
        MAJOR_NUM,
        YEAR,
        WITH_BASELINE,
    ],
};

/// Demographics per matched institution, ranked by completions.
pub const COMPLETIONS_BY_SCHOOL: NamedQuery = NamedQuery {
    name: "completions_by_school",
    purpose: "completion demographics per institution, ranked by completions",
    sql: include_str!("catalog/completions_by_school.sql"),
    params: &[
        UNITID,
        CARNEGIE_CLASS,
        CONTROL,
        STATE,
        HBCU,
        TRIBAL,
        INST_SIZE_MIN,
        CIP_PREFIX,
        CIP_CODES,
        AWARD_LEVEL,
        MAJOR_NUM,
        YEAR,
        WITH_BASELINE,
        param(
            "min_completions",
            ParamKind::Int,
            "skip schools with fewer selected completions",
        ),
        param("limit", ParamKind::Int, "schools to return"),
    ],
};

/// Demographics per CIP code at one institution.
pub const COMPLETIONS_BY_CIP: NamedQuery = NamedQuery {
    name: "completions_by_cip",
    purpose: "completion demographics per CIP code at one institution",
    sql: include_str!("catalog/completions_by_cip.sql"),
    params: &[
        UNITID,
        CIP_PREFIX,
        CIP_CODES,
        AWARD_LEVEL,
        MAJOR_NUM,
        YEAR,
        WITH_BASELINE,
    ],
};

/// Every curated query. A `.sql` file in `catalog/` that is not listed here fails a test.
pub const CATALOG: &[&NamedQuery] = &[
    &INSTITUTIONS_WITH_PROGRAMS,
    &COMPLETIONS_TOTAL,
    &COMPLETIONS_BY_SCHOOL,
    &COMPLETIONS_BY_CIP,
];

/// Run a query that returns one envelope row, and decode it.
///
/// # Errors
/// As [`sql::call`], plus [`SqlError::Shape`] when the row is missing or does not decode
/// into `R` — the query and its Rust type disagree.
pub(crate) async fn run_one<P: Serialize + Sync, R: DeserializeOwned>(
    client: &DbClient,
    query: &NamedQuery,
    params: &P,
) -> Result<R, SqlError> {
    let params = serde_json::to_value(params)
        .map_err(|e| SqlError::Shape(format!("{}: params did not encode: {e}", query.name)))?;
    let rows = sql::call(client, query.sql, Some(&params), 1)
        .await
        .map_err(|e| e.in_query(query.name))?;
    let row = rows
        .as_array()
        .and_then(|rows| rows.first())
        .cloned()
        .ok_or_else(|| SqlError::Shape(format!("{} returned no row", query.name)))?;
    serde_json::from_value(row).map_err(|e| SqlError::Shape(format!("{}: {e}", query.name)))
}

/// Keys `sql` reads from `$1` with the cast each gets, and anything else that looks like
/// a bound parameter.
///
/// Comments are skipped, so prose may mention `$1->>'x'`. The cast is read from what
/// follows: `$1->'k'` is a JSON value ([`ParamKind::TextArray`]); `($1->>'k')::int` and
/// `::boolean` are [`ParamKind::Int`] and [`ParamKind::Bool`]; a bare `$1->>'k'` is
/// [`ParamKind::Text`]. `strays` holds each `$n` that is not `$1->…` — a `$2` the backend
/// would not bind, or a bare `$1` read as a whole object.
#[cfg(test)]
pub(crate) fn referenced_params(
    sql: &str,
) -> (std::collections::BTreeMap<String, ParamKind>, Vec<String>) {
    let text: String = sql::lex(sql)
        .into_iter()
        .filter(|(kind, _)| *kind != sql::Span::Comment)
        .map(|(_, t)| t)
        .collect();
    let mut keys = std::collections::BTreeMap::new();
    let mut strays = Vec::new();
    for (at, _) in text.match_indices('$') {
        let rest = &text[at + 1..];
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        if digits.is_empty() {
            continue;
        }
        let after = &rest[digits.len()..];
        let read = after
            .strip_prefix("->>'")
            .map(|k| (k, false))
            .or_else(|| after.strip_prefix("->'").map(|k| (k, true)));
        let key =
            read.and_then(|(k, json)| k.split_once('\'').map(|(key, tail)| (key, tail, json)));
        match (digits.as_str(), key) {
            ("1", Some((key, tail, json))) => {
                let kind = if json {
                    ParamKind::TextArray
                } else if tail.starts_with(")::int") {
                    ParamKind::Int
                } else if tail.starts_with(")::boolean") {
                    ParamKind::Bool
                } else {
                    ParamKind::Text
                };
                keys.insert(key.to_string(), kind);
            }
            _ => strays.push(format!(
                "${digits}{}",
                after.chars().take(8).collect::<String>()
            )),
        }
    }
    (keys, strays)
}

/// Assert a params struct sends exactly the keys `query` declares.
///
/// The third side of the check: the catalog tests tie the file to its [`ParamSpec`]s,
/// and each engine calls this to tie its struct to them. A key the struct omits would
/// arrive as a silent `null` — "not filtered".
#[cfg(test)]
pub(crate) fn assert_params_match<P: Serialize>(query: &NamedQuery, params: &P) {
    let value = serde_json::to_value(params).expect("params encode");
    let sent: std::collections::BTreeSet<&str> = value
        .as_object()
        .expect("params encode as a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    let declared: std::collections::BTreeSet<&str> = query.params.iter().map(|p| p.name).collect();
    assert_eq!(sent, declared, "{}: params struct vs ParamSpec", query.name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn every_sql_file_in_the_catalog_directory_is_registered_and_vice_versa() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/core/query/catalog");
        let on_disk: BTreeSet<String> = std::fs::read_dir(&dir)
            .expect("catalog directory exists")
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
            .filter_map(|path| path.file_name()?.to_str().map(str::to_string))
            .collect();
        let registered: BTreeSet<String> =
            CATALOG.iter().map(|q| format!("{}.sql", q.name)).collect();
        assert_eq!(on_disk, registered, "catalog/*.sql vs CATALOG");
    }

    #[test]
    fn every_catalog_query_passes_the_read_only_check() {
        for query in CATALOG {
            assert_eq!(
                sql::reject_if_not_read_only(query.sql),
                None,
                "{} would be refused",
                query.name
            );
        }
    }

    #[test]
    fn every_catalog_query_reads_exactly_its_declared_params_and_nothing_else() {
        for query in CATALOG {
            let (read, strays) = referenced_params(query.sql);
            let declared: std::collections::BTreeMap<String, ParamKind> = query
                .params
                .iter()
                .map(|p| (p.name.to_string(), p.kind))
                .collect();
            assert_eq!(
                read, declared,
                "{}: keys and casts read vs ParamSpec",
                query.name
            );
            assert!(
                strays.is_empty(),
                "{}: unbound parameters {strays:?}",
                query.name
            );
            let unique: BTreeSet<&str> = query.params.iter().map(|p| p.name).collect();
            assert_eq!(
                unique.len(),
                query.params.len(),
                "{}: duplicate ParamSpec",
                query.name
            );
        }
    }

    #[test]
    fn catalog_names_are_unique() {
        let names: BTreeSet<&str> = CATALOG.iter().map(|q| q.name).collect();
        assert_eq!(names.len(), CATALOG.len());
    }

    #[test]
    fn referenced_params_finds_keys_with_their_casts_and_flags_anything_unbound() {
        let (read, strays) = referenced_params(
            "WITH arg AS (SELECT $1->>'a' AS a, ($1->>'n')::int AS n, ($1->>'b')::boolean AS b, \
             $1->'xs' AS xs) -- $1->>'prose'\nSELECT $2, $1",
        );
        assert_eq!(
            read,
            std::collections::BTreeMap::from([
                ("a".to_string(), ParamKind::Text),
                ("b".to_string(), ParamKind::Bool),
                ("n".to_string(), ParamKind::Int),
                ("xs".to_string(), ParamKind::TextArray),
            ])
        );
        assert_eq!(strays.len(), 2, "{strays:?}");
        assert!(strays[0].starts_with("$2"));
        assert!(strays[1].starts_with("$1"));
    }
}
