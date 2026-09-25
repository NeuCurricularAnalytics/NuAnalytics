//! Analysis metrics for a stored program.
//!
//! A new engine rather than a move from `src/mcp/tools/`. `analysis_runs` was previously
//! only written (`db import`) and scanned for deletion (`db prune`, `fetch_run_refs`);
//! this is the first code that reads it to report it. The MCP server does not expose it —
//! MCP wiring is deliberately deferred, so this engine is CLI-only for now.
//!
//! **Runs append, they do not replace.** Re-importing a program adds a row rather than
//! overwriting one, so a program accumulates runs across analyzer versions and corpus
//! generations. "The metrics for this degree" therefore means *the newest run*, which is
//! why every query here orders by `created_at` descending. Asking without an order would
//! return whichever row the backend felt like.

use std::sync::Arc;

use crate::core::database::{tables, DbClient, QueryFilters};
use crate::core::json::{error_json, parse_first, parse_json_array, to_json_pretty};
use serde::{Deserialize, Serialize};

/// Columns worth returning for a run. `degree_metrics` is the full JSONB block; the
/// `*_mean` columns are promoted copies the schema keeps for ranking and filtering.
const RUN_COLS: &str = "run_key,program_key,variant,trimmed,variations_run,sample_type,\
calc_strategy,sampling_strategy,max_plans,full_run,degree_metrics,complexity_mean,\
delay_mean,credits_mean,analyzer_version,random_seed,config_fingerprint,created_at";

/// Request parameters for `db query metrics`.
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

/// One `program_key` column, for the identity probes below.
#[derive(Debug, Deserialize)]
struct KeyRow {
    program_key: String,
}

/// How many `degree_id` matches to read before reporting the list as truncated.
///
/// One more than we would ever want to print, so `keys.len() > AMBIGUITY_PROBE` is a
/// reliable "there are more than this" rather than a guess.
const AMBIGUITY_PROBE: usize = 50;

/// Resolve `degree` to a `program_key`.
///
/// Tries `program_key` first because it is unique. A `degree_id` can match several
/// programs (one per catalog year), and silently reporting one of them as "the" answer
/// would be worse than saying so — hence the ambiguity error.
///
/// `Err` carries a finished JSON payload, not a message: callers return it verbatim.
async fn resolve_program_key(client: &Arc<DbClient>, degree: &str) -> Result<String, String> {
    let by_key = QueryFilters::new().eq("program_key", Some(degree));
    match client
        .select(tables::PROGRAMS, "program_key", &by_key, Some(1))
        .await
    {
        Ok(v) if parse_first::<KeyRow>(&v).is_some() => return Ok(degree.to_string()),
        Ok(_) => {}
        Err(e) => return Err(error_json(e)),
    }

    let by_id = QueryFilters::new().eq("degree_id", Some(degree));
    let rows = match client
        .select(
            tables::PROGRAMS,
            "program_key",
            &by_id,
            Some(AMBIGUITY_PROBE + 1),
        )
        .await
    {
        Ok(v) => v,
        Err(e) => return Err(error_json(e)),
    };
    let mut keys: Vec<String> = parse_json_array::<KeyRow>(&rows)
        .into_iter()
        .map(|r| r.program_key)
        .collect();

    match keys.len() {
        0 => Err(serde_json::json!({
            "error": format!(
                "no row in `programs` has program_key or degree_id = \"{degree}\""
            ),
            "degree": degree,
            "tip": "List candidates with `nuanalytics db query degrees --school <UNITID>`",
        })
        .to_string()),
        1 => Ok(keys.remove(0)),
        n => {
            // Say so rather than present a capped list as if it were complete.
            let truncated = n > AMBIGUITY_PROBE;
            keys.truncate(AMBIGUITY_PROBE);
            Err(serde_json::json!({
                "error": format!(
                    "degree_id \"{degree}\" matches {}{} rows in `programs` — pass one program_key",
                    if truncated { "more than " } else { "" },
                    keys.len()
                ),
                "degree": degree,
                "matches": keys,
                "truncated": truncated,
            })
            .to_string())
        }
    }
}

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
        Err(e) => return error_json(e),
    };

    let mut runs: Vec<RunRow> = parse_json_array(&value);

    let latest_only = req.latest.unwrap_or(true);
    if latest_only {
        keep_newest_per_variant(&mut runs);
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
