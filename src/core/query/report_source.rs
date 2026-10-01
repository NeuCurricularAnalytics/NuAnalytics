//! Rebuild a degree report's inputs from stored analysis rows.
//!
//! The HTML report is normally produced at the end of an analysis run, from objects that
//! only exist in memory. Everything it actually reads, though, has been persisted: the
//! reduced statistics in `analysis_runs.degree_metrics` and `analysis_course_metrics`,
//! the curated plans in `analysis_plans`, and the canonical degree in
//! `programs.document`. Loading those is both faster than re-running the analysis (no
//! plan enumeration) and reproducible, which a fresh run is not — the sampled plans a
//! run selects vary between invocations.
//!
//! What cannot be rebuilt is the [`MetricsAggregator`](crate::core::statistics::aggregator::MetricsAggregator)
//! itself: its Welford accumulators and quantile reservoirs hold every per-plan
//! observation and only their reduction is stored. That is why the report takes a
//! [`ReportStats`] — see its module docs.
//!
//! Fields the schema does not carry are filled with values the renderer provably does not
//! read (`PlanScore::avg_chain_length`, `PlanVariant::requirement_choices`,
//! `ScoredPlan::course_metrics`). Each is noted at the site rather than silently defaulted.

use std::collections::HashMap;
use std::sync::Arc;

use serde::Deserialize;

use super::degrees::resolve_program_key;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::degree::plan_selector::PlanCategory;
use crate::core::degree::{from_unified_value, PlanScore, PlanVariant, ScoredPlan, SelectedPlans};
use crate::core::json::parse_json_array;
use crate::core::report::inputs::build_report_inputs;
use crate::core::report::report_stats::ReportStats;
use crate::core::report::term_scheduler::{Term, TermPlan};
use crate::core::report::{DegreeReportContext, DegreeReportGenerator};
use crate::core::statistics::aggregator::{
    AggregatedCourseStats, AggregatedDegreeStats, MetricStats,
};
use crate::core::DegreeProgram;

/// Plan categories as written by the importer — each category's display name. Matched
/// exactly, not fuzzily.
const CAT_SHORTEST: &str = PlanCategory::Shortest.display_name();
const CAT_LONGEST: &str = PlanCategory::Longest.display_name();
const CAT_CALC_READY: &str = PlanCategory::CalcReadyShortest.display_name();

/// Cap on plan rows read for one run. A run stores a handful of curated plans, so this
/// only bounds a pathological row set.
pub(crate) const MAX_PLAN_ROWS: usize = 500;

/// Cap on per-course metric rows for one run.
pub(crate) const MAX_COURSE_ROWS: usize = 5_000;

/// Identity of the stored run a report was built from, as `analysis_runs` records it.
#[derive(Debug, Clone, Deserialize)]
pub struct StoredRun {
    /// Unique key of the analysis run.
    pub run_key: String,
    /// Variant label, e.g. `full` or `trimmed`.
    pub variant: String,
    /// When the run was recorded.
    pub created_at: Option<String>,
    /// Analyzer version that produced it.
    pub analyzer_version: Option<String>,
    /// Plans the run enumerated.
    pub variations_run: Option<i64>,
    /// The plan cap the run was given.
    pub max_plans: Option<i64>,
    /// The seed the run enumerated with, as stored (a `u64`'s digits).
    pub random_seed: Option<String>,
}

/// Everything needed to render a report for one stored program.
#[derive(Debug)]
pub struct StoredReport {
    /// Canonical degree, parsed back from `programs.document`.
    pub program: DegreeProgram,
    /// Reduced statistics the renderer consumes.
    pub stats: ReportStats,
    /// Curated plans.
    pub selected: SelectedPlans,
    /// Which run this came from.
    pub run: StoredRun,
}

impl StoredReport {
    /// Render the HTML report for this run.
    ///
    /// The one rendering both `db report` and the MCP `render_stored_report` use, so the
    /// two cannot produce different pages for the same stored run.
    ///
    /// # Errors
    /// Returns the renderer's message when the report template cannot be filled.
    pub fn render_html(&self) -> Result<String, String> {
        let (school, equivalences) = build_report_inputs(&self.program);
        let ctx = DegreeReportContext::new(
            &school,
            &self.program.degree,
            &self.stats,
            &self.selected,
            &equivalences,
        );
        DegreeReportGenerator::new()
            .render(&ctx)
            .map_err(|e| format!("could not render the report: {e}"))
    }
}

/// Why a stored run could not be loaded.
#[derive(Debug)]
pub enum LoadError {
    /// The backend answered: no run of that variant is stored for the program.
    NoRun(String),
    /// The backend failed, or what it returned could not be used.
    Failed(String),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoRun(message) | Self::Failed(message) => f.write_str(message),
        }
    }
}

/// A stored run's report, rendered, with the run it came from.
#[derive(Debug)]
pub struct RenderedReport {
    /// The program the reference resolved to.
    pub program_key: String,
    /// The degree's display name.
    pub degree_name: String,
    /// Which run the report was built from.
    pub run: StoredRun,
    /// The HTML page.
    pub html: String,
}

// ============================================================================
// Row types
// ============================================================================

#[derive(Debug, Deserialize)]
struct RunRow {
    #[serde(flatten)]
    run: StoredRun,
    degree_metrics: Option<serde_json::Value>,
    /// The degree the run analyzed, when it is not the program's own document — a
    /// trimmed run's trimmed degree. `NULL` for full runs.
    analyzed_document: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct CourseMetricRow {
    course_code: String,
    plan_count: Option<i64>,
    metrics: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
struct PlanRow {
    category: Option<String>,
    terms_required: Option<i64>,
    total_complexity: Option<f64>,
    longest_delay: Option<f64>,
    credits: Option<f64>,
    is_calc_ready: Option<bool>,
    critical_path: Option<serde_json::Value>,
    schedule: Option<serde_json::Value>,
}

/// The five metric summaries stored per course.
#[derive(Debug, Deserialize)]
struct StoredCourseMetrics {
    complexity: MetricStats,
    centrality: MetricStats,
    delay: MetricStats,
    blocking: MetricStats,
    chain_length: MetricStats,
}

/// The four metric summaries stored per run.
#[derive(Debug, Deserialize)]
struct StoredDegreeMetrics {
    complexity: MetricStats,
    delay: MetricStats,
    credits: MetricStats,
    avg_chain_length: MetricStats,
}

// ============================================================================
// Pure conversions
// ============================================================================

/// Turn `analysis_runs.degree_metrics` into the degree-level summary.
///
/// The JSON keys are the metric's own name (`complexity`, `delay`, …) while the struct
/// names the quantity measured (`total_complexity`, `longest_delay`, …), so the mapping
/// is written out rather than derived.
fn degree_stats_from_json(
    metrics: &serde_json::Value,
    plan_count: usize,
) -> Option<AggregatedDegreeStats> {
    let stored: StoredDegreeMetrics = serde_json::from_value(metrics.clone()).ok()?;
    Some(AggregatedDegreeStats {
        plan_count,
        total_complexity: stored.complexity,
        longest_delay: stored.delay,
        total_credits: stored.credits,
        avg_chain_length: stored.avg_chain_length,
    })
}

/// Turn one `analysis_course_metrics` row into a course summary.
fn course_stats_from_row(row: &CourseMetricRow) -> Option<AggregatedCourseStats> {
    let stored: StoredCourseMetrics = serde_json::from_value(row.metrics.clone()?).ok()?;
    Some(AggregatedCourseStats {
        course_id: row.course_code.clone(),
        plan_count: usize::try_from(row.plan_count.unwrap_or(0)).unwrap_or(0),
        complexity: stored.complexity,
        centrality: stored.centrality,
        delay: stored.delay,
        blocking: stored.blocking,
        chain_length: stored.chain_length,
    })
}

/// Rebuild a term-by-term schedule from the stored `[{term, courses, credits}, …]`.
fn term_plan_from_schedule(schedule: Option<&serde_json::Value>) -> TermPlan {
    #[derive(Deserialize)]
    struct StoredTerm {
        term: usize,
        #[serde(default)]
        courses: Vec<String>,
        #[serde(default)]
        credits: f32,
    }

    let terms: Vec<Term> = schedule
        .and_then(|v| serde_json::from_value::<Vec<StoredTerm>>(v.clone()).ok())
        .unwrap_or_default()
        .into_iter()
        .map(|t| Term {
            number: t.term,
            courses: t.courses,
            total_credits: t.credits,
        })
        .collect();

    TermPlan {
        terms,
        // Not stored, and not read by the renderer — it walks `terms` directly.
        is_quarter_system: false,
        target_credits: 0.0,
        unscheduled: Vec::new(),
    }
}

/// Rebuild one curated plan.
fn scored_plan_from_row(row: &PlanRow) -> ScoredPlan {
    let schedule = term_plan_from_schedule(row.schedule.as_ref());
    let courses: Vec<String> = schedule
        .terms
        .iter()
        .flat_map(|t| t.courses.iter().cloned())
        .collect();
    let credits = row.credits.unwrap_or(0.0);

    #[allow(clippy::cast_possible_truncation)]
    let total_credits = credits as f32;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let total_complexity = row.total_complexity.unwrap_or(0.0).max(0.0) as usize;
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let longest_delay = row.longest_delay.unwrap_or(0.0).max(0.0) as usize;

    ScoredPlan {
        // `requirement_choices` is not stored. The renderer reads only `variant.courses`,
        // so an empty map cannot change the output.
        variant: PlanVariant::from_parts(courses, HashMap::new(), total_credits),
        score: PlanScore {
            terms_required: usize::try_from(row.terms_required.unwrap_or(0)).unwrap_or(0),
            total_complexity,
            longest_delay,
            longest_delay_chain: row
                .critical_path
                .as_ref()
                .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
                .unwrap_or_default(),
            is_calc_ready: row.is_calc_ready.unwrap_or(false),
            // Not a column on `analysis_plans`, and not read by the renderer.
            avg_chain_length: 0.0,
        },
        schedule,
        // Per-plan course metrics are not stored. The renderer builds empty maps of its
        // own for the plans it constructs, so this matches what it already does.
        course_metrics: HashMap::new(),
    }
}

/// Sort stored plan rows into the curated buckets the report expects.
fn selected_plans_from_rows(rows: &[PlanRow], total_plans_seen: usize) -> SelectedPlans {
    let find = |category: &str| {
        rows.iter()
            .find(|r| r.category.as_deref() == Some(category))
            .map(scored_plan_from_row)
    };
    let random_samples: Vec<ScoredPlan> = rows
        .iter()
        .filter(|r| {
            !matches!(
                r.category.as_deref(),
                Some(CAT_SHORTEST | CAT_LONGEST | CAT_CALC_READY)
            )
        })
        .map(scored_plan_from_row)
        .collect();

    SelectedPlans {
        shortest: find(CAT_SHORTEST),
        longest: find(CAT_LONGEST),
        calc_ready_shortest: find(CAT_CALC_READY),
        random_samples,
        total_plans_seen,
        // Only meaningful to the selector that dropped a duplicate; nothing is stored,
        // and a stored run that suppressed one simply has no calc-ready row.
        calc_ready_suppressed: false,
    }
}

// ============================================================================
// Loading
// ============================================================================

/// Columns needed from `analysis_runs`.
const RUN_COLS: &str = "run_key,variant,created_at,analyzer_version,variations_run,max_plans,\
                        random_seed,degree_metrics,analyzed_document";

/// Load the newest stored run for `program_key`, optionally pinned to one `variant`.
///
/// # Errors
/// [`LoadError::NoRun`] when no run matches; [`LoadError::Failed`] when the backend fails
/// or the stored rows cannot be read back into a degree and its statistics.
pub async fn load(
    client: &Arc<DbClient>,
    program_key: &str,
    variant: Option<&str>,
) -> Result<StoredReport, LoadError> {
    load_run(client, program_key, variant)
        .await?
        .ok_or_else(|| {
            LoadError::NoRun(variant.map_or_else(
                || format!("no analysis run stored for {program_key}"),
                |v| format!("no `{v}` analysis run stored for {program_key}"),
            ))
        })
}

/// [`load`], with "no such run" as `None`.
async fn load_run(
    client: &Arc<DbClient>,
    program_key: &str,
    variant: Option<&str>,
) -> Result<Option<StoredReport>, LoadError> {
    let failed = LoadError::Failed;
    let filters = QueryFilters::new()
        .eq("program_key", Some(program_key))
        .eq("variant", variant)
        .order_desc("created_at");
    let runs: Vec<RunRow> = parse_json_array(
        &client
            .select(tables::ANALYSIS_RUNS, RUN_COLS, &filters, Some(1))
            .await
            .map_err(|e| failed(e.to_string()))?,
    );
    let Some(run) = runs.into_iter().next() else {
        return Ok(None);
    };

    // The degree the run analyzed: a trimmed run's statistics describe the trimmed degree,
    // and read against the program's full document they would sit beside courses the run
    // never saw.
    let program = match &run.analyzed_document {
        Some(document) => program_from_document(document, program_key),
        None => load_program(client, program_key).await,
    }
    .map_err(failed)?;
    let stats = load_stats(client, &run).await.map_err(failed)?;
    let plans = load_plans(client, &run.run.run_key).await.map_err(failed)?;

    let total_plans_seen = usize::try_from(run.run.variations_run.unwrap_or(0)).unwrap_or(0);
    Ok(Some(StoredReport {
        program,
        stats,
        selected: selected_plans_from_rows(&plans, total_plans_seen),
        run: run.run,
    }))
}

/// Why a stored program's run could not be read from a reference to it.
#[derive(Debug)]
pub enum ReferenceError {
    /// The reference named no program, or several, or resolving it failed: a finished JSON
    /// payload for the caller to return as is.
    Unresolved(String),
    /// It named one program, whose run could not be loaded.
    Load {
        /// The program the reference resolved to.
        program_key: String,
        /// Why its run could not be loaded.
        error: LoadError,
    },
}

impl ReferenceError {
    /// The JSON payload a caller returns: the resolution's own, or the load failure with
    /// the program it was for.
    #[must_use]
    pub fn into_payload(self) -> String {
        match self {
            Self::Unresolved(payload) => payload,
            Self::Load { program_key, error } => {
                serde_json::json!({ "error": error.to_string(), "program_key": program_key })
                    .to_string()
            }
        }
    }
}

/// Resolve `degree` — a `program_key`, or a `degree_id` naming one program — and load its
/// newest stored run, optionally pinned to one `variant`.
///
/// # Errors
/// [`ReferenceError::Unresolved`] when the reference does not name exactly one program;
/// [`ReferenceError::Load`] when that program's run cannot be loaded.
pub async fn load_reference(
    client: &Arc<DbClient>,
    degree: &str,
    variant: Option<&str>,
) -> Result<(String, StoredReport), ReferenceError> {
    let program_key = resolve_program_key(client, degree.trim())
        .await
        .map_err(ReferenceError::Unresolved)?;
    match load(client, &program_key, variant).await {
        Ok(report) => Ok((program_key, report)),
        Err(error) => Err(ReferenceError::Load { program_key, error }),
    }
}

/// Resolve `degree` — a `program_key`, or a `degree_id` naming one program — and render
/// the report for its newest stored run, optionally pinned to one `variant`.
///
/// # Errors
/// A finished JSON payload, which callers return verbatim: the resolution's own when the
/// reference names no program or several, otherwise `{"error": ...}` carrying [`load`]'s
/// or the renderer's message.
pub async fn render_reference(
    client: &Arc<DbClient>,
    degree: &str,
    variant: Option<&str>,
) -> Result<RenderedReport, String> {
    let (program_key, stored) = load_reference(client, degree, variant)
        .await
        .map_err(ReferenceError::into_payload)?;
    let html = stored.render_html().map_err(|message| {
        serde_json::json!({ "error": message, "program_key": program_key }).to_string()
    })?;
    Ok(RenderedReport {
        degree_name: stored.program.degree.name.clone(),
        program_key,
        run: stored.run,
        html,
    })
}

/// Fetch and parse the canonical degree document.
async fn load_program(client: &Arc<DbClient>, program_key: &str) -> Result<DegreeProgram, String> {
    let document = super::degrees::document_for_key(client, program_key)
        .await
        .map_err(|e| format!("reading the document of {program_key}: {e}"))?
        .ok_or_else(|| format!("no stored program `{program_key}` with a degree document"))?;
    program_from_document(&document, program_key)
}

/// Parse a stored degree document, naming the program when it does not parse.
fn program_from_document(
    document: &serde_json::Value,
    program_key: &str,
) -> Result<DegreeProgram, String> {
    from_unified_value(document)
        .map_err(|e| format!("stored document for `{program_key}` did not parse: {e}"))
}

/// Assemble the statistics the renderer reads.
async fn load_stats(client: &Arc<DbClient>, row: &RunRow) -> Result<ReportStats, String> {
    let run = &row.run;
    let plan_count = usize::try_from(run.variations_run.unwrap_or(0)).unwrap_or(0);
    let degree = row
        .degree_metrics
        .as_ref()
        .and_then(|m| degree_stats_from_json(m, plan_count))
        .ok_or_else(|| {
            format!(
                "run {} has no usable degree_metrics — re-import the program",
                run.run_key
            )
        })?;

    let filters = QueryFilters::new().eq("run_key", Some(&run.run_key));
    let rows: Vec<CourseMetricRow> = parse_json_array(
        &client
            .select(
                tables::ANALYSIS_COURSE_METRICS,
                "course_code,plan_count,metrics",
                &filters,
                Some(MAX_COURSE_ROWS),
            )
            .await
            .map_err(|e| e.to_string())?,
    );
    let courses: HashMap<String, AggregatedCourseStats> = rows
        .iter()
        .filter_map(|r| course_stats_from_row(r).map(|s| (r.course_code.clone(), s)))
        .collect();

    Ok(ReportStats::new(degree, courses))
}

/// Fetch the curated plans for a run.
async fn load_plans(client: &Arc<DbClient>, run_key: &str) -> Result<Vec<PlanRow>, String> {
    let filters = QueryFilters::new().eq("run_key", Some(run_key));
    Ok(parse_json_array(
        &client
            .select(
                tables::ANALYSIS_PLANS,
                "category,terms_required,total_complexity,longest_delay,credits,is_calc_ready,critical_path,schedule",
                &filters,
                Some(MAX_PLAN_ROWS),
            )
            .await
            .map_err(|e| e.to_string())?,
    ))
}

#[cfg(test)]
mod tests {
    /// Every `RunRow` field is optional and `PostgREST` returns only the columns selected, so
    /// a column dropped from `RUN_COLS` would read as absent without any error — a trimmed
    /// run against the full document, or a stored run's cap and seed lost.
    #[test]
    fn run_cols_select_every_column_a_run_row_reads() {
        let cols: std::collections::HashSet<&str> = RUN_COLS.split(',').map(str::trim).collect();
        for column in [
            "run_key",
            "variant",
            "created_at",
            "analyzer_version",
            "variations_run",
            "max_plans",
            "random_seed",
            "degree_metrics",
            "analyzed_document",
        ] {
            assert!(cols.contains(column), "RUN_COLS lacks {column}");
        }
    }

    /// A row of the select's shape reads into the run and its own degree; a document that
    /// is not a degree is refused naming the program.
    #[test]
    fn a_trimmed_run_reads_back_with_its_own_document() {
        let row: RunRow = serde_json::from_value(json!({
            "run_key": "run_1", "variant": "trimmed", "created_at": null,
            "analyzer_version": "0.5.4", "variations_run": 12, "max_plans": 500,
            "random_seed": "42", "degree_metrics": degree_metrics_fixture(),
            "analyzed_document": {
                "degree": {"name": "CS", "degree_type": "BS", "system_type": "semester",
                           "institution": "T", "total_credits": 4},
                "requirements": {"core": {"type": "all", "category": "major", "courses": ["CS101"]}},
                "courses": {"CS101": {"name": "Intro", "prefix": "CS", "number": "101",
                                      "credit_hours": 4.0}}
            }
        }))
        .expect("the select's shape");
        assert_eq!(
            (
                row.run.variant.as_str(),
                row.run.max_plans,
                row.run.random_seed.as_deref()
            ),
            ("trimmed", Some(500), Some("42"))
        );
        let program =
            program_from_document(row.analyzed_document.as_ref().unwrap(), "prog:1").unwrap();
        assert_eq!(program.courses.len(), 1);
        let err = program_from_document(&json!({"not_a_degree": true}), "prog:1").unwrap_err();
        assert!(err.contains("prog:1"), "{err}");
    }

    use super::*;
    use serde_json::json;

    /// The exact shape stored in `analysis_runs.degree_metrics`, copied from a live row.
    fn degree_metrics_fixture() -> serde_json::Value {
        json!({
            "delay":      {"q1": 5.0,   "q3": 5.0,   "max": 5.0,   "min": 5.0,   "mean": 5.0,   "median": 5.0,   "std_dev": 0.0},
            "credits":    {"q1": 120.3, "q3": 121.0, "max": 121.0, "min": 120.0, "mean": 120.7, "median": 120.7, "std_dev": 0.45},
            "complexity": {"q1": 117.0, "q3": 125.0, "max": 125.0, "min": 113.0, "mean": 120.3, "median": 121.0, "std_dev": 4.17},
            "avg_chain_length": {"q1": 1.6, "q3": 1.9, "max": 2.0, "min": 1.5, "mean": 1.7, "median": 1.7, "std_dev": 0.1}
        })
    }

    #[test]
    fn degree_metrics_map_onto_the_field_that_measures_them() {
        // The JSON keys name the metric and the struct names the quantity, so the
        // mapping is written by hand — which is exactly how complexity and delay end up
        // swapped. Each assertion pins one pair by a value unique to it.
        let stats = degree_stats_from_json(&degree_metrics_fixture(), 42).expect("parsed");
        assert_eq!(stats.plan_count, 42);
        assert!(
            (stats.total_complexity.median - 121.0).abs() < 1e-9,
            "complexity"
        );
        assert!((stats.longest_delay.median - 5.0).abs() < 1e-9, "delay");
        assert!((stats.total_credits.median - 120.7).abs() < 1e-9, "credits");
        assert!(
            (stats.avg_chain_length.median - 1.7).abs() < 1e-9,
            "chain length"
        );
    }

    #[test]
    fn the_whole_five_number_summary_survives_because_the_box_plots_need_it() {
        let stats = degree_stats_from_json(&degree_metrics_fixture(), 1).expect("parsed");
        let c = &stats.total_complexity;
        assert!((c.min - 113.0).abs() < 1e-9);
        assert!((c.q1 - 117.0).abs() < 1e-9);
        assert!((c.median - 121.0).abs() < 1e-9);
        assert!((c.q3 - 125.0).abs() < 1e-9);
        assert!((c.max - 125.0).abs() < 1e-9);
        assert!((c.mean - 120.3).abs() < 1e-9);
    }

    #[test]
    fn degree_metrics_missing_a_metric_is_refused_rather_than_zero_filled() {
        // A partial block would render a report full of zeroes that looks legitimate.
        let partial = json!({"delay": {"q1":1.0,"q3":1.0,"max":1.0,"min":1.0,"mean":1.0,"median":1.0,"std_dev":0.0}});
        assert!(degree_stats_from_json(&partial, 1).is_none());
    }

    fn course_row() -> CourseMetricRow {
        let m =
            json!({"q1":1.0,"q3":3.0,"max":4.0,"min":0.0,"mean":2.0,"median":2.0,"std_dev":0.5});
        CourseMetricRow {
            course_code: "CS101".to_string(),
            plan_count: Some(7),
            metrics: Some(json!({
                "complexity": m, "centrality": m, "delay": m, "blocking": m, "chain_length": m
            })),
        }
    }

    #[test]
    fn a_course_row_becomes_a_course_summary() {
        let stats = course_stats_from_row(&course_row()).expect("parsed");
        assert_eq!(stats.course_id, "CS101");
        assert_eq!(stats.plan_count, 7);
        assert!((stats.complexity.median - 2.0).abs() < 1e-9);
    }

    #[test]
    fn a_course_row_without_metrics_is_skipped_not_defaulted() {
        let mut row = course_row();
        row.metrics = None;
        assert!(course_stats_from_row(&row).is_none());
    }

    #[test]
    fn a_schedule_rebuilds_its_terms_in_order() {
        let schedule = json!([
            {"term": 1, "courses": ["MATH215", "ICS111"], "credits": 17.0},
            {"term": 2, "courses": ["ICS141"], "credits": 15.0}
        ]);
        let plan = term_plan_from_schedule(Some(&schedule));
        assert_eq!(plan.terms.len(), 2);
        assert_eq!(plan.terms[0].number, 1);
        assert_eq!(plan.terms[0].courses, ["MATH215", "ICS111"]);
        assert!((plan.terms[0].total_credits - 17.0).abs() < f32::EPSILON);
        assert_eq!(plan.terms[1].courses, ["ICS141"]);
    }

    #[test]
    fn an_absent_or_malformed_schedule_yields_no_terms_rather_than_panicking() {
        assert!(term_plan_from_schedule(None).terms.is_empty());
        assert!(term_plan_from_schedule(Some(&json!("nonsense")))
            .terms
            .is_empty());
    }

    fn plan_row(category: &str, terms: i64) -> PlanRow {
        PlanRow {
            category: Some(category.to_string()),
            terms_required: Some(terms),
            total_complexity: Some(250.0),
            longest_delay: Some(7.0),
            credits: Some(128.0),
            is_calc_ready: Some(false),
            critical_path: Some(json!(["MATH215", "MATH216"])),
            schedule: Some(json!([{"term": 1, "courses": ["MATH215"], "credits": 4.0}])),
        }
    }

    #[test]
    fn plan_rows_land_in_the_bucket_their_category_names() {
        let rows = vec![
            plan_row(CAT_SHORTEST, 8),
            plan_row(CAT_LONGEST, 12),
            plan_row(CAT_CALC_READY, 9),
            plan_row("Random Sample", 10),
            plan_row("Random Sample", 11),
        ];
        let selected = selected_plans_from_rows(&rows, 984);
        assert_eq!(selected.shortest.expect("shortest").score.terms_required, 8);
        assert_eq!(selected.longest.expect("longest").score.terms_required, 12);
        assert_eq!(
            selected
                .calc_ready_shortest
                .expect("calc ready")
                .score
                .terms_required,
            9
        );
        assert_eq!(selected.random_samples.len(), 2, "samples miscounted");
        assert_eq!(selected.total_plans_seen, 984);
    }

    #[test]
    fn an_unrecognised_category_is_treated_as_a_sample_rather_than_dropped() {
        // The importer may add a category later; losing those rows silently would
        // shrink the report with nothing to show why.
        let rows = vec![plan_row("Some Future Category", 7)];
        let selected = selected_plans_from_rows(&rows, 1);
        assert_eq!(selected.random_samples.len(), 1);
        assert!(selected.shortest.is_none());
    }

    #[test]
    fn a_run_with_no_plans_still_produces_a_usable_selection() {
        let selected = selected_plans_from_rows(&[], 0);
        assert!(selected.shortest.is_none());
        assert!(selected.random_samples.is_empty());
    }

    #[test]
    fn a_plan_row_carries_the_fields_the_renderer_reads() {
        // These five are the entire surface the HTML report reads off a plan.
        let plan = scored_plan_from_row(&plan_row(CAT_SHORTEST, 8));
        assert_eq!(plan.score.terms_required, 8);
        assert_eq!(plan.score.total_complexity, 250);
        assert_eq!(plan.score.longest_delay, 7);
        assert_eq!(plan.score.longest_delay_chain, ["MATH215", "MATH216"]);
        assert_eq!(plan.schedule.terms.len(), 1);
        // Courses come from the schedule, since no course list is stored separately.
        assert_eq!(plan.variant.courses, ["MATH215"]);
    }
}
