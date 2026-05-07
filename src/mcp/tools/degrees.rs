//! `search_degrees`, `get_degree`, `compare_degrees`, and `store_degree` MCP tools

use std::sync::Arc;

use crate::core::database::models::StoredDegree;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{error_json, parse_first, parse_json_array, to_json_pretty};
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
    pub unitid: Option<i32>,
    /// CIP code prefix to filter by (e.g. `\"11.\"` for all CS, `\"11.01.\"` for one family)
    #[schemars(description = "CIP code prefix (e.g. \"11.\" for computer science)")]
    pub cip_prefix: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
    /// Maximum results to return (default 20, max 50)
    #[schemars(description = "Maximum results (default 20, max 50)")]
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
    pub unitid: Option<i32>,
    /// Full 7-character CIP code in dot notation (e.g. `\"11.0101\"`)
    #[schemars(description = "CIP code in dot notation (e.g. \"11.0101\" for CS General)")]
    pub cip_code: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
}

/// Request parameters for `compare_degrees`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDegreesRequest {
    /// Comma-separated degree IDs to compare (e.g. `\"neu-cs-2024,mit-cs-2024\"`)
    #[schemars(description = "Comma-separated degree IDs to compare")]
    pub degree_ids: String,
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
pub async fn execute_compare_json(client: &Arc<DbClient>, req: CompareDegreesRequest) -> String {
    let ids: Vec<&str> = req
        .degree_ids
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();

    if ids.is_empty() {
        return error_json("No degree IDs provided");
    }

    let mut degrees = Vec::new();
    let mut not_found = Vec::new();

    for id in &ids {
        let result_str = fetch_by_id(client, id).await;
        // Try to parse back — if it has "error" key it's not found
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&result_str) {
            if parsed.get("error").is_some() {
                not_found.push(*id);
            } else if let Ok(d) = serde_json::from_value::<DegreeDetail>(parsed) {
                degrees.push(d);
            } else {
                not_found.push(*id);
            }
        } else {
            not_found.push(*id);
        }
    }

    to_json_pretty(&serde_json::json!({
        "count": degrees.len(),
        "degrees": degrees,
        "not_found": not_found
    }))
}

/// Execute `store_degree` and return JSON. Requires authentication.
pub async fn execute_store_json(client: &Arc<DbClient>, req: StoreDegreeRequest) -> String {
    if !client.is_authenticated() {
        return serde_json::json!({
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
