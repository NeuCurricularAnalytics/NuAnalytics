//! One-call curriculum-graph rendering tool.
//!
//! Provides the `render_plan_graph` MCP tool that compresses the four-step
//! "run analyze with `include_graph_spec`, pluck a plan, serialise its
//! `graph_spec`, hand off to `get_curriculum_visualization`" sequence into
//! a single call. Caller picks a plan by `plan_category` (`"shortest"`,
//! `"longest"`, `"calc-ready-shortest"`, `"sample"` + optional
//! `sample_index`) or by raw `plan_index` into `selected_plans`.

use crate::core::degree::plan_selector::PlanCategory;
use crate::core::report::visualization::{
    spec_from_scored_plan, CurriculumGraphRenderer, VanillaJsRenderer,
};
use crate::mcp::cache::cached_artifacts;
use crate::mcp::tools::visualize::VisualizationFormat;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for `render_plan_graph`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct RenderPlanGraphRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Filesystem path the server will read. Mutually exclusive with the others.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored degree id (DB lookup). Mutually exclusive with the others.
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

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
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub sample_index: Option<usize>,

    /// Raw 0-indexed offset into the analyze response's `selected_plans`
    /// list. Mutually exclusive with `plan_category`.
    #[schemars(
        description = "0-indexed offset into selected_plans (advanced). Mutually exclusive with plan_category."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub plan_index: Option<usize>,

    /// Rendering format. Defaults to `"standalone"` (full HTML page).
    #[schemars(
        description = "Render format: \"standalone\" (default, full HTML page), \"fragment\", or \"fragment-no-library\"."
    )]
    #[serde(default)]
    pub format: VisualizationFormat,

    /// Forwarded to `analyze_degree`: cap on plans generated.
    #[schemars(description = "Maximum plans to generate during analysis (default 500)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub max_plans: Option<usize>,

    /// Forwarded to `analyze_degree`: courses every generated plan must include.
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
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub dry_run: Option<bool>,
}

/// Response for `render_plan_graph`.
#[derive(Debug, Serialize)]
pub struct RenderPlanGraphResponse {
    /// True when the YAML parsed, the plan was found, and rendering succeeded.
    pub success: bool,
    /// Error message when `success` is false.
    pub error: Option<String>,
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
    /// Number of course nodes in the generated `CurriculumGraphSpec`.
    /// Useful as a complexity proxy during dry-run probing.
    pub node_count: Option<usize>,
    /// Whether this response was a dry-run probe (no HTML payload).
    pub dry_run: bool,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `render_plan_graph` tool.
///
/// Argument count crosses clippy's default ceiling because the tool exposes
/// every option of the analyze pipeline plus the picker fields; grouping
/// them into a struct would just move the same data.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute(
    yaml_content: &str,
    plan_category: Option<&str>,
    sample_index: Option<usize>,
    plan_index: Option<usize>,
    format: VisualizationFormat,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    dry_run: bool,
) -> RenderPlanGraphResponse {
    if plan_category.is_none() && plan_index.is_none() {
        return error_response(
            "Provide either plan_category (\"shortest\" / \"longest\" / \"calc-ready-shortest\" / \"sample\") or plan_index.",
        );
    }
    if plan_category.is_some() && plan_index.is_some() {
        return error_response("Provide plan_category OR plan_index, not both.");
    }

    let artifacts = match cached_artifacts(yaml_content, max_plans, include_courses) {
        Ok(a) => a,
        Err(e) => return error_response(e),
    };

    // PlanCategory is Copy; capture each tuple as (index, category, &ScoredPlan)
    // so the picker functions can return owned `PlanCategory` values cheaply.
    let entries: Vec<(usize, PlanCategory, _)> = artifacts
        .selected
        .iter()
        .enumerate()
        .map(|(idx, (cat, plan))| (idx, cat, plan))
        .collect();

    let picked = plan_index.map_or_else(
        || {
            let category = plan_category.unwrap_or("");
            pick_by_category(&entries, category, sample_index)
        },
        |idx| entries.iter().find(|(i, _, _)| *i == idx).copied(),
    );

    let Some((idx, category, plan)) = picked else {
        return error_response(format!(
            "No selected plan matches plan_category={plan_category:?} / plan_index={plan_index:?} / sample_index={sample_index:?}. Selected_plans has {} entries.",
            entries.len()
        ));
    };

    let graph_id = category.file_name().to_string();
    let spec = spec_from_scored_plan(
        &artifacts.school,
        &artifacts.equivalences,
        plan,
        Some(&artifacts.aggregator),
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
        plan_category: Some(category.display_name().to_string()),
        plan_index: Some(idx),
        terms: Some(plan.score.terms_required),
        complexity: Some(plan.score.total_complexity),
        longest_delay: Some(plan.score.longest_delay),
        html: html_field,
        html_bytes,
        node_count: Some(node_count),
        dry_run,
    }
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

/// Execute and serialize as JSON.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute_json(
    yaml_content: &str,
    plan_category: Option<&str>,
    sample_index: Option<usize>,
    plan_index: Option<usize>,
    format: VisualizationFormat,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    dry_run: bool,
) -> String {
    let response = execute(
        yaml_content,
        plan_category,
        sample_index,
        plan_index,
        format,
        max_plans,
        include_courses,
        dry_run,
    );
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

/// Pick the matching entry from `selected_plans` for a named category.
/// `category` is case-insensitive and accepts the kebab-case (`"calc-ready-shortest"`)
/// or `PlanCategory::file_name` form. `sample_index` is 1-indexed when
/// `category="sample"`; default 1.
fn pick_by_category<'a, P>(
    entries: &'a [(usize, PlanCategory, &'a P)],
    category: &str,
    sample_index: Option<usize>,
) -> Option<(usize, PlanCategory, &'a P)> {
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
        plan_category: None,
        plan_index: None,
        terms: None,
        complexity: None,
        longest_delay: None,
        html: None,
        html_bytes: 0,
        node_count: None,
        dry_run: false,
    }
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
    fn test_render_shortest_returns_standalone_html() {
        let response = execute(
            TEST_YAML,
            Some("shortest"),
            None,
            None,
            VisualizationFormat::Standalone,
            Some(10),
            None,
            false,
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
            None,
            None,
            Some(0),
            VisualizationFormat::Fragment,
            Some(10),
            None,
            false,
        );
        assert!(response.success);
        assert_eq!(response.plan_index, Some(0));
        // Fragment mode must NOT include the DOCTYPE wrapper.
        let html = response.html.unwrap();
        assert!(!html.starts_with("<!DOCTYPE html>"));
    }

    #[test]
    fn test_requires_either_category_or_index() {
        let response = execute(
            TEST_YAML,
            None,
            None,
            None,
            VisualizationFormat::Standalone,
            None,
            None,
            false,
        );
        assert!(!response.success);
        let err = response.error.unwrap();
        assert!(err.contains("plan_category") && err.contains("plan_index"));
    }

    #[test]
    fn test_rejects_both_category_and_index() {
        let response = execute(
            TEST_YAML,
            Some("shortest"),
            None,
            Some(0),
            VisualizationFormat::Standalone,
            None,
            None,
            false,
        );
        assert!(!response.success);
        assert!(response.error.unwrap().contains("not both"));
    }

    #[test]
    fn test_unknown_category_surfaces_error() {
        let response = execute(
            TEST_YAML,
            Some("nonsense"),
            None,
            None,
            VisualizationFormat::Standalone,
            Some(10),
            None,
            false,
        );
        assert!(!response.success);
        assert!(response.error.unwrap().contains("No selected plan matches"));
    }

    #[test]
    fn test_out_of_range_plan_index_surfaces_error() {
        let response = execute(
            TEST_YAML,
            None,
            None,
            Some(999),
            VisualizationFormat::Standalone,
            Some(10),
            None,
            false,
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
            Some("shortest"),
            None,
            None,
            VisualizationFormat::Standalone,
            Some(10),
            None,
            true,
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
    fn test_fragment_no_library_drops_shared_prelude() {
        let response = execute(
            TEST_YAML,
            Some("shortest"),
            None,
            None,
            VisualizationFormat::FragmentNoLibrary,
            Some(10),
            None,
            false,
        );
        assert!(response.success);
        let html = response.html.unwrap();
        assert!(html.contains("nuGraphs.register"));
        // The shared GRAPH_VANILLA_JS prelude marker must be absent.
        assert!(!html.contains("window.nuGraphs ="));
    }
}
