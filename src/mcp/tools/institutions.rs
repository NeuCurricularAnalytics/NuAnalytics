//! `search_institutions` MCP tool

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{error_json, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Request parameters for `search_institutions`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchInstitutionsRequest {
    /// Institution name substring to search for (case-insensitive)
    #[schemars(description = "Institution name substring (case-insensitive)")]
    pub name: Option<String>,
    /// Two-letter state abbreviation (e.g. `\"MA\"`, `\"CA\"`)
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// Carnegie classification code (15=R1 doctoral, 16=R2 doctoral)
    #[schemars(description = "Carnegie classification code (15=R1, 16=R2)")]
    pub carnegie_class: Option<i32>,
    /// Control type (1=public, 2=private nonprofit, 3=for-profit)
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, 3=for-profit")]
    pub control: Option<i32>,
    /// If true, return only HBCUs
    #[schemars(description = "Filter to Historically Black Colleges and Universities")]
    pub hbcu: Option<bool>,
    /// Maximum results to return (default 25, max 100)
    #[schemars(description = "Maximum results (default 25, max 100)")]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct InstitutionResult {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    iclevel: Option<i32>,
    hbcu: Option<bool>,
}

#[derive(Debug, Serialize)]
struct SearchInstitutionsResponse {
    count: usize,
    institutions: Vec<InstitutionResult>,
}

/// Execute the `search_institutions` tool and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: SearchInstitutionsRequest) -> String {
    let limit = req.limit.unwrap_or(25).min(100);

    // Equality and ilike filters are pushed to Supabase; name uses ilike for
    // case-insensitive substring matching. hbcu uses bool's Display ("true"/"false").
    let filters = QueryFilters::new()
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref())
        .eq("hbcu", req.hbcu)
        .ilike("name", req.name.as_deref());

    let result = match client
        .select(
            tables::INSTITUTIONS,
            "unitid,name,city,state,carnegie_class,control,iclevel,hbcu",
            &filters,
            Some(limit),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institutions: Vec<InstitutionResult> = parse_json_array(&result);

    to_json_pretty(&SearchInstitutionsResponse {
        count: institutions.len(),
        institutions,
    })
}
