//! `search_degrees`, `get_degree`, and `compare_degrees` MCP tools

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{error_json, parse_json_array, to_json_pretty};
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
    /// CIP code prefix to filter by (e.g. `\"11\"` for all CS, `\"11.01\"` for one family)
    #[schemars(description = "CIP code prefix (e.g. \"11\" for computer science)")]
    pub cip_prefix: Option<String>,
    /// Catalog year string (e.g. `\"2024-2025\"`)
    #[schemars(description = "Catalog year (e.g. \"2024-2025\")")]
    pub catalog_year: Option<String>,
    /// Maximum results to return (default 20, max 50)
    #[schemars(description = "Maximum results (default 20, max 50)")]
    pub limit: Option<usize>,
}

/// Request parameters for `get_degree`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetDegreeRequest {
    /// Unique degree identifier (e.g. `\"neu-khoury-bscs-2024\"`)
    #[schemars(description = "Unique degree ID to retrieve")]
    pub degree_id: String,
}

/// Request parameters for `compare_degrees`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDegreesRequest {
    /// Comma-separated degree IDs to compare (e.g. `\"neu-cs-2024,mit-cs-2024\"`)
    #[schemars(description = "Comma-separated degree IDs to compare")]
    pub degree_ids: String,
}

// ============================================================================
// Response types
// ============================================================================

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

    // cip_prefix uses `starts_with` (LIKE 'prefix%') pushed to Supabase
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
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetDegreeRequest) -> String {
    let filters = QueryFilters::new().eq("degree_id", Some(req.degree_id.as_str()));

    let result = match client
        .select(tables::DEGREES, DEGREE_DETAIL_COLS, &filters, Some(1))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let degree: Option<DegreeDetail> = result
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|item| serde_json::from_value(item.clone()).ok());

    match degree {
        Some(d) => to_json_pretty(&d),
        None => serde_json::json!({
            "error": "Degree not found",
            "degree_id": req.degree_id
        })
        .to_string(),
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

    // Each degree is fetched individually; tolerable for small comparison sets (≤10)
    for id in &ids {
        let filters = QueryFilters::new().eq("degree_id", Some(*id));
        let result = match client
            .select(tables::DEGREES, DEGREE_DETAIL_COLS, &filters, Some(1))
            .await
        {
            Ok(v) => v,
            Err(e) => return error_json(e),
        };

        if let Some(degree) = result
            .as_array()
            .and_then(|arr| arr.first())
            .and_then(|item| serde_json::from_value::<DegreeDetail>(item.clone()).ok())
        {
            degrees.push(degree);
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
