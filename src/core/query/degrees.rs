//! Stored degree programs: `search_degrees`, `get_degree`, and resolving a reference.
//!
//! Everything here reads the normalized `programs` table `db import` writes; its
//! `document` column is the lossless unified-JSON degree.

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{error_code, parse_first, parse_json_array, to_json_pretty};
use serde::{Deserialize, Serialize};

/// The `programs` columns every program record carries, as a literal both projections
/// are built from.
macro_rules! program_summary_cols {
    () => {
        "program_key,name,unitid,cip_code,catalog_year,degree_type,program_kind,discipline,verified,institution_resolved,has_impossible_requirements"
    };
}

/// Lightweight projection for `search_degrees` results.
const PROGRAM_SUMMARY_COLS: &str = program_summary_cols!();
/// Full projection for `get_degree` — the summary fields plus provenance and
/// the lossless `document` (unified-JSON degree) for downstream analysis.
const PROGRAM_DETAIL_COLS: &str = concat!(
    program_summary_cols!(),
    ",degree_id,institution_raw,total_credits,source_url,document"
);

// ============================================================================
// Request types
// ============================================================================

/// Request parameters for `search_degrees`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchDegreesRequest {
    /// Words from the program's name, matched anywhere in it, case-insensitively.
    #[schemars(
        description = "Words in the program name, e.g. \"computer science\" (case-insensitive substring)"
    )]
    pub name: Option<String>,
    /// IPEDS UNITID of the institution
    #[schemars(description = "IPEDS UNITID of the institution")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// CIP code prefix to filter by (e.g. `\"11.\"` for all CS, `\"11.01.\"` for one family)
    #[schemars(description = "CIP code prefix (e.g. \"11.\" for computer science)")]
    pub cip_prefix: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
    /// Normalized degree-type code (e.g. `\"BS\"`, `\"BA\"`, `\"MS\"`, `\"MINOR\"`)
    #[schemars(
        description = "Normalized degree type code (e.g. \"BS\", \"BA\", \"MS\", \"MINOR\")"
    )]
    pub degree_type: Option<String>,
    /// Program kind (e.g. `\"major\"`, `\"minor\"`, `\"concentration\"`, `\"certificate\"`)
    #[schemars(
        description = "Program kind (e.g. \"major\", \"minor\", \"concentration\", \"certificate\")"
    )]
    pub program_kind: Option<String>,
    /// Discipline tag (e.g. `\"cs\"`, `\"ai\"`, `\"ds\"`, `\"cy\"`)
    #[schemars(description = "Discipline (e.g. \"cs\", \"ai\", \"ds\", \"cy\")")]
    pub discipline: Option<String>,
    /// Maximum results to return (default 20, max 50)
    #[schemars(description = "Maximum results (default 20, max 50)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

/// Request parameters for `get_degree`
///
/// Lookup precedence: `program_key` (unique) → `degree_id` → natural key
/// `(unitid, cip_code, catalog_year)`. If multiple programs match, returns a
/// list of summaries — narrow with more filters.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetDegreeRequest {
    /// Deterministic program key (the strongest, unique lookup). Takes priority
    /// over every other field.
    #[schemars(
        description = "Stored program_key (unique, strongest lookup). Takes priority over other fields."
    )]
    pub program_key: Option<String>,
    /// Lookup by `Degree.id` slug (e.g. `\"neu-khoury-bscs-2024\"`). May match
    /// several programs across catalog years. Used when `program_key` is absent.
    #[schemars(
        description = "Degree id slug. May match multiple catalog years; prefer program_key when known."
    )]
    pub degree_id: Option<String>,
    /// IPEDS UNITID of the institution
    #[schemars(description = "IPEDS UNITID of the institution")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// Full 7-character CIP code in dot notation (e.g. `\"11.0101\"`)
    #[schemars(description = "CIP code in dot notation (e.g. \"11.0101\" for CS General)")]
    pub cip_code: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
    /// Include the lossless degree document (default false).
    #[schemars(
        description = "Include the lossless unified-JSON document, about 30 KB (default false). Any degree tool takes the program_key as degree instead."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub include_document: Option<bool>,
}

// ============================================================================
// Response types
// ============================================================================

/// Lightweight program record for search results (no `document`).
#[derive(Debug, Serialize, Deserialize)]
struct ProgramSummary {
    program_key: String,
    name: String,
    unitid: Option<i32>,
    cip_code: Option<String>,
    catalog_year: Option<String>,
    degree_type: Option<String>,
    program_kind: Option<String>,
    discipline: Option<String>,
    #[serde(default)]
    verified: bool,
    #[serde(default)]
    institution_resolved: bool,
    #[serde(default)]
    has_impossible_requirements: bool,
}

/// Full program record for `get_degree`, including the lossless unified-JSON
/// `document`, which every degree tool accepts by `program_key`.
#[derive(Debug, Serialize, Deserialize)]
struct ProgramDetail {
    #[serde(flatten)]
    summary: ProgramSummary,
    degree_id: Option<String>,
    institution_raw: Option<String>,
    total_credits: Option<i32>,
    source_url: Option<String>,
    document: serde_json::Value,
}

// ============================================================================
// Execute functions
// ============================================================================

/// The `document` column alone.
#[derive(Debug, Deserialize)]
struct DocumentRow {
    document: serde_json::Value,
}

/// One `program_key` column, for the identity probes below.
#[derive(Debug, Deserialize)]
struct KeyRow {
    program_key: String,
}

/// How many `degree_id` matches to read before reporting the list as truncated.
///
/// One more than we would ever want to print, so `keys.len() > AMBIGUITY_PROBE` is a
/// reliable "there are more than this" rather than a guess.
const AMBIGUITY_PROBE: usize = 50;

/// Resolve `degree` to a `program_key`.
///
/// Tries `program_key` first because it is unique. A `degree_id` can match several
/// programs (one per catalog year), and silently reporting one of them as "the" answer
/// would be worse than saying so — hence the ambiguity error.
///
/// # Errors
/// A finished JSON payload, not a message — callers return it verbatim — when nothing
/// matches, when a `degree_id` matches several programs (listing them), or when the
/// backend fails.
pub async fn resolve_program_key(client: &DbClient, degree: &str) -> Result<String, String> {
    let by_key = QueryFilters::new().eq("program_key", Some(degree));
    match client
        .select(tables::PROGRAMS, "program_key", &by_key, Some(1))
        .await
    {
        Ok(v) if parse_first::<KeyRow>(&v).is_some() => return Ok(degree.to_string()),
        Ok(_) => {}
        Err(e) => return Err(e.to_json(&format!("looking up stored program \"{degree}\""))),
    }

    let by_id = QueryFilters::new().eq("degree_id", Some(degree));
    let rows = match client
        .select(
            tables::PROGRAMS,
            "program_key",
            &by_id,
            Some(AMBIGUITY_PROBE + 1),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return Err(e.to_json(&format!("looking up degree_id \"{degree}\""))),
    };
    let mut keys: Vec<String> = parse_json_array::<KeyRow>(&rows)
        .into_iter()
        .map(|r| r.program_key)
        .collect();

    match keys.len() {
        0 => Err(serde_json::json!({
            "error": format!(
                "no row in `programs` has program_key or degree_id = \"{degree}\""
            ),
            "code": error_code::SOURCE_NOT_FOUND,
            "degree": degree,
            "tip": "Search the stored programs by school or name for the exact program_key",
        })
        .to_string()),
        1 => Ok(keys.remove(0)),
        n => {
            // Say so rather than present a capped list as if it were complete.
            let truncated = n > AMBIGUITY_PROBE;
            keys.truncate(AMBIGUITY_PROBE);
            Err(serde_json::json!({
                "error": format!(
                    "degree_id \"{degree}\" matches {}{} rows in `programs` — pass one program_key",
                    if truncated { "more than " } else { "" },
                    keys.len()
                ),
                "degree": degree,
                "code": error_code::AMBIGUOUS_REFERENCE,
                "matches": keys,
                "truncated": truncated,
            })
            .to_string())
        }
    }
}

/// A stored program's lossless degree document, by `program_key` or `degree_id`.
///
/// The document is the unified-JSON degree `db import` wrote — the same thing `get_degree`
/// returns — so any tool that analyses a degree can take a stored one by reference.
/// Returns the resolved `program_key` beside it, so a caller can say which program it read.
///
/// # Errors
/// A finished JSON payload, as [`resolve_program_key`]'s, when the reference does not
/// resolve, when the backend fails, or when the program has no document.
pub async fn fetch_document(
    client: &DbClient,
    reference: &str,
) -> Result<(String, serde_json::Value), String> {
    let program_key = resolve_program_key(client, reference.trim()).await?;
    document_for_key(client, &program_key)
        .await
        .map_err(|e| e.to_json(&format!("reading the document of {program_key}")))?
        .map(|doc| (program_key.clone(), doc))
        .ok_or_else(|| {
            serde_json::json!({
                "error": format!("stored program `{program_key}` has no degree document"),
                "program_key": program_key,
            })
            .to_string()
        })
}

/// The `document` of the program with exactly this `program_key`: `None` when there is no
/// such program, or it has no document object.
///
/// # Errors
/// When the backend fails.
pub(crate) async fn document_for_key(
    client: &DbClient,
    program_key: &str,
) -> Result<Option<serde_json::Value>, crate::core::database::DatabaseError> {
    let filters = QueryFilters::new().eq("program_key", Some(program_key));
    let value = client
        .select(tables::PROGRAMS, "document", &filters, Some(1))
        .await?;
    Ok(parse_first::<DocumentRow>(&value)
        .map(|row| row.document)
        .filter(serde_json::Value::is_object))
}

/// Execute `search_degrees` and return JSON.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchDegreesRequest) -> String {
    let limit = req.limit.unwrap_or(20).min(50);

    let filters = QueryFilters::new()
        .eq("unitid", req.unitid)
        .eq("catalog_year", req.catalog_year.as_deref())
        .eq("degree_type", req.degree_type.as_deref())
        .eq("program_kind", req.program_kind.as_deref())
        .eq("discipline", req.discipline.as_deref())
        .ilike(
            "name",
            req.name.as_deref().map(str::trim).filter(|n| !n.is_empty()),
        )
        .starts_with("cip_code", req.cip_prefix.as_deref());

    let result = match client
        .select(
            tables::PROGRAMS,
            PROGRAM_SUMMARY_COLS,
            &filters,
            Some(limit),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_json("searching stored programs"),
    };

    let programs: Vec<ProgramSummary> = parse_json_array(&result);

    to_json_pretty(&serde_json::json!({
        "count": programs.len(),
        "programs": programs
    }))
}

/// Execute `get_degree` and return JSON: the program, which runs are stored for it, and —
/// with `include_document` — its unified-JSON `document`.
///
/// Lookup precedence: `program_key` (unique) → `degree_id` → natural key
/// `(unitid, cip_code, catalog_year)`. Exactly 1 match → full detail; >1 →
/// disambiguation summaries.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetDegreeRequest) -> String {
    let Some(filters) = get_filters(&req) else {
        return serde_json::json!({
            "error": "Provide at least one of: program_key, degree_id, unitid, cip_code, or catalog_year",
            "code": error_code::BAD_ARGUMENTS,
            "tip": "Search the stored programs first to find a program_key"
        })
        .to_string();
    };

    // Fetch up to 10 to detect ambiguity without fetching everything.
    let result = match client
        .select(tables::PROGRAMS, PROGRAM_DETAIL_COLS, &filters, Some(10))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_json("reading stored programs"),
    };
    let programs: Vec<ProgramDetail> = parse_json_array(&result);

    match programs.as_slice() {
        [] => serde_json::json!({
            "error": "no stored program matches the given fields",
            "code": error_code::SOURCE_NOT_FOUND,
            "program_key": req.program_key,
            "degree_id": req.degree_id,
            "unitid": req.unitid,
            "cip_code": req.cip_code,
            "catalog_year": req.catalog_year,
            "tip": "Search the stored programs by school or name for the exact program_key"
        })
        .to_string(),
        [program] => detail_json(client, program, req.include_document.unwrap_or(false)).await,
        many => matches_json(many),
    }
}

/// The filters `get_degree`'s fields select by, in precedence order; `None` when it names
/// no field at all.
fn get_filters(req: &GetDegreeRequest) -> Option<QueryFilters> {
    if let Some(pk) = req.program_key.as_deref() {
        return Some(QueryFilters::new().eq("program_key", Some(pk)));
    }
    if let Some(id) = req.degree_id.as_deref() {
        return Some(QueryFilters::new().eq("degree_id", Some(id)));
    }
    (req.unitid.is_some() || req.cip_code.is_some() || req.catalog_year.is_some()).then(|| {
        QueryFilters::new()
            .eq("unitid", req.unitid)
            .eq("cip_code", req.cip_code.as_deref())
            .eq("catalog_year", req.catalog_year.as_deref())
    })
}

/// One program in full, with the runs stored for it — which variants have been analysed,
/// and when: the figures a caller can read back instead of enumerating plans afresh.
async fn detail_json(client: &DbClient, program: &ProgramDetail, include_document: bool) -> String {
    let mut detail = serde_json::to_value(program).unwrap_or_default();
    if !include_document {
        if let Some(obj) = detail.as_object_mut() {
            obj.remove("document");
        }
    }
    let key = &program.summary.program_key;
    match super::metrics::run_summaries(client, key).await {
        Ok(runs) => detail["stored_runs"] = serde_json::Value::from(runs),
        Err(e) => return e.to_json(&format!("reading the analysis runs of {key}")),
    }
    to_json_pretty(&detail)
}

/// Several matching programs, in brief, to narrow down.
///
/// A list to choose from, not a failure: nothing was asked for by a name that should have
/// been unique.
fn matches_json(programs: &[ProgramDetail]) -> String {
    let summaries: Vec<_> = programs
        .iter()
        .map(|p| {
            let s = &p.summary;
            serde_json::json!({
                "program_key": s.program_key,
                "name": s.name,
                "unitid": s.unitid,
                "cip_code": s.cip_code,
                "catalog_year": s.catalog_year,
                "degree_type": s.degree_type
            })
        })
        .collect();
    serde_json::json!({
        "message": "Multiple programs match — provide program_key or more filters to narrow",
        "count": programs.len(),
        "matches": summaries
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the `PROGRAM_SUMMARY_COLS` → `ProgramSummary` contract: a row
    /// shaped like a `PostgREST` `programs` select must deserialize cleanly. A
    /// typo'd column name would break this.
    #[test]
    fn test_program_summary_deserializes_from_programs_row() {
        let row = serde_json::json!({
            "program_key": "prog:126818|11.0701|2025-2026|BS",
            "name": "Computer Science, BS",
            "unitid": 126_818,
            "cip_code": "11.0701",
            "catalog_year": "2025-2026",
            "degree_type": "BS",
            "program_kind": "concentration",
            "discipline": "cs",
            "verified": false,
            "institution_resolved": true,
            "has_impossible_requirements": false
        });
        let summary: ProgramSummary =
            serde_json::from_value(row).expect("programs summary row must deserialize");
        assert_eq!(summary.program_key, "prog:126818|11.0701|2025-2026|BS");
        assert_eq!(summary.unitid, Some(126_818));
        assert_eq!(summary.degree_type.as_deref(), Some("BS"));
        assert!(summary.institution_resolved);
    }

    fn get_request(v: serde_json::Value) -> GetDegreeRequest {
        serde_json::from_value(v).expect("request decodes")
    }

    #[test]
    fn test_get_filters_follow_the_documented_precedence() {
        let filters = |v| format!("{:?}", get_filters(&get_request(v)));
        let by_key =
            filters(serde_json::json!({"program_key": "k", "degree_id": "d", "unitid": 1}));
        assert!(
            by_key.contains("program_key") && !by_key.contains("degree_id"),
            "{by_key}"
        );
        let by_id = filters(serde_json::json!({"degree_id": "d", "unitid": 1}));
        assert!(
            by_id.contains("degree_id") && !by_id.contains("unitid"),
            "{by_id}"
        );
        let natural = filters(serde_json::json!({"unitid": 1, "catalog_year": "2024-2025"}));
        assert!(
            natural.contains("unitid") && natural.contains("catalog_year"),
            "{natural}"
        );
        assert_eq!(filters(serde_json::json!({})), "None");
    }

    #[test]
    fn test_matches_json_lists_each_program_in_brief() {
        let row = |key: &str| {
            serde_json::from_value::<ProgramDetail>(serde_json::json!({
                "program_key": key, "name": "BS", "unitid": 1, "cip_code": "11.0701",
                "catalog_year": "2024-2025", "degree_type": "BS", "program_kind": null,
                "discipline": null, "degree_id": "d", "institution_raw": null,
                "total_credits": 120, "source_url": null, "document": {"big": true}
            }))
            .expect("row decodes")
        };
        let out: serde_json::Value =
            serde_json::from_str(&matches_json(&[row("a"), row("b")])).unwrap();
        assert_eq!(out["count"], 2);
        assert_eq!(out["matches"][1]["program_key"], "b");
        assert!(
            out["matches"][0].get("document").is_none(),
            "no documents in the list"
        );
        assert!(
            out.get("error").is_none(),
            "a list to choose from, not a failure"
        );
    }

    /// Guards the `PROGRAM_DETAIL_COLS` → `ProgramDetail` contract, including
    /// the lossless `document` JSONB that downstream tools consume. Nullable
    /// columns (`cip_code`, `degree_id`) must tolerate JSON `null`.
    #[test]
    fn test_program_detail_deserializes_with_document_and_nulls() {
        let row = serde_json::json!({
            "program_key": "prog:141574||2024-2025|BS",
            "name": "BS in Computer Science - General Track",
            "unitid": 141_574,
            "cip_code": null,
            "catalog_year": "2024-2025",
            "degree_type": "BS",
            "program_kind": null,
            "discipline": null,
            "verified": false,
            "institution_resolved": true,
            "has_impossible_requirements": false,
            "degree_id": null,
            "institution_raw": "University of Hawaii at Manoa",
            "total_credits": 120,
            "source_url": null,
            "document": { "degree": { "institution": "University of Hawaii at Manoa" } }
        });
        let detail: ProgramDetail =
            serde_json::from_value(row).expect("programs detail row must deserialize");
        assert_eq!(detail.summary.cip_code, None);
        assert_eq!(detail.degree_id, None);
        assert_eq!(detail.total_credits, Some(120));
        let flat = serde_json::to_value(&detail).expect("serializes");
        assert_eq!(
            flat["program_key"], "prog:141574||2024-2025|BS",
            "the summary fields stay at the top level"
        );
        assert!(flat.get("summary").is_none());
        assert_eq!(
            detail.document["degree"]["institution"],
            serde_json::json!("University of Hawaii at Manoa"),
            "the lossless document must survive deserialization intact"
        );
    }
}
