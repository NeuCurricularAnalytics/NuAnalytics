//! Degree analysis tool
//!
//! Provides the `analyze_degree` MCP tool that runs full degree analysis:
//! generates plans, computes aggregate metrics, and returns structured results.
//!
//! The pipeline (parse → graph → plan generation → aggregation) is factored
//! into [`build_artifacts`] / [`AnalysisArtifacts`] so sibling tools (e.g.
//! `generate_degree_report`) can reuse the same flow without duplicating
//! ~50 lines of orchestration.

use crate::core::degree::{
    parse_degree_yaml, DegreeParseError, PlanGenerationStats, PlanGenerator, PlanGeneratorConfig,
    PlanSelector, PlanSelectorConfig, PlanVariant, SamplingStrategy, SelectedPlans,
};
use crate::core::metrics::compute_all_metrics;
use crate::core::models::{Course, CourseGraph, School, DAG};
use crate::core::report::visualization::{spec_from_scored_plan, CurriculumGraphSpec};
use crate::core::report::SchedulerConfig;
use crate::core::statistics::{AggregatorConfig, MetricStats, MetricsAggregator};
use crate::core::DegreeProgram;
use crate::mcp::tools::shared::{
    ToolFollowup, TOOL_ANALYZE_DEGREE, TOOL_AUDIT_DEGREE, TOOL_VALIDATE_DEGREE,
};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `analyze_degree` tool
///
/// Provide exactly one YAML source: `yaml_content` (inline), `yaml_path`
/// (workspace-relative file), or `degree_id` (stored in the database —
/// requires the `database` feature).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnalyzeDegreeRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Filesystem path the server will read. Mutually exclusive with the others.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored `degree_id` (DB lookup). Mutually exclusive with the others.
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

    /// Maximum number of plans to generate (default: 500)
    #[schemars(
        description = "Maximum plans to generate (default: 500, higher = more accurate but slower)"
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub max_plans: Option<usize>,

    /// Courses to always include in all generated plans
    #[schemars(
        description = "Comma-separated list of course codes to include in all plans (e.g., 'CS150B,MATH156,CS414'). These courses will be present in every generated plan."
    )]
    pub include_courses: Option<String>,

    /// Include full visualization `graph_spec` for each selected plan (default false).
    ///
    /// `selected_plans` always returns a curated set independent of `max_plans`:
    /// shortest + longest + (optional) calc-ready-shortest + 3 random samples.
    /// Typical size is 5–6 plans (5 when no calculus track, 6 with calc-ready).
    /// Each `graph_spec` is ~30 KB; pass true only when you'll render the
    /// visualization. Pair with `get_curriculum_visualization` to render the
    /// returned spec to HTML. Use `plan_indices` to limit which plans get a
    /// `graph_spec` instead of paying the cost for all of them.
    #[schemars(
        description = "Include full graph_spec per selected plan (default false). selected_plans is always a curated 5-6 plans (shortest + longest + optional calc-ready + 3 random) regardless of max_plans. Each spec is ~30 KB; opt in only when rendering. Combine with plan_indices to limit which plans receive a spec."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub include_graph_spec: Option<bool>,

    /// Comma-separated `selected_plans` indices that should receive a
    /// `graph_spec` (only consulted when `include_graph_spec=true`).
    ///
    /// E.g. `"0,2"` keeps the spec on the shortest path and the calc-ready
    /// shortest while dropping it from the longest and the random samples.
    /// Indices outside the returned `selected_plans` range are ignored.
    /// When omitted, every selected plan receives a spec (current behavior).
    #[schemars(
        description = "Comma-separated selected_plans indices to include graph_spec for (e.g. \"0,2\"). Only honored when include_graph_spec=true. Omit to include specs for all selected plans."
    )]
    pub plan_indices: Option<String>,

    /// Emit a `per_course_metrics` array alongside the degree-level
    /// statistics. Default false — the metrics are buried in the rendered
    /// `graph_spec` payload otherwise, which costs ~30 KB per plan and
    /// forces the caller to render HTML just to read the numbers.
    ///
    /// When true, the response gains one entry per course the aggregator
    /// tracked (typically the union of courses that appeared in any of
    /// the analyzed plans) with the standard 5-number summary for
    /// complexity, centrality, delay, and blocking.
    #[schemars(
        description = "Include per-course metric medians (complexity, centrality, delay, blocking) for every tracked course in the response. Default false. Adds ~50 entries for a typical CS degree."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub include_per_course_metrics: Option<bool>,
}

/// Serializable metric statistics (includes quartiles for box plots).
///
/// Implements `Default` for zero-state fallback when a course has not been
/// tracked by the aggregator (e.g. course-detail responses for an elective
/// placeholder).
#[derive(Debug, Default, Serialize)]
pub struct MetricStatsJson {
    /// Minimum value
    pub min: f64,
    /// First quartile (25th percentile)
    pub q1: f64,
    /// Median value (50th percentile)
    pub median: f64,
    /// Third quartile (75th percentile)
    pub q3: f64,
    /// Maximum value
    pub max: f64,
    /// Mean value
    pub mean: f64,
    /// Standard deviation
    pub std_dev: f64,
}

/// Summary of a selected plan
#[derive(Debug, Serialize)]
pub struct PlanSummaryJson {
    /// Plan category (e.g., "Shortest Path")
    pub category: String,
    /// Number of terms required
    pub terms: usize,
    /// Total structural complexity
    pub complexity: usize,
    /// Longest delay factor
    pub longest_delay: usize,
    /// Critical path (longest delay chain)
    pub critical_path: Vec<String>,
    /// Total credits
    pub credits: f32,
    /// Number of courses
    pub course_count: usize,
    /// Term-by-term schedule
    pub schedule: Vec<TermJson>,
    /// Complete visualization spec for this plan. Only populated when
    /// `include_graph_spec=true` is set on the `analyze_degree` request;
    /// otherwise the field is omitted from the response entirely.
    ///
    /// Pass the serialized form of this field directly to
    /// `get_curriculum_visualization` to render an interactive HTML graph.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub graph_spec: Option<CurriculumGraphSpec>,
}

/// Per-course aggregate metrics for one tracked course.
///
/// Surfaced on `analyze_degree` responses when
/// `include_per_course_metrics=true`. Each metric uses the same 5-number
/// summary shape as the degree-level [`MetricStatsJson`] so callers can
/// reuse the same boxplot-rendering code path.
#[derive(Debug, Serialize)]
pub struct CourseMetricsJson {
    /// Course identifier (matches the keys in `program.courses`).
    pub course_id: String,
    /// Number of generated plans that contained this course. Lets the
    /// caller weight metric reliability — a course that appeared in 3 of
    /// 500 plans has noisier numbers than one in 480.
    pub plan_count: usize,
    /// Structural complexity metric.
    pub complexity: MetricStatsJson,
    /// Centrality metric (how often the course sits on a critical path).
    pub centrality: MetricStatsJson,
    /// Delay factor — terms separating the course from the degree end.
    pub delay: MetricStatsJson,
    /// Blocking factor — number of downstream courses gated by this one.
    pub blocking: MetricStatsJson,
}

/// A single term in a plan schedule
#[derive(Debug, Serialize)]
pub struct TermJson {
    /// Term number
    pub term: usize,
    /// Courses in this term
    pub courses: Vec<String>,
    /// Total credits this term
    pub credits: f32,
}

/// Complete analysis response
#[derive(Debug, Serialize)]
pub struct AnalysisResponse {
    /// Whether analysis completed successfully
    pub success: bool,
    /// Error message if analysis failed
    pub error: Option<String>,

    /// Degree program name
    pub degree_name: Option<String>,
    /// Institution name
    pub institution: Option<String>,
    /// Total courses defined
    pub total_courses: usize,
    /// Total requirements defined
    pub total_requirements: usize,

    /// Number of plans analyzed
    pub plans_analyzed: usize,
    /// Whether the result was truncated (more plans exist)
    pub was_truncated: bool,
    /// Total population size — the unique plans that exist for this YAML.
    ///
    /// When `is_full_population` is true, this equals `plans_analyzed`. When
    /// false, this is an upper-bound estimate from the requirement-choice
    /// product (real unique count after dedup may be lower).
    pub population_size: usize,
    /// True when every unique plan was analyzed (no sampling, no truncation).
    /// Equivalent to `!was_truncated`; exposed so callers can frame results
    /// honestly as "full population" vs "sample of N plans".
    pub is_full_population: bool,

    /// Aggregate complexity statistics across all plans
    pub complexity: Option<MetricStatsJson>,
    /// Aggregate longest delay statistics
    pub longest_delay: Option<MetricStatsJson>,
    /// Aggregate total credits statistics
    pub total_credits: Option<MetricStatsJson>,

    /// Selected special plans
    pub selected_plans: Vec<PlanSummaryJson>,

    /// Per-course aggregate metrics, one entry per course the aggregator
    /// tracked. Empty (and omitted from the JSON) unless the request set
    /// `include_per_course_metrics=true` — the array runs ~50 entries for a
    /// typical CS degree and most callers don't need it.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub per_course_metrics: Vec<CourseMetricsJson>,

    /// Structured hints about the next MCP call worth making, based on the
    /// analyze outcome (truncation, long critical path, small full
    /// population, etc.).
    pub tool_followups: Vec<ToolFollowup>,
}

// ============================================================================
// Constants
// ============================================================================

const DEFAULT_MAX_PLANS: usize = 500;

// ============================================================================
// Tool Implementation
// ============================================================================

/// Execute the `analyze_degree` tool
///
/// # Arguments
/// * `yaml_content` - The degree program YAML content
/// * `max_plans` - Maximum number of plans to generate (default: 500)
/// * `include_courses` - Optional courses to always include in all plans
/// * `include_graph_spec` - When true, populates `graph_spec` on each
///   selected plan (default false; suppresses ~30 KB per plan)
/// * `plan_indices` - When `Some`, only the listed `selected_plans` indices
///   receive a `graph_spec` (only consulted when `include_graph_spec=true`)
/// * `include_per_course_metrics` - When true, populates the
///   `per_course_metrics` array on the response (default false)
#[must_use]
pub fn execute(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    include_graph_spec: bool,
    plan_indices: Option<&[usize]>,
    include_per_course_metrics: bool,
) -> AnalysisResponse {
    match crate::mcp::cache::cached_artifacts(yaml_content, max_plans, include_courses) {
        Ok(artifacts) => build_response(
            &artifacts,
            include_graph_spec,
            plan_indices,
            include_per_course_metrics,
        ),
        Err(e) => parse_error_response(&e),
    }
}

/// Owned bundle of every value the analysis pipeline produces — the parsed
/// program, the course graph (as a DAG + the equivalence map), the metrics
/// aggregator, and the curated [`SelectedPlans`] alongside the generation
/// stats. Sibling tools (e.g. the HTML report renderer) consume this struct
/// so the pipeline is implemented exactly once.
///
/// Exposed at `pub(crate)` so [`crate::mcp::cache::cached_artifacts`] can
/// Arc-wrap the bundle and share it across tools.
pub(crate) struct AnalysisArtifacts {
    /// Parsed degree program.
    pub program: DegreeProgram,
    /// School/course catalog derived from the program.
    pub school: School,
    /// Course DAG built from the prerequisite graph (cycles already broken).
    pub dag: DAG,
    /// Map from course key to the set of equivalent courses.
    pub equivalences: HashMap<String, HashSet<String>>,
    /// Aggregated metrics across every plan that was processed.
    pub aggregator: MetricsAggregator,
    /// Curated selected plans (shortest, longest, calc-ready, random samples).
    pub selected: SelectedPlans,
    /// Number of plans actually processed (after dedup, capped at `max_plans`).
    pub plans_processed: usize,
    /// Pre-generation stats; `stats.total_possible` is the upper bound on
    /// distinct plans for this YAML.
    pub stats: PlanGenerationStats,
    /// The effective `max_plans` cap used by the run.
    pub max_plans: usize,
}

impl AnalysisArtifacts {
    /// True when every distinct plan was analyzed — the cap was not hit, or
    /// the cap was hit but the underlying upper bound did not exceed it.
    /// Sibling tools (`analyze_degree` JSON output, the HTML report) use this
    /// to frame results honestly as "full population" vs "sample of N".
    pub const fn is_full_population(&self) -> bool {
        !(self.plans_processed >= self.max_plans && self.stats.total_possible > self.max_plans)
    }

    /// Effective population size: the actual processed count when the run
    /// covered everything, otherwise the upper-bound estimate from the
    /// requirement-choice product.
    pub const fn population_size(&self) -> usize {
        if self.is_full_population() {
            self.plans_processed
        } else {
            self.stats.total_possible
        }
    }
}

/// Run the full analysis pipeline against `yaml_content` and return the
/// produced artifacts. On YAML parse failure, returns a formatted error
/// string suitable for surfacing through MCP tools.
///
/// Prefer [`crate::mcp::cache::cached_artifacts`] over calling this directly
/// — the cache shares the resulting [`AnalysisArtifacts`] Arc across sibling
/// tools so the expensive pipeline runs once per `(yaml, max_plans,
/// include_courses)` combination.
///
/// # Errors
/// Returns a formatted parse-error string when the YAML cannot be parsed.
pub(crate) fn build_artifacts(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
) -> Result<AnalysisArtifacts, String> {
    let max = max_plans.unwrap_or(DEFAULT_MAX_PLANS);
    let include = include_courses.map(<[String]>::to_vec).unwrap_or_default();

    let program = parse_degree_yaml(yaml_content).map_err(|e| format_parse_error(&e))?;

    let school = build_school(&program);
    let mut graph_result = CourseGraph::from_degree_program(&program);
    if !graph_result.cycles.is_empty() {
        graph_result.graph.break_cycles(&graph_result.cycles);
        graph_result.cycles.clear();
    }
    let dag = build_dag(&graph_result.graph);
    let equivalences = build_equivalences(&program.requirements);

    let gen_config = PlanGeneratorConfig {
        max_plans: max,
        ignore_duplicates: true,
        sample_count: 3,
        target_credits: program.degree.total_credits,
        sampling_strategy: SamplingStrategy::Shuffled,
        include_courses: include,
        ..Default::default()
    };
    let generator = PlanGenerator::new(&program.requirements, &program.courses, gen_config.clone());
    let stats = generator.get_stats();

    let agg_config = AggregatorConfig {
        reservoir_size: 1000,
        track_per_course: true,
        exact_mode: stats.total_possible <= 10000,
    };
    let selector_config = PlanSelectorConfig {
        sample_count: gen_config.sample_count,
        scheduler_config: SchedulerConfig::default(),
        ..Default::default()
    };

    let mut aggregator = MetricsAggregator::new(agg_config);
    let plans_processed;
    let selected = {
        let mut selector = PlanSelector::new(&school, &dag, selector_config);
        let ctx = AnalysisCtx {
            graph: &graph_result.graph,
            equivalences: &equivalences,
            school: &school,
            target_credits: program.degree.total_credits,
        };
        plans_processed = run_plan_analysis(
            &generator,
            &gen_config,
            &ctx,
            max,
            &mut aggregator,
            &mut selector,
        );
        selector.into_selected_plans()
    };

    Ok(AnalysisArtifacts {
        program,
        school,
        dag,
        equivalences,
        aggregator,
        selected,
        plans_processed,
        stats,
        max_plans: max,
    })
}

/// Build the parse-error escape hatch for the analyze response.
fn parse_error_response(error: &str) -> AnalysisResponse {
    AnalysisResponse {
        success: false,
        error: Some(error.to_string()),
        degree_name: None,
        institution: None,
        total_courses: 0,
        total_requirements: 0,
        plans_analyzed: 0,
        was_truncated: false,
        population_size: 0,
        is_full_population: false,
        complexity: None,
        longest_delay: None,
        total_credits: None,
        selected_plans: vec![],
        per_course_metrics: vec![],
        tool_followups: vec![ToolFollowup {
            tool: TOOL_VALIDATE_DEGREE,
            reason: "analyze_degree couldn't parse the YAML; validate_degree surfaces the parse error in a more structured form.".to_string(),
            suggested_args: serde_json::json!({}),
        }],
    }
}

/// Context for plan analysis processing
struct AnalysisCtx<'a> {
    graph: &'a CourseGraph,
    equivalences: &'a HashMap<String, HashSet<String>>,
    school: &'a School,
    target_credits: Option<u32>,
}

/// Process plan variants, updating aggregator and selector
fn run_plan_analysis(
    generator: &PlanGenerator<'_>,
    gen_config: &PlanGeneratorConfig,
    ctx: &AnalysisCtx<'_>,
    max: usize,
    aggregator: &mut MetricsAggregator,
    selector: &mut PlanSelector<'_>,
) -> usize {
    let mut plans_processed = 0;
    let mut seen_fingerprints = HashSet::new();

    for variant in generator.generate() {
        if plans_processed >= max {
            break;
        }

        if gen_config.ignore_duplicates {
            let fp = variant.fingerprint();
            if seen_fingerprints.contains(&fp) {
                continue;
            }
            seen_fingerprints.insert(fp);
        }

        let expanded = expand_with_prereqs(&variant.courses, ctx.graph, ctx.equivalences);
        let plan_dag = build_plan_dag(&expanded, ctx.graph, ctx.equivalences);

        let Ok(course_metrics) = compute_all_metrics(&plan_dag) else {
            continue;
        };

        let expanded_variant =
            build_expanded_variant(&variant, &expanded, ctx.school, ctx.target_credits);

        aggregator.add_plan(&course_metrics, f64::from(expanded_variant.total_credits));
        selector.process_plan(&expanded_variant, &course_metrics, &plan_dag);

        plans_processed += 1;
    }

    plans_processed
}

/// Build the analysis response from a populated [`AnalysisArtifacts`] bundle.
fn build_response(
    artifacts: &AnalysisArtifacts,
    include_graph_spec: bool,
    plan_indices: Option<&[usize]>,
    include_per_course_metrics: bool,
) -> AnalysisResponse {
    let degree_stats = artifacts.aggregator.degree_stats();

    let selected_plans: Vec<PlanSummaryJson> = artifacts
        .selected
        .iter()
        .enumerate()
        .map(|(idx, (cat, plan))| {
            let spec_wanted =
                include_graph_spec && plan_indices.is_none_or(|allowed| allowed.contains(&idx));
            let graph_spec = if spec_wanted {
                let graph_id = cat.display_name().to_lowercase().replace(' ', "-");
                Some(spec_from_scored_plan(
                    &artifacts.school,
                    &artifacts.equivalences,
                    plan,
                    Some(&artifacts.aggregator),
                    &graph_id,
                ))
            } else {
                None
            };

            PlanSummaryJson {
                category: cat.display_name().to_string(),
                terms: plan.score.terms_required,
                complexity: plan.score.total_complexity,
                longest_delay: plan.score.longest_delay,
                critical_path: plan.score.longest_delay_chain.clone(),
                credits: plan.variant.total_credits,
                course_count: plan.variant.courses.len(),
                schedule: plan
                    .schedule
                    .terms
                    .iter()
                    .filter(|t| !t.courses.is_empty())
                    .map(|t| TermJson {
                        term: t.number,
                        courses: t.courses.clone(),
                        credits: t.total_credits,
                    })
                    .collect(),
                graph_spec,
            }
        })
        .collect();

    let is_full_population = artifacts.is_full_population();
    let was_truncated = !is_full_population;
    let population_size = artifacts.population_size();
    let tool_followups = build_analysis_followups(
        artifacts,
        &selected_plans,
        was_truncated,
        is_full_population,
    );
    let per_course_metrics = if include_per_course_metrics {
        build_per_course_metrics(artifacts)
    } else {
        Vec::new()
    };

    AnalysisResponse {
        success: true,
        error: None,
        degree_name: Some(artifacts.program.degree.name.clone()),
        institution: artifacts.program.degree.institution.clone(),
        total_courses: artifacts.program.courses.len(),
        total_requirements: artifacts.program.requirements.len(),
        plans_analyzed: artifacts.plans_processed,
        was_truncated,
        population_size,
        is_full_population,
        complexity: Some(metric_stats_json(&degree_stats.total_complexity)),
        longest_delay: Some(metric_stats_json(&degree_stats.longest_delay)),
        total_credits: Some(metric_stats_json(&degree_stats.total_credits)),
        selected_plans,
        per_course_metrics,
        tool_followups,
    }
}

/// Materialise every course the aggregator tracked into a sorted
/// `Vec<CourseMetricsJson>`. Sorting by `course_id` makes the response
/// deterministic across runs so diff-friendly snapshot tests are practical.
fn build_per_course_metrics(artifacts: &AnalysisArtifacts) -> Vec<CourseMetricsJson> {
    let mut ids = artifacts.aggregator.course_ids();
    ids.sort();
    ids.into_iter()
        .filter_map(|id| {
            artifacts.aggregator.course_stats(&id).map(|s| {
                let plan_count = s.plan_count;
                CourseMetricsJson {
                    course_id: id,
                    plan_count,
                    complexity: metric_stats_json(&s.complexity),
                    centrality: metric_stats_json(&s.centrality),
                    delay: metric_stats_json(&s.delay),
                    blocking: metric_stats_json(&s.blocking),
                }
            })
        })
        .collect()
}

/// Build follow-up suggestions for an analyze response. Triggered on three
/// signals: truncated sample (rerun with higher cap), tiny full population
/// (cheap to audit deeply), or long critical path on the shortest plan
/// (re-audit with a stricter chain threshold).
fn build_analysis_followups(
    artifacts: &AnalysisArtifacts,
    selected_plans: &[PlanSummaryJson],
    was_truncated: bool,
    is_full_population: bool,
) -> Vec<ToolFollowup> {
    let mut followups = Vec::new();

    if was_truncated {
        // Double the cap as a follow-up suggestion. `saturating_mul(2)`
        // guards against usize overflow on absurdly large caps; the
        // `.max(+1)` guard catches the `max_plans == usize::MAX` corner
        // where doubling would saturate back to the same value and we'd
        // otherwise echo the input.
        let next = artifacts
            .max_plans
            .saturating_mul(2)
            .max(artifacts.max_plans + 1);
        followups.push(ToolFollowup {
            tool: TOOL_ANALYZE_DEGREE,
            reason: format!(
                "Result was truncated at max_plans={} (population estimate {}). Rerun with a larger cap to widen the sample.",
                artifacts.max_plans, artifacts.stats.total_possible,
            ),
            suggested_args: serde_json::json!({ "max_plans": next }),
        });
    } else if is_full_population && artifacts.plans_processed > 0 && artifacts.plans_processed < 50
    {
        followups.push(ToolFollowup {
            tool: TOOL_AUDIT_DEGREE,
            reason: format!(
                "Full population is small ({}). audit_degree's deep-chain analysis is cheap here and surfaces structural issues.",
                artifacts.plans_processed,
            ),
            suggested_args: serde_json::json!({}),
        });
    }

    if let Some(shortest) = selected_plans
        .iter()
        .find(|p| p.category == "Shortest Path")
    {
        if shortest.critical_path.len() >= 6 {
            followups.push(ToolFollowup {
                tool: TOOL_AUDIT_DEGREE,
                reason: format!(
                    "Shortest path's critical chain is {} courses long; rerunning audit_degree with a stricter chain_threshold surfaces every chain at that depth.",
                    shortest.critical_path.len(),
                ),
                suggested_args: serde_json::json!({ "chain_threshold": 4 }),
            });
        }
    }

    followups
}

/// Execute and serialize as JSON
///
/// # Arguments
/// * `yaml_content` - The degree program YAML content
/// * `max_plans` - Maximum number of plans to generate
/// * `include_courses` - Optional courses to always include in all plans
/// * `include_graph_spec` - When true, include `graph_spec` per selected plan
/// * `plan_indices` - Optional whitelist of `selected_plans` indices for
///   `graph_spec` inclusion; consulted only when `include_graph_spec=true`
/// * `include_per_course_metrics` - When true, populate `per_course_metrics`
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute_json(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    include_graph_spec: bool,
    plan_indices: Option<&[usize]>,
    include_per_course_metrics: bool,
) -> String {
    let response = execute(
        yaml_content,
        max_plans,
        include_courses,
        include_graph_spec,
        plan_indices,
        include_per_course_metrics,
    );
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError(msg) => format!("YAML syntax error: {msg}"),
    }
}

pub(super) const fn metric_stats_json(s: &MetricStats) -> MetricStatsJson {
    MetricStatsJson {
        min: s.min,
        q1: s.q1,
        median: s.median,
        q3: s.q3,
        max: s.max,
        mean: s.mean,
        std_dev: s.std_dev,
    }
}

fn build_school(program: &crate::core::DegreeProgram) -> School {
    let mut school = School::new(
        program
            .degree
            .institution
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );

    for (key, course) in &program.courses {
        let mut sc = Course::new(
            course.name.clone(),
            course.prefix.clone(),
            course.number.clone(),
            course.credit_hours,
        );
        sc.canonical_name = Some(key.clone());
        sc.prerequisites_raw.clone_from(&course.prerequisites_raw);
        if let Some(raw) = &course.prerequisites_raw {
            sc.prerequisites = parse_prereqs(raw);
        }
        sc.corequisites.clone_from(&course.corequisites);
        school.add_course(sc);
    }

    school
}

fn parse_prereqs(raw: &str) -> Vec<String> {
    let cleaned = raw.replace(['(', ')', '&', '|', '[', ']'], " ");
    cleaned
        .split_whitespace()
        .filter(|s| s.len() > 1)
        .map(String::from)
        .collect()
}

fn build_dag(graph: &CourseGraph) -> DAG {
    let mut dag = DAG::new();
    for key in graph.course_keys() {
        dag.add_course(key.to_string());
        if let Some(node) = graph.get(key) {
            for edge in &node.prerequisites {
                if edge.prereq_type == crate::core::models::course_graph::PrerequisiteType::Required
                {
                    dag.add_prerequisite(key.to_string(), &edge.prerequisite);
                } else if edge.prereq_type
                    == crate::core::models::course_graph::PrerequisiteType::Corequisite
                {
                    dag.add_corequisite(key.to_string(), &edge.prerequisite);
                }
            }
        }
    }
    dag
}

fn build_equivalences(
    requirements: &HashMap<String, crate::core::models::degree::Requirement>,
) -> HashMap<String, HashSet<String>> {
    let mut equivs: HashMap<String, HashSet<String>> = HashMap::new();
    for req in requirements.values() {
        if let Some(courses) = &req.courses {
            for course_ref in courses {
                if course_ref.starts_with('{') && course_ref.ends_with('}') {
                    let inner = &course_ref[1..course_ref.len() - 1];
                    let parts: Vec<String> =
                        inner.split(',').map(|s| s.trim().to_string()).collect();
                    for a in &parts {
                        for b in &parts {
                            if a != b {
                                equivs.entry(a.clone()).or_default().insert(b.clone());
                            }
                        }
                    }
                }
            }
        }
    }
    equivs
}

fn expand_with_prereqs(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
) -> Vec<String> {
    let mut expanded: HashSet<String> = courses.iter().cloned().collect();
    let mut to_process: Vec<String> = courses.to_vec();

    while let Some(key) = to_process.pop() {
        if let Some(chain) = graph.min_prerequisite_chain_with_context(&key, &expanded) {
            for prereq in chain {
                let has_equiv = equivalences
                    .get(&prereq)
                    .is_some_and(|eq| eq.iter().any(|e| expanded.contains(e)));
                if !has_equiv && !expanded.contains(&prereq) {
                    expanded.insert(prereq.clone());
                    to_process.push(prereq);
                }
            }
        }
    }

    let mut result: Vec<String> = expanded.into_iter().collect();
    result.sort();
    result
}

/// Build a DAG for the plan, considering course equivalences.
///
/// When a prerequisite isn't in the plan but an equivalent course is,
/// adds an edge from the equivalent to maintain proper sequencing.
fn build_plan_dag(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
) -> DAG {
    let plan_set: HashSet<&str> = courses.iter().map(String::as_str).collect();
    let mut dag = DAG::new();

    for key in courses {
        dag.add_course(key.clone());
        if let Some(node) = graph.get(key) {
            let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();

            for edge in &node.prerequisites {
                if edge.prereq_type
                    == crate::core::models::course_graph::PrerequisiteType::Corequisite
                {
                    continue;
                }
                if edge.prereq_type == crate::core::models::course_graph::PrerequisiteType::Required
                {
                    // Try direct match first
                    if plan_set.contains(edge.prerequisite.as_str()) {
                        dag.add_prerequisite(key.clone(), &edge.prerequisite);
                    } else {
                        // Check for equivalent course in plan
                        if let Some(equiv_in_plan) =
                            find_equivalent_in_plan(&edge.prerequisite, equivalences, &plan_set)
                        {
                            dag.add_prerequisite(key.clone(), equiv_in_plan);
                        }
                    }
                } else if let Some(group) = edge.or_group {
                    or_groups.entry(group).or_default().push(&edge.prerequisite);
                }
            }

            for (_group, options) in or_groups {
                for opt in options.iter().filter(|o| plan_set.contains(**o)) {
                    dag.add_prerequisite(key.clone(), opt);
                }
            }
        }
    }
    dag
}

/// Find an equivalent course that is in the plan.
///
/// Returns the first equivalent course found in the plan set, or None.
fn find_equivalent_in_plan<'a>(
    course: &str,
    equivalences: &HashMap<String, HashSet<String>>,
    plan_set: &HashSet<&'a str>,
) -> Option<&'a str> {
    equivalences.get(course).and_then(|equivs| {
        equivs
            .iter()
            .find_map(|eq| plan_set.get(eq.as_str()).copied())
    })
}

fn build_expanded_variant(
    original: &PlanVariant,
    expanded: &[String],
    school: &School,
    target_credits: Option<u32>,
) -> PlanVariant {
    let mut choices = original.requirement_choices.clone();

    let orig_set: HashSet<&str> = original.courses.iter().map(String::as_str).collect();
    let added: Vec<String> = expanded
        .iter()
        .filter(|c| !orig_set.contains(c.as_str()))
        .cloned()
        .collect();
    if !added.is_empty() {
        choices.insert("_prerequisites".to_string(), added);
    }

    let non_elec_credits: f32 = expanded
        .iter()
        .filter(|c| !c.starts_with("ELEC"))
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |co| co.credit_hours)
        })
        .sum();

    #[allow(clippy::cast_precision_loss)]
    let final_courses = target_credits.map_or_else(
        || expanded.to_vec(),
        |target| {
            let target_f32 = target as f32;
            if non_elec_credits >= target_f32 {
                choices.remove("_elective_placeholders");
                expanded
                    .iter()
                    .filter(|c| !c.starts_with("ELEC"))
                    .cloned()
                    .collect()
            } else {
                let needed = target_f32 - non_elec_credits;
                let electives = gen_elective_placeholders(needed);
                if electives.is_empty() {
                    choices.remove("_elective_placeholders");
                } else {
                    choices.insert("_elective_placeholders".to_string(), electives.clone());
                }
                let mut courses: Vec<String> = expanded
                    .iter()
                    .filter(|c| !c.starts_with("ELEC"))
                    .cloned()
                    .collect();
                courses.extend(electives);
                courses.sort();
                courses
            }
        },
    );

    let total: f32 = final_courses
        .iter()
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |co| co.credit_hours)
        })
        .sum();

    PlanVariant::from_parts(final_courses, choices, total)
}

fn placeholder_credits(key: &str) -> f32 {
    if key.ends_with('S') || key.ends_with("SM") {
        2.0
    } else {
        3.0
    }
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn gen_elective_placeholders(credits_needed: f32) -> Vec<String> {
    if credits_needed <= 0.0 {
        return Vec::new();
    }

    let mut result = Vec::new();
    let full = (credits_needed / 3.0).floor() as usize;
    #[allow(clippy::cast_precision_loss)] // full is small (< 50 electives)
    let remainder = (full as f32).mul_add(-3.0, credits_needed);

    for i in 1..=full {
        result.push(format!("ELEC_{i:02}"));
    }
    if remainder >= 1.5 {
        result.push(format!("ELEC_{:02}S", full + 1));
    }

    result
}

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

requirements:
  intro:
    name: Introduction
    type: all
    category: major
    courses:
      - CS101
      - CS201

courses:
  CS101:
    title: Intro to CS
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
    fn test_analyze_valid_degree() {
        let response = execute(TEST_YAML, Some(10), None, false, None, false);
        assert!(response.success, "error: {:?}", response.error);
        assert!(response.plans_analyzed > 0);
        assert!(response.complexity.is_some());
        assert!(response.total_credits.is_some());
        assert!(!response.selected_plans.is_empty());
    }

    #[test]
    fn test_build_artifacts_populates_pipeline_outputs() {
        // Direct coverage of the shared pipeline entry point. Every artifact
        // field must be populated so sibling tools (the HTML report) don't
        // have to defensively check for empty/None state.
        let artifacts =
            build_artifacts(TEST_YAML, Some(10), None).expect("build_artifacts on valid YAML");
        assert_eq!(artifacts.program.degree.name, "Test Program");
        assert_eq!(
            artifacts.program.degree.institution.as_deref(),
            Some("Test University")
        );
        assert_eq!(artifacts.max_plans, 10);
        assert!(artifacts.plans_processed > 0);
        assert!(artifacts.selected.total_count() > 0);
        assert!(artifacts.stats.total_possible > 0);
        // Pipeline outputs the analyzed-plan stats too.
        let stats = artifacts.aggregator.degree_stats();
        assert!(stats.plan_count > 0);
    }

    #[test]
    fn test_build_artifacts_returns_parse_error_for_malformed_yaml() {
        // AnalysisArtifacts deliberately doesn't derive Debug (it owns a
        // MetricsAggregator that wouldn't print usefully anyway), so use a
        // match instead of `unwrap_err` to interrogate the failure.
        let result = build_artifacts("not: valid: yaml: {{", Some(10), None);
        let Err(err) = result else {
            panic!("expected parse failure for malformed YAML");
        };
        assert!(
            err.to_lowercase().contains("yaml") || err.to_lowercase().contains("error"),
            "parse error must mention yaml/error context, got: {err}"
        );
    }

    #[test]
    fn test_build_artifacts_respects_include_courses() {
        // Every selected plan must contain the forced course.
        let artifacts = build_artifacts(TEST_YAML, Some(10), Some(&["CS101".to_string()])).unwrap();
        for (_cat, plan) in artifacts.selected.iter() {
            assert!(
                plan.variant.courses.iter().any(|c| c == "CS101"),
                "include_courses=CS101 must force every selected plan to contain CS101"
            );
        }
    }

    #[test]
    fn test_tool_followups_suggest_audit_on_small_full_population() {
        // TEST_YAML resolves to a single valid plan ⇒ is_full_population=true
        // and plans_processed < 50, which triggers the audit suggestion.
        let response = execute(TEST_YAML, Some(500), None, false, None, false);
        assert!(response.success);
        assert!(response.is_full_population);
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "audit_degree"),
            "small full population should suggest audit_degree; got {:?}",
            response.tool_followups
        );
    }

    #[test]
    fn test_artifacts_is_full_population_when_under_cap() {
        // TEST_YAML has only one valid plan; max=500 means we never hit the cap.
        let artifacts = build_artifacts(TEST_YAML, Some(500), None).unwrap();
        assert!(artifacts.is_full_population());
        assert_eq!(artifacts.population_size(), artifacts.plans_processed);
    }

    #[test]
    fn test_analyze_malformed_yaml() {
        let response = execute("not: valid: yaml: {{", Some(10), None, false, None, false);
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_analyze_json_output() {
        let json = execute_json(TEST_YAML, Some(10), None, false, None, false);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["success"].as_bool().unwrap());
        assert!(parsed["plans_analyzed"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_selected_plans_have_schedules() {
        let response = execute(TEST_YAML, Some(10), None, false, None, false);
        for plan in &response.selected_plans {
            assert!(
                !plan.schedule.is_empty(),
                "{} has no schedule",
                plan.category
            );
            assert!(plan.terms > 0);
            assert!(plan.credits > 0.0);
        }
    }

    #[test]
    fn test_include_courses() {
        let response = execute(
            TEST_YAML,
            Some(10),
            Some(&["CS101".to_string()]),
            false,
            None,
            false,
        );
        assert!(response.success, "error: {:?}", response.error);
        assert!(response.plans_analyzed > 0);
        // All plans should include CS101
        for plan in &response.selected_plans {
            let has_cs101 = plan
                .schedule
                .iter()
                .flat_map(|t| t.courses.iter().map(String::as_str))
                .any(|c| c == "CS101");
            assert!(has_cs101, "Plan {} should contain CS101", plan.category);
        }
    }

    #[test]
    fn test_placeholder_credits() {
        assert!((super::placeholder_credits("GE01") - 3.0).abs() < f32::EPSILON);
        assert!((super::placeholder_credits("GE01S") - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_analyze_omits_graph_spec_by_default() {
        // include_graph_spec=false (default) — graph_spec must be None in-memory and
        // skipped entirely from the JSON output (no `"graph_spec": null` either).
        let response = execute(TEST_YAML, Some(10), None, false, None, false);
        assert!(response.success);
        assert!(!response.selected_plans.is_empty());
        for plan in &response.selected_plans {
            assert!(
                plan.graph_spec.is_none(),
                "Plan {} unexpectedly carries graph_spec when flag is false",
                plan.category
            );
        }
        let json: serde_json::Value =
            serde_json::from_str(&execute_json(TEST_YAML, Some(10), None, false, None, false))
                .unwrap();
        for plan in json["selected_plans"].as_array().unwrap() {
            assert!(
                plan.get("graph_spec").is_none(),
                "graph_spec key must not appear in JSON when include_graph_spec=false"
            );
        }
    }

    #[test]
    fn test_analyze_includes_graph_spec_when_requested() {
        let response = execute(TEST_YAML, Some(10), None, true, None, false);
        assert!(response.success);
        assert!(!response.selected_plans.is_empty());
        for plan in &response.selected_plans {
            let spec = plan.graph_spec.as_ref().unwrap_or_else(|| {
                panic!(
                    "Plan {} should have graph_spec when flag is true",
                    plan.category
                )
            });
            assert!(!spec.graph_id.is_empty(), "graph_id must not be empty");
            assert!(!spec.nodes.is_empty(), "nodes must not be empty");
            assert!(!spec.terms.is_empty(), "terms must not be empty");
        }
    }

    #[test]
    fn test_plan_indices_filters_graph_spec_attachment() {
        // Only index 0 should carry graph_spec; the rest must be None even
        // though include_graph_spec=true.
        let response = execute(TEST_YAML, Some(10), None, true, Some(&[0]), false);
        assert!(response.success);
        let mut plans = response.selected_plans.into_iter();
        let first = plans.next().expect("at least one selected plan");
        assert!(
            first.graph_spec.is_some(),
            "plan_indices=[0] must keep graph_spec on the first plan"
        );
        for plan in plans {
            assert!(
                plan.graph_spec.is_none(),
                "plan_indices=[0] must drop graph_spec from plan '{}'",
                plan.category
            );
        }
    }

    #[test]
    fn test_plan_indices_ignored_when_include_graph_spec_false() {
        // plan_indices is a no-op when graph_spec attachment is off.
        let response = execute(TEST_YAML, Some(10), None, false, Some(&[0, 1, 2]), false);
        assert!(response.success);
        for plan in &response.selected_plans {
            assert!(
                plan.graph_spec.is_none(),
                "plan_indices must not force graph_spec when include_graph_spec=false"
            );
        }
    }

    #[test]
    fn test_plan_indices_out_of_range_silently_ignored() {
        // Index past the end is dropped without error.
        let response = execute(TEST_YAML, Some(10), None, true, Some(&[999]), false);
        assert!(response.success);
        for plan in &response.selected_plans {
            assert!(
                plan.graph_spec.is_none(),
                "out-of-range plan_indices should produce no graph_specs"
            );
        }
    }

    #[test]
    fn test_population_size_matches_plans_analyzed_when_full() {
        // The simple TEST_YAML has only one valid plan (CS101 → CS201).
        // With max_plans well above the population we expect:
        //   was_truncated=false, is_full_population=true,
        //   population_size==plans_analyzed.
        let response = execute(TEST_YAML, Some(500), None, false, None, false);
        assert!(response.success);
        assert!(!response.was_truncated);
        assert!(response.is_full_population);
        assert_eq!(response.population_size, response.plans_analyzed);
        assert!(response.population_size > 0);
    }

    #[test]
    fn test_per_course_metrics_omitted_by_default_present_when_flag_set() {
        // Default: per_course_metrics empty and skipped during serialisation.
        let off = execute(TEST_YAML, Some(10), None, false, None, false);
        assert!(off.per_course_metrics.is_empty());
        let off_json = serde_json::to_string(&off).unwrap();
        assert!(
            !off_json.contains("\"per_course_metrics\""),
            "field must be skipped when empty"
        );

        // Opted-in: one entry per tracked course, sorted by course_id,
        // each carrying the four metric stats objects.
        let on = execute(TEST_YAML, Some(10), None, false, None, true);
        assert!(on.success);
        assert!(
            !on.per_course_metrics.is_empty(),
            "tracked courses must appear when flag is set"
        );
        let ids: Vec<&str> = on
            .per_course_metrics
            .iter()
            .map(|c| c.course_id.as_str())
            .collect();
        let mut sorted = ids.clone();
        sorted.sort_unstable();
        assert_eq!(ids, sorted, "entries must be sorted by course_id");
        for entry in &on.per_course_metrics {
            assert!(entry.plan_count > 0);
        }
    }

    #[test]
    fn test_parse_prereqs_strips_punct_and_filters_short_tokens() {
        assert_eq!(parse_prereqs("CS101 & CS201"), vec!["CS101", "CS201"]);
        assert_eq!(
            parse_prereqs("(CS101 | CS201) & CS301"),
            vec!["CS101", "CS201", "CS301"]
        );
        assert_eq!(parse_prereqs("[MATH101]"), vec!["MATH101"]);
        assert!(parse_prereqs("").is_empty());
        // single-character tokens are filtered out (stray operators, junk)
        assert!(parse_prereqs("a b c").is_empty());
    }

    #[test]
    fn test_gen_elective_placeholders_zero_or_negative() {
        assert!(gen_elective_placeholders(0.0).is_empty());
        assert!(gen_elective_placeholders(-3.0).is_empty());
    }

    #[test]
    fn test_gen_elective_placeholders_full_only() {
        // 6.0 → exactly 2 full (3-credit) electives, no remainder
        assert_eq!(gen_elective_placeholders(6.0), vec!["ELEC_01", "ELEC_02"]);
    }

    #[test]
    fn test_gen_elective_placeholders_with_seminar_remainder() {
        // 7.5 → 2 full + remainder 1.5 ≥ threshold → 1 seminar (S suffix)
        assert_eq!(
            gen_elective_placeholders(7.5),
            vec!["ELEC_01", "ELEC_02", "ELEC_03S"]
        );
    }

    #[test]
    fn test_gen_elective_placeholders_remainder_below_threshold_dropped() {
        // 4.0 → 1 full + remainder 1.0 < 1.5 → no seminar emitted
        assert_eq!(gen_elective_placeholders(4.0), vec!["ELEC_01"]);
    }

    #[test]
    fn test_find_equivalent_in_plan_returns_match_when_present() {
        let mut equivs: HashMap<String, HashSet<String>> = HashMap::new();
        equivs.insert(
            "MATH101".to_string(),
            std::iter::once("MATH102".to_string()).collect(),
        );
        let plan: HashSet<&str> = ["MATH102", "CS101"].into_iter().collect();
        assert_eq!(
            find_equivalent_in_plan("MATH101", &equivs, &plan),
            Some("MATH102")
        );
    }

    #[test]
    fn test_find_equivalent_in_plan_returns_none_when_absent() {
        let mut equivs: HashMap<String, HashSet<String>> = HashMap::new();
        equivs.insert(
            "MATH101".to_string(),
            std::iter::once("MATH102".to_string()).collect(),
        );
        let plan: HashSet<&str> = std::iter::once("CS101").collect();
        assert_eq!(find_equivalent_in_plan("MATH101", &equivs, &plan), None);
    }

    #[test]
    fn test_find_equivalent_in_plan_unknown_course_returns_none() {
        let equivs: HashMap<String, HashSet<String>> = HashMap::new();
        let plan: HashSet<&str> = std::iter::once("CS101").collect();
        assert_eq!(find_equivalent_in_plan("MATH101", &equivs, &plan), None);
    }
}
