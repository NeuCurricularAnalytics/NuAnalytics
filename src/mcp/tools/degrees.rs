//! `search_degrees`, `get_degree`, `compare_degrees`, and `store_degree` MCP tools
//!
//! `search_degrees` / `get_degree` / `compare_degrees` read the normalized
//! `programs` table written by `import_degree` (the unified-JSON `document` is
//! the lossless source of truth). The legacy `store_degree` still targets the
//! old `degrees` yaml-blob table and is retained only for back-compat.

use std::sync::Arc;

use crate::core::database::models::StoredDegree;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{self, error_json, parse_first, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Lightweight projection for `search_degrees` results.
const PROGRAM_SUMMARY_COLS: &str = "program_key,name,unitid,cip_code,catalog_year,degree_type,program_kind,discipline,verified,institution_resolved,has_impossible_requirements";
/// Full projection for `get_degree` — the summary fields plus provenance and
/// the lossless `document` (unified-JSON degree) for downstream analysis.
const PROGRAM_DETAIL_COLS: &str = "program_key,name,unitid,cip_code,catalog_year,degree_type,program_kind,discipline,verified,institution_resolved,has_impossible_requirements,degree_id,institution_raw,total_credits,source_url,document";

// ============================================================================
// Request types
// ============================================================================

/// Request parameters for `search_degrees`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchDegreesRequest {
    /// IPEDS UNITID of the institution
    #[schemars(description = "IPEDS UNITID of the institution")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
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
    #[serde(default, deserialize_with = "shared::deserialize_opt_usize")]
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
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// Full 7-character CIP code in dot notation (e.g. `\"11.0101\"`)
    #[schemars(description = "CIP code in dot notation (e.g. \"11.0101\" for CS General)")]
    pub cip_code: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
}

/// Request parameters for `compare_degrees`.
///
/// Accept either the legacy `degree_ids` comma-separated form (DB-only) or
/// the structured `sources` array — the latter lets callers mix stored
/// degrees with inline YAML / filesystem paths in one comparison without
/// having to `store_degree` first.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDegreesRequest {
    /// Legacy comma-separated degree IDs to compare (e.g.
    /// `\"neu-cs-2024,mit-cs-2024\"`). Each ID is looked up in the stored
    /// degrees table.
    #[schemars(
        description = "Comma-separated stored degree IDs to compare (legacy form; prefer `sources` for mixed inline/stored input)."
    )]
    pub degree_ids: Option<String>,

    /// Structured list of degree sources to compare. Each entry resolves
    /// from one of `degree_id`, `yaml_content`, or `yaml_path`. Use this
    /// when you want to benchmark an in-progress YAML against a stored peer.
    #[schemars(
        description = "Structured per-degree sources (mix of stored IDs, inline YAMLs, and filesystem paths). Each entry must specify exactly one of degree_id/yaml_content/yaml_path. Optional `label` controls the response order key."
    )]
    pub sources: Option<Vec<DegreeSource>>,

    /// Include side-by-side analyze metrics (`complexity`, `longest_delay`,
    /// `total_credits`) for each degree. Default true. Set false to skip the
    /// analysis pass when you only want metadata + YAML.
    #[schemars(
        description = "Include analyze-style metrics per degree (default true). Set false to skip the analysis pass for performance."
    )]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub include_metrics: Option<bool>,

    /// Cap on plans generated per degree during the analysis pass. Forwarded
    /// to `analyze_degree`'s `max_plans`. Default 500.
    #[schemars(description = "max_plans for the per-degree analysis pass (default 500)")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_usize")]
    pub max_plans: Option<usize>,
}

/// One degree to include in a comparison.
///
/// Exactly one of the three source fields (`degree_id`, `yaml_content`,
/// `yaml_path`) must be set. An optional `label` controls the response's
/// display name when the caller wants something more readable than the
/// resolved slug or filename.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DegreeSource {
    /// Free-form label used in the response. Falls back to the resolved
    /// `degree_id` or YAML's degree id when omitted.
    #[schemars(description = "Display label for this source in the response (optional).")]
    pub label: Option<String>,

    /// Stored degree id — looked up in the database.
    #[schemars(description = "Stored degree ID (DB lookup).")]
    pub degree_id: Option<String>,

    /// Inline YAML body.
    #[schemars(description = "Inline YAML body for this degree.")]
    pub yaml_content: Option<String>,

    /// Filesystem path the MCP server will read.
    #[schemars(description = "Path to a YAML file on the MCP server's filesystem.")]
    pub yaml_path: Option<String>,
}

/// Request parameters for `store_degree`
///
/// Saves a validated degree YAML to the database. Requires authentication
/// (`nuanalytics db login`). Uses upsert on `degree_id` — safe to re-run.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct StoreDegreeRequest {
    /// Unique identifier for this degree program (e.g. `\"neu-khoury-bscs-2024\"`).
    /// Used as the upsert key — re-submitting the same ID updates the record.
    #[schemars(
        description = "Unique degree ID (e.g. \"neu-khoury-bscs-2024\"). Used as upsert key."
    )]
    pub degree_id: String,
    /// IPEDS UNITID of the institution offering this degree
    #[schemars(description = "IPEDS UNITID of the institution")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// CIP code in dot notation (e.g. `\"11.0101\"`)
    #[schemars(description = "CIP code (e.g. \"11.0101\")")]
    pub cip_code: Option<String>,
    /// Catalog year (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
    /// Full YAML content of the degree program (from `validate_degree` / `audit_degree`)
    #[schemars(description = "Full degree YAML content")]
    pub yaml_content: String,
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
/// `document` that downstream tools (`analyze_degree`, `cache_yaml`) accept.
#[derive(Debug, Serialize, Deserialize)]
struct ProgramDetail {
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
    degree_id: Option<String>,
    institution_raw: Option<String>,
    total_credits: Option<i32>,
    source_url: Option<String>,
    document: serde_json::Value,
}

// ============================================================================
// Execute functions
// ============================================================================

/// Execute `search_degrees` and return JSON.
///
/// Reads the normalized `programs` table, so the new queryable dimensions
/// (`degree_type`, `program_kind`, `discipline`) filter alongside the legacy
/// `unitid` / `cip_prefix`.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchDegreesRequest) -> String {
    let limit = req.limit.unwrap_or(20).min(50);

    let filters = QueryFilters::new()
        .eq("unitid", req.unitid)
        .eq("catalog_year", req.catalog_year.as_deref())
        .eq("degree_type", req.degree_type.as_deref())
        .eq("program_kind", req.program_kind.as_deref())
        .eq("discipline", req.discipline.as_deref())
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
        Err(e) => return error_json(e),
    };

    let programs: Vec<ProgramSummary> = parse_json_array(&result);

    to_json_pretty(&serde_json::json!({
        "count": programs.len(),
        "programs": programs
    }))
}

/// Execute `get_degree` and return JSON (includes the full unified-JSON
/// `document`).
///
/// Lookup precedence: `program_key` (unique) → `degree_id` → natural key
/// `(unitid, cip_code, catalog_year)`. Exactly 1 match → full detail; >1 →
/// disambiguation summaries.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetDegreeRequest) -> String {
    let filters = if let Some(pk) = req.program_key.as_deref() {
        QueryFilters::new().eq("program_key", Some(pk))
    } else if let Some(id) = req.degree_id.as_deref() {
        QueryFilters::new().eq("degree_id", Some(id))
    } else if req.unitid.is_some() || req.cip_code.is_some() || req.catalog_year.is_some() {
        QueryFilters::new()
            .eq("unitid", req.unitid)
            .eq("cip_code", req.cip_code.as_deref())
            .eq("catalog_year", req.catalog_year.as_deref())
    } else {
        return serde_json::json!({
            "error": "Provide at least one of: program_key, degree_id, unitid, cip_code, or catalog_year",
            "tip": "Use search_degrees to browse available programs first"
        })
        .to_string();
    };

    // Fetch up to 10 to detect ambiguity without fetching everything.
    let result = match client
        .select(tables::PROGRAMS, PROGRAM_DETAIL_COLS, &filters, Some(10))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let programs: Vec<ProgramDetail> = parse_json_array(&result);

    match programs.len() {
        0 => serde_json::json!({
            "error": "No program found matching the given filters",
            "tip": "Use search_degrees to see what is available"
        })
        .to_string(),
        1 => to_json_pretty(&programs[0]),
        _ => {
            // Return summaries and ask to narrow.
            let summaries: Vec<_> = programs
                .iter()
                .map(|p| {
                    serde_json::json!({
                        "program_key": p.program_key,
                        "name": p.name,
                        "unitid": p.unitid,
                        "cip_code": p.cip_code,
                        "catalog_year": p.catalog_year,
                        "degree_type": p.degree_type
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
    }
}

/// Execute `compare_degrees` and return JSON.
///
/// When `include_metrics=true` (default), each returned degree carries a
/// `metrics` object with the analyze pipeline's aggregate statistics so
/// callers can diff `complexity` / `longest_delay` / `total_credits` side-by-side
/// in a single call.
///
/// Accepts both the legacy `degree_ids` comma-separated form and the
/// structured `sources` list. When both are provided, `sources` is processed
/// first; the legacy IDs are appended without labels.
pub async fn execute_compare_json(client: &Arc<DbClient>, req: CompareDegreesRequest) -> String {
    let include_metrics = req.include_metrics.unwrap_or(true);
    let max_plans = req.max_plans;

    let mut resolved: Vec<ResolvedDegree> = Vec::new();
    let mut not_found: Vec<String> = Vec::new();

    if let Some(sources) = &req.sources {
        for (idx, source) in sources.iter().enumerate() {
            match resolve_source(client, source, idx).await {
                Ok(rd) => resolved.push(rd),
                Err(missing) => not_found.push(missing),
            }
        }
    }

    if let Some(ids_str) = req.degree_ids.as_deref() {
        let ids: Vec<&str> = ids_str
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        for id in ids {
            match fetch_program_detail(client, id).await {
                Some(detail) => resolved.push(ResolvedDegree::from_detail(None, detail)),
                None => not_found.push(id.to_string()),
            }
        }
    }

    if resolved.is_empty() && not_found.is_empty() {
        return error_json(
            "No degrees to compare. Provide `sources` (structured list) or `degree_ids` (legacy comma-separated form).",
        );
    }

    let degree_records: Vec<serde_json::Value> = resolved
        .iter()
        .map(|rd| {
            let mut record = serde_json::json!({
                "label": rd.label,
                "program_key": rd.program_key,
                "name": rd.name,
                "degree_id": rd.degree_id,
                "unitid": rd.unitid,
                "cip_code": rd.cip_code,
                "catalog_year": rd.catalog_year,
                "source": rd.source,
            });
            if include_metrics {
                record["metrics"] = compute_compare_metrics(&rd.source, max_plans);
            }
            record
        })
        .collect();

    to_json_pretty(&serde_json::json!({
        "count": resolved.len(),
        "degrees": degree_records,
        "not_found": not_found,
    }))
}

/// Internal: an already-resolved degree ready to fold into the response.
/// `source` holds the degree's source text — unified JSON for a stored program,
/// raw YAML for an inline/filesystem source.
struct ResolvedDegree {
    label: Option<String>,
    program_key: Option<String>,
    name: Option<String>,
    degree_id: Option<String>,
    unitid: Option<i32>,
    cip_code: Option<String>,
    catalog_year: Option<String>,
    source: String,
}

impl ResolvedDegree {
    fn from_detail(label: Option<String>, detail: ProgramDetail) -> Self {
        Self {
            label,
            program_key: Some(detail.program_key),
            name: Some(detail.name),
            degree_id: detail.degree_id,
            unitid: detail.unitid,
            cip_code: detail.cip_code,
            catalog_year: detail.catalog_year,
            source: serde_json::to_string(&detail.document).unwrap_or_default(),
        }
    }

    const fn from_inline(label: Option<String>, source: String) -> Self {
        Self {
            label,
            program_key: None,
            name: None,
            degree_id: None,
            unitid: None,
            cip_code: None,
            catalog_year: None,
            source,
        }
    }
}

/// Resolve one `DegreeSource` entry. Returns `Err(missing_label)` when the
/// source pointed at a stored id that doesn't exist (collected into
/// `not_found`) or when the entry's three input fields are misconfigured.
async fn resolve_source(
    client: &Arc<DbClient>,
    source: &DegreeSource,
    idx: usize,
) -> Result<ResolvedDegree, String> {
    let label = source
        .label
        .clone()
        .unwrap_or_else(|| format!("sources[{idx}]"));

    // Surface validation + lookup failures via `not_found` rather than a
    // top-level error so the rest of the compare call still produces useful
    // output for the good entries. Each error label embeds the underlying
    // cause so the caller can debug without a second round-trip.
    if let Err(msg) = validate_source_count(source) {
        return Err(format!("{label}: {msg}"));
    }

    if let Some(id) = source.degree_id.as_deref() {
        return fetch_program_detail(client, id)
            .await
            .map(|detail| ResolvedDegree::from_detail(source.label.clone(), detail))
            .ok_or_else(|| format!("{label}: stored id {id:?} not found in database"));
    }
    if let Some(yaml) = source.yaml_content.clone() {
        return Ok(ResolvedDegree::from_inline(source.label.clone(), yaml));
    }
    if let Some(path) = source.yaml_path.as_deref() {
        return match shared::read_yaml_file(path) {
            Ok(yaml) => Ok(ResolvedDegree::from_inline(source.label.clone(), yaml)),
            Err(read_err) => Err(format!(
                "{label}: yaml_path={path:?} failed to read — {read_err}"
            )),
        };
    }
    // Unreachable given `validate_source_count` succeeded above.
    Err(format!(
        "{label}: internal error — no source field resolved"
    ))
}

/// Validate that a [`DegreeSource`] sets exactly one of its three source
/// fields. Returned as a free function so unit tests can exercise the
/// invariant without needing an async test harness or a stub `DbClient`.
fn validate_source_count(source: &DegreeSource) -> Result<(), &'static str> {
    let count = u8::from(source.degree_id.is_some())
        + u8::from(source.yaml_content.is_some())
        + u8::from(source.yaml_path.is_some());
    match count {
        0 => Err("expected exactly one of degree_id, yaml_content, yaml_path (got none)"),
        1 => Ok(()),
        _ => Err("expected exactly one of degree_id, yaml_content, yaml_path (got multiple)"),
    }
}

/// Fetch a stored program's full detail by `id`, trying `program_key` (unique)
/// first and then `degree_id`. Returns `None` if neither matches or the row
/// fails to deserialize.
async fn fetch_program_detail(client: &Arc<DbClient>, id: &str) -> Option<ProgramDetail> {
    for col in ["program_key", "degree_id"] {
        let filters = QueryFilters::new().eq(col, Some(id));
        if let Ok(value) = client
            .select(tables::PROGRAMS, PROGRAM_DETAIL_COLS, &filters, Some(1))
            .await
        {
            if let Some(detail) = parse_first::<ProgramDetail>(&value) {
                return Some(detail);
            }
        }
    }
    None
}

/// Run the analyze pipeline on a degree's YAML and pluck the side-by-side
/// fields useful for `compare_degrees`. Errors surface as a `parse_error`
/// payload so a single bad YAML doesn't fail the whole compare call.
fn compute_compare_metrics(yaml: &str, max_plans: Option<usize>) -> serde_json::Value {
    let response = crate::mcp::tools::analyze::execute(
        yaml, max_plans, None, false, None, false, false, None, None,
    );
    if !response.success {
        return serde_json::json!({
            "parse_error": response.error,
        });
    }
    serde_json::json!({
        "plans_analyzed": response.plans_analyzed,
        "population_size": response.population_size,
        "is_full_population": response.is_full_population,
        "complexity": response.complexity,
        "longest_delay": response.longest_delay,
        "total_credits": response.total_credits,
    })
}

/// Execute `store_degree` and return JSON. The client is always
/// authenticated by construction (`DbClient::from_config` refuses to
/// build without a valid session), so this function just performs the
/// write.
pub async fn execute_store_json(client: &Arc<DbClient>, req: StoreDegreeRequest) -> String {
    let degree = StoredDegree {
        id: None,
        degree_id: req.degree_id.clone(),
        unitid: req.unitid,
        cip_code: req.cip_code,
        catalog_year: req.catalog_year,
        yaml_content: req.yaml_content,
        created_at: None,
    };

    match client
        .upsert_batch(tables::DEGREES, vec![degree], &["degree_id"])
        .await
    {
        Ok(()) => serde_json::json!({
            "stored": true,
            "degree_id": req.degree_id
        })
        .to_string(),
        Err(e) => error_json(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the `PROGRAM_SUMMARY_COLS` → `ProgramSummary` contract: a row
    /// shaped like a `PostgREST` `programs` select must deserialize cleanly. A
    /// typo'd column name (the original Issue A failure mode) would break this.
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
        assert_eq!(detail.cip_code, None);
        assert_eq!(detail.degree_id, None);
        assert_eq!(detail.total_credits, Some(120));
        assert_eq!(
            detail.document["degree"]["institution"],
            serde_json::json!("University of Hawaii at Manoa"),
            "the lossless document must survive deserialization intact"
        );
    }

    #[test]
    fn test_compute_compare_metrics_returns_metrics_for_valid_yaml() {
        // Minimal valid YAML lets us assert the metrics object carries the
        // analyze fields rather than a parse_error escape hatch.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 8
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS201]

courses:
  CS101:
    title: A
    prefix: CS
    number: "101"
    credits: 4
  CS201:
    title: B
    prefix: CS
    number: "201"
    credits: 4
    prerequisites_raw: "CS101"
"#;
        let value = compute_compare_metrics(yaml, Some(10));
        assert!(value.is_object(), "metrics must be a JSON object");
        assert!(value.get("plans_analyzed").is_some());
        assert!(value.get("complexity").is_some());
        assert!(value.get("longest_delay").is_some());
        assert!(value.get("total_credits").is_some());
        assert!(
            value.get("parse_error").is_none(),
            "valid YAML must not surface a parse_error key"
        );
    }

    #[test]
    fn test_validate_source_count_rejects_zero_fields_set() {
        let source = DegreeSource {
            label: Some("none-set".to_string()),
            degree_id: None,
            yaml_content: None,
            yaml_path: None,
        };
        let err = validate_source_count(&source).unwrap_err();
        assert!(err.contains("none"), "unexpected error: {err}");
    }

    #[test]
    fn test_validate_source_count_rejects_multiple_fields_set() {
        let source = DegreeSource {
            label: None,
            degree_id: Some("id".to_string()),
            yaml_content: Some("yaml".to_string()),
            yaml_path: None,
        };
        let err = validate_source_count(&source).unwrap_err();
        assert!(err.contains("multiple"), "unexpected error: {err}");
    }

    #[test]
    fn test_validate_source_count_accepts_exactly_one_field() {
        for source in [
            DegreeSource {
                label: None,
                degree_id: Some("id".to_string()),
                yaml_content: None,
                yaml_path: None,
            },
            DegreeSource {
                label: None,
                degree_id: None,
                yaml_content: Some("yaml".to_string()),
                yaml_path: None,
            },
            DegreeSource {
                label: None,
                degree_id: None,
                yaml_content: None,
                yaml_path: Some("/tmp/x.yaml".to_string()),
            },
        ] {
            assert!(validate_source_count(&source).is_ok());
        }
    }

    #[test]
    fn test_compute_compare_metrics_surfaces_parse_error_for_invalid_yaml() {
        // Failure mode the field report cared about: if a single bad YAML
        // shows up in compare_degrees, return its parse error inline so the
        // good degrees still come back with metrics.
        let value = compute_compare_metrics("not: valid: yaml: {{", None);
        assert!(value.is_object());
        assert!(
            value.get("parse_error").is_some(),
            "invalid YAML must surface parse_error"
        );
        assert!(
            value.get("plans_analyzed").is_none(),
            "no analysis fields when parsing fails"
        );
    }
}
