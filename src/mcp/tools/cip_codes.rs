//! `search_cip_codes` MCP tool
//!
//! CIP codes (Classification of Instructional Programs) are 6-digit dot-notation codes
//! used to classify academic programs. This tool searches the `cip_codes` lookup table.
//!
//! ## CIP prefix convention
//!
//! Codes are stored in dot notation (e.g. `"11.0101"`). Use `starts_with` prefix matching:
//! - `"11."` → all Computer & Information Sciences (family 11)
//! - `"11.01."` → CS General sub-family
//! - `"11.0101"` → exact code (CS General)
//! - `"30.70"` → Data Science area (`30.7001`, `30.7099`)
//!
//! Use a **trailing dot** for family/sub-family queries to avoid partial number matches.

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{self, error_json, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Request parameters for `search_cip_codes`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct SearchCipCodesRequest {
    /// Keyword search on program title (case-insensitive substring). E.g. `\"computer science\"`, `\"cybersecurity\"`, `\"data science\"`.
    #[schemars(
        description = "Keyword search on program title (e.g. \"cybersecurity\", \"data science\")"
    )]
    pub query: Option<String>,
    /// CIP code prefix in dot notation. Use trailing dot for families: `\"11.\"` all CS, `\"11.01.\"` sub-family, `\"11.0101\"` exact.
    #[schemars(
        description = "CIP prefix in dot notation: \"11.\" all CS, \"11.01.\" sub-family, \"11.0101\" exact match"
    )]
    pub prefix: Option<String>,
    /// Maximum results to return (default 25, max 100)
    #[schemars(description = "Maximum results (default 25, max 100)")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, Deserialize)]
struct CipCodeResult {
    cip_code: String,
    title: String,
}

#[derive(Debug, Serialize)]
struct SearchCipCodesResponse {
    count: usize,
    cip_codes: Vec<CipCodeResult>,
}

/// Execute `search_cip_codes` and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: SearchCipCodesRequest) -> String {
    if req.query.is_none() && req.prefix.is_none() {
        return serde_json::json!({
            "error": "Provide at least one of: query (title keyword) or prefix (CIP code prefix)",
            "examples": {
                "query": "search_cip_codes {\"query\": \"computer science\"}",
                "prefix": "search_cip_codes {\"prefix\": \"11.\"}",
                "combined": "search_cip_codes {\"query\": \"cybersecurity\", \"prefix\": \"11.\"}"
            }
        })
        .to_string();
    }

    let limit = req.limit.unwrap_or(25).min(100);

    let filters = QueryFilters::new()
        .starts_with("cip_code", req.prefix.as_deref())
        .ilike("title", req.query.as_deref());

    let result = match client
        .select(tables::CIP_CODES, "cip_code,title", &filters, Some(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let cip_codes: Vec<CipCodeResult> = parse_json_array(&result);

    to_json_pretty(&SearchCipCodesResponse {
        count: cip_codes.len(),
        cip_codes,
    })
}
