//! `search_institutions` and `get_institution` MCP tools

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{self, error_json, parse_first, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request types
// ============================================================================

/// Parameters for filtering and searching IPEDS institutions.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchInstitutionsRequest {
    /// Institution name substring to search for (case-insensitive)
    #[schemars(description = "Institution name substring (case-insensitive)")]
    pub name: Option<String>,
    /// Two-letter state abbreviation (e.g. `\"MA\"`, `\"CA\"`)
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// Carnegie classification code (15=R1 doctoral, 16=R2 doctoral, 21=R1 2021-scheme, 22=R2 2021-scheme). Use `get_lookup_codes` for full list.
    #[schemars(
        description = "Carnegie classification (15=R1, 16=R2, 21=R1-2021, 22=R2-2021). Use get_lookup_codes(\"carnegie_class\") for full list."
    )]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub carnegie_class: Option<i32>,
    /// Control type (1=public, 2=private nonprofit, 3=for-profit)
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, 3=for-profit")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub control: Option<i32>,
    /// If true, return only HBCUs
    #[schemars(description = "Filter to Historically Black Colleges and Universities")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub hbcu: Option<bool>,
    /// If true, return only Tribal colleges
    #[schemars(description = "Filter to Tribal colleges and universities")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub tribal: Option<bool>,
    /// Minimum institution size bucket (1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+)
    #[schemars(
        description = "Minimum size bucket: 1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+"
    )]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub inst_size_min: Option<i32>,
    /// Maximum results to return (default 25, max 100)
    #[schemars(description = "Maximum results (default 25, max 100)")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

/// Request parameters for `get_institution`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetInstitutionRequest {
    /// IPEDS Unit ID of the institution (from `search_institutions`)
    #[schemars(description = "IPEDS Unit ID of the institution")]
    pub unitid: i32,
}

// ============================================================================
// Response types
// ============================================================================

/// Summary fields returned by `search_institutions` — no sector, locale, or `updated_year`.
#[derive(Debug, Serialize, Deserialize)]
struct InstitutionSummary {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    iclevel: Option<i32>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    inst_size: Option<i32>,
}

/// Full institution record returned by `get_institution` (all columns).
#[derive(Debug, Serialize, Deserialize)]
struct InstitutionDetail {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    sector: Option<i32>,
    control: Option<i32>,
    iclevel: Option<i32>,
    carnegie_class: Option<i32>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    locale: Option<i32>,
    inst_size: Option<i32>,
    updated_year: Option<i32>,
}

/// Response wrapper for `search_institutions`.
#[derive(Debug, Serialize)]
struct SearchInstitutionsResponse {
    count: usize,
    institutions: Vec<InstitutionSummary>,
}

// ============================================================================
// Execute functions
// ============================================================================

/// Execute the `search_institutions` tool and return JSON.
pub async fn execute_search_json(client: &Arc<DbClient>, req: SearchInstitutionsRequest) -> String {
    let limit = req.limit.unwrap_or(25).min(100);

    let filters = QueryFilters::new()
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref())
        .eq("hbcu", req.hbcu)
        .eq("tribal", req.tribal)
        .gte("inst_size", req.inst_size_min)
        .ilike("name", req.name.as_deref());

    let result = match client
        .select(
            tables::INSTITUTIONS,
            "unitid,name,city,state,carnegie_class,control,iclevel,hbcu,tribal,inst_size",
            &filters,
            Some(limit),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institutions: Vec<InstitutionSummary> = parse_json_array(&result);

    to_json_pretty(&SearchInstitutionsResponse {
        count: institutions.len(),
        institutions,
    })
}

/// Execute the `get_institution` tool and return JSON.
pub async fn execute_get_json(client: &Arc<DbClient>, req: GetInstitutionRequest) -> String {
    let filters = QueryFilters::new().eq("unitid", Some(req.unitid));

    let result = match client
        .select(tables::INSTITUTIONS, "*", &filters, Some(1))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institution: Option<InstitutionDetail> = parse_first(&result);

    institution.map_or_else(
        || {
            serde_json::json!({
                "error": "Institution not found",
                "unitid": req.unitid
            })
            .to_string()
        },
        |inst| to_json_pretty(&inst),
    )
}
