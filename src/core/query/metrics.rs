//! Analysis metrics for a stored program: `db query metrics` and `get_stored_analysis`.
//!
//! **Runs append, they do not replace.** Re-importing a program adds a row rather than
//! overwriting one, so a program accumulates runs across analyzer versions and corpus
//! generations. "The metrics for this degree" therefore means *the newest run*, which is
//! why every query here orders by `created_at` descending. Asking without an order would
//! return whichever row the backend felt like.

use std::sync::Arc;

use super::degrees::resolve_program_key;
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{parse_json_array, to_json_pretty};
use serde::{Deserialize, Serialize};

/// Columns worth returning for a run. `degree_metrics` is the full JSONB block; the
/// `*_mean` columns are promoted copies the schema keeps for ranking and filtering.
const RUN_COLS: &str = "run_key,program_key,variant,trimmed,variations_run,sample_type,\
calc_strategy,sampling_strategy,max_plans,full_run,degree_metrics,complexity_mean,\
delay_mean,credits_mean,analyzer_version,random_seed,config_fingerprint,created_at";

/// Request parameters for `db query metrics` and `get_stored_analysis`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetDegreeMetricsRequest {
    /// Program to report on. Matched against `program_key` first, then `degree_id` —
    /// `program_key` is unique, `degree_id` may span catalog years.
    #[schemars(description = "Program key (exact) or degree id slug")]
    pub degree: String,
    /// Restrict to one analysed variant, e.g. `full` or `trimmed`. Omit for all.
    #[schemars(description = "Variant label: \"full\" or \"trimmed\"")]
    pub variant: Option<String>,
    /// Return only the newest run per variant. Defaults to true; set false for history.
    #[schemars(description = "Only the newest run per variant (default true)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub latest: Option<bool>,
    /// Maximum runs to read back. Defaults to 50, capped at 200.
    #[schemars(description = "Maximum runs to read back (default 50, max 200)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub limit: Option<usize>,
    /// Attach each run's selected plans (shortest, longest, samples) with their schedules.
    #[schemars(
        description = "Attach each run's selected plans with their term schedules (default false)"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub include_plans: Option<bool>,
    /// Attach each run's per-course metrics.
    #[schemars(
        description = "Attach each run's per-course metrics, most complex first (default false)"
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub include_course_metrics: Option<bool>,
}

/// One stored analysis run.
#[derive(Debug, Serialize, Deserialize)]
struct RunRow {
    run_key: String,
    program_key: String,
    variant: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    trimmed: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    variations_run: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    calc_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sampling_strategy: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_plans: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_run: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    degree_metrics: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    complexity_mean: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    delay_mean: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    credits_mean: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    analyzer_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    random_seed: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    config_fingerprint: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_at: Option<String>,
    /// The run's selected plans, when asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plans: Option<serde_json::Value>,
    /// The run's per-course metrics, when asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    course_metrics: Option<serde_json::Value>,
}

/// Response for `db query metrics`.
#[derive(Debug, Serialize)]
struct MetricsResponse {
    program_key: String,
    count: usize,
    /// True when only the newest run per variant was requested.
    ///
    /// Echoes the request — it is not a claim that any run was actually dropped, and a
    /// program with one run per variant still reports `true`. Always emitted, unlike the
    /// absent-field handling on `RunRow`, because it is the only thing telling a reader
    /// whether `count` is the whole history or a filtered view.
    latest_only: bool,
    runs: Vec<RunRow>,
}

/// Plan columns worth returning: the curated plan and its term-by-term schedule.
const PLAN_COLS: &str = "plan_index,category,terms_required,total_complexity,longest_delay,\
credits,course_count,is_calc_ready,critical_path,schedule";

/// Per-course columns worth returning; `metrics` (the full five-number summaries) is left
/// out to keep a run's answer readable.
const COURSE_COLS: &str = "course_code,plan_count,complexity_mean,centrality_mean,delay_mean,\
blocking_mean,chain_length_mean";

/// The rows a run owns in one of its child tables, capped as the report reads them.
struct Children {
    table: &'static str,
    cols: &'static str,
    order_desc: &'static str,
    max_rows: usize,
}

/// A run's selected plans.
const PLANS: Children = Children {
    table: tables::ANALYSIS_PLANS,
    cols: PLAN_COLS,
    order_desc: "plan_index",
    max_rows: super::report_source::MAX_PLAN_ROWS,
};

/// A run's per-course metrics, most complex first.
const COURSE_METRICS: Children = Children {
    table: tables::ANALYSIS_COURSE_METRICS,
    cols: COURSE_COLS,
    order_desc: "complexity_mean",
    max_rows: super::report_source::MAX_COURSE_ROWS,
};

/// Rows a run owns in `children.table`.
async fn run_children(
    client: &DbClient,
    children: &Children,
    run_key: &str,
) -> Result<serde_json::Value, crate::core::database::DatabaseError> {
    let filters = QueryFilters::new()
        .eq("run_key", Some(run_key))
        .order_desc(children.order_desc);
    client
        .select(
            children.table,
            children.cols,
            &filters,
            Some(children.max_rows),
        )
        .await
}

/// Attach the plans and course metrics `req` asks for to `run`.
///
/// # Errors
/// A failure payload naming the run and the table that could not be read.
async fn attach_children(
    client: &DbClient,
    run: &mut RunRow,
    req: &GetDegreeMetricsRequest,
) -> Result<(), String> {
    if req.include_plans.unwrap_or(false) {
        let mut plans = run_children(client, &PLANS, &run.run_key)
            .await
            .map_err(|e| e.to_json(&format!("reading the plans of run {}", run.run_key)))?;
        sort_by_key_field(&mut plans, "plan_index");
        run.plans = Some(plans);
    }
    if req.include_course_metrics.unwrap_or(false) {
        let metrics = run_children(client, &COURSE_METRICS, &run.run_key)
            .await
            .map_err(|e| {
                e.to_json(&format!(
                    "reading the course metrics of run {}",
                    run.run_key
                ))
            })?;
        run.course_metrics = Some(metrics);
    }
    Ok(())
}

/// Sort an array of JSON objects by an integer field, ascending.
fn sort_by_key_field(rows: &mut serde_json::Value, field: &str) {
    if let Some(rows) = rows.as_array_mut() {
        rows.sort_by_key(|r| r.get(field).and_then(serde_json::Value::as_i64));
    }
}

/// The newest run of each variant for `program_key`, in brief.
///
/// What `get_degree` shows beside a program: which variants have been analysed, when, by
/// which analyzer, and the headline means.
///
/// # Errors
/// When the backend fails.
pub async fn run_summaries(
    client: &DbClient,
    program_key: &str,
) -> Result<Vec<serde_json::Value>, crate::core::database::DatabaseError> {
    let filters = QueryFilters::new()
        .eq("program_key", Some(program_key))
        .order_desc("created_at");
    let value = client
        .select(tables::ANALYSIS_RUNS, SUMMARY_COLS, &filters, Some(200))
        .await?;
    let mut seen = std::collections::HashSet::new();
    Ok(value
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| {
            let variant = r
                .get("variant")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("");
            seen.insert(variant.to_string())
        })
        .cloned()
        .collect())
}

/// The newest `variant` run for `program_key`, in brief, or `None` when it has none.
///
/// # Errors
/// When the backend fails.
pub async fn latest_run_summary(
    client: &DbClient,
    program_key: &str,
    variant: &str,
) -> Result<Option<serde_json::Value>, crate::core::database::DatabaseError> {
    let filters = QueryFilters::new()
        .eq("program_key", Some(program_key))
        .eq("variant", Some(variant))
        .order_desc("created_at");
    let value = client
        .select(tables::ANALYSIS_RUNS, SUMMARY_COLS, &filters, Some(1))
        .await?;
    Ok(value.as_array().and_then(|rows| rows.first()).cloned())
}

/// A run in brief: identity, provenance, headline means.
const SUMMARY_COLS: &str = "run_key,variant,created_at,analyzer_version,variations_run,\
complexity_mean,delay_mean,credits_mean";

/// Drop all but the newest run of each variant.
///
/// Relies on `runs` arriving newest-first — it keeps the first sighting of each variant.
/// That contract is the `order_desc("created_at")` on the query; oldest-first input would
/// silently report a superseded run as current.
fn keep_newest_per_variant(runs: &mut Vec<RunRow>) {
    let mut seen = std::collections::HashSet::new();
    runs.retain(|r| seen.insert(r.variant.clone()));
}

/// Execute the degree-metrics query and return a JSON payload.
pub async fn execute_json(client: &Arc<DbClient>, req: GetDegreeMetricsRequest) -> String {
    let program_key = match resolve_program_key(client, req.degree.trim()).await {
        Ok(k) => k,
        Err(payload) => return payload,
    };

    let filters = QueryFilters::new()
        .eq("program_key", Some(&program_key))
        .eq("variant", req.variant.as_deref())
        .order_desc("created_at");

    // Bounded like every sibling engine. Runs append and `degree_metrics` is a full
    // JSONB blob per row, so an unbounded read grows with a program's whole history.
    let limit = req.limit.unwrap_or(50).min(200);
    let value = match client
        .select(tables::ANALYSIS_RUNS, RUN_COLS, &filters, Some(limit))
        .await
    {
        Ok(v) => v,
        Err(e) => return e.to_json(&format!("reading the analysis runs of {program_key}")),
    };

    let mut runs: Vec<RunRow> = parse_json_array(&value);

    let latest_only = req.latest.unwrap_or(true);
    if latest_only {
        keep_newest_per_variant(&mut runs);
    }

    for run in &mut runs {
        if let Err(payload) = attach_children(client, run, &req).await {
            return payload;
        }
    }

    to_json_pretty(&MetricsResponse {
        program_key,
        count: runs.len(),
        latest_only,
        runs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sort_by_key_field_orders_by_the_field_and_leaves_non_arrays_alone() {
        let mut rows = serde_json::json!([
            {"plan_index": 2}, {"plan_index": 0}, {"other": 1}, {"plan_index": 1}
        ]);
        sort_by_key_field(&mut rows, "plan_index");
        let order: Vec<Option<i64>> = rows
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r["plan_index"].as_i64())
            .collect();
        assert_eq!(
            order,
            [None, Some(0), Some(1), Some(2)],
            "a missing field sorts first"
        );

        let mut object = serde_json::json!({"plan_index": 3});
        sort_by_key_field(&mut object, "plan_index");
        assert_eq!(object, serde_json::json!({"plan_index": 3}));
    }

    fn run(variant: &str, created: &str, complexity: f64) -> RunRow {
        RunRow {
            run_key: format!("{variant}-{created}"),
            program_key: "prog:1".to_string(),
            variant: variant.to_string(),
            trimmed: Some(variant == "trimmed"),
            variations_run: None,
            sample_type: None,
            calc_strategy: None,
            sampling_strategy: None,
            max_plans: None,
            full_run: None,
            degree_metrics: None,
            complexity_mean: Some(complexity),
            delay_mean: None,
            credits_mean: None,
            analyzer_version: None,
            random_seed: None,
            config_fingerprint: None,
            created_at: Some(created.to_string()),
            plans: None,
            course_metrics: None,
        }
    }

    /// Thin wrapper over the production reduction — deliberately not a reimplementation,
    /// or these tests would pass while `execute_json` did something else.
    fn keep_latest(mut runs: Vec<RunRow>) -> Vec<RunRow> {
        keep_newest_per_variant(&mut runs);
        runs
    }

    #[test]
    fn latest_keeps_one_run_per_variant() {
        // The corpus was imported twice, so every program has two runs per variant. A
        // caller asking for "the metrics" must get the newer one, not both.
        let runs = vec![
            run("full", "2026-09-24", 89.0),
            run("trimmed", "2026-09-24", 61.0),
            run("full", "2026-09-23", 110.0),
            run("trimmed", "2026-09-23", 70.0),
        ];
        let kept = keep_latest(runs);
        assert_eq!(kept.len(), 2, "one per variant");
        assert_eq!(kept[0].complexity_mean, Some(89.0), "newest full run");
        assert_eq!(kept[1].complexity_mean, Some(61.0), "newest trimmed run");
    }

    #[test]
    fn latest_relies_on_input_order_being_newest_first() {
        // This is the contract with `order_desc("created_at")`: the reduction takes the
        // first sighting, so a backend returning oldest-first would silently report a
        // superseded run as current. Pinned so the ordering cannot be dropped quietly.
        let runs = vec![
            run("full", "2026-09-23", 110.0),
            run("full", "2026-09-24", 89.0),
        ];
        let kept = keep_latest(runs);
        assert_eq!(
            kept[0].complexity_mean,
            Some(110.0),
            "reduction takes the first row, so ordering is the query's job"
        );
    }

    #[test]
    fn a_single_run_survives_unchanged() {
        let kept = keep_latest(vec![run("full", "2026-09-24", 89.0)]);
        assert_eq!(kept.len(), 1);
    }

    #[test]
    fn no_runs_reduces_to_nothing_rather_than_panicking() {
        assert!(keep_latest(Vec::new()).is_empty());
    }

    #[test]
    fn run_rows_omit_absent_fields_rather_than_emitting_null() {
        // A report full of `"delay_mean": null` is noise for the LLM that reads it.
        let json = serde_json::to_string(&run("full", "2026-09-24", 89.0)).expect("serialize");
        assert!(!json.contains("delay_mean"), "absent field emitted: {json}");
        assert!(
            json.contains("complexity_mean"),
            "present field lost: {json}"
        );
    }
}
