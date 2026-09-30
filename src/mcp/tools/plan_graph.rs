//! `render_plan_graph`: one selected plan of a degree as a curriculum graph.
//!
//! Analyses the degree, picks a plan by `plan_category` (`"shortest"`, `"longest"`,
//! `"calc-ready-shortest"`, `"sample"` + optional `sample_index`) or by raw `plan_index`
//! into `selected_plans`, and renders it — returning the HTML, or writing it to
//! `output_path`.

use crate::core::degree::plan_selector::PlanCategory;
use crate::core::degree::{ScoredPlan, SelectedPlans};
use crate::core::report::visualization::{
    spec_from_scored_plan, CurriculumGraphRenderer, VanillaJsRenderer,
};
use crate::mcp::cache::cached_artifacts;
use crate::mcp::tools::shared::DegreeSourceArgs;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Output shape for `render_plan_graph`.
#[derive(Debug, Default, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VisualizationFormat {
    /// Full `<!DOCTYPE html>…</html>` page that opens directly in a browser.
    #[default]
    Standalone,
    /// Self-contained fragment (`<style>` + `<div>` + `<script>`) suitable for
    /// embedding inside another HTML document. Includes the shared library
    /// inline so the fragment is self-sufficient.
    Fragment,
    /// Same as `Fragment`, but omits the shared `GRAPH_VANILLA_JS` library.
    /// Use when embedding multiple graphs on one page: emit one `Fragment`
    /// (or include the library once via another mechanism), then use this
    /// variant for subsequent graphs to drop ~20 KB per fragment.
    FragmentNoLibrary,
}

/// Request parameters for `render_plan_graph`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenderPlanGraphRequest {
    /// Where the degree comes from: exactly one of `degree`, `content`, `path`.
    #[serde(flatten)]
    pub source: DegreeSourceArgs,

    /// Named plan category. Accepts `"shortest"`, `"longest"`,
    /// `"calc-ready-shortest"`, or `"sample"` (paired with `sample_index`).
    /// Mutually exclusive with `plan_index`.
    #[schemars(
        description = "Named plan: \"shortest\" | \"longest\" | \"calc-ready-shortest\" | \"sample\" (with sample_index). Mutually exclusive with plan_index."
    )]
    pub plan_category: Option<String>,

    /// Index of the random sample to render when `plan_category="sample"`.
    /// 1-indexed (`1` = Sample 1). Default 1.
    #[schemars(description = "1-indexed sample number when plan_category=\"sample\". Default 1.")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub sample_index: Option<usize>,

    /// Raw 0-indexed offset into the analyze response's `selected_plans`
    /// list. Mutually exclusive with `plan_category`.
    #[schemars(
        description = "0-indexed offset into selected_plans (advanced). Mutually exclusive with plan_category."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub plan_index: Option<usize>,

    /// Rendering format. Defaults to `"standalone"` (full HTML page).
    #[schemars(
        description = "Render format: \"standalone\" (default, full HTML page), \"fragment\", or \"fragment-no-library\"."
    )]
    #[serde(default)]
    pub format: VisualizationFormat,

    /// Forwarded to the analysis: cap on plans generated.
    #[schemars(description = "Maximum plans to generate during analysis (default 500)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub max_plans: Option<usize>,

    /// Forwarded to the analysis: courses every generated plan must include.
    #[schemars(
        description = "Comma-separated course codes every generated plan must include (e.g. \"CS150B,MATH156\")"
    )]
    pub include_courses: Option<String>,

    /// Skip the HTML render and return only the picker metadata + a heuristic
    /// `html_bytes` estimate. Cheap probe (<1 s) for confirming a plan exists
    /// and budgeting the response size before paying the full render cost.
    #[schemars(
        description = "Skip rendering and return picker metadata + node_count + a heuristic html_bytes estimate (default false). Use this to budget before paying the full render cost."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub dry_run: Option<bool>,

    /// Write the HTML here instead of returning it inline.
    #[schemars(description = "Write the HTML to this file instead of returning it inline")]
    pub output_path: Option<String>,

    /// Replace `output_path` if it already exists.
    #[schemars(description = "Replace output_path if it already exists (default false)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub overwrite: Option<bool>,
}

/// Response for `render_plan_graph`.
#[derive(Debug, Serialize)]
pub struct RenderPlanGraphResponse {
    /// True when the YAML parsed, the plan was found, and rendering succeeded.
    pub success: bool,
    /// Error message when `success` is false.
    pub error: Option<String>,
    /// Why, when the cause is known: see [`crate::core::json::error_code`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// Echoed: the resolved plan category for the rendered plan.
    pub plan_category: Option<String>,
    /// Resolved 0-indexed offset into `selected_plans`.
    pub plan_index: Option<usize>,
    /// Term count for the rendered plan.
    pub terms: Option<usize>,
    /// Total complexity score.
    pub complexity: Option<usize>,
    /// Longest delay factor.
    pub longest_delay: Option<usize>,
    /// Rendered HTML body. Omitted when `dry_run=true`.
    pub html: Option<String>,
    /// Size of the rendered HTML in bytes. When `dry_run=true` this is a
    /// heuristic estimate (`node_count * 200 + edge_count * 80 + fixed`) so
    /// the dry-run probe stays sub-second; expect ±15 % vs the actual render.
    pub html_bytes: usize,
    /// The file written, when `output_path` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Number of course nodes in the generated `CurriculumGraphSpec`.
    /// Useful as a complexity proxy during dry-run probing.
    pub node_count: Option<usize>,
    /// Whether this response was a dry-run probe (no HTML payload).
    pub dry_run: bool,
    /// Number of `RandomSample` entries actually present in `selected_plans`.
    /// Populated only on the sample-index-out-of-range error path so callers
    /// can retry with a valid index without re-running analyze.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub available_samples: Option<usize>,
    /// The 1-indexed sample number the caller requested when the
    /// out-of-range error fired. Echoed for symmetry with
    /// `available_samples`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub requested_sample_index: Option<usize>,
}

// ============================================================================
// Execution
// ============================================================================

/// Which selected plan to draw, and how.
#[derive(Debug, Default, Clone, Copy)]
pub struct PlanGraphOptions<'a> {
    /// `shortest`, `longest`, `calc-ready-shortest` or `sample`; or give `plan_index`.
    pub plan_category: Option<&'a str>,
    /// Which random sample, 1-based, for `plan_category = "sample"`.
    pub sample_index: Option<usize>,
    /// A position in `selected_plans`, instead of a category.
    pub plan_index: Option<usize>,
    /// The HTML's shape.
    pub format: VisualizationFormat,
    /// Forwarded to the analysis: cap on plans generated.
    pub max_plans: Option<usize>,
    /// Forwarded to the analysis: courses every generated plan must include.
    pub include_courses: Option<&'a [String]>,
    /// Estimate the HTML's size instead of rendering it.
    pub dry_run: bool,
}

/// Execute the `render_plan_graph` tool.
#[must_use]
pub fn execute(yaml_content: &str, opts: &PlanGraphOptions<'_>) -> RenderPlanGraphResponse {
    let PlanGraphOptions {
        plan_category,
        sample_index,
        plan_index,
        format,
        max_plans,
        include_courses,
        dry_run,
    } = *opts;
    if plan_category.is_none() && plan_index.is_none() {
        return error_response(
            "Provide either plan_category (\"shortest\" / \"longest\" / \"calc-ready-shortest\" / \"sample\") or plan_index.",
        );
    }
    if plan_category.is_some() && plan_index.is_some() {
        return error_response("Provide plan_category OR plan_index, not both.");
    }

    let artifacts =
        match cached_artifacts(yaml_content, max_plans, include_courses, None, None, None) {
            Ok(a) => a,
            Err(e) => return error_response(e),
        };

    let (idx, category, plan) =
        match pick_plan(&artifacts.selected, plan_category, sample_index, plan_index) {
            Ok(picked) => picked,
            Err(response) => return *response,
        };

    let graph_id = category.file_name().to_string();
    let spec = spec_from_scored_plan(
        &artifacts.school,
        &artifacts.equivalences,
        plan,
        Some(&artifacts.report_stats),
        &graph_id,
    );
    let node_count = spec.nodes.len();
    let edge_count = spec.edges.len();

    let (html_field, html_bytes) = if dry_run {
        (None, estimate_html_bytes(node_count, edge_count, format))
    } else {
        let html = match format {
            VisualizationFormat::Standalone => VanillaJsRenderer.render_standalone(&spec),
            VisualizationFormat::Fragment => VanillaJsRenderer.render(&spec),
            VisualizationFormat::FragmentNoLibrary => {
                VanillaJsRenderer.render_without_library(&spec)
            }
        };
        let bytes = html.len();
        (Some(html), bytes)
    };

    RenderPlanGraphResponse {
        success: true,
        error: None,
        code: None,
        plan_category: Some(category.display_name().to_string()),
        plan_index: Some(idx),
        terms: Some(plan.score.terms_required),
        complexity: Some(plan.score.total_complexity),
        longest_delay: Some(plan.score.longest_delay),
        html: html_field,
        html_bytes,
        path: None,
        node_count: Some(node_count),
        dry_run,
        available_samples: None,
        requested_sample_index: None,
    }
}

/// The selected plan the arguments name, or the response saying why none matches.
///
/// Asking for sample N when only M exist gets its own response, so a caller can retry
/// with a valid index without another analysis.
fn pick_plan<'a>(
    selected: &'a SelectedPlans,
    plan_category: Option<&str>,
    sample_index: Option<usize>,
    plan_index: Option<usize>,
) -> Result<(usize, PlanCategory, &'a ScoredPlan), Box<RenderPlanGraphResponse>> {
    // PlanCategory is Copy; capture each tuple as (index, category, &ScoredPlan)
    // so the picker functions can return owned `PlanCategory` values cheaply.
    let entries: Vec<(usize, PlanCategory, &ScoredPlan)> = selected
        .iter()
        .enumerate()
        .map(|(idx, (cat, plan))| (idx, cat, plan))
        .collect();

    let picked = plan_index.map_or_else(
        || pick_by_category(&entries, plan_category.unwrap_or(""), sample_index),
        |idx| entries.iter().find(|(i, _, _)| *i == idx).copied(),
    );
    if let Some(found) = picked {
        return Ok(found);
    }
    let wants_sample = plan_category
        .and_then(PlanCategory::from_user_input)
        .is_some_and(|c| c == PlanCategory::RandomSample);
    if wants_sample {
        let available = entries
            .iter()
            .filter(|(_, cat, _)| *cat == PlanCategory::RandomSample)
            .count();
        let want = sample_index.unwrap_or(1);
        if want > available {
            return Err(Box::new(sample_index_out_of_range_response(
                want, available,
            )));
        }
    }
    Err(Box::new(error_response(format!(
        "No selected plan matches plan_category={plan_category:?} / plan_index={plan_index:?} / sample_index={sample_index:?}. Selected_plans has {} entries.",
        entries.len()
    ))))
}

/// Approximate `html_bytes` for a dry-run probe without paying the renderer
/// cost. Tuned against observed renders: ~200 B per course node, ~80 B per
/// edge, plus a fixed overhead that covers the CSS + embedded JS for
/// standalone / fragment formats. The variance against the actual render is
/// within ~15 % for the common cases — good enough for "should I pay for the
/// full render?" budgeting without blowing the 4-minute MCP ceiling.
const fn estimate_html_bytes(
    node_count: usize,
    edge_count: usize,
    format: VisualizationFormat,
) -> usize {
    let fixed = match format {
        VisualizationFormat::Standalone | VisualizationFormat::Fragment => 24_000, // CSS + JS library
        VisualizationFormat::FragmentNoLibrary => 6_000,                           // CSS only
    };
    node_count * 200 + edge_count * 80 + fixed
}

/// Where a rendered graph goes: inline, or a file.
#[derive(Debug, Default, Clone, Copy)]
pub struct GraphOutput<'a> {
    /// File to write instead of returning the HTML inline.
    pub path: Option<&'a str>,
    /// Replace `path` if it exists.
    pub overwrite: bool,
}

/// Write a rendered response's HTML to `output.path`, if one was given.
fn deliver(
    mut response: RenderPlanGraphResponse,
    output: GraphOutput<'_>,
) -> RenderPlanGraphResponse {
    // Checked before taking the HTML: with no path it stays inline.
    if output.path.is_none() {
        return response;
    }
    let Some(html) = response.html.take() else {
        return response;
    };
    match crate::mcp::tools::shared::deliver_output(html, output.path, output.overwrite) {
        Ok((html, path)) => {
            response.html = html;
            response.path = path;
            response
        }
        Err(refusal) => RenderPlanGraphResponse {
            code: Some(refusal.code),
            ..error_response(refusal.message)
        },
    }
}

/// [`execute`], delivered inline or to `output.path`, as JSON.
#[must_use]
pub fn execute_json(
    yaml_content: &str,
    opts: &PlanGraphOptions<'_>,
    output: GraphOutput<'_>,
) -> String {
    crate::core::json::to_json_pretty(&deliver(execute(yaml_content, opts), output))
}

// ============================================================================
// Helpers
// ============================================================================

/// Pick the matching entry from `selected_plans` for a named category.
/// `category` is case-insensitive and accepts the kebab-case (`"calc-ready-shortest"`)
/// or `PlanCategory::file_name` form. `sample_index` is 1-indexed when
/// `category="sample"`; default 1.
fn pick_by_category<'p, P>(
    entries: &[(usize, PlanCategory, &'p P)],
    category: &str,
    sample_index: Option<usize>,
) -> Option<(usize, PlanCategory, &'p P)> {
    let target = PlanCategory::from_user_input(category)?;

    if target == PlanCategory::RandomSample {
        let want = sample_index.unwrap_or(1);
        return entries
            .iter()
            .filter(|(_, cat, _)| *cat == PlanCategory::RandomSample)
            .nth(want.saturating_sub(1))
            .copied();
    }

    entries.iter().find(|(_, cat, _)| *cat == target).copied()
}

fn error_response(error: impl Into<String>) -> RenderPlanGraphResponse {
    RenderPlanGraphResponse {
        success: false,
        error: Some(error.into()),
        code: None,
        plan_category: None,
        plan_index: None,
        terms: None,
        complexity: None,
        longest_delay: None,
        html: None,
        html_bytes: 0,
        path: None,
        node_count: None,
        dry_run: false,
        available_samples: None,
        requested_sample_index: None,
    }
}

/// Specific failure response for the "`sample_index` past the end" case.
/// Surfaces `available_samples` + `requested_sample_index` so the caller
/// can retry against a valid index without re-running analyze.
fn sample_index_out_of_range_response(
    requested: usize,
    available: usize,
) -> RenderPlanGraphResponse {
    let mut response = error_response(format!(
        "sample_index={requested} exceeds available Random Sample plans ({available}). Retry with sample_index in 1..={available}."
    ));
    response.available_samples = Some(available);
    response.requested_sample_index = Some(requested);
    response
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_YAML: &str = r#"
degree:
  id: test-degree
  institution: Test University
  program: Test Program
  total_credits: 16
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS201]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
  CS201:
    title: Data Structures
    prefix: CS
    number: "201"
    credits: 4
    prerequisites_raw: "CS101"
"#;

    #[test]
    fn test_output_path_writes_the_graph_and_replaces_it_only_with_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("graphs/plan.html");
        let target = file.to_str().expect("utf8");
        let run = |overwrite: bool, dry_run: bool| -> serde_json::Value {
            let opts = PlanGraphOptions {
                plan_category: Some("shortest"),
                max_plans: Some(10),
                dry_run,
                ..PlanGraphOptions::default()
            };
            let output = GraphOutput {
                path: Some(target),
                overwrite,
            };
            serde_json::from_str(&execute_json(TEST_YAML, &opts, output)).expect("json")
        };

        let dry = run(false, true);
        assert_eq!(dry["success"], true);
        assert!(!file.exists(), "dry_run must not write");

        let written = run(false, false);
        assert_eq!(written["path"], target, "{written}");
        assert!(written["html"].is_null(), "written, not inline");

        std::fs::write(&file, "keep me").unwrap();
        let refused = run(false, false);
        assert_eq!(refused["success"], false);
        assert_eq!(refused["code"], "bad_arguments");
        assert!(refused["error"]
            .as_str()
            .unwrap()
            .contains("overwrite=true"));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "keep me");

        assert_eq!(run(true, false)["success"], true);
        assert_ne!(std::fs::read_to_string(&file).unwrap(), "keep me");
    }

    #[test]
    fn test_without_output_path_the_graph_stays_inline() {
        let opts = PlanGraphOptions {
            plan_category: Some("shortest"),
            max_plans: Some(10),
            ..PlanGraphOptions::default()
        };
        let out: serde_json::Value =
            serde_json::from_str(&execute_json(TEST_YAML, &opts, GraphOutput::default()))
                .expect("json");
        assert!(
            out["html"].as_str().is_some_and(|h| h.contains("<html")),
            "inline HTML"
        );
        assert!(out["path"].is_null());
    }

    #[test]
    fn test_render_shortest_returns_standalone_html() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("shortest"),
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(response.success, "error: {:?}", response.error);
        let html = response.html.expect("html must be populated on success");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("nuGraphs.register"));
        assert_eq!(
            response.plan_category.as_deref(),
            Some("Shortest Path"),
            "named-category response must echo the display name"
        );
        assert!(response.terms.is_some_and(|t| t > 0));
        assert!(response.html_bytes > 0);
        // dry_run=false must still populate node_count so callers always see
        // the graph-complexity proxy regardless of the rendering decision.
        assert!(!response.dry_run);
        assert!(response.node_count.is_some_and(|n| n > 0));
    }

    #[test]
    fn test_render_plan_index_zero_targets_first_selected_plan() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_index: Some(0),
                format: VisualizationFormat::Fragment,
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(response.success);
        assert_eq!(response.plan_index, Some(0));
        // Fragment mode must NOT include the DOCTYPE wrapper.
        let html = response.html.unwrap();
        assert!(!html.starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn test_requires_either_category_or_index() {
        let response = execute(TEST_YAML, &PlanGraphOptions::default());
        assert!(!response.success);
        let err = response.error.unwrap();
        assert!(err.contains("plan_category") && err.contains("plan_index"));
    }

    #[test]
    fn test_rejects_both_category_and_index() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("shortest"),
                plan_index: Some(0),
                ..PlanGraphOptions::default()
            },
        );
        assert!(!response.success);
        assert!(response.error.unwrap().contains("not both"));
    }

    #[test]
    fn test_unknown_category_surfaces_error() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("nonsense"),
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(!response.success);
        assert!(response.error.unwrap().contains("No selected plan matches"));
    }

    #[test]
    fn test_out_of_range_plan_index_surfaces_error() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_index: Some(999),
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(!response.success);
        assert!(response.error.unwrap().contains("Selected_plans has"));
    }

    #[test]
    fn test_pick_by_category_accepts_canonical_and_alias_inputs() {
        // Use stub i32 plans so we can build a synthetic entries slice without
        // dragging in the full analyze pipeline. PlanCategory is Copy and the
        // picker is generic over the plan type. The chosen P=i32 avoids the
        // clippy::ignored_unit_patterns warning that fires when the picker's
        // `_` placeholders match against unit-typed reference holes.
        let stub: i32 = 0;
        let entries: Vec<(usize, PlanCategory, &i32)> = vec![
            (0, PlanCategory::Shortest, &stub),
            (1, PlanCategory::Longest, &stub),
            (2, PlanCategory::CalcReadyShortest, &stub),
            (3, PlanCategory::RandomSample, &stub),
            (4, PlanCategory::RandomSample, &stub),
        ];

        // Each variant accepts the canonical form + at least one alias.
        for input in ["shortest", "Shortest-Path", "SHORTEST"] {
            let pick = pick_by_category(&entries, input, None);
            assert_eq!(
                pick.map(|(i, _, _)| i),
                Some(0),
                "input {input:?} should resolve to Shortest"
            );
        }
        for input in ["longest", "longest-path"] {
            assert_eq!(
                pick_by_category(&entries, input, None).map(|(i, _, _)| i),
                Some(1)
            );
        }
        for input in [
            "calc-ready-shortest",
            "calculus-ready-shortest",
            "calc_ready_shortest",
        ] {
            assert_eq!(
                pick_by_category(&entries, input, None).map(|(i, _, _)| i),
                Some(2),
                "input {input:?} should resolve to CalcReadyShortest"
            );
        }

        // sample without index → first sample; sample with index → nth.
        assert_eq!(
            pick_by_category(&entries, "sample", None).map(|(i, _, _)| i),
            Some(3)
        );
        assert_eq!(
            pick_by_category(&entries, "random-sample", Some(2)).map(|(i, _, _)| i),
            Some(4)
        );

        // Unknown strings return None.
        assert!(pick_by_category(&entries, "nonsense", None).is_none());
    }

    #[test]
    fn test_dry_run_skips_html_payload_but_reports_size_and_nodes() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("shortest"),
                max_plans: Some(10),
                dry_run: true,
                ..PlanGraphOptions::default()
            },
        );
        assert!(response.success, "error: {:?}", response.error);
        assert!(response.dry_run);
        assert!(
            response.html.is_none(),
            "dry_run must drop the HTML payload"
        );
        assert!(
            response.html_bytes > 0,
            "html_bytes must be the projected (not actual-returned) size"
        );
        assert!(response.node_count.is_some_and(|n| n > 0));
        // Picker metadata still populated so the caller can verify the
        // resolution before paying the render cost.
        assert_eq!(response.plan_category.as_deref(), Some("Shortest Path"));
    }

    #[test]
    fn test_sample_index_out_of_range_surfaces_available_and_requested() {
        // TEST_YAML has a tiny population (2 courses), so reservoir likely
        // holds far fewer than 99 random samples. Asking for sample_index=99
        // must surface the structured error rather than the generic
        // "No selected plan matches" message.
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("sample"),
                sample_index: Some(99),
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(!response.success);
        assert_eq!(response.requested_sample_index, Some(99));
        let available = response
            .available_samples
            .expect("available_samples must be populated on out-of-range error");
        assert!(available < 99);
        let err = response.error.expect("error message must be populated");
        assert!(
            err.contains("sample_index=99"),
            "error must echo the requested index: {err}"
        );
        assert!(
            err.contains(&format!("({available})")),
            "error must surface available_samples count: {err}"
        );
    }

    #[test]
    fn test_fragment_no_library_drops_shared_prelude() {
        let response = execute(
            TEST_YAML,
            &PlanGraphOptions {
                plan_category: Some("shortest"),
                format: VisualizationFormat::FragmentNoLibrary,
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(response.success);
        let html = response.html.unwrap();
        assert!(html.contains("nuGraphs.register"));
        // The shared GRAPH_VANILLA_JS prelude marker must be absent.
        assert!(!html.contains("window.nuGraphs ="));
    }

    #[test]
    fn test_render_does_not_hang_on_pathological_prerequisites() {
        // Regression for the field-report "render_plan_graph → Tool execution
        // failed" hang: an in-plan course whose prerequisites_raw is a wide
        // AND-of-ORs makes build_edges_from_courses → parse_to_dnf explode
        // (2^30 paths) and the tool times out. The expression resolves to just
        // CS101 (so the plan is valid and CS201 is schedulable) but the raw
        // string still drives the unbounded expansion. With the DNF cap this
        // returns promptly; without it, this test hangs.
        let groups = vec!["(CS101|CS101)"; 30].join(" & ");
        let yaml = format!(
            r#"
degree:
  id: pathological
  institution: Test University
  program: Test Program
  total_credits: 8
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS201]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
  CS201:
    title: Data Structures
    prefix: CS
    number: "201"
    credits: 4
    prerequisites_raw: "{groups}"
"#
        );

        let response = execute(
            &yaml,
            &PlanGraphOptions {
                plan_category: Some("shortest"),
                max_plans: Some(10),
                ..PlanGraphOptions::default()
            },
        );
        assert!(
            response.success,
            "render must stay bounded on a pathological prereq: {:?}",
            response.error
        );
        assert!(response.node_count.is_some_and(|n| n > 0));
    }
}
