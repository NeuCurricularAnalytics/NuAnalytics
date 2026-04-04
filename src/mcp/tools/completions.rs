//! `get_completion_demographics` MCP tool
//!
//! Queries IPEDS completion data to produce demographic representation metrics.
//! The representation ratio is:
//! `cs_completion% / institution_total_completion%`
//! where 1.0 means proportional representation and <1 means underrepresented.

use std::collections::HashSet;
use std::sync::Arc;

use crate::core::database::models::DemographicRepresentation;
use crate::core::database::{tables, DbClient, QueryFilters};
// tables::INSTITUTION_COMPLETION_TOTALS used as denominator cache
use crate::mcp::tools::shared::{error_json, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Request parameters for `get_completion_demographics`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompletionDemographicsRequest {
    /// Carnegie classification code (15=R1, 16=R2, `None`=all)
    #[schemars(description = "Carnegie classification (15=R1, 16=R2, None=all)")]
    pub carnegie_class: Option<i32>,
    /// Control type: 1=public, 2=private nonprofit, `None`=all
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, None=all")]
    pub control: Option<i32>,
    /// Two-letter state abbreviation (e.g. `\"MA\"`)
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// CIP code prefix (default `\"11\"` for all Computer Science)
    #[schemars(
        description = "CIP code prefix (default \"11\" for CS, or \"30.70\" for Data Science)"
    )]
    pub cip_prefix: Option<String>,
    /// Award level: 5=bachelors, 7=masters, 9=doctoral, `None`=all
    #[schemars(description = "Award level: 5=bachelors, 7=masters, 9=doctoral, None=all")]
    pub award_level: Option<i32>,
    /// Academic year (e.g. 2023 for 2023-2024); uses latest available if `None`
    #[schemars(description = "Academic year (e.g. 2023). Uses most recent if None.")]
    pub year: Option<i32>,
    /// Include representation ratios comparing CS completions to all-major completion totals (default true)
    #[schemars(
        description = "Include representation ratios: CS completion% / institution total completion% (default true)"
    )]
    pub include_representation: Option<bool>,
}

/// Aggregated demographic counts across one or more records.
///
/// Used for both CS completions (`completions` table) and all-major totals
/// (`institution_completions` table) aggregation.
#[derive(Debug, Default)]
struct DemographicCounts {
    total: i64,
    total_men: i64,
    total_women: i64,
    nonresident_alien_men: i64,
    nonresident_alien_women: i64,
    hispanic_men: i64,
    hispanic_women: i64,
    american_indian_men: i64,
    american_indian_women: i64,
    asian_men: i64,
    asian_women: i64,
    black_men: i64,
    black_women: i64,
    native_hawaiian_men: i64,
    native_hawaiian_women: i64,
    white_men: i64,
    white_women: i64,
    two_or_more_men: i64,
    two_or_more_women: i64,
    unknown_race_men: i64,
    unknown_race_women: i64,
}

#[derive(Debug, Serialize)]
struct DemographicsResponse {
    filters: FilterSummary,
    institutions_matched: usize,
    total_completions: i64,
    demographics: Vec<DemographicRepresentation>,
}

#[derive(Debug, Serialize)]
struct FilterSummary {
    carnegie_class: Option<i32>,
    control: Option<i32>,
    state: Option<String>,
    cip_prefix: String,
    award_level: Option<i32>,
    year: Option<i32>,
}

/// Execute `get_completion_demographics` and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: CompletionDemographicsRequest) -> String {
    let cip_prefix = req.cip_prefix.clone().unwrap_or_else(|| "11".to_string());
    let include_representation = req.include_representation.unwrap_or(true);

    let institution_unitids = match get_matching_unitids(client, &req).await {
        Ok(ids) => ids,
        Err(e) => return error_json(e),
    };

    if institution_unitids.is_empty() {
        return serde_json::json!({
            "error": "No institutions found matching the given filters",
            "suggestion": "Try broadening institution filters (carnegie_class, control, state)"
        })
        .to_string();
    }

    // Build the set once and pass to both query functions to avoid double construction
    let unitid_set: HashSet<i32> = institution_unitids.iter().copied().collect();

    let completions = match get_counts(
        client,
        &unitid_set,
        &cip_prefix,
        req.award_level,
        req.year,
        tables::COMPLETIONS,
        "cip_code",
    )
    .await
    {
        Ok(c) => c,
        Err(e) => return error_json(e),
    };

    if completions.total == 0 {
        return serde_json::json!({
            "message": "No completion records found for the given filters",
            "institutions_checked": institution_unitids.len(),
            "cip_prefix": cip_prefix
        })
        .to_string();
    }

    // Denominator: use the pre-aggregated institution totals cache — much faster than
    // scanning the full completions table (100K+ rows) for every query.
    let enrollment = if include_representation {
        get_counts(
            client,
            &unitid_set,
            &cip_prefix,
            req.award_level,
            req.year,
            tables::INSTITUTION_COMPLETION_TOTALS,
            "",
        )
        .await
        .ok()
    } else {
        None
    };

    let response = DemographicsResponse {
        filters: FilterSummary {
            carnegie_class: req.carnegie_class,
            control: req.control,
            state: req.state,
            cip_prefix,
            award_level: req.award_level,
            year: req.year,
        },
        institutions_matched: institution_unitids.len(),
        total_completions: completions.total,
        demographics: build_demographics(&completions, enrollment.as_ref()),
    };

    to_json_pretty(&response)
}

// ============================================================================
// Query helpers
// ============================================================================

async fn get_matching_unitids(
    client: &Arc<DbClient>,
    req: &CompletionDemographicsRequest,
) -> Result<Vec<i32>, String> {
    let filters = QueryFilters::new()
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref());

    let result = client
        .select(tables::INSTITUTIONS, "unitid", &filters, Some(5000))
        .await
        .map_err(|e| e.to_string())?;

    let ids = result
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    item.get("unitid")
                        .and_then(serde_json::Value::as_i64)
                        .and_then(|v| i32::try_from(v).ok())
                })
                .collect()
        })
        .unwrap_or_default();

    Ok(ids)
}

/// Fetch and aggregate demographic counts from either `completions` or `enrollment`.
///
/// `cip_col` is the column to filter by CIP prefix; pass `""` to skip CIP filtering
/// (enrollment data has no CIP column).
async fn get_counts(
    client: &Arc<DbClient>,
    unitid_set: &HashSet<i32>,
    cip_prefix: &str,
    award_level: Option<i32>,
    year: Option<i32>,
    table: &'static str,
    cip_col: &'static str,
) -> Result<DemographicCounts, String> {
    let mut filters = QueryFilters::new()
        .eq("award_level", award_level)
        .eq("year", year);

    // Push CIP prefix filter to the server when querying completions
    if !cip_col.is_empty() {
        filters = filters.starts_with(cip_col, Some(cip_prefix));
    }

    let result = client
        .select(
            table,
            "unitid,cip_code,total,total_men,total_women,\
             nonresident_alien_men,nonresident_alien_women,\
             hispanic_men,hispanic_women,american_indian_men,american_indian_women,\
             asian_men,asian_women,black_men,black_women,native_hawaiian_men,native_hawaiian_women,\
             white_men,white_women,two_or_more_men,two_or_more_women,\
             unknown_race_men,unknown_race_women",
            &filters,
            Some(50_000),
        )
        .await
        .map_err(|e| e.to_string())?;

    let mut agg = DemographicCounts::default();

    if let Some(arr) = result.as_array() {
        for item in arr {
            let uid = item
                .get("unitid")
                .and_then(serde_json::Value::as_i64)
                .and_then(|v| i32::try_from(v).ok());
            if !uid.is_some_and(|u| unitid_set.contains(&u)) {
                continue;
            }

            macro_rules! add {
                ($field:ident) => {
                    agg.$field += item
                        .get(stringify!($field))
                        .and_then(|v| v.as_i64())
                        .unwrap_or(0);
                };
            }

            add!(total);
            add!(total_men);
            add!(total_women);
            add!(nonresident_alien_men);
            add!(nonresident_alien_women);
            add!(hispanic_men);
            add!(hispanic_women);
            add!(american_indian_men);
            add!(american_indian_women);
            add!(asian_men);
            add!(asian_women);
            add!(black_men);
            add!(black_women);
            add!(native_hawaiian_men);
            add!(native_hawaiian_women);
            add!(white_men);
            add!(white_women);
            add!(two_or_more_men);
            add!(two_or_more_women);
            add!(unknown_race_men);
            add!(unknown_race_women);
        }
    }

    Ok(agg)
}

// ============================================================================
// Demographic calculations
// ============================================================================

fn pct(part: i64, total: i64) -> f64 {
    if total == 0 {
        return 0.0;
    }
    // Demographic counts never exceed ~50M nationally, well within f64's exact integer
    // range (2^53), so the i64→f64 conversion is lossless in practice.
    #[allow(clippy::cast_precision_loss)]
    let result = part as f64 / total as f64 * 10_000.0;
    result.round() / 100.0
}

fn representation_ratio(comp_pct: f64, enroll_pct: f64) -> Option<f64> {
    if enroll_pct < 0.001 {
        None
    } else {
        Some((comp_pct / enroll_pct * 100.0).round() / 100.0)
    }
}

fn build_demographics(
    c: &DemographicCounts,
    e: Option<&DemographicCounts>,
) -> Vec<DemographicRepresentation> {
    let total_comp = c.total;
    let total_enroll = e.map(|e| e.total);

    macro_rules! group {
        ($label:expr, $comp_men:expr, $comp_women:expr, $enroll_men:expr, $enroll_women:expr) => {{
            let comp = $comp_men + $comp_women;
            let comp_pct = pct(comp, total_comp);
            let enrolled = e.map(|_| $enroll_men + $enroll_women);
            let enrollment_pct = enrolled.map(|n| pct(n, total_enroll.unwrap_or(0)));
            let ratio = enrollment_pct.and_then(|ep| representation_ratio(comp_pct, ep));
            DemographicRepresentation {
                group: $label.to_string(),
                completions: comp,
                total_completions: total_comp,
                completion_pct: comp_pct,
                enrolled,
                total_enrolled: total_enroll,
                enrollment_pct,
                representation_ratio: ratio,
            }
        }};
    }

    vec![
        group!("Women", 0, c.total_women, 0, e.map_or(0, |e| e.total_women)),
        group!("Men", c.total_men, 0, e.map_or(0, |e| e.total_men), 0),
        group!(
            "Hispanic/Latino",
            c.hispanic_men,
            c.hispanic_women,
            e.map_or(0, |e| e.hispanic_men),
            e.map_or(0, |e| e.hispanic_women)
        ),
        group!(
            "Black or African American",
            c.black_men,
            c.black_women,
            e.map_or(0, |e| e.black_men),
            e.map_or(0, |e| e.black_women)
        ),
        group!(
            "Asian",
            c.asian_men,
            c.asian_women,
            e.map_or(0, |e| e.asian_men),
            e.map_or(0, |e| e.asian_women)
        ),
        group!(
            "White",
            c.white_men,
            c.white_women,
            e.map_or(0, |e| e.white_men),
            e.map_or(0, |e| e.white_women)
        ),
        group!(
            "American Indian/Alaska Native",
            c.american_indian_men,
            c.american_indian_women,
            e.map_or(0, |e| e.american_indian_men),
            e.map_or(0, |e| e.american_indian_women)
        ),
        group!(
            "Native Hawaiian/Pacific Islander",
            c.native_hawaiian_men,
            c.native_hawaiian_women,
            e.map_or(0, |e| e.native_hawaiian_men),
            e.map_or(0, |e| e.native_hawaiian_women)
        ),
        group!(
            "Two or More Races",
            c.two_or_more_men,
            c.two_or_more_women,
            e.map_or(0, |e| e.two_or_more_men),
            e.map_or(0, |e| e.two_or_more_women)
        ),
        group!(
            "Nonresident Alien",
            c.nonresident_alien_men,
            c.nonresident_alien_women,
            e.map_or(0, |e| e.nonresident_alien_men),
            e.map_or(0, |e| e.nonresident_alien_women)
        ),
        group!(
            "Unknown Race/Ethnicity",
            c.unknown_race_men,
            c.unknown_race_women,
            e.map_or(0, |e| e.unknown_race_men),
            e.map_or(0, |e| e.unknown_race_women)
        ),
    ]
}
