//! Stored degree programs: `search_degrees`, `get_degree`, and resolving a reference.
//!
//! Everything here reads the normalized `programs` table `db import` writes; its
//! `document` column is the lossless unified-JSON degree.

use std::sync::Arc;

use crate::core::database::cips;
use crate::core::database::import::VERSION_KEY_SEPARATOR;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{error_code, parse_first, parse_json_array, to_json_pretty};
use serde::{Deserialize, Serialize};

/// The `programs` columns every program record carries, as a literal both projections
/// are built from.
macro_rules! program_summary_cols {
    () => {
        "program_key,name,unitid,cip_code,catalog_year,degree_type,program_kind,discipline,verified,institution_resolved,has_impossible_requirements,degree_id,institution_ref,version_date"
    };
}

/// Lightweight projection for `search_degrees` results.
const PROGRAM_SUMMARY_COLS: &str = program_summary_cols!();
/// Full projection for `get_degree` — the summary fields plus provenance and
/// the lossless `document` (unified-JSON degree) for downstream analysis.
const PROGRAM_DETAIL_COLS: &str = concat!(
    program_summary_cols!(),
    ",institution_raw,total_credits,source_url,document"
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
    /// The program across catalog years — what a confirmed CIP (`db cip set`) names.
    #[serde(default)]
    degree_id: Option<String>,
    /// Read to look up confirmed CIPs; not part of the answer.
    #[serde(default, skip_serializing)]
    institution_ref: Option<String>,
    /// Read to look up confirmed CIPs; not part of the answer.
    #[serde(default, skip_serializing)]
    version_date: Option<String>,
    /// Whether `cip_code` is the CIP the school confirmed for this program's year
    /// (`program_cips`), rather than a guess. Absent when the registry could not be read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    cip_confirmed: Option<bool>,
}

/// Full program record for `get_degree`, including the lossless unified-JSON
/// `document`, which every degree tool accepts by `program_key`.
#[derive(Debug, Serialize, Deserialize)]
struct ProgramDetail {
    #[serde(flatten)]
    summary: ProgramSummary,
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

/// The columns [`VersionRow`] reads.
const VERSION_ROW_COLS: &str = "program_key,version_date,generation";

/// A `degree_id` match with what separates dated versions of one program.
#[derive(Debug, Deserialize)]
struct VersionRow {
    program_key: String,
    #[serde(default)]
    version_date: Option<String>,
    #[serde(default)]
    generation: Option<i64>,
}

impl VersionRow {
    /// The key without its version-date suffix: what every dated version of one program
    /// shares. Comparing it, rather than a few columns, means two programs that differ in
    /// any part of the key (school, catalog year, degree type, degree id) are never taken for
    /// versions of one another.
    fn base_key(&self) -> &str {
        self.version_date
            .as_deref()
            .and_then(|date| {
                self.program_key
                    .strip_suffix(date)?
                    .strip_suffix(VERSION_KEY_SEPARATOR)
            })
            .unwrap_or(&self.program_key)
    }
}

/// When every match is a version of the same program (same key once the version date is
/// removed), the newest one: `version_date` first (the undated row is the oldest), then
/// the latest write. `None` when the matches are genuinely different programs.
fn latest_version(rows: &[VersionRow]) -> Option<String> {
    let first = rows.first()?;
    if !rows.iter().all(|r| r.base_key() == first.base_key()) {
        return None;
    }
    rows.iter()
        .max_by(|a, b| {
            (a.version_date.as_deref(), a.generation)
                .cmp(&(b.version_date.as_deref(), b.generation))
        })
        .map(|r| r.program_key.clone())
}

/// How many `degree_id` matches to read before reporting the list as truncated.
///
/// One more than we would ever want to print, so `keys.len() > AMBIGUITY_PROBE` is a
/// reliable "there are more than this" rather than a guess.
const AMBIGUITY_PROBE: usize = 50;

/// The programs a reference names, tried the way a person means one: an exact
/// `program_key`, then an exact `degree_id`, then programs whose name contains it.
///
/// The first tier that matches wins, so a key or id is never read as a name pattern, where
/// it would match nothing. `scope` narrows every tier (to one school, say) and `limit` caps
/// each select.
///
/// # Errors
/// The backend's error.
pub async fn find_programs<T: serde::de::DeserializeOwned>(
    client: &DbClient,
    cols: &str,
    scope: impl Fn() -> QueryFilters,
    reference: &str,
    limit: usize,
) -> crate::core::database::DatabaseResult<Vec<T>> {
    for column in ["program_key", "degree_id"] {
        let filters = scope().eq(column, Some(reference));
        let rows: Vec<T> = parse_json_array(
            &client
                .select(tables::PROGRAMS, cols, &filters, Some(limit))
                .await?,
        );
        if !rows.is_empty() {
            return Ok(rows);
        }
    }
    let filters = scope().ilike("name", Some(reference));
    Ok(parse_json_array(
        &client
            .select(tables::PROGRAMS, cols, &filters, Some(limit))
            .await?,
    ))
}

/// Resolve `degree` to a `program_key`.
///
/// Tries `program_key` first because it is unique. A `degree_id` can match several
/// programs (one per catalog year), and silently reporting one of them as "the" answer
/// would be worse than saying so — hence the ambiguity error. Dated versions of one
/// program (`--version-date` imports: the same key apart from its `|v<date>` suffix) are
/// not ambiguous: the newest version is the program.
///
/// Deliberately narrower than the `programs_latest` view, which keeps one row per school
/// and `degree_id` *across* catalog years. The view answers "what is current", where
/// collapsing years is the point; this resolves a reference to one program, where two
/// catalog years are two answers and the caller must pick.
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
        // Existence is all this probe asks, so any row will do.
        Ok(v) if parse_first::<serde_json::Value>(&v).is_some() => return Ok(degree.to_string()),
        Ok(_) => {}
        Err(e) => return Err(e.to_json(&format!("looking up stored program \"{degree}\""))),
    }

    let by_id = QueryFilters::new().eq("degree_id", Some(degree));
    let rows = match client
        .select(
            tables::PROGRAMS,
            VERSION_ROW_COLS,
            &by_id,
            Some(AMBIGUITY_PROBE + 1),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return Err(e.to_json(&format!("looking up degree_id \"{degree}\""))),
    };
    degree_id_matches(degree, parse_json_array::<VersionRow>(&rows))
}

/// What a `degree_id` lookup's matches resolve to: the one program, the newest dated
/// version when they are all versions of one program, else an ambiguity (marked
/// truncated past [`AMBIGUITY_PROBE`]) or not-found payload.
fn degree_id_matches(degree: &str, versions: Vec<VersionRow>) -> Result<String, String> {
    if versions.len() > 1 && versions.len() <= AMBIGUITY_PROBE {
        if let Some(latest) = latest_version(&versions) {
            return Ok(latest);
        }
    }
    let mut keys: Vec<String> = versions.into_iter().map(|r| r.program_key).collect();

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

    let mut programs: Vec<ProgramSummary> = parse_json_array(&result);
    mark_confirmed(client, &mut programs).await;

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
    let mut programs: Vec<ProgramDetail> = parse_json_array(&result);
    if let [program] = programs.as_mut_slice() {
        mark_confirmed(client, std::slice::from_mut(&mut program.summary)).await;
    }

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

/// Set each summary's `cip_confirmed` from the `program_cips` registry: true when the
/// row in force for its catalog year names the CIP it has. Left unset when the registry
/// cannot be read (a schema from before `program_cips` has no table), so a missing table
/// never reads as "every CIP is a guess".
async fn mark_confirmed(client: &DbClient, programs: &mut [ProgramSummary]) {
    let mut refs: Vec<String> = programs
        .iter()
        .filter_map(|p| p.institution_ref.clone())
        .collect();
    refs.sort();
    refs.dedup();
    if refs.is_empty() {
        return;
    }
    let Ok(rows) = cips::fetch_for_schools(client, &refs).await else {
        return;
    };
    let mut by_program: std::collections::HashMap<(&str, &str), Vec<cips::ProgramCip>> =
        std::collections::HashMap::new();
    for r in &rows {
        by_program
            .entry((r.institution_ref.as_str(), r.degree_id.as_str()))
            .or_default()
            .push(r.clone());
    }
    for p in programs {
        let mine = p
            .institution_ref
            .as_deref()
            .zip(p.degree_id.as_deref())
            .and_then(|key| by_program.get(&key))
            .map_or(&[][..], Vec::as_slice);
        let year = cips::program_year(p.catalog_year.as_deref(), p.version_date.as_deref());
        p.cip_confirmed = Some(
            cips::resolve(mine, year)
                .is_some_and(|r| p.cip_code.as_deref() == Some(r.cip_code.as_str())),
        );
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

    const BS_2025: &str = "prog:1|2025-2026|BS|cs";

    /// A stored row of `base`, dated (`base|v<date>`) when `date` is given.
    fn version(base: &str, date: Option<&str>, generation: i64) -> VersionRow {
        VersionRow {
            program_key: date.map_or_else(
                || base.to_string(),
                |d| format!("{base}{VERSION_KEY_SEPARATOR}{d}"),
            ),
            version_date: date.map(Into::into),
            generation: Some(generation),
        }
    }

    #[test]
    fn test_latest_version_picks_newest_dated_row() {
        let rows = [
            version(BS_2025, None, 9),
            version(BS_2025, Some("2026-10-09"), 5),
            version(BS_2025, Some("2026-09-01"), 7),
        ];
        assert_eq!(
            latest_version(&rows).as_deref(),
            Some("prog:1|2025-2026|BS|cs|v2026-10-09")
        );
    }

    #[test]
    fn test_latest_version_refuses_programs_differing_in_any_key_part() {
        // Different catalog year, school, degree type, id: each is another program, and
        // two undated programs must stay ambiguous rather than be split by import time.
        for other in [
            "prog:1|2024-2025|BS|cs",
            "prog:2|2025-2026|BS|cs",
            "prog:ca:university-of-toronto|2025-2026|BS|cs",
            "prog:1|2025-2026|BS|ai",
            "prog:1|2025-2026|BA|cs",
        ] {
            for (date, other_date) in [(None, None), (None, Some("2026-10-09"))] {
                let rows = [version(BS_2025, date, 1), version(other, other_date, 2)];
                assert_eq!(latest_version(&rows), None, "{other} {other_date:?}");
            }
        }
    }

    #[test]
    fn test_base_key_strips_only_its_own_version_suffix() {
        assert_eq!(version(BS_2025, Some("2026-10-09"), 1).base_key(), BS_2025);
        assert_eq!(version(BS_2025, None, 1).base_key(), BS_2025);
        // A date that does not match the key's suffix leaves the key whole.
        let mismatched = VersionRow {
            program_key: format!("{BS_2025}|v2026-10-09"),
            version_date: Some("2026-01-01".into()),
            generation: None,
        };
        assert_eq!(mismatched.base_key(), mismatched.program_key);
    }

    /// A backend that finds no `program_key` match and answers every `degree_id` select
    /// with `by_id`, so `resolve_program_key` reaches its version logic.
    async fn degree_id_backend(by_id: &str) -> DbClient {
        crate::core::database::test_backend::routed(vec![
            ("program_key=eq", "[]".to_string()),
            ("/rest/v1/programs", by_id.to_string()),
        ])
        .await
        .client
    }

    #[test]
    fn test_too_many_versions_are_reported_truncated_not_collapsed() {
        let rows: Vec<VersionRow> = (0..=AMBIGUITY_PROBE)
            .map(|i| version(BS_2025, Some(&format!("2026-01-{i:02}")), 1))
            .collect();
        let err: serde_json::Value =
            serde_json::from_str(&degree_id_matches("cs", rows).unwrap_err()).unwrap();
        assert_eq!(err["truncated"], true, "{err}");
        assert_eq!(
            err["matches"].as_array().map(Vec::len),
            Some(AMBIGUITY_PROBE)
        );
        let none: serde_json::Value =
            serde_json::from_str(&degree_id_matches("cs", Vec::new()).unwrap_err()).unwrap();
        assert_eq!(none["code"], error_code::SOURCE_NOT_FOUND);
        assert_eq!(
            degree_id_matches("cs", vec![version(BS_2025, None, 1)]).as_deref(),
            Ok(BS_2025)
        );
    }

    #[tokio::test]
    async fn test_resolve_program_key_returns_the_newest_dated_version() {
        let client = degree_id_backend(
            r#"[{"program_key":"prog:1|2025-2026|BS|cs","version_date":null,"generation":9},
                {"program_key":"prog:1|2025-2026|BS|cs|v2026-10-09","version_date":"2026-10-09","generation":5}]"#,
        )
        .await;
        assert_eq!(
            resolve_program_key(&client, "cs").await.unwrap(),
            "prog:1|2025-2026|BS|cs|v2026-10-09"
        );
    }

    #[tokio::test]
    async fn test_resolve_program_key_still_reports_two_programs_as_ambiguous() {
        let client = degree_id_backend(
            r#"[{"program_key":"prog:1|2024-2025|BS|cs","version_date":null,"generation":1},
                {"program_key":"prog:1|2025-2026|BS|cs","version_date":null,"generation":2}]"#,
        )
        .await;
        let err = resolve_program_key(&client, "cs").await.unwrap_err();
        let err: serde_json::Value = serde_json::from_str(&err).unwrap();
        assert_eq!(err["code"], error_code::AMBIGUOUS_REFERENCE, "{err}");
        assert_eq!(err["matches"].as_array().map(Vec::len), Some(2), "{err}");
    }

    fn summary(degree_id: Option<&str>, catalog: &str, cip: &str) -> ProgramSummary {
        serde_json::from_value(serde_json::json!({
            "program_key": format!("prog:1|{catalog}|BS|x"),
            "name": "CS", "unitid": 1, "cip_code": cip, "catalog_year": catalog,
            "degree_id": degree_id, "institution_ref": "1",
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn test_mark_confirmed_compares_the_cip_in_force_for_each_year() {
        let rows = serde_json::json!([{
            "institution_ref": "1", "degree_id": "cs", "from_year": 2024, "cip_code": "11.0102",
            "name": "CS", "source": "registrar",
        }])
        .to_string();
        let backend =
            crate::core::database::test_backend::routed(vec![("/rest/v1/program_cips", rows)])
                .await;
        let mut programs = [
            summary(Some("cs"), "2024-2025", "11.0102"),
            summary(Some("cs"), "2023-2024", "11.0701"),
            summary(Some("cs"), "2025-2026", "11.0701"),
            summary(None, "2024-2025", "11.0102"),
        ];
        mark_confirmed(&backend.client, &mut programs).await;
        let got: Vec<Option<bool>> = programs.iter().map(|p| p.cip_confirmed).collect();
        assert_eq!(got, [Some(true), Some(false), Some(false), Some(false)]);

        // An unreadable registry leaves the field unset rather than reading as "guessed".
        let mut programs = [summary(Some("cs"), "2024-2025", "11.0102")];
        mark_confirmed(
            &crate::core::database::test_backend::unreachable(),
            &mut programs,
        )
        .await;
        assert_eq!(programs[0].cip_confirmed, None);
    }

    #[test]
    fn test_latest_version_single_row_is_itself() {
        let rows = [version(BS_2025, None, 1)];
        assert_eq!(latest_version(&rows).as_deref(), Some(BS_2025));
        assert_eq!(latest_version(&[]), None);
    }

    /// Guards the `PROGRAM_SUMMARY_COLS` → `ProgramSummary` contract: a row
    /// shaped like a `PostgREST` `programs` select must deserialize cleanly. A
    /// typo'd column name would break this.
    #[test]
    fn test_program_summary_deserializes_from_programs_row() {
        let row = serde_json::json!({
            "program_key": "prog:126818|2025-2026|BS",
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
        assert_eq!(summary.program_key, "prog:126818|2025-2026|BS");
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
            "program_key": "prog:141574|2024-2025|BS",
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
        assert_eq!(detail.summary.degree_id, None);
        assert_eq!(detail.total_credits, Some(120));
        let flat = serde_json::to_value(&detail).expect("serializes");
        assert_eq!(
            flat["program_key"], "prog:141574|2024-2025|BS",
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
