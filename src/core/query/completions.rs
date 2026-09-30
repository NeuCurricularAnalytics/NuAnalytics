//! Completion demographics — who earns degrees, by race and gender.
//!
//! One request, [`CompletionDemographicsRequest`], whose `group_by` picks the shape of the
//! answer:
//!
//! - `total` — one row per race/gender group, aggregated over every matched institution;
//! - `school` — one row per institution, ranked by completions;
//! - `cip` — one row per CIP code at a single institution.
//!
//! Each is one catalog query ([`super::catalog`], `completions_*.sql`), one round
//! trip. The database filters, picks the year, sums, computes each baseline, attaches CIP
//! titles and ranks; this module turns those sums into percentages and ratios. An
//! institution is matched only if it has an `institutions` row.
//!
//! **CIP 99 is never summed.** IPEDS files a grand-total row under CIP 99 — the sum of every
//! other CIP at that institution, award level and year — so an "all CIPs" sum that includes
//! it counts every graduate twice. Each baseline is likewise summed from the detail rows.
//!
//! ## Representation ratio
//!
//! `(group_completions / total_completions) / (group_baseline / total_baseline)`, where the
//! baseline is every completion in the same year and award level, across all CIPs and both
//! majors — pooled over the matched institutions for `total`, each school's own otherwise.
//!
//! A ratio of 1.0 means the group is proportionally represented relative to the
//! institution's overall completion profile. Values <1 indicate underrepresentation, >1
//! overrepresentation. The response names the baseline `baseline_completions`,
//! `baseline_total` and `baseline_pct`.

use std::ops::AddAssign;
use std::sync::Arc;

use super::catalog;
use super::sql::SqlError;
use crate::core::database::models::DemographicRepresentation;
use crate::core::database::DbClient;
use crate::core::json::{parse_comma_list, to_json_pretty};
use serde::{Deserialize, Serialize};

// ============================================================================
// Request
// ============================================================================

/// What a row of the answer is.
#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum DemographicsGroupBy {
    /// One row per race/gender group, aggregated over every matched institution.
    #[default]
    Total,
    /// One row per institution, ranked by completions.
    School,
    /// One row per CIP code at a single institution; needs `unitid`.
    Cip,
}

impl DemographicsGroupBy {
    /// The value as the caller writes it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::School => "school",
            Self::Cip => "cip",
        }
    }
}

/// Completion demographics, at whichever grouping `group_by` names.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct CompletionDemographicsRequest {
    /// Shape of the answer. Defaults to `total`.
    #[schemars(
        description = "Shape of the answer: \"total\" (aggregated over every matched school, default), \"school\" (one row per school, ranked), or \"cip\" (one row per CIP code at one school; needs unitid)"
    )]
    #[serde(default)]
    pub group_by: Option<DemographicsGroupBy>,
    /// One institution. Required by `cip`; narrows `total` and `school` to it.
    #[schemars(
        description = "IPEDS Unit ID. Required for group_by=cip; narrows the others to one school"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub unitid: Option<i32>,
    /// Carnegie classification code (15=R1, 16=R2, 17=doctoral/professional).
    #[schemars(
        description = "Carnegie classification, 2021 Basic (15=R1, 16=R2, 17=Doctoral/Professional). See get_lookup_codes(\"carnegie_class\")."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub carnegie_class: Option<i32>,
    /// Control: 1 public, 2 private non-profit, 3 for-profit.
    #[schemars(description = "Control: 1=public, 2=private nonprofit, 3=for-profit")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub control: Option<i32>,
    /// Two-letter state code.
    #[schemars(description = "Two-letter state abbreviation")]
    pub state: Option<String>,
    /// Historically Black colleges and universities only (or, if false, none of them).
    #[schemars(description = "Only Historically Black Colleges and Universities")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub hbcu: Option<bool>,
    /// Tribal colleges only (or, if false, none of them).
    #[schemars(description = "Only tribal colleges and universities")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub tribal: Option<bool>,
    /// Minimum institution size bucket (1=<1000, 2=1000-4999, 3=5000-9999, …).
    #[schemars(description = "Minimum size bucket: 2=1000+ students, 3=5000+, 4=10000+, 5=20000+")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub inst_size_min: Option<i32>,
    /// CIP code prefix in dot notation, e.g. `11.` for all computing.
    #[schemars(
        description = "CIP prefix (dot notation): \"11.\" all computing, \"11.07\" computer science, \"30.70\" data science. Omit for all CIPs."
    )]
    pub cip_prefix: Option<String>,
    /// Exact CIP codes, comma-separated. Takes priority over `cip_prefix`.
    #[schemars(
        description = "Comma-separated exact CIP codes, e.g. \"11.0101,11.0701\". Takes priority over cip_prefix."
    )]
    pub cip_codes: Option<String>,
    /// Award level: 3 associate, 5 bachelor's, 7 master's, 9 doctoral.
    #[schemars(
        description = "Award level: 3=associate, 5=bachelors, 7=masters, 9=doctoral. Omit for all."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub award_level: Option<i32>,
    /// Academic year, e.g. 2024. Defaults to the latest year with matching data.
    #[schemars(
        description = "Academic year, e.g. 2024. Defaults to the latest year with matching data."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub year: Option<i32>,
    /// Major number: 1 first major, 2 second major. Omit to count both.
    #[schemars(description = "Major number: 1=first major, 2=second major. Omit to count both.")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i32")]
    pub major_num: Option<i32>,
    /// Include representation ratios (default true).
    #[schemars(
        description = "Include representation ratios against the whole graduating population — pooled over the matched schools for total, each school's own otherwise (default true)"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub include_representation: Option<bool>,
    /// `school` only: skip schools with fewer selected completions than this.
    #[schemars(description = "group_by=school only: skip schools with fewer selected completions")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_i64")]
    pub min_completions: Option<i64>,
    /// `school` only: schools to return (default 50, max 200).
    #[schemars(description = "group_by=school only: schools to return (default 50, max 200)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub limit: Option<usize>,
}

/// A filter not every grouping applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DemoFilter {
    /// `carnegie_class`.
    CarnegieClass,
    /// `control`.
    Control,
    /// `state`.
    State,
    /// `hbcu`.
    Hbcu,
    /// `tribal`.
    Tribal,
    /// `inst_size_min`.
    InstSizeMin,
    /// `min_completions`.
    MinCompletions,
    /// `limit`.
    Limit,
}

impl DemoFilter {
    /// The request field's name.
    #[must_use]
    pub const fn field(self) -> &'static str {
        match self {
            Self::CarnegieClass => "carnegie_class",
            Self::Control => "control",
            Self::State => "state",
            Self::Hbcu => "hbcu",
            Self::Tribal => "tribal",
            Self::InstSizeMin => "inst_size_min",
            Self::MinCompletions => "min_completions",
            Self::Limit => "limit",
        }
    }
}

/// Filters the request sets that its grouping cannot apply, in field order.
///
/// Refused rather than ignored: `cip` is one school, so an institution-group filter would
/// be silently dropped and the answer would look like it honoured it; `total` is one
/// aggregate, so there is nothing to limit. The one list every surface refuses from.
#[must_use]
pub fn inapplicable_filters(req: &CompletionDemographicsRequest) -> Vec<DemoFilter> {
    let set = [
        (req.carnegie_class.is_some(), DemoFilter::CarnegieClass),
        (req.control.is_some(), DemoFilter::Control),
        (req.state.is_some(), DemoFilter::State),
        (req.hbcu.is_some(), DemoFilter::Hbcu),
        (req.tribal.is_some(), DemoFilter::Tribal),
        (req.inst_size_min.is_some(), DemoFilter::InstSizeMin),
        (req.min_completions.is_some(), DemoFilter::MinCompletions),
        (req.limit.is_some(), DemoFilter::Limit),
    ];
    let applies = |f: DemoFilter| match req.group_by.unwrap_or_default() {
        DemographicsGroupBy::School => true,
        DemographicsGroupBy::Total => !matches!(f, DemoFilter::MinCompletions | DemoFilter::Limit),
        DemographicsGroupBy::Cip => false,
    };
    set.into_iter()
        .filter(|&(is_set, f)| is_set && !applies(f))
        .map(|(_, f)| f)
        .collect()
}

/// The CIP selection: exact codes when given, else the prefix, else every CIP.
struct CipSelection<'a> {
    prefix: Option<&'a str>,
    codes: Option<Vec<String>>,
    /// How the filter is shown back to the caller.
    label: String,
}

impl<'a> CipSelection<'a> {
    fn of(req: &'a CompletionDemographicsRequest) -> Self {
        let codes = req
            .cip_codes
            .as_deref()
            .map(parse_comma_list)
            .filter(|c| !c.is_empty());
        let prefix = if codes.is_some() {
            None
        } else {
            req.cip_prefix.as_deref()
        };
        let label = match (&codes, prefix) {
            (Some(c), _) => c.join(","),
            (None, Some(p)) => p.to_string(),
            (None, None) => "(all CIPs)".to_string(),
        };
        Self {
            prefix,
            codes,
            label,
        }
    }
}

// ============================================================================
// Counts
// ============================================================================

/// The 21 demographic columns IPEDS reports, summed.
///
/// Field names are the column names, which is what the catalog queries emit — a test
/// checks each `completions_*.sql` names all of them.
#[derive(Debug, Default, Clone, PartialEq, Eq, Deserialize, Serialize)]
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

impl AddAssign<&Self> for DemographicCounts {
    fn add_assign(&mut self, src: &Self) {
        macro_rules! add {
            ($($field:ident),+) => { $( self.$field += src.$field; )+ };
        }
        add!(
            total,
            total_men,
            total_women,
            nonresident_alien_men,
            nonresident_alien_women,
            hispanic_men,
            hispanic_women,
            american_indian_men,
            american_indian_women,
            asian_men,
            asian_women,
            black_men,
            black_women,
            native_hawaiian_men,
            native_hawaiian_women,
            white_men,
            white_women,
            two_or_more_men,
            two_or_more_women,
            unknown_race_men,
            unknown_race_women
        );
    }
}

/// A baseline worth dividing by: one with any completions in it.
fn usable(baseline: Option<DemographicCounts>) -> Option<DemographicCounts> {
    baseline.filter(|b| b.total > 0)
}

// ============================================================================
// Response types
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
    /// Women of this race as % of **all** selected completions
    pub women_pct_of_total: f64,
    /// Men of this race as % of **all** selected completions
    pub men_pct_of_total: f64,
    /// Representation ratio for women: `(women_of_race / total) / (women_of_race_baseline /
    /// total_baseline)`. 1.0 = proportional. `None` without a baseline.
    pub women_representation_ratio: Option<f64>,
    /// Representation ratio for men (same formula as women).
    pub men_representation_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct DemographicsResponse {
    group_by: &'static str,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    hbcu: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tribal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    inst_size_min: Option<i32>,
    cip_prefix: String,
    award_level: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    major_num: Option<i32>,
    year: Option<i32>,
}

#[derive(Debug, Serialize)]
struct RowDemographic {
    group: String,
    count: i64,
    cip_pct: f64,
    baseline_pct: Option<f64>,
    representation_ratio: Option<f64>,
}

#[derive(Debug, Serialize)]
struct CompletionRow {
    /// Reporting cycle year (e.g. 2024 for 2023-2024). Always present.
    year: Option<i32>,
    cip_code: String,
    cip_title: Option<String>,
    award_level: Option<i32>,
    major_num: Option<i32>,
    total: i64,
    demographics: Vec<RowDemographic>,
}

#[derive(Debug, Serialize)]
struct InstitutionCompletionsResponse {
    group_by: &'static str,
    unitid: i32,
    name: Option<String>,
    year: Option<i32>,
    award_level: Option<i32>,
    cip_prefix: String,
    total_rows: usize,
    note: &'static str,
    rows: Vec<CompletionRow>,
    /// Race × gender cross-tabulation aggregated across all selected CIP codes.
    cross_tab: Vec<CrossTabRow>,
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
    /// Race × gender cross-tabulation for this school's selected completions.
    cross_tab: Vec<CrossTabRow>,
}

/// A CIP with completions at the school, suggested when the caller's filter matched none.
#[derive(Debug, Serialize, Deserialize, PartialEq, Eq)]
struct NearbyCip {
    cip_code: String,
    cip_title: Option<String>,
    total_completions: i64,
}

// ============================================================================
// Query params and envelopes — what the catalog queries read and return
// ============================================================================

/// What `completions_total.sql` reads from `$1`. Every key is always sent.
#[derive(Debug, Default, Serialize)]
struct TotalParams<'a> {
    unitid: Option<i32>,
    carnegie_class: Option<i32>,
    control: Option<i32>,
    state: Option<&'a str>,
    hbcu: Option<bool>,
    tribal: Option<bool>,
    inst_size_min: Option<i32>,
    cip_prefix: Option<&'a str>,
    cip_codes: Option<&'a [String]>,
    award_level: Option<i32>,
    major_num: Option<i32>,
    year: Option<i32>,
    with_baseline: bool,
}

impl<'a> TotalParams<'a> {
    fn of(req: &'a CompletionDemographicsRequest, cip: &'a CipSelection<'a>) -> Self {
        Self {
            unitid: req.unitid,
            carnegie_class: req.carnegie_class,
            control: req.control,
            state: req.state.as_deref(),
            hbcu: req.hbcu,
            tribal: req.tribal,
            inst_size_min: req.inst_size_min,
            cip_prefix: cip.prefix,
            cip_codes: cip.codes.as_deref(),
            award_level: req.award_level,
            major_num: req.major_num,
            year: req.year,
            with_baseline: req.include_representation.unwrap_or(true),
        }
    }
}

/// What `completions_by_school.sql` reads: the total's keys plus ranking and limit.
#[derive(Debug, Default, Serialize)]
struct SchoolParams<'a> {
    #[serde(flatten)]
    common: TotalParams<'a>,
    min_completions: Option<i64>,
    limit: usize,
}

/// What `completions_by_cip.sql` reads from `$1`.
#[derive(Debug, Default, Serialize)]
struct CipParams<'a> {
    unitid: i32,
    cip_prefix: Option<&'a str>,
    cip_codes: Option<&'a [String]>,
    award_level: Option<i32>,
    major_num: Option<i32>,
    year: Option<i32>,
    with_baseline: bool,
}

/// Schools a `school` answer lists when the request does not say.
const DEFAULT_SCHOOL_LIMIT: usize = 50;

/// The most schools a `school` answer lists. `completions_by_school.sql` applies the same
/// bounds, which a test checks.
const MAX_SCHOOL_LIMIT: usize = 200;

/// `min_completions` as the query can read it: `::int`, so clamped — beyond `i32::MAX` no
/// school qualifies anyway, and an unclamped value would fail the cast (SQLSTATE 22003).
fn min_completions_param(requested: i64) -> i64 {
    requested.clamp(0, i64::from(i32::MAX))
}

/// Schools a `school` answer lists: the request's own limit, or the default, at most the cap.
fn school_limit(req: &CompletionDemographicsRequest) -> usize {
    req.limit
        .unwrap_or(DEFAULT_SCHOOL_LIMIT)
        .min(MAX_SCHOOL_LIMIT)
}

#[derive(Debug, Deserialize)]
struct TotalEnvelope {
    year: Option<i32>,
    institutions_matched: usize,
    counts: DemographicCounts,
    baseline: Option<DemographicCounts>,
}

#[derive(Debug, Deserialize)]
struct SchoolsEnvelope {
    year: Option<i32>,
    institutions_matched: usize,
    schools: Vec<SchoolRow>,
}

#[derive(Debug, Deserialize)]
struct SchoolRow {
    unitid: i32,
    name: String,
    city: Option<String>,
    state: Option<String>,
    carnegie_class: Option<i32>,
    counts: DemographicCounts,
    baseline: Option<DemographicCounts>,
}

#[derive(Debug, Deserialize)]
struct CipEnvelope {
    unitid: i32,
    name: Option<String>,
    year: Option<i32>,
    rows: Vec<CipRow>,
    baseline: Option<DemographicCounts>,
    nearby_year: Option<i32>,
    nearby_cips_with_data: Option<Vec<NearbyCip>>,
}

#[derive(Debug, Deserialize)]
struct CipRow {
    year: Option<i32>,
    cip_code: String,
    cip_title: Option<String>,
    award_level: Option<i32>,
    major_num: Option<i32>,
    #[serde(flatten)]
    counts: DemographicCounts,
}

// ============================================================================
// Entry points
// ============================================================================

/// The answer, whatever its grouping: a response, or a payload saying why there is none.
///
/// Opaque, and serialised field by field in the order the response structs declare —
/// going through a `serde_json::Value` would sort the keys alphabetically.
#[derive(Debug, Serialize)]
#[serde(transparent)]
pub struct Demographics(Answer);

#[derive(Debug, Serialize)]
#[serde(untagged)]
enum Answer {
    Total(DemographicsResponse),
    Cip(InstitutionCompletionsResponse),
    /// The schools answer, and every no-result payload.
    Json(serde_json::Value),
}

/// Answer `req` at its grouping, as JSON.
///
/// A result with nothing in it — no institution matched, no completions for the filters —
/// is an `Ok` payload with an `error` or `note` saying so.
/// Callers are expected to have refused [`inapplicable_filters`] already; [`execute_json`]
/// does.
///
/// # Errors
/// [`SqlError`] when the query does not run or its answer does not decode.
pub async fn execute(
    client: &DbClient,
    req: &CompletionDemographicsRequest,
) -> Result<Demographics, SqlError> {
    let cip = CipSelection::of(req);
    match req.group_by.unwrap_or_default() {
        DemographicsGroupBy::Total => {
            let params = TotalParams::of(req, &cip);
            let env = catalog::run_one(client, &catalog::COMPLETIONS_TOTAL, &params).await?;
            Ok(Demographics(total_response(env, req, cip.label)))
        }
        DemographicsGroupBy::School => {
            let params = SchoolParams {
                common: TotalParams::of(req, &cip),
                min_completions: req.min_completions.map(min_completions_param),
                limit: school_limit(req),
            };
            let env = catalog::run_one(client, &catalog::COMPLETIONS_BY_SCHOOL, &params).await?;
            Ok(Demographics(schools_response(env, req, &cip.label)))
        }
        DemographicsGroupBy::Cip => {
            let Some(unitid) = req.unitid else {
                return Ok(Demographics(Answer::Json(serde_json::json!({
                    "error": "group_by cip needs unitid: per-CIP rows are scoped to one institution",
                    "tip": "Pass unitid, or group by school to compare institutions",
                }))));
            };
            let params = CipParams {
                unitid,
                cip_prefix: cip.prefix,
                cip_codes: cip.codes.as_deref(),
                award_level: req.award_level,
                major_num: req.major_num,
                year: req.year,
                with_baseline: req.include_representation.unwrap_or(true),
            };
            let env = catalog::run_one(client, &catalog::COMPLETIONS_BY_CIP, &params).await?;
            Ok(Demographics(cip_response(env, req, cip.label)))
        }
    }
}

/// [`execute`] as pretty JSON, refusing inapplicable filters by field name first.
pub async fn execute_json(client: &Arc<DbClient>, req: CompletionDemographicsRequest) -> String {
    let refused = inapplicable_filters(&req);
    if !refused.is_empty() {
        let names: Vec<&str> = refused.iter().map(|f| f.field()).collect();
        let grouping = req.group_by.unwrap_or_default().as_str();
        return serde_json::json!({
            "error": format!("group_by {grouping} cannot apply {}", names.join(", ")),
            "code": "bad_arguments",
            "group_by": grouping,
            "unsupported": names,
        })
        .to_string();
    }
    match execute(client, &req).await {
        Ok(answer) => to_json_pretty(&answer),
        Err(e) => e.to_json_value().to_string(),
    }
}

// ============================================================================
// Envelope → response
// ============================================================================

fn total_response(
    env: TotalEnvelope,
    req: &CompletionDemographicsRequest,
    cip_label: String,
) -> Answer {
    if env.institutions_matched == 0 {
        return Answer::Json(serde_json::json!({
            "error": "No institutions found matching the given filters",
            "suggestion": "Try broadening institution filters (carnegie_class, control, state, unitid)"
        }));
    }
    if env.counts.total == 0 {
        return Answer::Json(serde_json::json!({
            "total_rows": 0,
            "note": "No completion records found for the given filters",
            "institutions_checked": env.institutions_matched,
            "cip_filter": cip_label,
            "year": env.year,
        }));
    }
    let baseline = usable(env.baseline);
    Answer::Total(DemographicsResponse {
        group_by: DemographicsGroupBy::Total.as_str(),
        filters: FilterSummary {
            unitid: req.unitid,
            carnegie_class: req.carnegie_class,
            control: req.control,
            state: req.state.clone(),
            hbcu: req.hbcu,
            tribal: req.tribal,
            inst_size_min: req.inst_size_min,
            cip_prefix: cip_label,
            award_level: req.award_level,
            major_num: req.major_num,
            year: env.year,
        },
        institutions_matched: env.institutions_matched,
        total_completions: env.counts.total,
        demographics: build_demographics(&env.counts, baseline.as_ref()),
        cross_tab: build_cross_tab(&env.counts, baseline.as_ref()),
    })
}

fn schools_response(
    env: SchoolsEnvelope,
    req: &CompletionDemographicsRequest,
    cip_label: &str,
) -> Answer {
    if env.institutions_matched == 0 {
        let suggestion = if req.unitid.is_some() {
            "Check that the unitid is correct (search institutions to find it)"
        } else {
            "Try broadening carnegie_class, control, state, or inst_size_min filters"
        };
        return Answer::Json(serde_json::json!({
            "error": "No institutions matched the given filters",
            "suggestion": suggestion
        }));
    }
    let year = env.year;
    let schools: Vec<SchoolDemographicsResult> = env
        .schools
        .into_iter()
        .map(|s| {
            let baseline = usable(s.baseline);
            SchoolDemographicsResult {
                unitid: s.unitid,
                name: s.name,
                city: s.city,
                state: s.state,
                carnegie_class: s.carnegie_class,
                year,
                total_completions: s.counts.total,
                demographics: build_demographics(&s.counts, baseline.as_ref()),
                cross_tab: build_cross_tab(&s.counts, baseline.as_ref()),
            }
        })
        .collect();
    Answer::Json(serde_json::json!({
        "group_by": DemographicsGroupBy::School.as_str(),
        "count": schools.len(),
        "institutions_matched": env.institutions_matched,
        "filters": {
            "unitid": req.unitid,
            "carnegie_class": req.carnegie_class,
            "control": req.control,
            "state": req.state,
            "hbcu": req.hbcu,
            "tribal": req.tribal,
            "inst_size_min": req.inst_size_min,
            "cip_filter": cip_label,
            "award_level": req.award_level,
            "major_num": req.major_num,
            "year": year,
        },
        "schools": schools
    }))
}

fn cip_response(
    env: CipEnvelope,
    req: &CompletionDemographicsRequest,
    cip_label: String,
) -> Answer {
    if env.rows.is_empty() {
        return Answer::Json(serde_json::json!({
            "group_by": DemographicsGroupBy::Cip.as_str(),
            "unitid": env.unitid,
            "name": env.name,
            "year": env.year,
            "award_level": req.award_level,
            "cip_prefix": cip_label,
            "total_rows": 0,
            "note": "No completion records found for the given filters",
            // The year the suggestions describe: the requested one, or the school's latest.
            "nearby_year": env.nearby_year,
            "nearby_cips_with_data": env.nearby_cips_with_data.unwrap_or_default(),
        }));
    }
    let baseline = usable(env.baseline);
    let mut selected = DemographicCounts::default();
    for row in &env.rows {
        selected += &row.counts;
    }
    let rows: Vec<CompletionRow> = env
        .rows
        .into_iter()
        .map(|row| build_completion_row(row, baseline.as_ref()))
        .collect();
    Answer::Cip(InstitutionCompletionsResponse {
        group_by: DemographicsGroupBy::Cip.as_str(),
        unitid: env.unitid,
        name: env.name,
        year: env.year,
        award_level: req.award_level,
        cip_prefix: cip_label,
        total_rows: rows.len(),
        note: "baseline_pct and representation_ratio compare this CIP row to the school's completions across all CIPs",
        rows,
        cross_tab: build_cross_tab(&selected, baseline.as_ref()),
    })
}

/// One CIP row with its demographic breakdown against the school baseline.
fn build_completion_row(row: CipRow, school: Option<&DemographicCounts>) -> CompletionRow {
    CompletionRow {
        year: row.year,
        demographics: build_row_demographics(&row.counts, school),
        cip_code: row.cip_code,
        cip_title: row.cip_title,
        award_level: row.award_level,
        major_num: row.major_num,
        total: row.counts.total,
    }
}

/// Per-group breakdown of one CIP row, compared with the school baseline.
fn build_row_demographics(
    row: &DemographicCounts,
    school: Option<&DemographicCounts>,
) -> Vec<RowDemographic> {
    let row_total = row.total;
    let entry = |label: &str, count: i64, school_count: Option<i64>| {
        let cip_pct = pct(count, row_total);
        let baseline_pct = school.zip(school_count).map(|(s, n)| pct(n, s.total));
        RowDemographic {
            group: label.to_string(),
            count,
            cip_pct,
            baseline_pct,
            representation_ratio: baseline_pct.and_then(|bp| representation_ratio(cip_pct, bp)),
        }
    };
    macro_rules! race {
        ($label:expr, $men:ident, $women:ident) => {
            entry(
                $label,
                row.$men + row.$women,
                school.map(|s| s.$men + s.$women),
            )
        };
    }
    vec![
        entry("Women", row.total_women, school.map(|s| s.total_women)),
        entry("Men", row.total_men, school.map(|s| s.total_men)),
        race!("Hispanic/Latino", hispanic_men, hispanic_women),
        race!("Black or African American", black_men, black_women),
        race!("Asian", asian_men, asian_women),
        race!("White", white_men, white_women),
        race!(
            "American Indian/Alaska Native",
            american_indian_men,
            american_indian_women
        ),
        race!(
            "Native Hawaiian/Pacific Islander",
            native_hawaiian_men,
            native_hawaiian_women
        ),
        race!("Two or More Races", two_or_more_men, two_or_more_women),
        race!(
            "Nonresident Alien",
            nonresident_alien_men,
            nonresident_alien_women
        ),
        race!(
            "Unknown Race/Ethnicity",
            unknown_race_men,
            unknown_race_women
        ),
    ]
}

// ============================================================================
// Shared presentation math
// ============================================================================

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

fn representation_ratio(comp_pct: f64, baseline_pct: f64) -> Option<f64> {
    if baseline_pct < 0.001 {
        None
    } else {
        Some((comp_pct / baseline_pct * 100.0).round() / 100.0)
    }
}

fn build_demographics(
    c: &DemographicCounts,
    baseline: Option<&DemographicCounts>,
) -> Vec<DemographicRepresentation> {
    let total_comp = c.total;
    let baseline_total = baseline.map(|b| b.total);

    macro_rules! group {
        ($label:expr, $comp_men:expr, $comp_women:expr, $base_men:expr, $base_women:expr) => {{
            let comp = $comp_men + $comp_women;
            let comp_pct = pct(comp, total_comp);
            let baseline_completions = baseline.map(|_| $base_men + $base_women);
            let baseline_pct = baseline_completions.map(|n| pct(n, baseline_total.unwrap_or(0)));
            let ratio = baseline_pct.and_then(|bp| representation_ratio(comp_pct, bp));
            DemographicRepresentation {
                group: $label.to_string(),
                completions: comp,
                total_completions: total_comp,
                completion_pct: comp_pct,
                baseline_completions,
                baseline_total,
                baseline_pct,
                representation_ratio: ratio,
            }
        }};
    }

    vec![
        group!(
            "Women",
            0,
            c.total_women,
            0,
            baseline.map_or(0, |b| b.total_women)
        ),
        group!(
            "Men",
            c.total_men,
            0,
            baseline.map_or(0, |b| b.total_men),
            0
        ),
        group!(
            "Hispanic/Latino",
            c.hispanic_men,
            c.hispanic_women,
            baseline.map_or(0, |b| b.hispanic_men),
            baseline.map_or(0, |b| b.hispanic_women)
        ),
        group!(
            "Black or African American",
            c.black_men,
            c.black_women,
            baseline.map_or(0, |b| b.black_men),
            baseline.map_or(0, |b| b.black_women)
        ),
        group!(
            "Asian",
            c.asian_men,
            c.asian_women,
            baseline.map_or(0, |b| b.asian_men),
            baseline.map_or(0, |b| b.asian_women)
        ),
        group!(
            "White",
            c.white_men,
            c.white_women,
            baseline.map_or(0, |b| b.white_men),
            baseline.map_or(0, |b| b.white_women)
        ),
        group!(
            "American Indian/Alaska Native",
            c.american_indian_men,
            c.american_indian_women,
            baseline.map_or(0, |b| b.american_indian_men),
            baseline.map_or(0, |b| b.american_indian_women)
        ),
        group!(
            "Native Hawaiian/Pacific Islander",
            c.native_hawaiian_men,
            c.native_hawaiian_women,
            baseline.map_or(0, |b| b.native_hawaiian_men),
            baseline.map_or(0, |b| b.native_hawaiian_women)
        ),
        group!(
            "Two or More Races",
            c.two_or_more_men,
            c.two_or_more_women,
            baseline.map_or(0, |b| b.two_or_more_men),
            baseline.map_or(0, |b| b.two_or_more_women)
        ),
        group!(
            "Nonresident Alien",
            c.nonresident_alien_men,
            c.nonresident_alien_women,
            baseline.map_or(0, |b| b.nonresident_alien_men),
            baseline.map_or(0, |b| b.nonresident_alien_women)
        ),
        group!(
            "Unknown Race/Ethnicity",
            c.unknown_race_men,
            c.unknown_race_women,
            baseline.map_or(0, |b| b.unknown_race_men),
            baseline.map_or(0, |b| b.unknown_race_women)
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

    use crate::core::database::ipeds::ingest::GRAND_TOTAL_CIP;

    fn to_json(answer: impl Serialize) -> serde_json::Value {
        serde_json::to_value(answer).expect("answer serialises")
    }

    fn counts(total: i64, men: i64, women: i64) -> DemographicCounts {
        DemographicCounts {
            total,
            total_men: men,
            total_women: women,
            white_men: men,
            white_women: women,
            ..Default::default()
        }
    }

    fn cip_row(cip_code: &str, title: Option<&str>, c: DemographicCounts) -> CipRow {
        CipRow {
            year: Some(2024),
            cip_code: cip_code.to_string(),
            cip_title: title.map(str::to_string),
            award_level: Some(5),
            major_num: Some(1),
            counts: c,
        }
    }

    fn request(group_by: DemographicsGroupBy) -> CompletionDemographicsRequest {
        CompletionDemographicsRequest {
            group_by: Some(group_by),
            ..Default::default()
        }
    }

    // ── catalog agreement ────────────────────────────────────────────────────

    #[test]
    fn each_params_struct_sends_exactly_the_keys_its_query_reads() {
        catalog::assert_params_match(&catalog::COMPLETIONS_TOTAL, &TotalParams::default());
        catalog::assert_params_match(&catalog::COMPLETIONS_BY_SCHOOL, &SchoolParams::default());
        catalog::assert_params_match(&catalog::COMPLETIONS_BY_CIP, &CipParams::default());
    }

    #[test]
    fn every_completions_query_emits_all_21_demographic_columns() {
        // A column a query forgets to emit is a decode failure at run time, since
        // `DemographicCounts` has no defaults; this catches it before then.
        let fields = serde_json::to_value(DemographicCounts::default()).expect("encode");
        for query in [
            &catalog::COMPLETIONS_TOTAL,
            &catalog::COMPLETIONS_BY_SCHOOL,
            &catalog::COMPLETIONS_BY_CIP,
        ] {
            for field in fields.as_object().expect("object").keys() {
                assert!(
                    query.sql.contains(&format!(" AS {field}")),
                    "{} does not emit {field}",
                    query.name
                );
            }
            let excluded = format!("cip_code <> '{GRAND_TOTAL_CIP}'");
            assert_eq!(
                query.sql.matches("FROM completions c").count(),
                query.sql.matches(excluded.as_str()).count(),
                "{}: every scan of completions must leave out the CIP 99 grand totals",
                query.name
            );
        }
    }

    // ── request handling ─────────────────────────────────────────────────────

    #[test]
    fn inapplicable_filters_refuse_what_each_grouping_cannot_apply() {
        let mut req = CompletionDemographicsRequest {
            carnegie_class: Some(15),
            state: Some("MA".into()),
            hbcu: Some(true),
            limit: Some(10),
            min_completions: Some(5),
            ..Default::default()
        };
        req.group_by = Some(DemographicsGroupBy::School);
        assert!(
            inapplicable_filters(&req).is_empty(),
            "school applies every filter"
        );

        req.group_by = Some(DemographicsGroupBy::Total);
        assert_eq!(
            inapplicable_filters(&req),
            [DemoFilter::MinCompletions, DemoFilter::Limit],
            "total applies hbcu and the other institution filters"
        );

        req.group_by = Some(DemographicsGroupBy::Cip);
        let refused: Vec<&str> = inapplicable_filters(&req)
            .iter()
            .map(|f| f.field())
            .collect();
        assert_eq!(
            refused,
            [
                "carnegie_class",
                "state",
                "hbcu",
                "min_completions",
                "limit"
            ]
        );

        assert!(inapplicable_filters(&request(DemographicsGroupBy::Cip)).is_empty());
    }

    #[test]
    fn exact_cip_codes_take_priority_over_a_prefix() {
        let req = CompletionDemographicsRequest {
            cip_prefix: Some("11.".into()),
            cip_codes: Some("11.0101, 11.0701".into()),
            ..Default::default()
        };
        let cip = CipSelection::of(&req);
        assert_eq!(cip.prefix, None);
        assert_eq!(
            cip.codes.as_deref(),
            Some(&["11.0101".to_string(), "11.0701".to_string()][..])
        );
        assert_eq!(cip.label, "11.0101,11.0701");

        let only_prefix = CompletionDemographicsRequest {
            cip_prefix: Some("11.".into()),
            cip_codes: Some(" , ".into()),
            ..Default::default()
        };
        let cip = CipSelection::of(&only_prefix);
        assert_eq!((cip.prefix, cip.codes.is_none()), (Some("11."), true));
        assert_eq!(
            CipSelection::of(&CompletionDemographicsRequest::default()).label,
            "(all CIPs)"
        );
    }

    #[test]
    fn group_by_reads_and_defaults_the_way_callers_write_it() {
        let req: CompletionDemographicsRequest =
            serde_json::from_value(serde_json::json!({ "group_by": "school" })).expect("decode");
        assert_eq!(req.group_by, Some(DemographicsGroupBy::School));
        assert_eq!(
            CompletionDemographicsRequest::default()
                .group_by
                .unwrap_or_default(),
            DemographicsGroupBy::Total
        );
    }

    #[test]
    fn school_limit_defaults_and_caps_the_same_in_rust_and_sql() {
        let mut req = request(DemographicsGroupBy::School);
        assert_eq!(school_limit(&req), DEFAULT_SCHOOL_LIMIT);
        req.limit = Some(1_000);
        assert_eq!(school_limit(&req), MAX_SCHOOL_LIMIT);
        assert!(
            catalog::COMPLETIONS_BY_SCHOOL.sql.contains(&format!(
                "least(coalesce(lim, {DEFAULT_SCHOOL_LIMIT}), {MAX_SCHOOL_LIMIT})"
            )),
            "completions_by_school.sql must bound the limit as Rust does"
        );
    }

    #[test]
    fn total_and_school_match_institutions_and_pick_the_year_identically() {
        // The two files repeat these CTEs (catalog queries are compiled in verbatim, with
        // no templating). If they drifted, `total` and `school` would silently disagree
        // about which schools matched or which year was reported.
        let blocks = |sql: &str| -> String {
            let code: String = crate::core::query::sql::lex(sql)
                .into_iter()
                .filter(|(kind, _)| *kind != crate::core::query::sql::Span::Comment)
                .map(|(_, t)| t)
                .collect();
            let start = code.find("matched AS (").expect("matched CTE");
            let end = code.find("yr AS (").expect("yr CTE");
            let end = end + code[end..].find("\n)").expect("yr CTE closes");
            code[start..end]
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        };
        assert_eq!(
            blocks(catalog::COMPLETIONS_TOTAL.sql),
            blocks(catalog::COMPLETIONS_BY_SCHOOL.sql)
        );
    }

    #[test]
    fn min_completions_is_clamped_to_what_the_query_can_read() {
        assert_eq!(min_completions_param(i64::MAX), i64::from(i32::MAX));
        assert_eq!(min_completions_param(-5), 0);
        assert_eq!(min_completions_param(20), 20);
    }

    /// A client that fails any query it is asked to run: nothing is listening on port 9.
    fn offline() -> Arc<DbClient> {
        Arc::new(DbClient::new("http://127.0.0.1:9", "anon", "jwt".into()).expect("stub client"))
    }

    #[tokio::test]
    async fn execute_json_refuses_inapplicable_filters_by_field_before_querying() {
        let req = CompletionDemographicsRequest {
            group_by: Some(DemographicsGroupBy::Cip),
            unitid: Some(1),
            state: Some("MA".into()),
            limit: Some(5),
            ..Default::default()
        };
        let v: serde_json::Value =
            serde_json::from_str(&execute_json(&offline(), req).await).expect("json");
        assert_eq!(v["unsupported"], serde_json::json!(["state", "limit"]));
        assert_eq!(v["group_by"], "cip");
    }

    #[tokio::test]
    async fn cip_without_unitid_is_an_error_payload_not_a_query() {
        let out = execute(&offline(), &request(DemographicsGroupBy::Cip))
            .await
            .expect("answered without a query");
        let out = to_json(out);
        assert!(out["error"].as_str().unwrap().contains("unitid"), "{out}");
    }

    #[test]
    fn a_response_keeps_its_declared_field_order() {
        // Through a `serde_json::Value` the keys would come out sorted, putting
        // `cross_tab` first; the text a reader sees follows the struct instead.
        let env = TotalEnvelope {
            year: Some(2024),
            institutions_matched: 1,
            counts: counts(10, 5, 5),
            baseline: None,
        };
        let text = to_json_pretty(&Demographics(total_response(
            env,
            &request(DemographicsGroupBy::Total),
            "11.".into(),
        )));
        let at = |key: &str| text.find(&format!("\"{key}\"")).expect(key);
        assert!(
            at("group_by") < at("filters") && at("filters") < at("cross_tab"),
            "{text}"
        );
    }

    // ── envelope → response ─────────────────────────────────────────────────

    #[test]
    fn a_total_envelope_becomes_demographics_with_ratios_against_its_baseline() {
        let env: TotalEnvelope = serde_json::from_value(serde_json::json!({
            "year": 2024,
            "institutions_matched": 3,
            "counts": serde_json::to_value(counts(100, 60, 40)).unwrap(),
            "baseline": serde_json::to_value(counts(1000, 600, 400)).unwrap(),
        }))
        .expect("envelope decodes");
        let out = to_json(total_response(
            env,
            &request(DemographicsGroupBy::Total),
            "11.".into(),
        ));
        assert_eq!(out["group_by"], "total");
        assert_eq!(out["institutions_matched"], 3);
        assert_eq!(out["total_completions"], 100);
        assert_eq!(out["filters"]["year"], 2024);
        assert_eq!(out["filters"]["cip_prefix"], "11.");
        let women = &out["demographics"][0];
        assert_eq!(women["group"], "Women");
        assert_eq!(women["representation_ratio"], 1.0);
    }

    #[test]
    fn a_total_with_no_institutions_or_no_completions_says_which() {
        let none_matched = TotalEnvelope {
            year: None,
            institutions_matched: 0,
            counts: DemographicCounts::default(),
            baseline: None,
        };
        let out = to_json(total_response(
            none_matched,
            &request(DemographicsGroupBy::Total),
            String::new(),
        ));
        assert!(out["error"].as_str().unwrap().contains("No institutions"));

        let no_rows = TotalEnvelope {
            year: Some(2024),
            institutions_matched: 5,
            counts: DemographicCounts::default(),
            baseline: None,
        };
        let out = to_json(total_response(
            no_rows,
            &request(DemographicsGroupBy::Total),
            "01.".into(),
        ));
        assert_eq!(out["total_rows"], 0);
        assert_eq!(out["institutions_checked"], 5);
        assert!(
            out.get("error").is_none(),
            "an empty result is a note, not an error"
        );
    }

    #[test]
    fn an_empty_baseline_gives_no_ratio_rather_than_a_division_by_zero() {
        let env = TotalEnvelope {
            year: Some(2024),
            institutions_matched: 1,
            counts: counts(10, 5, 5),
            baseline: Some(DemographicCounts::default()),
        };
        let out = to_json(total_response(
            env,
            &request(DemographicsGroupBy::Total),
            String::new(),
        ));
        assert!(out["demographics"][0]["representation_ratio"].is_null());
    }

    #[test]
    fn a_schools_envelope_keeps_the_databases_ranking_and_carries_the_year() {
        let school = |unitid: i32, total: i64| {
            serde_json::json!({
                "unitid": unitid, "name": format!("School {unitid}"), "city": null,
                "state": "MA", "carnegie_class": 15,
                "counts": serde_json::to_value(counts(total, total / 2, total - total / 2)).unwrap(),
                "baseline": null,
            })
        };
        let env: SchoolsEnvelope = serde_json::from_value(serde_json::json!({
            "year": 2024, "institutions_matched": 147,
            "schools": [school(2, 900), school(1, 400)],
        }))
        .expect("envelope decodes");
        let out = to_json(schools_response(
            env,
            &request(DemographicsGroupBy::School),
            "11.",
        ));
        assert_eq!(out["count"], 2);
        assert_eq!(out["institutions_matched"], 147);
        assert_eq!(
            out["schools"][0]["unitid"], 2,
            "ranked by the query, not re-sorted"
        );
        assert_eq!(out["schools"][1]["year"], 2024);
        assert!(out["schools"][0]["demographics"][0]["representation_ratio"].is_null());
    }

    #[test]
    fn a_school_search_matching_nothing_is_an_error_naming_the_fix() {
        let env = SchoolsEnvelope {
            year: None,
            institutions_matched: 0,
            schools: Vec::new(),
        };
        let mut req = request(DemographicsGroupBy::School);
        req.unitid = Some(1);
        let out = to_json(schools_response(env, &req, ""));
        assert!(out["suggestion"].as_str().unwrap().contains("unitid"));
    }

    #[test]
    fn a_cip_envelope_gives_rows_and_a_cross_tab_summed_over_them() {
        let env = CipEnvelope {
            unitid: 167_358,
            name: Some("Northeastern University".into()),
            year: Some(2024),
            rows: vec![
                cip_row("11.0101", Some("Computer Science"), counts(100, 60, 40)),
                cip_row("11.0701", None, counts(50, 30, 20)),
            ],
            baseline: Some(counts(1000, 500, 500)),
            nearby_year: None,
            nearby_cips_with_data: None,
        };
        let mut req = request(DemographicsGroupBy::Cip);
        req.cip_prefix = Some("11.".into());
        let out = to_json(cip_response(env, &req, "11.".into()));
        assert_eq!(out["total_rows"], 2);
        assert_eq!(out["rows"][0]["cip_title"], "Computer Science");
        assert!(out["rows"][1]["cip_title"].is_null());
        let white = out["cross_tab"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["group"] == "White")
            .unwrap();
        assert_eq!(white["women_count"], 60, "40 + 20 across both rows");
    }

    #[test]
    fn an_empty_cip_answer_suggests_the_schools_other_cips() {
        let env = CipEnvelope {
            unitid: 167_358,
            name: None,
            year: None,
            rows: Vec::new(),
            baseline: None,
            nearby_year: Some(2025),
            nearby_cips_with_data: Some(vec![NearbyCip {
                cip_code: "11.0101".into(),
                cip_title: Some("CS".into()),
                total_completions: 2043,
            }]),
        };
        let out = to_json(cip_response(
            env,
            &request(DemographicsGroupBy::Cip),
            "01.".into(),
        ));
        assert_eq!(out["total_rows"], 0);
        assert_eq!(out["nearby_cips_with_data"][0]["cip_code"], "11.0101");
        assert_eq!(
            out["nearby_year"], 2025,
            "says which year the suggestions describe"
        );
        assert_eq!(
            out["cip_prefix"], "01.",
            "echoes the filter that came back empty"
        );
    }

    #[test]
    fn a_cip_row_decodes_its_counts_beside_its_identity() {
        let mut row = serde_json::to_value(counts(7, 3, 4)).unwrap();
        for (k, v) in [
            ("year", serde_json::json!(2024)),
            ("cip_code", serde_json::json!("11.0701")),
            ("cip_title", serde_json::Value::Null),
            ("award_level", serde_json::json!(5)),
            ("major_num", serde_json::json!(2)),
        ] {
            row[k] = v;
        }
        let row: CipRow = serde_json::from_value(row).expect("row decodes");
        assert_eq!(
            (row.cip_code.as_str(), row.major_num, row.counts.total),
            ("11.0701", Some(2), 7)
        );
    }

    // ── build_row_demographics() / build_completion_row() ────────────────────

    #[test]
    fn test_build_row_demographics_returns_11_groups() {
        assert_eq!(build_row_demographics(&counts(100, 60, 40), None).len(), 11);
    }

    #[test]
    fn test_build_row_demographics_gender_counts() {
        let groups = build_row_demographics(&counts(100, 60, 40), None);
        let women = groups.iter().find(|d| d.group == "Women").unwrap();
        let men = groups.iter().find(|d| d.group == "Men").unwrap();
        assert_eq!(women.count, 40);
        assert_float_eq(women.cip_pct, 40.0);
        assert_eq!(men.count, 60);
        assert_float_eq(men.cip_pct, 60.0);
        assert!(women.baseline_pct.is_none()); // no school totals provided
    }

    #[test]
    fn test_build_row_demographics_race_group_sums_men_and_women() {
        let row = DemographicCounts {
            total: 200,
            total_men: 100,
            total_women: 100,
            hispanic_men: 20,
            hispanic_women: 15,
            white_men: 80,
            white_women: 85,
            ..Default::default()
        };
        let groups = build_row_demographics(&row, None);
        let hispanic = groups
            .iter()
            .find(|d| d.group == "Hispanic/Latino")
            .unwrap();
        assert_eq!(hispanic.count, 35); // 20 + 15
        assert_float_eq(hispanic.cip_pct, 17.5);
    }

    #[test]
    fn test_build_row_demographics_with_school_totals() {
        let school = DemographicCounts {
            total: 200,
            total_men: 80,
            total_women: 120,
            ..Default::default()
        };
        let groups = build_row_demographics(&counts(100, 40, 60), Some(&school));
        let women = groups.iter().find(|d| d.group == "Women").unwrap();
        // women cip_pct=60%, baseline_pct=60% → ratio=1.0
        assert_float_opt_eq(women.baseline_pct, Some(60.0));
        assert_float_opt_eq(women.representation_ratio, Some(1.0));
    }

    #[test]
    fn test_build_completion_row_carries_identity_and_computes_ratios() {
        let school = DemographicCounts {
            total: 1000,
            total_men: 600,
            total_women: 400,
            ..Default::default()
        };
        let row = build_completion_row(
            cip_row("11.0101", Some("Computer Science"), counts(100, 60, 40)),
            Some(&school),
        );
        assert_eq!(row.cip_code, "11.0101");
        assert_eq!(row.cip_title.as_deref(), Some("Computer Science"));
        assert_eq!(
            (row.award_level, row.major_num, row.total),
            (Some(5), Some(1), 100)
        );
        let women = row
            .demographics
            .iter()
            .find(|d| d.group == "Women")
            .unwrap();
        // cip_pct = 40%, baseline_pct = 40% → ratio = 1.0
        assert_float_eq(women.cip_pct, 40.0);
        assert_float_opt_eq(women.representation_ratio, Some(1.0));
        // Public schema guarantee: every row serialises a `year`.
        let json = serde_json::to_value(&row).unwrap();
        assert_eq!(json.get("year"), Some(&serde_json::json!(2024)));
    }

    // ── AddAssign (was merge_counts) ─────────────────────────────────────────

    #[test]
    fn test_add_assign_basic() {
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
        dst += &src;
        assert_eq!(dst.total, 150);
        assert_eq!(dst.total_men, 60);
        assert_eq!(dst.total_women, 90);
    }

    #[test]
    fn test_add_assign_all_demographic_fields() {
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
        dst += &src;
        assert_eq!(dst.hispanic_men, 10);
        assert_eq!(dst.hispanic_women, 12);
        assert_eq!(dst.asian_men, 8);
        assert_eq!(dst.black_women, 6);
    }

    #[test]
    fn test_add_assign_sequential_batches() {
        let mut result = DemographicCounts::default();
        result += &DemographicCounts {
            total: 50,
            total_men: 20,
            ..Default::default()
        };
        result += &DemographicCounts {
            total: 30,
            total_men: 12,
            ..Default::default()
        };
        assert_eq!(result.total, 80);
        assert_eq!(result.total_men, 32);
    }

    // ── pct() / representation_ratio() / build_cross_tab() / build_demographics() ──

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
    fn test_representation_ratio_zero_baseline_returns_none() {
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
    fn test_serialized_baseline_fields_are_named_for_what_they_count() {
        let c = DemographicCounts {
            total: 10,
            total_women: 4,
            total_men: 6,
            ..Default::default()
        };
        let group = serde_json::to_value(&build_demographics(&c, Some(&c))[0]).unwrap();
        let keys: std::collections::BTreeSet<&str> = group
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            keys,
            [
                "baseline_completions",
                "baseline_pct",
                "baseline_total",
                "completion_pct",
                "completions",
                "group",
                "representation_ratio",
                "total_completions",
            ]
            .into_iter()
            .collect()
        );
        let row = serde_json::to_value(&build_row_demographics(&c, Some(&c))[0]).unwrap();
        let row_keys: std::collections::BTreeSet<&str> = row
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(
            row_keys,
            [
                "baseline_pct",
                "cip_pct",
                "count",
                "group",
                "representation_ratio"
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn test_build_demographics_no_baseline_no_ratio() {
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
        // Both completions and the baseline are 60% women → ratio = 1.0 (proportional)
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
