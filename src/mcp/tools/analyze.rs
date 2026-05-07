//! Degree analysis tool
//!
//! Provides the `analyze_degree` MCP tool that runs full degree analysis:
//! generates plans, computes aggregate metrics, and returns structured results.

use crate::core::degree::{
    parse_degree_yaml, DegreeParseError, PlanGenerator, PlanGeneratorConfig, PlanSelector,
    PlanSelectorConfig, PlanVariant, SamplingStrategy,
};
use crate::core::metrics::compute_all_metrics;
use crate::core::models::{Course, CourseGraph, School, DAG};
use crate::core::report::visualization::{spec_from_scored_plan, CurriculumGraphSpec};
use crate::core::report::SchedulerConfig;
use crate::core::statistics::{AggregatorConfig, MetricStats, MetricsAggregator};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `analyze_degree` tool
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AnalyzeDegreeRequest {
    /// The complete degree YAML content as a string
    #[schemars(description = "Complete degree program YAML content to analyze")]
    pub yaml_content: String,

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
    /// Each spec is ~30 KB; pass true only when you'll render the visualization.
    /// Pair with `get_curriculum_visualization` to render the returned spec to HTML.
    #[schemars(
        description = "Include full graph_spec per selected plan (default false). Each spec is ~30 KB; opt in only when rendering."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub include_graph_spec: Option<bool>,
}

/// Serializable metric statistics (includes quartiles for box plots)
#[derive(Debug, Serialize)]
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

    /// Aggregate complexity statistics across all plans
    pub complexity: Option<MetricStatsJson>,
    /// Aggregate longest delay statistics
    pub longest_delay: Option<MetricStatsJson>,
    /// Aggregate total credits statistics
    pub total_credits: Option<MetricStatsJson>,

    /// Selected special plans
    pub selected_plans: Vec<PlanSummaryJson>,
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
#[must_use]
pub fn execute(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<Vec<String>>,
    include_graph_spec: bool,
) -> AnalysisResponse {
    let max = max_plans.unwrap_or(DEFAULT_MAX_PLANS);
    let include = include_courses.unwrap_or_default();

    // Parse YAML
    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => {
            return AnalysisResponse {
                success: false,
                error: Some(format_parse_error(&e)),
                degree_name: None,
                institution: None,
                total_courses: 0,
                total_requirements: 0,
                plans_analyzed: 0,
                was_truncated: false,
                complexity: None,
                longest_delay: None,
                total_credits: None,
                selected_plans: vec![],
            };
        }
    };

    // Build school and graph
    let school = build_school(&program);
    let graph_result = CourseGraph::from_degree_program(&program);

    // Handle cycles
    let mut graph_result = graph_result;
    if !graph_result.cycles.is_empty() {
        graph_result.graph.break_cycles(&graph_result.cycles);
        graph_result.cycles.clear();
    }

    let dag = build_dag(&graph_result.graph);
    let equivalences = build_equivalences(&program.requirements);

    // Configure and run plan generation
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
    let mut selector = PlanSelector::new(&school, &dag, selector_config);

    let ctx = AnalysisCtx {
        graph: &graph_result.graph,
        equivalences: &equivalences,
        school: &school,
        target_credits: program.degree.total_credits,
    };

    let plans_processed = run_plan_analysis(
        &generator,
        &gen_config,
        &ctx,
        max,
        &mut aggregator,
        &mut selector,
    );

    build_response(
        &program,
        &school,
        &equivalences,
        &aggregator,
        selector,
        plans_processed,
        max,
        &stats,
        include_graph_spec,
    )
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

/// Build the analysis response from aggregated results.
///
/// The function has 9 parameters because it synthesises data from every stage
/// of the analysis pipeline; grouping them into a context struct would just
/// move the same data without reducing coupling.
#[allow(clippy::too_many_arguments)]
fn build_response(
    program: &crate::core::DegreeProgram,
    school: &School,
    equivalences: &HashMap<String, HashSet<String>>,
    aggregator: &MetricsAggregator,
    selector: PlanSelector<'_>,
    plans_processed: usize,
    max: usize,
    stats: &crate::core::degree::PlanGenerationStats,
    include_graph_spec: bool,
) -> AnalysisResponse {
    let degree_stats = aggregator.degree_stats();
    let selected = selector.into_selected_plans();

    let selected_plans: Vec<PlanSummaryJson> = selected
        .iter()
        .map(|(cat, plan)| {
            let graph_spec = if include_graph_spec {
                let graph_id = cat.display_name().to_lowercase().replace(' ', "-");
                Some(spec_from_scored_plan(
                    school,
                    equivalences,
                    plan,
                    Some(aggregator),
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

    AnalysisResponse {
        success: true,
        error: None,
        degree_name: Some(program.degree.name.clone()),
        institution: program.degree.institution.clone(),
        total_courses: program.courses.len(),
        total_requirements: program.requirements.len(),
        plans_analyzed: plans_processed,
        was_truncated: plans_processed >= max && stats.total_possible > max,
        complexity: Some(metric_stats_json(&degree_stats.total_complexity)),
        longest_delay: Some(metric_stats_json(&degree_stats.longest_delay)),
        total_credits: Some(metric_stats_json(&degree_stats.total_credits)),
        selected_plans,
    }
}

/// Execute and serialize as JSON
///
/// # Arguments
/// * `yaml_content` - The degree program YAML content
/// * `max_plans` - Maximum number of plans to generate
/// * `include_courses` - Optional courses to always include in all plans
/// * `include_graph_spec` - When true, include `graph_spec` per selected plan
#[must_use]
pub fn execute_json(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<Vec<String>>,
    include_graph_spec: bool,
) -> String {
    let response = execute(yaml_content, max_plans, include_courses, include_graph_spec);
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

const fn metric_stats_json(s: &MetricStats) -> MetricStatsJson {
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
        let response = execute(TEST_YAML, Some(10), None, false);
        assert!(response.success, "error: {:?}", response.error);
        assert!(response.plans_analyzed > 0);
        assert!(response.complexity.is_some());
        assert!(response.total_credits.is_some());
        assert!(!response.selected_plans.is_empty());
    }

    #[test]
    fn test_analyze_malformed_yaml() {
        let response = execute("not: valid: yaml: {{", Some(10), None, false);
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_analyze_json_output() {
        let json = execute_json(TEST_YAML, Some(10), None, false);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["success"].as_bool().unwrap());
        assert!(parsed["plans_analyzed"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_selected_plans_have_schedules() {
        let response = execute(TEST_YAML, Some(10), None, false);
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
        let response = execute(TEST_YAML, Some(10), Some(vec!["CS101".to_string()]), false);
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
        let response = execute(TEST_YAML, Some(10), None, false);
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
            serde_json::from_str(&execute_json(TEST_YAML, Some(10), None, false)).unwrap();
        for plan in json["selected_plans"].as_array().unwrap() {
            assert!(
                plan.get("graph_spec").is_none(),
                "graph_spec key must not appear in JSON when include_graph_spec=false"
            );
        }
    }

    #[test]
    fn test_analyze_includes_graph_spec_when_requested() {
        let response = execute(TEST_YAML, Some(10), None, true);
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
