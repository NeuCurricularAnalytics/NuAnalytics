//! `search_degrees`, `get_degree`, `compare_degrees`, and `store_degree` MCP tools

use std::sync::Arc;

use crate::core::database::models::StoredDegree;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{self, error_json, parse_first, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

const DEGREE_SUMMARY_COLS: &str = "degree_id,unitid,cip_code,catalog_year,created_at";
const DEGREE_DETAIL_COLS: &str = "degree_id,unitid,cip_code,catalog_year,yaml_content";

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
    /// Maximum results to return (default 20, max 50)
    #[schemars(description = "Maximum results (default 20, max 50)")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

/// Request parameters for `get_degree`
///
/// Lookup by natural key `(unitid, cip_code, catalog_year)` or by `degree_id`.
/// If multiple degrees match, returns a list of summaries — narrow with more filters.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetDegreeRequest {
    /// Direct lookup by unique degree identifier (e.g. `\"neu-khoury-bscs-2024\"`).
    /// Takes priority over natural-key fields if provided.
    #[schemars(
        description = "Unique degree ID (fastest lookup). Takes priority over other fields."
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

/// Lightweight degree record for search results (no YAML content).
#[derive(Debug, Serialize, Deserialize)]
struct DegreeSummary {
    degree_id: String,
    unitid: Option<i32>,
    cip_code: Option<String>,
    catalog_year: Option<String>,
    created_at: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct DegreeDetail {
    degree_id: String,
    unitid: Option<i32>,
    cip_code: Option<String>,
    catalog_year: Option<String>,
    yaml_content: String,
}

// ============================================================================
// Execute functions
// ============================================================================

/// Execute `search_degrees` and return JSON.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchDegreesRequest) -> String {
    let limit = req.limit.unwrap_or(20).min(50);

    let filters = QueryFilters::new()
        .eq("unitid", req.unitid)
        .eq("catalog_year", req.catalog_year.as_deref())
        .starts_with("cip_code", req.cip_prefix.as_deref());

    let result = match client
        .select(tables::DEGREES, DEGREE_SUMMARY_COLS, &filters, Some(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let summaries: Vec<DegreeSummary> = parse_json_array(&result);

    to_json_pretty(&serde_json::json!({
        "count": summaries.len(),
        "degrees": summaries
    }))
}

/// Execute `get_degree` and return JSON (includes full YAML content).
///
/// - If `degree_id` provided: direct fetch by ID.
/// - Otherwise: filter by `unitid`, `cip_code`, `catalog_year` (any combination).
/// - If exactly 1 result: return full YAML.
/// - If >1 results: return list of summaries with disambiguation message.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetDegreeRequest) -> String {
    // Direct lookup by degree_id takes priority
    if let Some(ref id) = req.degree_id {
        return fetch_by_id(client, id).await;
    }

    // Natural-key lookup — need at least one filter
    if req.unitid.is_none() && req.cip_code.is_none() && req.catalog_year.is_none() {
        return serde_json::json!({
            "error": "Provide at least one of: degree_id, unitid, cip_code, or catalog_year",
            "tip": "Use search_degrees to browse available degrees first"
        })
        .to_string();
    }

    let filters = QueryFilters::new()
        .eq("unitid", req.unitid)
        .eq("cip_code", req.cip_code.as_deref())
        .eq("catalog_year", req.catalog_year.as_deref());

    // Fetch up to 2 to detect ambiguity without fetching everything
    let result = match client
        .select(tables::DEGREES, DEGREE_DETAIL_COLS, &filters, Some(10))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let degrees: Vec<DegreeDetail> = parse_json_array(&result);

    match degrees.len() {
        0 => serde_json::json!({
            "error": "No degree found matching the given filters",
            "tip": "Use search_degrees to see what is available"
        })
        .to_string(),
        1 => to_json_pretty(&degrees[0]),
        _ => {
            // Return summaries and ask to narrow
            let summaries: Vec<_> = degrees
                .iter()
                .map(|d| {
                    serde_json::json!({
                        "degree_id": d.degree_id,
                        "unitid": d.unitid,
                        "cip_code": d.cip_code,
                        "catalog_year": d.catalog_year
                    })
                })
                .collect();
            serde_json::json!({
                "message": "Multiple degrees match — provide degree_id or more filters to narrow",
                "count": degrees.len(),
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
            match fetch_detail_by_id(client, id).await {
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
                "degree_id": rd.degree_id,
                "unitid": rd.unitid,
                "cip_code": rd.cip_code,
                "catalog_year": rd.catalog_year,
                "yaml_content": rd.yaml_content,
            });
            if include_metrics {
                record["metrics"] = compute_compare_metrics(&rd.yaml_content, max_plans);
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
struct ResolvedDegree {
    label: Option<String>,
    degree_id: Option<String>,
    unitid: Option<i32>,
    cip_code: Option<String>,
    catalog_year: Option<String>,
    yaml_content: String,
}

impl ResolvedDegree {
    fn from_detail(label: Option<String>, detail: DegreeDetail) -> Self {
        Self {
            label,
            degree_id: Some(detail.degree_id),
            unitid: detail.unitid,
            cip_code: detail.cip_code,
            catalog_year: detail.catalog_year,
            yaml_content: detail.yaml_content,
        }
    }

    const fn from_inline(label: Option<String>, yaml_content: String) -> Self {
        Self {
            label,
            degree_id: None,
            unitid: None,
            cip_code: None,
            catalog_year: None,
            yaml_content,
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
    let count = u8::from(source.degree_id.is_some())
        + u8::from(source.yaml_content.is_some())
        + u8::from(source.yaml_path.is_some());
    if count != 1 {
        // Surface as not_found rather than a top-level error so the rest of
        // the compare call still produces useful output for the good entries.
        let label = source
            .label
            .clone()
            .unwrap_or_else(|| format!("sources[{idx}]"));
        return Err(format!(
            "{label} (expected exactly one of degree_id, yaml_content, yaml_path)"
        ));
    }

    if let Some(id) = source.degree_id.as_deref() {
        return fetch_detail_by_id(client, id)
            .await
            .map(|detail| ResolvedDegree::from_detail(source.label.clone(), detail))
            .ok_or_else(|| source.label.clone().unwrap_or_else(|| id.to_string()));
    }
    if let Some(yaml) = source.yaml_content.clone() {
        return Ok(ResolvedDegree::from_inline(source.label.clone(), yaml));
    }
    if let Some(path) = source.yaml_path.as_deref() {
        let Ok(yaml) = shared::read_yaml_file(path) else {
            return Err(source
                .label
                .clone()
                .unwrap_or_else(|| format!("yaml_path={path}")));
        };
        return Ok(ResolvedDegree::from_inline(source.label.clone(), yaml));
    }
    // Unreachable given the count check above.
    Err(source
        .label
        .clone()
        .unwrap_or_else(|| format!("sources[{idx}]")))
}

/// Fetch a stored degree's full detail by id; returns `None` if missing or
/// the row fails to deserialize. Wraps the JSON-string helper used elsewhere
/// in this module.
async fn fetch_detail_by_id(client: &Arc<DbClient>, id: &str) -> Option<DegreeDetail> {
    let result_str = fetch_by_id(client, id).await;
    let parsed: serde_json::Value = serde_json::from_str(&result_str).ok()?;
    if parsed.get("error").is_some() {
        return None;
    }
    serde_json::from_value::<DegreeDetail>(parsed).ok()
}

/// Run the analyze pipeline on a degree's YAML and pluck the side-by-side
/// fields useful for `compare_degrees`. Errors surface as a `parse_error`
/// payload so a single bad YAML doesn't fail the whole compare call.
fn compute_compare_metrics(yaml: &str, max_plans: Option<usize>) -> serde_json::Value {
    let response = crate::mcp::tools::analyze::execute(yaml, max_plans, None, false, None);
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

/// Execute `store_degree` and return JSON. Requires authentication.
pub async fn execute_store_json(client: &Arc<DbClient>, req: StoreDegreeRequest) -> String {
    if !client.is_authenticated() {
        // `error_code` is the stable programmatic key callers should branch on;
        // `error` and `solution` remain the human-readable surface.
        return serde_json::json!({
            "error_code": "auth_required",
            "error": "Write operations require authentication",
            "solution": "Run `nuanalytics db login` to sign in, then retry"
        })
        .to_string();
    }

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

// ============================================================================
// Internal helpers
// ============================================================================

/// Fetch a single degree by its `degree_id` and return full YAML detail JSON.
/// Returns an error JSON object if not found or the query fails.
async fn fetch_by_id(client: &Arc<DbClient>, id: &str) -> String {
    let filters = QueryFilters::new().eq("degree_id", Some(id));

    let result = match client
        .select(tables::DEGREES, DEGREE_DETAIL_COLS, &filters, Some(1))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let degree: Option<DegreeDetail> = parse_first(&result);

    degree.map_or_else(
        || {
            serde_json::json!({
                "error": "Degree not found",
                "degree_id": id
            })
            .to_string()
        },
        |d| to_json_pretty(&d),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
