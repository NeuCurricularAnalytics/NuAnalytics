//! Per-course detail tool.
//!
//! Provides the `get_course_detail` MCP tool that returns everything an LLM
//! typically wants to know about a single course in a degree YAML: the
//! course header, direct + transitive prerequisites, dependents,
//! requirements that reference it, and (optionally) the analysis-derived
//! statistics + term placement in each curated selected plan.
//!
//! Static data is cheap (just parse + walk the course graph). Analysis data
//! requires `build_artifacts`, so we run it only when `include_analysis` is
//! true (default).

use crate::core::degree::audit::extract_course_level;
use crate::core::degree::{parse_degree_yaml, DegreeParseError};
use crate::core::models::CourseGraph;
use crate::core::DegreeProgram;
use crate::mcp::tools::analyze::{metric_stats_json, AnalysisArtifacts, MetricStatsJson};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for `get_course_detail`.
///
/// Provide exactly one YAML source plus the target `course_id`. When
/// `include_analysis` is true (default) the response is enriched with
/// per-course metric statistics and term placement in each selected plan;
/// set it to false to skip the analysis pipeline and get static-only data
/// in roughly an order of magnitude less time.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetCourseDetailRequest {
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

    /// Target course identifier (must match a key under `courses:` in the YAML).
    #[schemars(description = "Course key (e.g. \"CS165\") to inspect.")]
    pub course_id: String,

    /// When true (default), also run the analysis pipeline and include
    /// `analysis` with per-course metric medians + plan-level term placement.
    /// Set false to skip plan generation entirely.
    #[schemars(
        description = "Include analysis-derived stats + term placement per selected plan (default true). Set false to skip the analysis pass (~10x faster, static data only)."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub include_analysis: Option<bool>,

    /// Forwarded to `analyze_degree` when `include_analysis=true`. Default 500.
    #[schemars(description = "max_plans for the analysis pass (default 500)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub max_plans: Option<usize>,
}

/// Term placement of the course within one curated selected plan.
#[derive(Debug, Serialize)]
pub struct PlanPlacement {
    /// Plan category name (e.g. "Shortest Path", "Sample 1").
    pub category: String,
    /// 1-indexed term number where the course appears, or `None` when the
    /// course is not in this plan's schedule.
    pub term: Option<usize>,
}

/// Analysis-derived statistics for the course across all generated plans.
#[derive(Debug, Serialize)]
pub struct CourseAnalysis {
    /// Complexity contribution distribution.
    pub complexity: MetricStatsJson,
    /// Centrality (incoming + outgoing edge count) distribution.
    pub centrality: MetricStatsJson,
    /// Delay-factor distribution.
    pub delay: MetricStatsJson,
    /// Blocking-factor distribution (downstream impact).
    pub blocking: MetricStatsJson,
    /// Term placement in each entry of `selected_plans`.
    pub appears_in_selected_plans: Vec<PlanPlacement>,
}

/// Complete response for `get_course_detail`.
#[derive(Debug, Serialize)]
pub struct CourseDetailResponse {
    /// True when the YAML parsed and the requested course was found.
    pub success: bool,
    /// Error message when `success` is false (parse error or unknown course).
    pub error: Option<String>,

    /// Echoed back from the request so callers can correlate responses.
    pub course_id: String,
    /// Course title.
    pub title: Option<String>,
    /// Subject prefix (e.g. `"CS"`).
    pub prefix: Option<String>,
    /// Course number (e.g. `"165"`).
    pub number: Option<String>,
    /// Credit hours per the YAML.
    pub credits: f32,
    /// Detected course level (e.g. 100, 200, 1000).
    pub level: Option<u32>,
    /// Whether the course's subject is in `degree.major_subjects`. False when
    /// the degree omits `major_subjects` (case-insensitive prefix match).
    pub in_major_subjects: bool,

    /// Raw prerequisite expression copied verbatim from the YAML (if any).
    pub raw_prerequisites: Option<String>,
    /// Direct prerequisites — the immediate predecessors in the prerequisite graph.
    pub direct_prerequisites: Vec<String>,
    /// Transitive prerequisite branches: each entry is one branch of an
    /// AND-of-paths, listed leaf-to-immediate-prereq.
    pub transitive_prerequisites: Vec<Vec<String>>,
    /// Courses that list this one as a prerequisite (outgoing edges).
    pub dependents: Vec<String>,
    /// Cross-listed course keys (as declared by the course's `cross_listed_as`).
    pub cross_listed_as: Vec<String>,
    /// Requirement IDs that reference this course (directly or via a `from`
    /// clause / nested `one_of` option).
    pub requirements_referencing: Vec<String>,

    /// Analysis-derived data — only populated when `include_analysis=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analysis: Option<CourseAnalysis>,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `get_course_detail` tool.
#[must_use]
pub fn execute(
    yaml_content: &str,
    course_id: &str,
    include_analysis: bool,
    max_plans: Option<usize>,
) -> CourseDetailResponse {
    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => return error_response(course_id, format_parse_error(&e)),
    };

    if !program.courses.contains_key(course_id) {
        return error_response(
            course_id,
            format!("Course {course_id:?} is not defined in this degree's `courses:` map."),
        );
    }

    if include_analysis {
        match crate::mcp::cache::cached_artifacts(yaml_content, max_plans, None, None) {
            Ok(artifacts) => build_response_with_analysis(course_id, &artifacts),
            Err(e) => error_response(course_id, e),
        }
    } else {
        let mut graph_result = CourseGraph::from_degree_program(&program);
        if !graph_result.cycles.is_empty() {
            graph_result.graph.break_cycles(&graph_result.cycles);
            graph_result.cycles.clear();
        }
        populate_static_fields(course_id, &program, &graph_result.graph, None)
    }
}

/// Execute and serialize as JSON.
#[must_use]
pub fn execute_json(
    yaml_content: &str,
    course_id: &str,
    include_analysis: bool,
    max_plans: Option<usize>,
) -> String {
    let response = execute(yaml_content, course_id, include_analysis, max_plans);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

/// Populate every static field (parse + graph derived) on a response builder.
fn populate_static_fields(
    course_id: &str,
    program: &DegreeProgram,
    graph: &CourseGraph,
    analysis: Option<CourseAnalysis>,
) -> CourseDetailResponse {
    let course = program.courses.get(course_id);

    let title = course.map(|c| c.name.clone());
    let prefix = course.map(|c| c.prefix.clone());
    let number = course.map(|c| c.number.clone());
    let credits = course.map_or(0.0, |c| c.credit_hours);
    let raw_prerequisites = course.and_then(|c| c.prerequisites_raw.clone());
    let cross_listed_as = course
        .and_then(|c| c.cross_listed_as.clone())
        .unwrap_or_default();

    let level = extract_course_level(course_id);
    let in_major_subjects = is_major_subject(course_id, program);

    let (direct_prerequisites, dependents) = graph.get(course_id).map_or_else(
        || (Vec::new(), Vec::new()),
        |node| {
            let direct: Vec<String> = node
                .prerequisites
                .iter()
                .map(|edge| edge.prerequisite.clone())
                .collect();
            (direct, node.dependents.clone())
        },
    );

    let transitive_prerequisites = graph
        .structured_prerequisite_chain(course_id)
        .map(|chain| chain.branches)
        .unwrap_or_default();

    let requirements_referencing = find_requirements_referencing(program, course_id);

    CourseDetailResponse {
        success: true,
        error: None,
        course_id: course_id.to_string(),
        title,
        prefix,
        number,
        credits,
        level,
        in_major_subjects,
        raw_prerequisites,
        direct_prerequisites,
        transitive_prerequisites,
        dependents,
        cross_listed_as,
        requirements_referencing,
        analysis,
    }
}

/// Build the response from already-computed `AnalysisArtifacts`.
fn build_response_with_analysis(
    course_id: &str,
    artifacts: &AnalysisArtifacts,
) -> CourseDetailResponse {
    // The artifacts already broke cycles in their internal CourseGraph copy,
    // but the graph itself is owned inside `build_artifacts`. Re-derive the
    // graph from the parsed program here — cheap, milliseconds.
    let mut graph_result = CourseGraph::from_degree_program(&artifacts.program);
    if !graph_result.cycles.is_empty() {
        graph_result.graph.break_cycles(&graph_result.cycles);
        graph_result.cycles.clear();
    }
    let analysis = course_analysis(course_id, artifacts);
    populate_static_fields(
        course_id,
        &artifacts.program,
        &graph_result.graph,
        Some(analysis),
    )
}

/// Course analysis derived from the aggregated metrics + selected plans.
fn course_analysis(course_id: &str, artifacts: &AnalysisArtifacts) -> CourseAnalysis {
    let stats = artifacts.aggregator.course_stats(course_id);
    let appears_in_selected_plans: Vec<PlanPlacement> = artifacts
        .selected
        .iter()
        .map(|(cat, plan)| {
            let term = plan
                .schedule
                .terms
                .iter()
                .find(|t| t.courses.iter().any(|c| c == course_id))
                .map(|t| t.number);
            PlanPlacement {
                category: cat.display_name().to_string(),
                term,
            }
        })
        .collect();

    let (complexity, centrality, delay, blocking) = stats.map_or_else(
        || {
            (
                MetricStatsJson::default(),
                MetricStatsJson::default(),
                MetricStatsJson::default(),
                MetricStatsJson::default(),
            )
        },
        |s| {
            (
                metric_stats_json(&s.complexity),
                metric_stats_json(&s.centrality),
                metric_stats_json(&s.delay),
                metric_stats_json(&s.blocking),
            )
        },
    );

    CourseAnalysis {
        complexity,
        centrality,
        delay,
        blocking,
        appears_in_selected_plans,
    }
}

/// Return the requirement IDs whose recursively-collected course set
/// contains `course_id`. Walks the full requirement tree (including `from`
/// clauses and nested `one_of` options) via `audit::collect_from_requirement`.
fn find_requirements_referencing(program: &DegreeProgram, course_id: &str) -> Vec<String> {
    use crate::core::degree::audit::collect_from_requirement;
    use std::collections::HashSet;

    let mut hits: Vec<String> = program
        .requirements
        .iter()
        .filter_map(|(req_id, req)| {
            let mut courses: HashSet<String> = HashSet::new();
            collect_from_requirement(req, &mut courses);
            courses.contains(course_id).then(|| req_id.clone())
        })
        .collect();
    hits.sort();
    hits
}

/// Case-insensitive check that `course_id`'s prefix is in
/// `degree.major_subjects`. Returns false when no `major_subjects` is set.
fn is_major_subject(course_id: &str, program: &DegreeProgram) -> bool {
    let Some(subjects) = program.degree.major_subjects.as_ref() else {
        return false;
    };
    let digit_pos = course_id.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    if digit_pos == 0 {
        return false;
    }
    let prefix = &course_id[..digit_pos];
    subjects.iter().any(|s| s.eq_ignore_ascii_case(prefix))
}

fn error_response(course_id: &str, error: impl Into<String>) -> CourseDetailResponse {
    CourseDetailResponse {
        success: false,
        error: Some(error.into()),
        course_id: course_id.to_string(),
        title: None,
        prefix: None,
        number: None,
        credits: 0.0,
        level: None,
        in_major_subjects: false,
        raw_prerequisites: None,
        direct_prerequisites: Vec::new(),
        transitive_prerequisites: Vec::new(),
        dependents: Vec::new(),
        cross_listed_as: Vec::new(),
        requirements_referencing: Vec::new(),
        analysis: None,
    }
}

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError { message, .. } => format!("YAML syntax error: {message}"),
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
    name: Introduction
    type: all
    category: major
    courses: [CS101, CS201, CS301]

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
  CS301:
    title: Algorithms
    prefix: CS
    number: "301"
    credits: 4
    prerequisites_raw: "CS201"
"#;

    #[test]
    fn test_static_only_populates_prereqs_dependents_and_requirements() {
        let response = execute(TEST_YAML, "CS201", false, None);
        assert!(response.success, "error: {:?}", response.error);
        assert_eq!(response.title.as_deref(), Some("Data Structures"));
        assert!((response.credits - 4.0).abs() < f32::EPSILON);
        assert_eq!(response.level, Some(200));
        assert!(response.in_major_subjects);
        assert_eq!(response.direct_prerequisites, vec!["CS101".to_string()]);
        assert!(
            response.dependents.iter().any(|d| d == "CS301"),
            "CS301 depends on CS201"
        );
        assert!(
            response
                .requirements_referencing
                .iter()
                .any(|r| r == "intro"),
            "intro lists CS201"
        );
        // Static-only mode must skip the analysis section.
        assert!(response.analysis.is_none());
    }

    #[test]
    fn test_include_analysis_populates_term_placement_and_stats() {
        let response = execute(TEST_YAML, "CS201", true, Some(10));
        assert!(response.success);
        let analysis = response
            .analysis
            .as_ref()
            .expect("include_analysis=true must populate the analysis section");
        assert!(!analysis.appears_in_selected_plans.is_empty());
        // The Shortest Path must place CS201 in some term (>= 1).
        let shortest = analysis
            .appears_in_selected_plans
            .iter()
            .find(|p| p.category == "Shortest Path")
            .expect("shortest path must be among selected plans");
        assert!(shortest.term.is_some_and(|t| t >= 1));
    }

    #[test]
    fn test_unknown_course_returns_error_response() {
        let response = execute(TEST_YAML, "NOPE999", false, None);
        assert!(!response.success);
        let err = response.error.expect("error must be populated");
        assert!(err.contains("NOPE999"));
    }

    #[test]
    fn test_parse_error_yaml_surfaces_in_error_field() {
        let response = execute("not: valid: yaml: {{", "CS101", false, None);
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_transitive_prerequisites_walks_the_chain() {
        let response = execute(TEST_YAML, "CS301", false, None);
        assert!(response.success);
        // CS301 → CS201 → CS101: at least one branch must visit CS101.
        let has_cs101 = response
            .transitive_prerequisites
            .iter()
            .any(|branch| branch.iter().any(|c| c == "CS101"));
        assert!(
            has_cs101,
            "transitive prereqs of CS301 should reach CS101: {:?}",
            response.transitive_prerequisites
        );
    }

    #[test]
    fn test_cross_listed_field_is_empty_when_absent() {
        let response = execute(TEST_YAML, "CS101", false, None);
        assert!(response.cross_listed_as.is_empty());
    }

    #[test]
    fn test_in_major_subjects_handles_prefix_mismatch_and_case_insensitivity() {
        // major_subjects = ["cs"] (lowercase) should still match CS-prefixed
        // courses, and MATH-prefixed courses should not match.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 8
  gpa_minimum: 2.0
  major_subjects: ["cs"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, MATH101]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
  MATH101:
    title: Calc I
    prefix: MATH
    number: "101"
    credits: 4
"#;
        let cs = execute(yaml, "CS101", false, None);
        assert!(cs.success);
        assert!(
            cs.in_major_subjects,
            "CS101 should match major_subjects=[cs] case-insensitively"
        );

        let math = execute(yaml, "MATH101", false, None);
        assert!(math.success);
        assert!(
            !math.in_major_subjects,
            "MATH101 must not match major_subjects=[cs]"
        );
    }

    #[test]
    fn test_in_major_subjects_returns_false_when_major_subjects_absent() {
        // No major_subjects key in the degree → every course must report
        // in_major_subjects=false rather than defaulting to true.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 4
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
"#;
        let response = execute(yaml, "CS101", false, None);
        assert!(response.success);
        assert!(!response.in_major_subjects);
    }

    #[test]
    fn test_requirements_referencing_lists_every_match() {
        // CS101 appears in two requirements; both ids must come back.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101]
  advanced:
    name: Advanced
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
"#;
        let mut response = execute(yaml, "CS101", false, None);
        assert!(response.success);
        response.requirements_referencing.sort();
        assert_eq!(
            response.requirements_referencing,
            vec!["advanced".to_string(), "intro".to_string()]
        );
    }
}
