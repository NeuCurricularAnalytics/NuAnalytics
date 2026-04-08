//! `get_lookup_codes` MCP tool
//!
//! Queries the IPEDS lookup tables so an AI model can discover what numeric codes
//! mean before making filtered queries. The database has 7 lookup tables populated
//! from `lookup-seed.sql`.
//!
//! ## Common codes (inline for quick reference)
//!
//! | Parameter | Key codes |
//! |-----------|-----------|
//! | `carnegie_class` | 15=R1, 16=R2, 21=R1-2021, 22=R2-2021 |
//! | `award_level` | 3=associate, 5=bachelors, 7=masters, 9=doctoral |
//! | `control` | 1=public, 2=private nonprofit, 3=for-profit |
//! | `inst_size` | 1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+ |

use std::sync::Arc;

use crate::core::database::{DbClient, QueryFilters};
use crate::mcp::tools::shared::{error_json, to_json_pretty};
use rmcp::schemars;
use serde::Deserialize;

/// Names of queryable lookup tables
const KNOWN_TABLES: &[&str] = &[
    "award_levels",
    "carnegie_class",
    "institution_control",
    "institution_level",
    "institution_sector",
    "institution_locale",
    "institution_size",
];

/// Request parameters for `get_lookup_codes`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetLookupCodesRequest {
    /// Which lookup table to query. Options: `\"award_levels\"`, `\"carnegie_class\"`,
    /// `\"institution_control\"`, `\"institution_level\"`, `\"institution_sector\"`,
    /// `\"institution_locale\"`, `\"institution_size\"`
    #[schemars(
        description = "Lookup table: \"award_levels\", \"carnegie_class\", \"institution_control\", \"institution_level\", \"institution_sector\", \"institution_locale\", \"institution_size\""
    )]
    pub table: String,
}

/// Execute `get_lookup_codes` and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: GetLookupCodesRequest) -> String {
    let table = req.table.trim();

    if !KNOWN_TABLES.contains(&table) {
        return serde_json::json!({
            "error": format!("Unknown lookup table: \"{}\"", table),
            "valid_tables": KNOWN_TABLES,
            "tip": "Common usage: get_lookup_codes(\"carnegie_class\") to find R1/R2 codes before filtering institutions"
        })
        .to_string();
    }

    let result = match client.select(table, "*", &QueryFilters::new(), None).await {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    to_json_pretty(&serde_json::json!({
        "table": table,
        "rows": result
    }))
}
