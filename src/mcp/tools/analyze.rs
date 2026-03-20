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
    pub max_plans: Option<usize>,
}

/// Serializable metric statistics
#[derive(Debug, Serialize)]
pub struct MetricStatsJson {
    /// Minimum value
    pub min: f64,
    /// Maximum value
    pub max: f64,
    /// Mean value
    pub mean: f64,
    /// Median value
    pub median: f64,
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
#[must_use]
pub fn execute(yaml_content: &str, max_plans: Option<usize>) -> AnalysisResponse {
    let max = max_plans.unwrap_or(DEFAULT_MAX_PLANS);

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
        &aggregator,
        selector,
        plans_processed,
        max,
        &stats,
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
        let plan_dag = build_plan_dag(&expanded, ctx.graph);

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

/// Build the analysis response from aggregated results
fn build_response(
    program: &crate::core::DegreeProgram,
    aggregator: &MetricsAggregator,
    selector: PlanSelector<'_>,
    plans_processed: usize,
    max: usize,
    stats: &crate::core::degree::PlanGenerationStats,
) -> AnalysisResponse {
    let degree_stats = aggregator.degree_stats();
    let selected = selector.into_selected_plans();

    let selected_plans: Vec<PlanSummaryJson> = selected
        .iter()
        .map(|(cat, plan)| PlanSummaryJson {
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
#[must_use]
pub fn execute_json(yaml_content: &str, max_plans: Option<usize>) -> String {
    let response = execute(yaml_content, max_plans);
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
        max: s.max,
        mean: s.mean,
        median: s.median,
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

fn build_plan_dag(courses: &[String], graph: &CourseGraph) -> DAG {
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
                    if plan_set.contains(edge.prerequisite.as_str()) {
                        dag.add_prerequisite(key.clone(), &edge.prerequisite);
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
        let response = execute(TEST_YAML, Some(10));
        assert!(response.success, "error: {:?}", response.error);
        assert!(response.plans_analyzed > 0);
        assert!(response.complexity.is_some());
        assert!(response.total_credits.is_some());
        assert!(!response.selected_plans.is_empty());
    }

    #[test]
    fn test_analyze_malformed_yaml() {
        let response = execute("not: valid: yaml: {{", Some(10));
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_analyze_json_output() {
        let json = execute_json(TEST_YAML, Some(10));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["success"].as_bool().unwrap());
        assert!(parsed["plans_analyzed"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_selected_plans_have_schedules() {
        let response = execute(TEST_YAML, Some(10));
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
    fn test_placeholder_credits() {
        assert!((super::placeholder_credits("GE01") - 3.0).abs() < f32::EPSILON);
        assert!((super::placeholder_credits("GE01S") - 2.0).abs() < f32::EPSILON);
    }
}
