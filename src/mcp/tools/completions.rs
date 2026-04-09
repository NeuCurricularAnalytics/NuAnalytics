//! Completion demographics MCP tools
//!
//! Three tools:
//!
//! - `get_completion_demographics` — aggregate CS demographics across a filtered set of institutions
//! - `get_institution_completions` — all completions for a single institution with per-CIP representation ratios
//! - `get_schools_completion_demographics` — bulk per-institution demographics for many schools (batched DB calls)
//!
//! ## Representation ratio
//!
//! All tools compute: `(group_cs_completions / total_cs_completions) / (group_all_completions / total_all_completions)`
//!
//! A ratio of 1.0 means the group is proportionally represented in CS relative to the institution's
//! overall completion profile. Values <1 indicate underrepresentation, >1 overrepresentation.

use std::collections::HashMap;
use std::sync::Arc;

use crate::core::database::models::DemographicRepresentation;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{error_json, parse_comma_list, parse_json_array, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

/// Max institutions per `IN(...)` query batch.
///
/// Supabase's Cloudflare Worker proxy crashes when a single `PostgREST` response exceeds
/// a few MB. With all CIP codes stored, 150 R1 schools × many CIP rows can easily hit
/// that. Batching keeps each response small and independently retrievable.
const COMPLETIONS_BATCH_SIZE: usize = 40;

/// Demographic columns with `unitid` prefix — used when grouping by institution.
const DEMO_COLS_WITH_UNITID: &str = "unitid,total,total_men,total_women,\
    nonresident_alien_men,nonresident_alien_women,\
    hispanic_men,hispanic_women,american_indian_men,american_indian_women,\
    asian_men,asian_women,black_men,black_women,native_hawaiian_men,native_hawaiian_women,\
    white_men,white_women,two_or_more_men,two_or_more_women,\
    unknown_race_men,unknown_race_women";

/// Demographic columns without any key prefix — used when fetching totals for a single unit.
const DEMO_COLS_NO_KEY: &str = "total,total_men,total_women,\
    nonresident_alien_men,nonresident_alien_women,\
    hispanic_men,hispanic_women,american_indian_men,american_indian_women,\
    asian_men,asian_women,black_men,black_women,native_hawaiian_men,native_hawaiian_women,\
    white_men,white_women,two_or_more_men,two_or_more_women,\
    unknown_race_men,unknown_race_women";

// ============================================================================
// Request types
// ============================================================================

/// Request parameters for `get_completion_demographics`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompletionDemographicsRequest {
    /// Filter to a single institution by IPEDS Unit ID (overrides institution group filters)
    #[schemars(description = "IPEDS Unit ID — filter to one institution")]
    pub unitid: Option<i32>,
    /// Carnegie classification code (15=R1, 16=R2, 21=R1-2021, 22=R2-2021). Use `get_lookup_codes` for full list.
    #[schemars(
        description = "Carnegie classification (15=R1, 16=R2, 21=R1-2021). Use get_lookup_codes(\"carnegie_class\")."
    )]
    pub carnegie_class: Option<i32>,
    /// Control type: 1=public, 2=private nonprofit, `None`=all
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, None=all")]
    pub control: Option<i32>,
    /// Two-letter state abbreviation (e.g. `\"MA\"`)
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// CIP code prefix using dot notation with trailing dot for families (default `\"11.\"` for all CS).
    /// Examples: `\"11.\"` all CS, `\"11.01.\"` CS General sub-family, `\"30.70\"` Data Science.
    /// Use `cip_codes` instead when you need exact codes rather than a prefix range.
    #[schemars(
        description = "CIP prefix (dot notation): \"11.\" all CS, \"30.70\" Data Science. Omit for all CIPs."
    )]
    pub cip_prefix: Option<String>,
    /// Comma-separated exact CIP codes (dot notation). Takes priority over `cip_prefix`.
    /// Use when you need specific codes rather than a whole family, e.g. `\"11.0101,11.0701\"`.
    #[schemars(
        description = "Comma-separated exact CIP codes (dot notation), e.g. \"11.0101,11.0701\". Takes priority over cip_prefix."
    )]
    pub cip_codes: Option<String>,
    /// Award level: 5=bachelors, 7=masters, 9=doctoral, `None`=all. Use `get_lookup_codes("award_levels")` for full list.
    #[schemars(
        description = "Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral, None=all"
    )]
    pub award_level: Option<i32>,
    /// Academic year (e.g. 2024 for 2023-2024); uses all years if `None` (provide year for accurate ratios)
    #[schemars(
        description = "Academic year (e.g. 2024). Provide for accurate representation ratios."
    )]
    pub year: Option<i32>,
    /// Include representation ratios comparing CS completions to all-major completion totals (default true)
    #[schemars(
        description = "Include representation ratios: CS completion% / institution total completion% (default true)"
    )]
    pub include_representation: Option<bool>,
}

/// Request parameters for `get_institution_completions`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetInstitutionCompletionsRequest {
    /// IPEDS Unit ID of the institution (from `search_institutions`)
    #[schemars(description = "IPEDS Unit ID of the institution")]
    pub unitid: i32,
    /// Academic year (e.g. 2024). Provide for accurate representation ratios.
    #[schemars(description = "Academic year (e.g. 2024). Omit to aggregate all available years.")]
    pub year: Option<i32>,
    /// Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral, None=all
    #[schemars(
        description = "Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral, None=all"
    )]
    pub award_level: Option<i32>,
    /// CIP code prefix in dot notation with trailing dot for families (e.g. `\"11.\"` for all CS, `\"11.01.\"` for CS General).
    /// Omit for all CIP codes at this institution. Use `cip_codes` for exact code lists.
    #[schemars(
        description = "CIP prefix (\"11.\" all CS, \"11.01.\" sub-family). Omit for all programs."
    )]
    pub cip_prefix: Option<String>,
    /// Comma-separated exact CIP codes (dot notation). Takes priority over `cip_prefix`.
    /// E.g. `\"11.0101,11.0701\"` for CS General + Computer Science.
    #[schemars(
        description = "Comma-separated exact CIP codes, e.g. \"11.0101,11.0701\". Takes priority over cip_prefix."
    )]
    pub cip_codes: Option<String>,
    /// Major number: 1=primary major only (default), 2=second major, None=both
    #[schemars(description = "Major number: 1=primary (default), 2=second major, None=both")]
    pub major_num: Option<i32>,
    /// Include representation ratios comparing each CIP row to institution-wide totals (default true)
    #[schemars(
        description = "Include representation ratios vs. school-wide completion profile (default true)"
    )]
    pub include_representation: Option<bool>,
}

/// Request parameters for `get_schools_completion_demographics`
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSchoolsCompletionDemographicsRequest {
    // ── Institution filters ──
    /// IPEDS Unit ID — filter to a single institution. Takes priority over all other institution filters.
    #[schemars(
        description = "IPEDS Unit ID — target a single school. Takes priority over carnegie_class, control, state, etc."
    )]
    pub unitid: Option<i32>,
    /// Carnegie classification (15=R1, 16=R2, 21=R1-2021). Use `get_lookup_codes("carnegie_class")` for full list.
    #[schemars(
        description = "Carnegie classification (15=R1, 16=R2, 21=R1-2021). Use get_lookup_codes(\"carnegie_class\")."
    )]
    pub carnegie_class: Option<i32>,
    /// Control type: 1=public, 2=private nonprofit, 3=for-profit
    #[schemars(description = "Control type: 1=public, 2=private nonprofit, 3=for-profit")]
    pub control: Option<i32>,
    /// Two-letter state abbreviation
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// Filter to HBCUs only
    #[schemars(description = "Filter to Historically Black Colleges and Universities")]
    pub hbcu: Option<bool>,
    /// Filter to Tribal colleges only
    #[schemars(description = "Filter to Tribal colleges")]
    pub tribal: Option<bool>,
    /// Minimum institution size bucket (1=<1000, 2=1000-4999, 3=5000-9999, 4=10000-19999, 5=20000+)
    #[schemars(
        description = "Minimum size bucket: 2=\"above 1000 students\", 3=\"above 5000\", etc."
    )]
    pub inst_size_min: Option<i32>,

    // ── Completion filters ──
    /// CIP code prefix in dot notation (e.g. `\"11.\"` for all CS). Omit for all CIPs.
    /// Use `cip_codes` for an exact list of codes instead.
    #[schemars(
        description = "CIP prefix (\"11.\" all CS, \"30.70\" Data Science). Omit for all programs."
    )]
    pub cip_prefix: Option<String>,
    /// Comma-separated exact CIP codes (dot notation). Takes priority over `cip_prefix`.
    /// E.g. `\"11.0101,11.0701\"` to query specific programs across all matched schools.
    #[schemars(
        description = "Comma-separated exact CIP codes, e.g. \"11.0101,11.0701\". Takes priority over cip_prefix."
    )]
    pub cip_codes: Option<String>,
    /// Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral, None=all
    #[schemars(
        description = "Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral, None=all"
    )]
    pub award_level: Option<i32>,
    /// Academic year (e.g. 2024). Strongly recommended for accurate representation ratios.
    #[schemars(
        description = "Academic year (e.g. 2024). Strongly recommended for accurate ratios."
    )]
    pub year: Option<i32>,

    // ── Output options ──
    /// Include representation ratios (default true)
    #[schemars(
        description = "Include representation ratios vs. school-wide completions (default true)"
    )]
    pub include_representation: Option<bool>,
    /// Skip schools with fewer than this many completions in the filtered results (post-aggregation)
    #[schemars(
        description = "Skip schools with fewer total completions than this threshold (post-filter)"
    )]
    pub min_completions: Option<i64>,
    /// Maximum schools to return (default 50, max 200)
    #[schemars(description = "Maximum schools to return (default 50, max 200)")]
    pub limit: Option<usize>,
}

// ============================================================================
// CIP code filter
// ============================================================================

/// How to filter completion rows by CIP code.
///
/// - `Prefix` — LIKE pattern, e.g. `"11."` → all CS family codes.
/// - `Codes` — exact IN list, e.g. `["11.0101", "11.0701"]`.
///
/// The two are mutually exclusive; `Codes` takes priority if both are present.
enum CipFilter<'a> {
    /// LIKE prefix match — e.g. `"11."` matches all CS family codes.
    Prefix(&'a str),
    /// Exact IN list — e.g. `["11.0101", "11.0701"]` for specific programs.
    Codes(&'a [String]),
}

/// Parse a comma-separated CIP codes string. Thin wrapper over [`parse_comma_list`].
fn parse_cip_codes(s: &str) -> Vec<String> {
    parse_comma_list(s)
}

/// Apply `cip_filter` to `filters` for the given column. No-op when `cip_col` is empty.
fn apply_cip_filter(
    filters: QueryFilters,
    cip_filter: Option<&CipFilter<'_>>,
    cip_col: &'static str,
) -> QueryFilters {
    if cip_col.is_empty() {
        return filters;
    }
    match cip_filter {
        Some(CipFilter::Prefix(p)) => filters.starts_with(cip_col, Some(*p)),
        Some(CipFilter::Codes(codes)) => filters.in_list(cip_col, codes),
        None => filters,
    }
}

// ============================================================================
// Shared demographic accumulator
// ============================================================================

/// Aggregated demographic counts — accumulates across multiple rows.
#[derive(Debug, Default, Clone)]
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

/// Accumulate demographic fields from a JSON row into a `DemographicCounts` aggregator.
fn accumulate(agg: &mut DemographicCounts, item: &serde_json::Value) {
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

// ============================================================================
// Shared response types
// ============================================================================

/// Gender breakdown within a single racial/ethnic group.
///
/// This is the cross-tabulation layer: for each race group you can see both
/// gender parity within the group (`women_pct_within_group`) and how each
/// gender-race combination compares to the institution's overall profile
/// (`women_representation_ratio`, `men_representation_ratio`).
#[derive(Debug, Serialize)]
pub struct CrossTabRow {
    /// Racial/ethnic group (e.g. "Hispanic/Latino", "Black or African American")
    pub group: &'static str,
    /// Number of women completers in this race group
    pub women_count: i64,
    /// Number of men completers in this race group
    pub men_count: i64,
    /// % of this race group that are women — gender parity within race
    /// (e.g. 38.0 means 38 % of Hispanic CS graduates are women)
    pub women_pct_within_group: f64,
    /// Women of this race as % of **all** CS completions
    pub women_pct_of_total: f64,
    /// Men of this race as % of **all** CS completions
    pub men_pct_of_total: f64,
    /// Representation ratio for women: `(women_of_race / total_cs) / (women_of_race_inst / total_inst)`.
    /// 1.0 = proportional. <1 = underrepresented relative to institution baseline. `None` if no
    /// institution totals are available.
    pub women_representation_ratio: Option<f64>,
    /// Representation ratio for men (same formula as women).
    pub men_representation_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct DemographicsResponse {
    filters: FilterSummary,
    institutions_matched: usize,
    total_completions: i64,
    demographics: Vec<DemographicRepresentation>,
    /// Race × gender cross-tabulation — gender parity within each racial group
    /// and representation ratios for each gender-race combination.
    cross_tab: Vec<CrossTabRow>,
}

#[derive(Debug, Serialize)]
struct FilterSummary {
    unitid: Option<i32>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    state: Option<String>,
    cip_prefix: String,
    award_level: Option<i32>,
    year: Option<i32>,
}

// ============================================================================
// get_completion_demographics
// ============================================================================

/// Execute `get_completion_demographics` and return JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: CompletionDemographicsRequest) -> String {
    // Build CIP filter. Exact codes take priority over prefix.
    // When neither is supplied the filter is None → all CIPs are returned.
    // Callers who want only CS must pass cip_prefix="11." explicitly.
    let cip_codes_vec: Vec<String> = req
        .cip_codes
        .as_deref()
        .map(parse_cip_codes)
        .unwrap_or_default();
    let cip_filter: Option<CipFilter<'_>> = if cip_codes_vec.is_empty() {
        req.cip_prefix.as_deref().map(CipFilter::Prefix)
    } else {
        Some(CipFilter::Codes(&cip_codes_vec))
    };
    let cip_label = match &cip_filter {
        Some(CipFilter::Codes(c)) => c.join(","),
        Some(CipFilter::Prefix(p)) => (*p).to_string(),
        None => "(all CIPs)".to_string(),
    };

    let include_representation = req.include_representation.unwrap_or(true);

    let institution_unitids = match get_matching_unitids(client, &req).await {
        Ok(ids) => ids,
        Err(e) => return error_json(e),
    };

    if institution_unitids.is_empty() {
        return serde_json::json!({
            "error": "No institutions found matching the given filters",
            "suggestion": "Try broadening institution filters (carnegie_class, control, state, unitid)"
        })
        .to_string();
    }

    let unitid_set: std::collections::HashSet<i32> = institution_unitids.iter().copied().collect();

    let completions = match get_counts(
        client,
        &unitid_set,
        cip_filter.as_ref(),
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
            "cip_filter": cip_label
        })
        .to_string();
    }

    let enrollment = if include_representation {
        // Totals table has no CIP column — pass None CIP filter
        get_counts(
            client,
            &unitid_set,
            None,
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

    to_json_pretty(&DemographicsResponse {
        filters: FilterSummary {
            unitid: req.unitid,
            carnegie_class: req.carnegie_class,
            control: req.control,
            state: req.state,
            cip_prefix: cip_label,
            award_level: req.award_level,
            year: req.year,
        },
        institutions_matched: institution_unitids.len(),
        total_completions: completions.total,
        demographics: build_demographics(&completions, enrollment.as_ref()),
        cross_tab: build_cross_tab(&completions, enrollment.as_ref()),
    })
}

async fn get_matching_unitids(
    client: &Arc<DbClient>,
    req: &CompletionDemographicsRequest,
) -> Result<Vec<i32>, String> {
    // Single-institution shortcut
    if let Some(uid) = req.unitid {
        return Ok(vec![uid]);
    }

    let filters = QueryFilters::new()
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref());

    let result = client
        .select(tables::INSTITUTIONS, "unitid", &filters, Some(5000))
        .await
        .map_err(|e| e.to_string())?;

    Ok(result
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
        .unwrap_or_default())
}

/// Fetch and aggregate demographic counts using an in-memory `unitid_set` filter.
///
/// `cip_col` is the column to filter by CIP; pass `""` to skip CIP filtering.
async fn get_counts(
    client: &Arc<DbClient>,
    unitid_set: &std::collections::HashSet<i32>,
    cip_filter: Option<&CipFilter<'_>>,
    award_level: Option<i32>,
    year: Option<i32>,
    table: &'static str,
    cip_col: &'static str,
) -> Result<DemographicCounts, String> {
    let filters = apply_cip_filter(
        QueryFilters::new()
            .eq("award_level", award_level)
            .eq("year", year),
        cip_filter,
        cip_col,
    );

    let result = client
        .select(
            table,
            &format!("unitid,{DEMO_COLS_NO_KEY}"),
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
            accumulate(&mut agg, item);
        }
    }

    Ok(agg)
}

// ============================================================================
// get_institution_completions
// ============================================================================

#[derive(Debug, Serialize)]
struct RowDemographic {
    group: String,
    count: i64,
    cip_pct: f64,
    school_pct: Option<f64>,
    representation_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CompletionRow {
    cip_code: String,
    cip_title: Option<String>,
    award_level: Option<i32>,
    major_num: Option<i32>,
    total: i64,
    demographics: Vec<RowDemographic>,
}

#[derive(Debug, Serialize)]
struct InstitutionCompletionsResponse {
    unitid: i32,
    name: Option<String>,
    year: Option<i32>,
    award_level: Option<i32>,
    cip_prefix: Option<String>,
    total_rows: usize,
    note: &'static str,
    rows: Vec<CompletionRow>,
    /// Race × gender cross-tabulation aggregated across all selected CIP codes.
    cross_tab: Vec<CrossTabRow>,
}

/// Execute `get_institution_completions` and return JSON.
pub async fn execute_institution_json(
    client: &Arc<DbClient>,
    req: GetInstitutionCompletionsRequest,
) -> String {
    let include_representation = req.include_representation.unwrap_or(true);

    let inst_name = fetch_institution_name(client, req.unitid).await;

    let cip_codes_vec: Vec<String> = req
        .cip_codes
        .as_deref()
        .map(parse_cip_codes)
        .unwrap_or_default();
    let cip_filter: Option<CipFilter<'_>> = if cip_codes_vec.is_empty() {
        req.cip_prefix.as_deref().map(CipFilter::Prefix)
    } else {
        Some(CipFilter::Codes(&cip_codes_vec))
    };

    let comp_filters = apply_cip_filter(
        QueryFilters::new()
            .eq("unitid", Some(req.unitid))
            .eq("award_level", req.award_level)
            .eq("year", req.year)
            .eq("major_num", req.major_num), // None = no filter
        cip_filter.as_ref(),
        "cip_code",
    );

    let comp_result = match client
        .select(
            tables::COMPLETIONS,
            &format!("cip_code,award_level,major_num,{DEMO_COLS_NO_KEY}"),
            &comp_filters,
            Some(2000),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let rows_raw = comp_result.as_array().cloned().unwrap_or_default();
    if rows_raw.is_empty() {
        return serde_json::json!({
            "unitid": req.unitid,
            "name": inst_name,
            "message": "No completion records found for the given filters"
        })
        .to_string();
    }

    let cip_codes: Vec<String> = rows_raw
        .iter()
        .filter_map(|item| {
            item.get("cip_code")
                .and_then(serde_json::Value::as_str)
                .map(String::from)
        })
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let cip_titles = fetch_cip_titles(client, &cip_codes).await;

    let school_totals = if include_representation {
        fetch_school_totals_for(client, req.unitid, req.award_level, req.year).await
    } else {
        None
    };

    let rows: Vec<CompletionRow> = rows_raw
        .iter()
        .map(|item| build_completion_row(item, &cip_titles, school_totals.as_ref()))
        .collect();

    let total_rows = rows.len();

    // Aggregate all selected CIP rows into one DemographicCounts for the cross-tab
    let mut agg = DemographicCounts::default();
    for item in &rows_raw {
        accumulate(&mut agg, item);
    }
    let cross_tab = build_cross_tab(&agg, school_totals.as_ref());

    to_json_pretty(&InstitutionCompletionsResponse {
        unitid: req.unitid,
        name: inst_name,
        year: req.year,
        award_level: req.award_level,
        cip_prefix: req.cip_prefix,
        total_rows,
        note: "school_pct and representation_ratio compare this CIP row to institution-wide completion totals",
        rows,
        cross_tab,
    })
}

/// Build a single `CompletionRow` from a raw JSON completion record.
fn build_completion_row(
    item: &serde_json::Value,
    cip_titles: &HashMap<String, String>,
    school_totals: Option<&DemographicCounts>,
) -> CompletionRow {
    let cip_code = item
        .get("cip_code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let cip_title = cip_titles.get(&cip_code).cloned();
    let award_level = item
        .get("award_level")
        .and_then(serde_json::Value::as_i64)
        .and_then(|v| i32::try_from(v).ok());
    let major_num = item
        .get("major_num")
        .and_then(serde_json::Value::as_i64)
        .and_then(|v| i32::try_from(v).ok());
    let total = item
        .get("total")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(0);

    CompletionRow {
        cip_code,
        cip_title,
        award_level,
        major_num,
        total,
        demographics: build_row_demographics(item, total, school_totals),
    }
}

/// Build per-row demographic breakdown comparing this row to school totals.
fn build_row_demographics(
    item: &serde_json::Value,
    row_total: i64,
    school: Option<&DemographicCounts>,
) -> Vec<RowDemographic> {
    let school_total = school.map(|s| s.total);

    // Single-field gender group (total_women or total_men — not a men+women pair)
    macro_rules! gender_group {
        ($label:expr, $field:ident, $sf:ident) => {{
            let count = item
                .get(stringify!($field))
                .and_then(|v| v.as_i64())
                .unwrap_or(0);
            let cip_pct = pct(count, row_total);
            let school_pct = school.map(|s| pct(s.$sf, school_total.unwrap_or(0)));
            RowDemographic {
                group: $label.to_string(),
                count,
                cip_pct,
                school_pct,
                representation_ratio: school_pct.and_then(|sp| representation_ratio(cip_pct, sp)),
            }
        }};
    }

    // Race+ethnicity groups (men + women summed)
    macro_rules! demo_group {
        ($label:expr, $men:ident, $women:ident, $sm:ident, $sw:ident) => {{
            let count = item
                .get(stringify!($men))
                .and_then(|v| v.as_i64())
                .unwrap_or(0)
                + item
                    .get(stringify!($women))
                    .and_then(|v| v.as_i64())
                    .unwrap_or(0);
            let cip_pct = pct(count, row_total);
            let school_pct = school.map(|s| pct(s.$sm + s.$sw, school_total.unwrap_or(0)));
            RowDemographic {
                group: $label.to_string(),
                count,
                cip_pct,
                school_pct,
                representation_ratio: school_pct.and_then(|sp| representation_ratio(cip_pct, sp)),
            }
        }};
    }

    vec![
        gender_group!("Women", total_women, total_women),
        gender_group!("Men", total_men, total_men),
        demo_group!(
            "Hispanic/Latino",
            hispanic_men,
            hispanic_women,
            hispanic_men,
            hispanic_women
        ),
        demo_group!(
            "Black or African American",
            black_men,
            black_women,
            black_men,
            black_women
        ),
        demo_group!("Asian", asian_men, asian_women, asian_men, asian_women),
        demo_group!("White", white_men, white_women, white_men, white_women),
        demo_group!(
            "American Indian/Alaska Native",
            american_indian_men,
            american_indian_women,
            american_indian_men,
            american_indian_women
        ),
        demo_group!(
            "Native Hawaiian/Pacific Islander",
            native_hawaiian_men,
            native_hawaiian_women,
            native_hawaiian_men,
            native_hawaiian_women
        ),
        demo_group!(
            "Two or More Races",
            two_or_more_men,
            two_or_more_women,
            two_or_more_men,
            two_or_more_women
        ),
        demo_group!(
            "Nonresident Alien",
            nonresident_alien_men,
            nonresident_alien_women,
            nonresident_alien_men,
            nonresident_alien_women
        ),
        demo_group!(
            "Unknown Race/Ethnicity",
            unknown_race_men,
            unknown_race_women,
            unknown_race_men,
            unknown_race_women
        ),
    ]
}

// ============================================================================
// get_schools_completion_demographics
// ============================================================================

#[derive(Debug, Serialize, Deserialize)]
struct InstitutionMeta {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    inst_size: Option<i32>,
}

#[derive(Debug, Serialize)]
struct SchoolDemographicsResult {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    year: Option<i32>,
    total_completions: i64,
    demographics: Vec<DemographicRepresentation>,
    /// Race × gender cross-tabulation for this school's CS completions.
    cross_tab: Vec<CrossTabRow>,
}

/// Execute `get_schools_completion_demographics` and return JSON.
///
/// Uses the universal 3-call join pattern:
/// 1. Resolve institution filters → list of unitids
/// 2. Fetch completions `WHERE unitid IN (...)`
/// 3. Fetch `institution_completion_totals` `WHERE unitid IN (...)`
/// 4. Aggregate and compute representation ratios in Rust
pub async fn execute_schools_json(
    client: &Arc<DbClient>,
    req: GetSchoolsCompletionDemographicsRequest,
) -> String {
    let limit = req.limit.unwrap_or(50).min(200);
    let include_representation = req.include_representation.unwrap_or(true);

    // Build CIP filter: exact codes take priority over prefix
    let cip_codes_vec: Vec<String> = req
        .cip_codes
        .as_deref()
        .map(parse_cip_codes)
        .unwrap_or_default();
    let cip_filter: Option<CipFilter<'_>> = if cip_codes_vec.is_empty() {
        req.cip_prefix.as_deref().map(CipFilter::Prefix)
    } else {
        Some(CipFilter::Codes(&cip_codes_vec))
    };
    let cip_label = match &cip_filter {
        Some(CipFilter::Codes(c)) => c.join(","),
        Some(CipFilter::Prefix(p)) => (*p).to_string(),
        None => String::new(),
    };

    // Step 1: resolve institution filters — unitid shortcut bypasses group filters
    let inst_filters = QueryFilters::new()
        .eq("unitid", req.unitid)
        .eq("carnegie_class", req.carnegie_class)
        .eq("control", req.control)
        .eq("state", req.state.as_deref())
        .eq("hbcu", req.hbcu)
        .eq("tribal", req.tribal)
        .gte("inst_size", req.inst_size_min);

    let inst_result = match client
        .select(
            tables::INSTITUTIONS,
            "unitid,name,city,state,carnegie_class,inst_size",
            &inst_filters,
            Some(500),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return error_json(e),
    };

    let institutions: Vec<InstitutionMeta> = parse_json_array(&inst_result);

    if institutions.is_empty() {
        let suggestion = if req.unitid.is_some() {
            "Check that the unitid is correct (use search_institutions to find it)"
        } else {
            "Try broadening carnegie_class, control, state, or inst_size_min filters"
        };
        return serde_json::json!({
            "error": "No institutions matched the given filters",
            "suggestion": suggestion
        })
        .to_string();
    }

    let unitids: Vec<i32> = institutions.iter().map(|i| i.unitid).collect();

    // Step 2: fetch completions in batches to avoid Cloudflare Worker size limits.
    // All CIP codes are stored, so unfiltered queries for 150 institutions can
    // return 50K+ rows — a single large response crashes the Supabase proxy.
    let completions_by_uid = fetch_demo_by_unitid_batched(
        client,
        &unitids,
        cip_filter.as_ref(),
        req.award_level,
        req.year,
    )
    .await;

    // Step 3: fetch totals for representation denominators
    let totals_by_uid = if include_representation {
        fetch_totals_by_unitid(client, &unitids, req.award_level, req.year).await
    } else {
        HashMap::new()
    };

    // Step 4: build per-institution output
    let mut results = build_school_results(
        &institutions,
        &completions_by_uid,
        &totals_by_uid,
        req.year,
        req.min_completions,
    );

    results.sort_by(|a, b| b.total_completions.cmp(&a.total_completions));
    results.truncate(limit);

    to_json_pretty(&serde_json::json!({
        "count": results.len(),
        "filters": {
            "carnegie_class": req.carnegie_class,
            "control": req.control,
            "state": req.state,
            "inst_size_min": req.inst_size_min,
            "cip_filter": cip_label,
            "award_level": req.award_level,
            "year": req.year,
        },
        "schools": results
    }))
}

/// Fetch institution completion totals for a list of unitids (representation denominators).
///
/// Batched to stay within Cloudflare Worker response size limits.
async fn fetch_totals_by_unitid(
    client: &Arc<DbClient>,
    unitids: &[i32],
    award_level: Option<i32>,
    year: Option<i32>,
) -> HashMap<i32, DemographicCounts> {
    let mut result: HashMap<i32, DemographicCounts> = HashMap::new();
    for chunk in unitids.chunks(COMPLETIONS_BATCH_SIZE) {
        let filters = QueryFilters::new()
            .in_list("unitid", chunk)
            .eq("award_level", award_level)
            .eq("year", year);
        if let Ok(v) = client
            .select(
                tables::INSTITUTION_COMPLETION_TOTALS,
                DEMO_COLS_WITH_UNITID,
                &filters,
                Some(5_000),
            )
            .await
        {
            for (uid, counts) in aggregate_by_unitid(v.as_array()) {
                merge_counts(result.entry(uid).or_default(), &counts);
            }
        }
    }
    result
}

/// Fetch and aggregate completions for a list of unitids, batching queries to avoid
/// large `PostgREST` responses that crash the Cloudflare Worker proxy.
async fn fetch_demo_by_unitid_batched(
    client: &Arc<DbClient>,
    unitids: &[i32],
    cip_filter: Option<&CipFilter<'_>>,
    award_level: Option<i32>,
    year: Option<i32>,
) -> HashMap<i32, DemographicCounts> {
    let mut result: HashMap<i32, DemographicCounts> = HashMap::new();
    for chunk in unitids.chunks(COMPLETIONS_BATCH_SIZE) {
        let filters = apply_cip_filter(
            QueryFilters::new()
                .in_list("unitid", chunk)
                .eq("award_level", award_level)
                .eq("year", year),
            cip_filter,
            "cip_code",
        );
        if let Ok(v) = client
            .select(
                tables::COMPLETIONS,
                DEMO_COLS_WITH_UNITID,
                &filters,
                Some(5_000),
            )
            .await
        {
            for (uid, counts) in aggregate_by_unitid(v.as_array()) {
                merge_counts(result.entry(uid).or_default(), &counts);
            }
        }
    }
    result
}

/// Merge `src` demographic counts into `dst` (cross-batch accumulation).
// Not const: takes a mutable reference, which is incompatible with const context.
#[allow(clippy::missing_const_for_fn)]
fn merge_counts(dst: &mut DemographicCounts, src: &DemographicCounts) {
    macro_rules! add {
        ($field:ident) => {
            dst.$field += src.$field;
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

/// Build per-institution result rows, filtering by `min_completions`.
fn build_school_results(
    institutions: &[InstitutionMeta],
    completions_by_uid: &HashMap<i32, DemographicCounts>,
    totals_by_uid: &HashMap<i32, DemographicCounts>,
    year: Option<i32>,
    min_completions: Option<i64>,
) -> Vec<SchoolDemographicsResult> {
    institutions
        .iter()
        .filter_map(|inst| {
            let comp = completions_by_uid
                .get(&inst.unitid)
                .cloned()
                .unwrap_or_default();
            if comp.total == 0 || min_completions.is_some_and(|min| comp.total < min) {
                return None;
            }
            let inst_totals = totals_by_uid.get(&inst.unitid);
            Some(SchoolDemographicsResult {
                unitid: inst.unitid,
                name: inst.name.clone(),
                city: inst.city.clone(),
                state: inst.state.clone(),
                carnegie_class: inst.carnegie_class,
                year,
                total_completions: comp.total,
                demographics: build_demographics(&comp, inst_totals),
                cross_tab: build_cross_tab(&comp, inst_totals),
            })
        })
        .collect()
}

/// Aggregate a JSON array into a `HashMap<unitid, DemographicCounts>`.
fn aggregate_by_unitid(arr: Option<&Vec<serde_json::Value>>) -> HashMap<i32, DemographicCounts> {
    let mut map: HashMap<i32, DemographicCounts> = HashMap::new();
    if let Some(rows) = arr {
        for item in rows {
            let uid = item
                .get("unitid")
                .and_then(serde_json::Value::as_i64)
                .and_then(|v| i32::try_from(v).ok());
            if let Some(uid) = uid {
                accumulate(map.entry(uid).or_default(), item);
            }
        }
    }
    map
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Fetch the name of an institution by unitid. Returns `None` if not found.
async fn fetch_institution_name(client: &Arc<DbClient>, unitid: i32) -> Option<String> {
    let filters = QueryFilters::new().eq("unitid", Some(unitid));
    let result = client
        .select(tables::INSTITUTIONS, "name", &filters, Some(1))
        .await
        .ok()?;
    result
        .as_array()?
        .first()?
        .get("name")?
        .as_str()
        .map(String::from)
}

/// Fetch school-wide completion totals for representation ratio denominators.
///
/// Returns `None` if no matching totals exist or the query fails.
async fn fetch_school_totals_for(
    client: &Arc<DbClient>,
    unitid: i32,
    award_level: Option<i32>,
    year: Option<i32>,
) -> Option<DemographicCounts> {
    let filters = QueryFilters::new()
        .eq("unitid", Some(unitid))
        .eq("award_level", award_level)
        .eq("year", year);
    client
        .select(
            tables::INSTITUTION_COMPLETION_TOTALS,
            DEMO_COLS_NO_KEY,
            &filters,
            Some(50),
        )
        .await
        .ok()
        .and_then(|v| {
            let mut totals = DemographicCounts::default();
            if let Some(arr) = v.as_array() {
                for item in arr {
                    accumulate(&mut totals, item);
                }
            }
            (totals.total > 0).then_some(totals)
        })
}

/// Fetch CIP code titles for a set of codes. Returns a `HashMap<cip_code, title>`.
async fn fetch_cip_titles(client: &Arc<DbClient>, cip_codes: &[String]) -> HashMap<String, String> {
    if cip_codes.is_empty() {
        return HashMap::new();
    }
    let filters = QueryFilters::new().in_list("cip_code", cip_codes);
    let Ok(result) = client
        .select(tables::CIP_CODES, "cip_code,title", &filters, Some(500))
        .await
    else {
        return HashMap::new();
    };
    result
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| {
                    let code = item.get("cip_code")?.as_str()?.to_string();
                    let title = item.get("title")?.as_str()?.to_string();
                    Some((code, title))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Build the race × gender cross-tabulation from aggregated demographic counts.
///
/// For each racial/ethnic group this produces:
/// - `women_pct_within_group` — gender balance within the race (e.g. "38 % of Hispanic CS grads are women")
/// - `women/men_pct_of_total` — each gender-race cell as share of all CS completions
/// - `women/men_representation_ratio` — cell vs institution baseline (requires `inst_totals`)
fn build_cross_tab(c: &DemographicCounts, inst: Option<&DemographicCounts>) -> Vec<CrossTabRow> {
    macro_rules! cross_row {
        ($label:expr, $men:ident, $women:ident) => {{
            let women = c.$women;
            let men = c.$men;
            let group_total = women + men;
            let women_pct_within_group = if group_total == 0 {
                0.0
            } else {
                pct(women, group_total)
            };
            let women_pct_of_total = pct(women, c.total);
            let men_pct_of_total = pct(men, c.total);
            let women_representation_ratio =
                inst.and_then(|i| representation_ratio(women_pct_of_total, pct(i.$women, i.total)));
            let men_representation_ratio =
                inst.and_then(|i| representation_ratio(men_pct_of_total, pct(i.$men, i.total)));
            CrossTabRow {
                group: $label,
                women_count: women,
                men_count: men,
                women_pct_within_group,
                women_pct_of_total,
                men_pct_of_total,
                women_representation_ratio,
                men_representation_ratio,
            }
        }};
    }

    vec![
        cross_row!("Hispanic/Latino", hispanic_men, hispanic_women),
        cross_row!("Black or African American", black_men, black_women),
        cross_row!("Asian", asian_men, asian_women),
        cross_row!("White", white_men, white_women),
        cross_row!(
            "American Indian/Alaska Native",
            american_indian_men,
            american_indian_women
        ),
        cross_row!(
            "Native Hawaiian/Pacific Islander",
            native_hawaiian_men,
            native_hawaiian_women
        ),
        cross_row!("Two or More Races", two_or_more_men, two_or_more_women),
        cross_row!(
            "Nonresident Alien",
            nonresident_alien_men,
            nonresident_alien_women
        ),
        cross_row!(
            "Unknown Race/Ethnicity",
            unknown_race_men,
            unknown_race_women
        ),
    ]
}

fn pct(part: i64, total: i64) -> f64 {
    if total == 0 {
        return 0.0;
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Assert two floats are equal within a small epsilon. Avoids `float_cmp` lint
    /// on `assert_eq!` while keeping test assertions readable.
    #[track_caller]
    fn assert_float_eq(actual: f64, expected: f64) {
        assert!(
            (actual - expected).abs() < 1e-9,
            "float mismatch: {actual} != {expected}"
        );
    }

    #[track_caller]
    fn assert_float_opt_eq(actual: Option<f64>, expected: Option<f64>) {
        match (actual, expected) {
            (Some(a), Some(e)) => assert_float_eq(a, e),
            (None, None) => {}
            _ => panic!("float option mismatch: {actual:?} != {expected:?}"),
        }
    }

    // ── apply_cip_filter() ───────────────────────────────────────────────────
    // These tests verify branch coverage by checking whether filters were added
    // to QueryFilters using the public is_empty() method.

    #[test]
    fn test_apply_cip_filter_none_does_not_add_filter() {
        let result = apply_cip_filter(QueryFilters::new(), None, "cip_code");
        assert!(result.is_empty());
    }

    #[test]
    fn test_apply_cip_filter_prefix_adds_filter() {
        let result = apply_cip_filter(
            QueryFilters::new(),
            Some(&CipFilter::Prefix("11.")),
            "cip_code",
        );
        assert!(!result.is_empty());
    }

    #[test]
    fn test_apply_cip_filter_codes_adds_filter() {
        let codes = vec!["11.0101".to_string()];
        let result = apply_cip_filter(
            QueryFilters::new(),
            Some(&CipFilter::Codes(&codes)),
            "cip_code",
        );
        assert!(!result.is_empty());
    }

    #[test]
    fn test_apply_cip_filter_empty_col_is_noop() {
        // Empty cip_col means the table has no CIP column — filter must be skipped
        let result = apply_cip_filter(
            QueryFilters::new(),
            Some(&CipFilter::Prefix("11.")),
            "", // empty → no-op
        );
        assert!(result.is_empty());
    }

    // ── pct() ────────────────────────────────────────────────────────────────

    #[test]
    fn test_pct_basic() {
        assert_float_eq(pct(50, 100), 50.0);
    }

    #[test]
    fn test_pct_zero_total_returns_zero() {
        assert_float_eq(pct(50, 0), 0.0);
    }

    #[test]
    fn test_pct_zero_part() {
        assert_float_eq(pct(0, 100), 0.0);
    }

    #[test]
    fn test_pct_rounding_two_decimals() {
        // 1/3 = 33.3333... → rounds to 33.33
        assert_float_eq(pct(1, 3), 33.33);
    }

    #[test]
    fn test_pct_very_small() {
        assert_float_eq(pct(1, 10_000), 0.01);
    }

    #[test]
    fn test_pct_full_hundred() {
        assert_float_eq(pct(100, 100), 100.0);
    }

    // ── representation_ratio() ───────────────────────────────────────────────

    #[test]
    fn test_representation_ratio_proportional() {
        // (50/50 * 100).round() / 100 = 1.0 — proportional is 1.0, not 100.0
        assert_float_opt_eq(representation_ratio(50.0, 50.0), Some(1.0));
    }

    #[test]
    fn test_representation_ratio_underrepresented() {
        // (25/50 * 100).round() / 100 = 0.5
        assert_float_opt_eq(representation_ratio(25.0, 50.0), Some(0.5));
    }

    #[test]
    fn test_representation_ratio_overrepresented() {
        // (75/50 * 100).round() / 100 = 1.5
        assert_float_opt_eq(representation_ratio(75.0, 50.0), Some(1.5));
    }

    #[test]
    fn test_representation_ratio_zero_enroll_returns_none() {
        assert_eq!(representation_ratio(50.0, 0.0), None);
    }

    #[test]
    fn test_representation_ratio_below_threshold_returns_none() {
        assert_eq!(representation_ratio(50.0, 0.0009), None);
    }

    #[test]
    fn test_representation_ratio_at_threshold_returns_some() {
        // 0.001 exactly should produce a value
        assert!(representation_ratio(50.0, 0.001).is_some());
    }

    // ── accumulate() ────────────────────────────────────────────────────────

    fn demo_json(total: i64, men: i64, women: i64) -> serde_json::Value {
        serde_json::json!({
            "total": total, "total_men": men, "total_women": women,
            "nonresident_alien_men": 0, "nonresident_alien_women": 0,
            "hispanic_men": 0, "hispanic_women": 0,
            "american_indian_men": 0, "american_indian_women": 0,
            "asian_men": 0, "asian_women": 0,
            "black_men": 0, "black_women": 0,
            "native_hawaiian_men": 0, "native_hawaiian_women": 0,
            "white_men": men, "white_women": women,
            "two_or_more_men": 0, "two_or_more_women": 0,
            "unknown_race_men": 0, "unknown_race_women": 0
        })
    }

    #[test]
    fn test_accumulate_basic() {
        let mut agg = DemographicCounts::default();
        accumulate(&mut agg, &demo_json(100, 40, 60));
        assert_eq!(agg.total, 100);
        assert_eq!(agg.total_men, 40);
        assert_eq!(agg.total_women, 60);
    }

    #[test]
    fn test_accumulate_sums_across_rows() {
        let mut agg = DemographicCounts::default();
        accumulate(&mut agg, &demo_json(100, 40, 60));
        accumulate(&mut agg, &demo_json(50, 20, 30));
        assert_eq!(agg.total, 150);
        assert_eq!(agg.total_men, 60);
        assert_eq!(agg.total_women, 90);
    }

    #[test]
    fn test_accumulate_missing_fields_default_zero() {
        let mut agg = DemographicCounts::default();
        // Only total provided
        accumulate(&mut agg, &serde_json::json!({"total": 42}));
        assert_eq!(agg.total, 42);
        assert_eq!(agg.total_men, 0);
        assert_eq!(agg.hispanic_women, 0);
    }

    // ── aggregate_by_unitid() ────────────────────────────────────────────────

    #[test]
    fn test_aggregate_by_unitid_none_returns_empty() {
        assert!(aggregate_by_unitid(None).is_empty());
    }

    #[test]
    fn test_aggregate_by_unitid_empty_array() {
        assert!(aggregate_by_unitid(Some(&vec![])).is_empty());
    }

    #[test]
    fn test_aggregate_by_unitid_groups_by_unitid() {
        let rows = vec![
            serde_json::json!({"unitid": 1, "total": 50, "total_men": 20, "total_women": 30,
                "nonresident_alien_men": 0, "nonresident_alien_women": 0,
                "hispanic_men": 0, "hispanic_women": 0, "american_indian_men": 0, "american_indian_women": 0,
                "asian_men": 0, "asian_women": 0, "black_men": 0, "black_women": 0,
                "native_hawaiian_men": 0, "native_hawaiian_women": 0, "white_men": 20, "white_women": 30,
                "two_or_more_men": 0, "two_or_more_women": 0, "unknown_race_men": 0, "unknown_race_women": 0}),
            serde_json::json!({"unitid": 1, "total": 30, "total_men": 15, "total_women": 15,
                "nonresident_alien_men": 0, "nonresident_alien_women": 0,
                "hispanic_men": 0, "hispanic_women": 0, "american_indian_men": 0, "american_indian_women": 0,
                "asian_men": 0, "asian_women": 0, "black_men": 0, "black_women": 0,
                "native_hawaiian_men": 0, "native_hawaiian_women": 0, "white_men": 15, "white_women": 15,
                "two_or_more_men": 0, "two_or_more_women": 0, "unknown_race_men": 0, "unknown_race_women": 0}),
            serde_json::json!({"unitid": 2, "total": 75, "total_men": 35, "total_women": 40,
                "nonresident_alien_men": 0, "nonresident_alien_women": 0,
                "hispanic_men": 0, "hispanic_women": 0, "american_indian_men": 0, "american_indian_women": 0,
                "asian_men": 0, "asian_women": 0, "black_men": 0, "black_women": 0,
                "native_hawaiian_men": 0, "native_hawaiian_women": 0, "white_men": 35, "white_women": 40,
                "two_or_more_men": 0, "two_or_more_women": 0, "unknown_race_men": 0, "unknown_race_women": 0}),
        ];
        let result = aggregate_by_unitid(Some(&rows));
        assert_eq!(result.len(), 2);
        assert_eq!(result[&1].total, 80); // 50 + 30
        assert_eq!(result[&2].total, 75);
    }

    #[test]
    fn test_aggregate_by_unitid_skips_missing_unitid() {
        let rows = vec![
            serde_json::json!({"unitid": 1, "total": 50, "total_men": 20, "total_women": 30,
                "nonresident_alien_men": 0, "nonresident_alien_women": 0,
                "hispanic_men": 0, "hispanic_women": 0, "american_indian_men": 0, "american_indian_women": 0,
                "asian_men": 0, "asian_women": 0, "black_men": 0, "black_women": 0,
                "native_hawaiian_men": 0, "native_hawaiian_women": 0, "white_men": 20, "white_women": 30,
                "two_or_more_men": 0, "two_or_more_women": 0, "unknown_race_men": 0, "unknown_race_women": 0}),
            // row with no unitid — should be skipped
            serde_json::json!({"total": 100}),
        ];
        let result = aggregate_by_unitid(Some(&rows));
        assert_eq!(result.len(), 1);
    }

    // ── build_school_results() ───────────────────────────────────────────────

    fn make_inst(unitid: i32) -> InstitutionMeta {
        InstitutionMeta {
            unitid,
            name: format!("School {unitid}"),
            city: None,
            state: None,
            carnegie_class: None,
            inst_size: None,
        }
    }

    fn make_counts(total: i64) -> DemographicCounts {
        DemographicCounts {
            total,
            total_men: total / 2,
            total_women: total - total / 2,
            ..Default::default()
        }
    }

    #[test]
    fn test_build_school_results_empty_institutions() {
        let result = build_school_results(&[], &HashMap::new(), &HashMap::new(), None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_school_results_skips_zero_completions() {
        let institutions = vec![make_inst(1)];
        let result =
            build_school_results(&institutions, &HashMap::new(), &HashMap::new(), None, None);
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_school_results_applies_min_completions() {
        let institutions = vec![make_inst(1), make_inst(2)];
        let mut completions = HashMap::new();
        completions.insert(1, make_counts(10));
        completions.insert(2, make_counts(150));

        let result =
            build_school_results(&institutions, &completions, &HashMap::new(), None, Some(50));
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].unitid, 2);
    }

    #[test]
    fn test_build_school_results_propagates_year() {
        let institutions = vec![make_inst(1)];
        let mut completions = HashMap::new();
        completions.insert(1, make_counts(100));

        let result = build_school_results(
            &institutions,
            &completions,
            &HashMap::new(),
            Some(2024),
            None,
        );
        assert_eq!(result[0].year, Some(2024));
    }

    // ── merge_counts() ───────────────────────────────────────────────────────

    #[test]
    fn test_merge_counts_basic() {
        let mut dst = DemographicCounts {
            total: 100,
            total_men: 40,
            total_women: 60,
            ..Default::default()
        };
        let src = DemographicCounts {
            total: 50,
            total_men: 20,
            total_women: 30,
            ..Default::default()
        };
        merge_counts(&mut dst, &src);
        assert_eq!(dst.total, 150);
        assert_eq!(dst.total_men, 60);
        assert_eq!(dst.total_women, 90);
    }

    #[test]
    fn test_merge_counts_all_demographic_fields() {
        let mut dst = DemographicCounts::default();
        let src = DemographicCounts {
            total: 100,
            hispanic_men: 10,
            hispanic_women: 12,
            asian_men: 8,
            asian_women: 15,
            black_men: 5,
            black_women: 6,
            ..Default::default()
        };
        merge_counts(&mut dst, &src);
        assert_eq!(dst.hispanic_men, 10);
        assert_eq!(dst.hispanic_women, 12);
        assert_eq!(dst.asian_men, 8);
        assert_eq!(dst.black_women, 6);
    }

    #[test]
    fn test_merge_counts_sequential_batches() {
        let mut result = DemographicCounts::default();
        merge_counts(
            &mut result,
            &DemographicCounts {
                total: 50,
                total_men: 20,
                ..Default::default()
            },
        );
        merge_counts(
            &mut result,
            &DemographicCounts {
                total: 30,
                total_men: 12,
                ..Default::default()
            },
        );
        assert_eq!(result.total, 80);
        assert_eq!(result.total_men, 32);
    }

    // ── build_completion_row() ────────────────────────────────────────────────

    fn full_demo_item(cip_code: &str, total: i64, men: i64, women: i64) -> serde_json::Value {
        serde_json::json!({
            "cip_code": cip_code, "award_level": 5, "major_num": 1,
            "total": total, "total_men": men, "total_women": women,
            "nonresident_alien_men": 0, "nonresident_alien_women": 0,
            "hispanic_men": 0, "hispanic_women": 0,
            "american_indian_men": 0, "american_indian_women": 0,
            "asian_men": 0, "asian_women": 0,
            "black_men": 0, "black_women": 0,
            "native_hawaiian_men": 0, "native_hawaiian_women": 0,
            "white_men": men, "white_women": women,
            "two_or_more_men": 0, "two_or_more_women": 0,
            "unknown_race_men": 0, "unknown_race_women": 0
        })
    }

    #[test]
    fn test_build_completion_row_with_title() {
        let item = full_demo_item("11.0101", 100, 60, 40);
        let mut titles = HashMap::new();
        titles.insert("11.0101".to_string(), "Computer Science".to_string());

        let row = build_completion_row(&item, &titles, None);
        assert_eq!(row.cip_code, "11.0101");
        assert_eq!(row.cip_title, Some("Computer Science".to_string()));
        assert_eq!(row.award_level, Some(5));
        assert_eq!(row.major_num, Some(1));
        assert_eq!(row.total, 100);
        assert_eq!(row.demographics.len(), 11);
    }

    #[test]
    fn test_build_completion_row_missing_title_returns_none() {
        let item = full_demo_item("99.9999", 50, 25, 25);
        let row = build_completion_row(&item, &HashMap::new(), None);
        assert_eq!(row.cip_code, "99.9999");
        assert_eq!(row.cip_title, None);
    }

    #[test]
    fn test_build_completion_row_with_school_totals_computes_ratios() {
        let item = full_demo_item("11.0101", 100, 60, 40);
        let school = DemographicCounts {
            total: 1000,
            total_men: 600,
            total_women: 400,
            ..Default::default()
        };
        let row = build_completion_row(&item, &HashMap::new(), Some(&school));
        let women = row
            .demographics
            .iter()
            .find(|d| d.group == "Women")
            .unwrap();
        // cip_pct = 40%, school_pct = 40% → ratio = 1.0
        assert_float_eq(women.cip_pct, 40.0);
        assert_float_opt_eq(women.school_pct, Some(40.0));
        assert_float_opt_eq(women.representation_ratio, Some(1.0));
    }

    // ── build_row_demographics() ──────────────────────────────────────────────

    #[test]
    fn test_build_row_demographics_returns_11_groups() {
        let item = full_demo_item("11.0101", 100, 60, 40);
        let groups = build_row_demographics(&item, 100, None);
        assert_eq!(groups.len(), 11);
    }

    #[test]
    fn test_build_row_demographics_gender_counts() {
        let item = full_demo_item("11.0101", 100, 60, 40);
        let groups = build_row_demographics(&item, 100, None);
        let women = groups.iter().find(|d| d.group == "Women").unwrap();
        let men = groups.iter().find(|d| d.group == "Men").unwrap();
        assert_eq!(women.count, 40);
        assert_float_eq(women.cip_pct, 40.0);
        assert_eq!(men.count, 60);
        assert_float_eq(men.cip_pct, 60.0);
        assert!(women.school_pct.is_none()); // no school totals provided
    }

    #[test]
    fn test_build_row_demographics_race_group_sums_men_and_women() {
        let item = serde_json::json!({
            "total_men": 100, "total_women": 100,
            "hispanic_men": 20, "hispanic_women": 15,
            "nonresident_alien_men": 0, "nonresident_alien_women": 0,
            "american_indian_men": 0, "american_indian_women": 0,
            "asian_men": 0, "asian_women": 0,
            "black_men": 0, "black_women": 0,
            "native_hawaiian_men": 0, "native_hawaiian_women": 0,
            "white_men": 80, "white_women": 85,
            "two_or_more_men": 0, "two_or_more_women": 0,
            "unknown_race_men": 0, "unknown_race_women": 0
        });
        let groups = build_row_demographics(&item, 200, None);
        let hispanic = groups
            .iter()
            .find(|d| d.group == "Hispanic/Latino")
            .unwrap();
        assert_eq!(hispanic.count, 35); // 20 + 15
        assert_float_eq(hispanic.cip_pct, 17.5);
    }

    #[test]
    fn test_build_row_demographics_missing_fields_default_zero() {
        // Sparse item — unset fields should default to 0
        let item = serde_json::json!({ "total_women": 30 });
        let groups = build_row_demographics(&item, 100, None);
        let men = groups.iter().find(|d| d.group == "Men").unwrap();
        assert_eq!(men.count, 0);
        assert_float_eq(men.cip_pct, 0.0);
    }

    #[test]
    fn test_build_row_demographics_with_school_totals() {
        let item = full_demo_item("11.0101", 100, 40, 60);
        let school = DemographicCounts {
            total: 200,
            total_men: 80,
            total_women: 120,
            ..Default::default()
        };
        let groups = build_row_demographics(&item, 100, Some(&school));
        let women = groups.iter().find(|d| d.group == "Women").unwrap();
        // women cip_pct=60%, school_pct=60% → ratio=1.0
        assert_float_opt_eq(women.school_pct, Some(60.0));
        assert_float_opt_eq(women.representation_ratio, Some(1.0));
    }

    // ── build_cross_tab() ────────────────────────────────────────────────────

    #[test]
    fn test_build_cross_tab_returns_9_groups() {
        let result = build_cross_tab(&DemographicCounts::default(), None);
        assert_eq!(result.len(), 9); // one per racial/ethnic group (no gender-only rows)
    }

    #[test]
    fn test_build_cross_tab_group_names() {
        let result = build_cross_tab(&DemographicCounts::default(), None);
        let names: Vec<&str> = result.iter().map(|r| r.group).collect();
        assert!(names.contains(&"Hispanic/Latino"));
        assert!(names.contains(&"Black or African American"));
        assert!(names.contains(&"Asian"));
        assert!(names.contains(&"White"));
    }

    #[test]
    fn test_build_cross_tab_gender_parity_within_group() {
        let c = DemographicCounts {
            total: 100,
            black_men: 30,
            black_women: 70,
            ..Default::default()
        };
        let result = build_cross_tab(&c, None);
        let black = result
            .iter()
            .find(|r| r.group == "Black or African American")
            .unwrap();
        assert_eq!(black.women_count, 70);
        assert_eq!(black.men_count, 30);
        // 70 / 100 = 70% women within the group
        assert_float_eq(black.women_pct_within_group, 70.0);
        // 70 / 100 total CS completions = 70% of all
        assert_float_eq(black.women_pct_of_total, 70.0);
        assert!(black.women_representation_ratio.is_none()); // no inst totals
    }

    #[test]
    fn test_build_cross_tab_with_institution_totals() {
        // 40 CS completions: hispanic_women=10, hispanic_men=10
        // institution total: 200, hispanic_women=40, hispanic_men=40
        let c = DemographicCounts {
            total: 100,
            hispanic_men: 10,
            hispanic_women: 10,
            ..Default::default()
        };
        let inst = DemographicCounts {
            total: 200,
            hispanic_men: 40,
            hispanic_women: 40,
            ..Default::default()
        };
        let result = build_cross_tab(&c, Some(&inst));
        let hispanic = result
            .iter()
            .find(|r| r.group == "Hispanic/Latino")
            .unwrap();
        // women_pct_of_total = 10/100 = 10%
        // inst women_pct = 40/200 = 20%
        // ratio = 10/20 = 0.5 (underrepresented)
        assert_float_eq(hispanic.women_pct_of_total, 10.0);
        assert_float_opt_eq(hispanic.women_representation_ratio, Some(0.5));
    }

    #[test]
    fn test_build_cross_tab_zero_group_total_no_divide_by_zero() {
        let c = DemographicCounts::default(); // all zeros
        let result = build_cross_tab(&c, None);
        for row in &result {
            assert_float_eq(row.women_pct_within_group, 0.0); // no division by zero
        }
    }

    #[test]
    fn test_build_cross_tab_institution_zero_subgroup_ratio_is_none() {
        // Program has Asian students but institution baseline has none for that group.
        // representation_ratio should be None (below threshold) rather than inf.
        let c = DemographicCounts {
            total: 100,
            asian_men: 5,
            asian_women: 10,
            ..Default::default()
        };
        let inst = DemographicCounts {
            total: 200,
            asian_men: 0,
            asian_women: 0, // 0% baseline → ratio undefined
            ..Default::default()
        };
        let result = build_cross_tab(&c, Some(&inst));
        let asian = result.iter().find(|r| r.group == "Asian").unwrap();
        assert_float_eq(asian.women_pct_of_total, 10.0);
        assert_eq!(asian.women_representation_ratio, None);
        assert_eq!(asian.men_representation_ratio, None);
    }

    #[test]
    fn test_build_cross_tab_men_and_women_ratios_computed_independently() {
        let c = DemographicCounts {
            total: 100,
            hispanic_men: 20,
            hispanic_women: 5,
            ..Default::default()
        };
        let inst = DemographicCounts {
            total: 200,
            hispanic_men: 20,   // inst pct = 10%
            hispanic_women: 30, // inst pct = 15%
            ..Default::default()
        };
        let result = build_cross_tab(&c, Some(&inst));
        let hispanic = result
            .iter()
            .find(|r| r.group == "Hispanic/Latino")
            .unwrap();
        // men: 20/100=20%, inst 20/200=10% → ratio = 20/10 = 2.0
        assert_float_opt_eq(hispanic.men_representation_ratio, Some(2.0));
        // women: 5/100=5%, inst 30/200=15% → ratio = 5/15 ≈ 0.33
        let ratio = hispanic
            .women_representation_ratio
            .expect("should have ratio");
        assert!((ratio - 0.33).abs() < 0.01);
    }

    #[test]
    fn test_build_cross_tab_all_9_groups_present_for_zero_counts() {
        // Even when all counts are zero, all 9 race groups must be returned.
        let result = build_cross_tab(&DemographicCounts::default(), None);
        let names: Vec<&str> = result.iter().map(|r| r.group).collect();
        assert!(names.contains(&"Unknown Race/Ethnicity"));
        assert!(names.contains(&"Nonresident Alien"));
        assert_eq!(names.len(), 9);
    }

    // ── build_demographics() — representative subset ─────────────────────────

    #[test]
    fn test_build_demographics_returns_11_groups() {
        let result = build_demographics(&DemographicCounts::default(), None);
        assert_eq!(result.len(), 11);
    }

    #[test]
    fn test_build_demographics_group_names() {
        let result = build_demographics(&DemographicCounts::default(), None);
        let names: Vec<&str> = result.iter().map(|g| g.group.as_str()).collect();
        assert!(names.contains(&"Women"));
        assert!(names.contains(&"Men"));
        assert!(names.contains(&"Hispanic/Latino"));
        assert!(names.contains(&"Asian"));
    }

    #[test]
    fn test_build_demographics_no_enrollment_no_ratio() {
        let c = DemographicCounts {
            total: 100,
            total_women: 60,
            total_men: 40,
            ..Default::default()
        };
        let result = build_demographics(&c, None);
        let women = result.iter().find(|g| g.group == "Women").unwrap();
        assert_eq!(women.completions, 60);
        assert_float_eq(women.completion_pct, 60.0);
        assert!(women.representation_ratio.is_none());
    }

    #[test]
    fn test_build_demographics_proportional_ratio_is_one() {
        // Both completions and enrollment are 60% women → ratio = 1.0 (proportional)
        let c = DemographicCounts {
            total: 100,
            total_women: 60,
            total_men: 40,
            ..Default::default()
        };
        let e = DemographicCounts {
            total: 200,
            total_women: 120,
            total_men: 80,
            ..Default::default()
        };
        let result = build_demographics(&c, Some(&e));
        let women = result.iter().find(|g| g.group == "Women").unwrap();
        assert_float_opt_eq(women.representation_ratio, Some(1.0));
    }
}
