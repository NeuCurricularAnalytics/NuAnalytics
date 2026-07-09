//! Canonical data format for curriculum graph visualization.
//!
//! [`CurriculumGraphSpec`] is the fully-computed, serializable description of
//! everything a renderer needs to draw a curriculum graph.  It is constructed
//! from already-computed pipeline outputs (metrics, term plan, DAG) — none of
//! its fields come directly from the degree YAML.
//!
//! Two builder paths are provided:
//! - [`spec_from_components`] — primary builder; accepts raw computed pieces.
//! - [`spec_from_report_context`] — convenience wrapper for the CLI single-plan
//!   HTML report path (`ReportContext` already aggregates all computed data).
//! - [`spec_from_scored_plan`] — convenience wrapper for degree reports and the
//!   `analyze_degree` MCP tool (`ScoredPlan` carries its own metrics & schedule).

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

use crate::core::metrics::{CourseMetrics, CurriculumMetrics};
use crate::core::models::{School, DAG};
use crate::core::report::term_scheduler::{course_credits_with_fallback, TermPlan};
use crate::core::report::ReportContext;
use crate::core::statistics::aggregator::MetricsAggregator;

// ============================================================================
// Public types
// ============================================================================

/// Whether a graph edge is a hard prerequisite or a corequisite.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EdgeType {
    /// Must be taken before the destination course.
    Prerequisite,
    /// May be taken concurrently with the destination course.
    Corequisite,
}

/// A course node in the visualization graph.
///
/// The per-plan metrics (`complexity`, `delay`, `blocking`) describe this
/// course's position in *this* plan only.  The optional `median_*` fields
/// carry the cross-plan median for the same metric, populated when the spec
/// is built with a [`MetricsAggregator`]; they are `None` for single-plan
/// reports where no aggregator exists.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseNode {
    /// Unique course identifier, e.g. `"CS2500"`.
    pub id: String,
    /// Human-readable course name, e.g. `"Fundamentals of CS 1"`.
    pub name: String,
    /// Credit hours.
    pub credits: f32,
    /// Structural complexity score (delay + blocking). Computed by the analysis
    /// pipeline; not present in the source YAML.
    pub complexity: usize,
    /// Delay metric: longest path from this course to any leaf.
    #[serde(default)]
    pub delay: usize,
    /// Blocking metric: number of downstream courses gated by this one.
    #[serde(default)]
    pub blocking: usize,
    /// Whether this course lies on the longest-delay (critical) path.
    pub on_critical_path: bool,
    /// 1-indexed term number this course is scheduled into.
    pub term: usize,
    /// Median complexity across all plans analysed; `None` when no aggregator
    /// was available.
    #[serde(default)]
    pub median_complexity: Option<f32>,
    /// Median delay across all plans analysed; `None` when no aggregator was
    /// available.
    #[serde(default)]
    pub median_delay: Option<f32>,
    /// Median blocking across all plans analysed; `None` when no aggregator
    /// was available.
    #[serde(default)]
    pub median_blocking: Option<f32>,
}

/// A directed edge between two courses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphEdge {
    /// Source course ID (the prerequisite or co-taken course).
    pub from: String,
    /// Destination course ID.
    pub to: String,
    /// Relationship type.
    pub edge_type: EdgeType,
}

/// One term column in the visualization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TermGroup {
    /// 1-indexed term number.
    pub number: usize,
    /// Ordered list of course IDs in this term.
    pub course_ids: Vec<String>,
}

/// Complete, self-describing specification for a curriculum graph visualization.
///
/// This struct is the "intermediate format" that travels from the analysis
/// pipeline to the renderer.  It is fully serializable to JSON — the
/// `analyze_degree` MCP tool embeds one per selected plan in its response, and
/// the `get_curriculum_visualization` tool accepts one as input and returns HTML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurriculumGraphSpec {
    /// Unique identifier for this graph instance; used as a DOM element ID
    /// prefix.  Use `"main"` for single-plan reports; a kebab-case category
    /// name (e.g., `"shortest-path"`) for degree reports.
    pub graph_id: String,
    /// All course nodes, in display order (follows term order, then term slot
    /// order within each term).
    pub nodes: Vec<CourseNode>,
    /// All edges (prerequisite and corequisite).
    pub edges: Vec<GraphEdge>,
    /// Terms in display order.
    pub terms: Vec<TermGroup>,
    /// IDs of courses on the critical (longest-delay) path.
    pub critical_path_ids: Vec<String>,
}

// ============================================================================
// Primary builder
// ============================================================================

/// Build a [`CurriculumGraphSpec`] from already-computed pipeline outputs.
///
/// This is the core builder; all other builders delegate to it.
///
/// * `school` — provides course names and credit hours.
/// * `dag` — provides resolved prerequisite / corequisite edges (used when
///   edge data is not already available from a `ScoredPlan`).
/// * `term_plan` — provides term assignments for each course.
/// * `metrics` — per-course metrics for *this* plan (delay, blocking,
///   complexity), computed by [`crate::core::metrics::compute_all_metrics`].
/// * `critical_path` — ordered list of course IDs on the longest-delay path.
/// * `aggregator` — optional cross-plan aggregator. When `Some`, populates
///   each node's `median_*` fields with the corresponding course's median
///   across all analysed plans; when `None`, those fields stay `None`
///   (used by the CLI single-plan path).
/// * `graph_id` — DOM ID prefix; `"main"` for single-plan reports.
#[must_use]
pub fn spec_from_components(
    school: &School,
    dag: &DAG,
    term_plan: &TermPlan,
    metrics: &CurriculumMetrics,
    critical_path: &[String],
    aggregator: Option<&MetricsAggregator>,
    graph_id: &str,
) -> CurriculumGraphSpec {
    let critical_path_ids = expand_critical_path(critical_path);
    let critical_set: HashSet<&str> = critical_path_ids.iter().map(String::as_str).collect();

    // Collect all course IDs in the plan (from all terms).
    let plan_courses: HashSet<&str> = term_plan
        .terms
        .iter()
        .flat_map(|t| t.courses.iter())
        .map(String::as_str)
        .collect();

    let (nodes, terms) =
        build_nodes_and_terms(school, term_plan, metrics, &critical_set, aggregator);

    // Build edges from DAG, filtered to courses in the plan.
    let mut edges = Vec::new();

    for (course, prereqs) in &dag.dependencies {
        if !plan_courses.contains(course.as_str()) {
            continue;
        }
        for prereq in prereqs {
            if plan_courses.contains(prereq.as_str()) {
                edges.push(GraphEdge {
                    from: prereq.clone(),
                    to: course.clone(),
                    edge_type: EdgeType::Prerequisite,
                });
            }
        }
    }

    for (course, coreqs) in &dag.corequisites {
        if !plan_courses.contains(course.as_str()) {
            continue;
        }
        for coreq in coreqs {
            if plan_courses.contains(coreq.as_str()) {
                edges.push(GraphEdge {
                    from: coreq.clone(),
                    to: course.clone(),
                    edge_type: EdgeType::Corequisite,
                });
            }
        }
    }

    CurriculumGraphSpec {
        graph_id: graph_id.to_string(),
        nodes,
        edges,
        terms,
        critical_path_ids,
    }
}

// ============================================================================
// Convenience wrappers
// ============================================================================

/// Build a [`CurriculumGraphSpec`] from a single-plan [`ReportContext`].
///
/// Used by the CLI HTML report generator (`html.rs`), where all computed data
/// is already bundled in the context.  No aggregator is available on this
/// path, so the node `median_*` fields will all be `None`.
#[must_use]
pub fn spec_from_report_context(ctx: &ReportContext, graph_id: &str) -> CurriculumGraphSpec {
    spec_from_components(
        ctx.school,
        ctx.dag,
        ctx.term_plan,
        ctx.metrics,
        &ctx.summary.longest_delay_path,
        None,
        graph_id,
    )
}

/// Build a [`CurriculumGraphSpec`] from a `ScoredPlan`
/// (`crate::core::degree::plan_selector::ScoredPlan`).
///
/// Used by the degree-report HTML generator and the `analyze_degree` MCP tool.
/// The `ScoredPlan` already carries its own `schedule`, `course_metrics`, and
/// `score.longest_delay_chain`, so no re-computation is needed.
///
/// When `aggregator` is `Some`, each node's `median_*` fields are populated
/// from the per-course aggregated stats; pass `None` if cross-plan medians
/// should be omitted from the spec.
///
/// Edge data is re-derived from course prerequisites with equivalence
/// resolution (the same logic as the former `build_plan_edges` in
/// `degree_report.rs`).
/// All callers use the default hasher; generalising over `BuildHasher` would
/// add noise to every call site for no practical benefit.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn spec_from_scored_plan(
    school: &School,
    equivalences: &HashMap<String, HashSet<String>>,
    plan: &crate::core::degree::ScoredPlan,
    aggregator: Option<&MetricsAggregator>,
    graph_id: &str,
) -> CurriculumGraphSpec {
    let critical_path_ids = plan.score.longest_delay_chain.clone();
    let critical_set: HashSet<&str> = critical_path_ids.iter().map(String::as_str).collect();

    let plan_courses: HashSet<&str> = plan.variant.courses.iter().map(String::as_str).collect();

    let (nodes, terms) = build_nodes_and_terms(
        school,
        &plan.schedule,
        &plan.course_metrics,
        &critical_set,
        aggregator,
    );

    // Build edges via prerequisite resolution with equivalence awareness.
    let edges = build_edges_from_courses(school, equivalences, &plan_courses);

    CurriculumGraphSpec {
        graph_id: graph_id.to_string(),
        nodes,
        edges,
        terms,
        critical_path_ids,
    }
}

/// Build the per-term [`CourseNode`] and [`TermGroup`] sequences for a spec.
///
/// Shared between [`spec_from_components`] and [`spec_from_scored_plan`].
/// Both pass the same `metrics` shape (`HashMap<String, CourseMetrics>`),
/// since [`CurriculumMetrics`] is just a type alias for that map.
fn build_nodes_and_terms(
    school: &School,
    term_plan: &TermPlan,
    metrics: &HashMap<String, CourseMetrics>,
    critical_set: &HashSet<&str>,
    aggregator: Option<&MetricsAggregator>,
) -> (Vec<CourseNode>, Vec<TermGroup>) {
    let mut nodes = Vec::new();
    let mut terms = Vec::new();

    for term in &term_plan.terms {
        if term.courses.is_empty() {
            continue;
        }
        let mut group = TermGroup {
            number: term.number,
            course_ids: Vec::new(),
        };
        for course_key in &term.courses {
            nodes.push(build_course_node(
                school,
                course_key,
                term.number,
                metrics.get(course_key),
                critical_set.contains(course_key.as_str()),
                aggregator,
            ));
            group.course_ids.push(course_key.clone());
        }
        terms.push(group);
    }

    (nodes, terms)
}

/// Construct a single [`CourseNode`] from already-looked-up pieces.
fn build_course_node(
    school: &School,
    course_key: &str,
    term_number: usize,
    course_metric: Option<&CourseMetrics>,
    on_critical_path: bool,
    aggregator: Option<&MetricsAggregator>,
) -> CourseNode {
    let name = school
        .get_course(course_key)
        .map_or_else(|| course_key.to_string(), |c| c.name.clone());
    let credits = course_credits_with_fallback(school, course_key);
    let complexity = course_metric.map_or(0, |m| m.complexity);
    let delay = course_metric.map_or(0, |m| m.delay);
    let blocking = course_metric.map_or(0, |m| m.blocking);
    let (median_complexity, median_delay, median_blocking) =
        aggregator_medians(aggregator, course_key);

    CourseNode {
        id: course_key.to_string(),
        name,
        credits,
        complexity,
        delay,
        blocking,
        on_critical_path,
        term: term_number,
        median_complexity,
        median_delay,
        median_blocking,
    }
}

/// Look up cross-plan medians for one course from the aggregator.
///
/// Returns `(None, None, None)` when no aggregator is provided or when the
/// course has no aggregated stats (it never appeared in any analysed plan).
fn aggregator_medians(
    aggregator: Option<&MetricsAggregator>,
    course_id: &str,
) -> (Option<f32>, Option<f32>, Option<f32>) {
    let Some(agg) = aggregator else {
        return (None, None, None);
    };
    let Some(stats) = agg.course_stats(course_id) else {
        return (None, None, None);
    };
    #[allow(clippy::cast_possible_truncation)]
    (
        Some(stats.complexity.median as f32),
        Some(stats.delay.median as f32),
        Some(stats.blocking.median as f32),
    )
}

// ============================================================================
// Internal helpers
// ============================================================================

/// Expand the critical-path list, splitting grouped corequisite entries like
/// `"(CS1321+CS1321L)"` into individual course IDs.
fn expand_critical_path(path: &[String]) -> Vec<String> {
    let mut result = Vec::new();
    for entry in path {
        let trimmed = entry.trim();
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            let inner = &trimmed[1..trimmed.len() - 1];
            for id in inner.split('+') {
                result.push(id.trim().to_string());
            }
        } else {
            result.push(trimmed.to_string());
        }
    }
    result
}

/// Build edges for a plan by re-parsing course prerequisites and resolving
/// equivalences.  Mirrors the logic of the former `build_plan_edges` in
/// `degree_report.rs`.
fn build_edges_from_courses(
    school: &School,
    equivalences: &HashMap<String, HashSet<String>>,
    plan_courses: &HashSet<&str>,
) -> Vec<GraphEdge> {
    use crate::core::prerequisite_parser::parse_to_dnf;

    let mut edges = Vec::new();

    for &course_key in plan_courses {
        let Some(course) = school.get_course(course_key) else {
            continue;
        };

        // Resolve prerequisite edges via DNF path selection.
        let prereq_raw = course.prerequisites_raw.clone().unwrap_or_else(|| {
            if course.prerequisites.is_empty() {
                String::new()
            } else {
                course.prerequisites.join(" & ")
            }
        });

        if !prereq_raw.is_empty() {
            let dnf_paths = parse_to_dnf(&prereq_raw);
            let selected = select_best_prereq_path(&dnf_paths, plan_courses, equivalences);
            for prereq in selected {
                edges.push(GraphEdge {
                    from: prereq,
                    to: course_key.to_string(),
                    edge_type: EdgeType::Prerequisite,
                });
            }
        }

        // Corequisite edges.
        for coreq in &course.corequisites {
            if plan_courses.contains(coreq.as_str()) {
                edges.push(GraphEdge {
                    from: coreq.clone(),
                    to: course_key.to_string(),
                    edge_type: EdgeType::Corequisite,
                });
            }
        }
    }

    edges
}

/// Choose the best prerequisite path from a DNF expression.
///
/// Prefers a complete path (all prereqs in the plan), then falls back to the
/// longest partial match.  Resolves each prerequisite through equivalences when
/// the direct course is not in the plan.
fn select_best_prereq_path<'a>(
    dnf_paths: &'a [Vec<String>],
    plan_courses: &HashSet<&str>,
    equivalences: &HashMap<String, HashSet<String>>,
) -> Vec<String> {
    let resolve = |p: &'a String| -> Option<String> {
        if plan_courses.contains(p.as_str()) {
            return Some(p.clone());
        }
        equivalences
            .get(p)
            .and_then(|eq| eq.iter().find(|e| plan_courses.contains(e.as_str())))
            .cloned()
    };

    // First pass: find a fully-satisfied path.
    for path in dnf_paths {
        let resolved: Vec<String> = path.iter().filter_map(resolve).collect();
        if resolved.len() == path.len() {
            return resolved;
        }
    }

    // Second pass: longest partial match.
    dnf_paths
        .iter()
        .map(|path| path.iter().filter_map(resolve).collect::<Vec<_>>())
        .max_by_key(Vec::len)
        .unwrap_or_default()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_expand_critical_path_plain() {
        let path = vec!["CS101".to_string(), "CS201".to_string()];
        assert_eq!(expand_critical_path(&path), vec!["CS101", "CS201"]);
    }

    #[test]
    fn test_expand_critical_path_grouped() {
        let path = vec!["CS101".to_string(), "(CS101L+CS102)".to_string()];
        let expanded = expand_critical_path(&path);
        assert_eq!(expanded, vec!["CS101", "CS101L", "CS102"]);
    }

    #[test]
    fn test_expand_critical_path_empty() {
        assert!(expand_critical_path(&[]).is_empty());
    }

    #[test]
    fn test_spec_from_components_basic() {
        use crate::core::metrics::CourseMetrics;
        use crate::core::models::{Course, DAG};
        use crate::core::report::term_scheduler::{Term, TermPlan};

        let mut school = School::new("Test".to_string());
        let mut c1 = Course::new(
            "CS101".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        );
        c1.prerequisites = vec![];
        let mut c2 = Course::new(
            "CS201".to_string(),
            "CS".to_string(),
            "201".to_string(),
            4.0,
        );
        c2.prerequisites = vec!["CS101".to_string()];
        school.add_course(c1);
        school.add_course(c2);

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());
        dag.add_course("CS201".to_string());
        dag.add_prerequisite("CS201".to_string(), "CS101");

        let term_plan = TermPlan {
            terms: vec![
                Term {
                    number: 1,
                    courses: vec!["CS101".to_string()],
                    total_credits: 4.0,
                },
                Term {
                    number: 2,
                    courses: vec!["CS201".to_string()],
                    total_credits: 4.0,
                },
            ],
            is_quarter_system: false,
            target_credits: 15.0,
            unscheduled: vec![],
        };

        let mut metrics = CurriculumMetrics::new();
        metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                delay: 1,
                blocking: 1,
                complexity: 2,
                centrality: 1,
                chain_length: 1,
            },
        );
        metrics.insert(
            "CS201".to_string(),
            CourseMetrics {
                delay: 2,
                blocking: 0,
                complexity: 2,
                centrality: 0,
                chain_length: 2,
            },
        );

        let spec = spec_from_components(
            &school,
            &dag,
            &term_plan,
            &metrics,
            &["CS101".to_string(), "CS201".to_string()],
            None,
            "test",
        );

        assert_eq!(spec.graph_id, "test");
        assert_eq!(spec.nodes.len(), 2);
        assert_eq!(spec.terms.len(), 2);
        assert_eq!(spec.edges.len(), 1);
        assert_eq!(spec.edges[0].from, "CS101");
        assert_eq!(spec.edges[0].to, "CS201");
        assert_eq!(spec.edges[0].edge_type, EdgeType::Prerequisite);
        let cs101 = spec.nodes.iter().find(|n| n.id == "CS101").unwrap();
        assert!(cs101.on_critical_path);
        assert_eq!(cs101.delay, 1);
        assert_eq!(cs101.blocking, 1);
        assert!(cs101.median_complexity.is_none());
        assert_eq!(spec.critical_path_ids, vec!["CS101", "CS201"]);
    }

    #[test]
    fn test_spec_from_components_populates_medians_when_aggregator_present() {
        use crate::core::metrics::CourseMetrics;
        use crate::core::models::{Course, DAG};
        use crate::core::report::term_scheduler::{Term, TermPlan};
        use crate::core::statistics::aggregator::{AggregatorConfig, MetricsAggregator};

        let mut school = School::new("Test".to_string());
        school.add_course(Course::new(
            "CS101".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        ));

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());

        let term_plan = TermPlan {
            terms: vec![Term {
                number: 1,
                courses: vec!["CS101".to_string()],
                total_credits: 4.0,
            }],
            is_quarter_system: false,
            target_credits: 15.0,
            unscheduled: vec![],
        };

        let mut metrics = CurriculumMetrics::new();
        metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                delay: 3,
                blocking: 5,
                complexity: 8,
                centrality: 1,
                chain_length: 2,
            },
        );

        // Feed two plans into the aggregator so median is well-defined.
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
        agg.add_plan(&metrics, 60.0);
        agg.add_plan(&metrics, 60.0);

        let spec =
            spec_from_components(&school, &dag, &term_plan, &metrics, &[], Some(&agg), "agg");

        let cs101 = spec.nodes.iter().find(|n| n.id == "CS101").unwrap();
        assert_eq!(cs101.median_complexity, Some(8.0));
        assert_eq!(cs101.median_delay, Some(3.0));
        assert_eq!(cs101.median_blocking, Some(5.0));
    }

    #[test]
    fn test_spec_from_report_context_basic() {
        use crate::core::metrics::CourseMetrics;
        use crate::core::metrics_export::CurriculumSummary;
        use crate::core::models::{Course, Degree, Plan, DAG};
        use crate::core::report::term_scheduler::{Term, TermPlan};
        use crate::core::report::ReportContext;

        let mut school = School::new("Test".to_string());
        let c = Course::new(
            "CS101".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        );
        school.add_course(c);

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());

        let term_plan = TermPlan {
            terms: vec![Term {
                number: 1,
                courses: vec!["CS101".to_string()],
                total_credits: 4.0,
            }],
            is_quarter_system: false,
            target_credits: 15.0,
            unscheduled: vec![],
        };

        let mut metrics = CurriculumMetrics::new();
        metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                delay: 1,
                blocking: 0,
                complexity: 1,
                centrality: 0,
                chain_length: 1,
            },
        );

        let summary = CurriculumSummary {
            total_complexity: 1,
            highest_centrality: 0,
            highest_centrality_course: "CS101".to_string(),
            longest_delay: 1,
            longest_delay_course: "CS101".to_string(),
            longest_delay_path: vec!["CS101".to_string()],
        };

        let degree = Degree::new(
            "Test".to_string(),
            "BS".to_string(),
            None,
            "semester".to_string(),
        );
        let mut plan = Plan::new("Plan".to_string(), degree.degree_id());
        plan.add_course("CS101".to_string());

        let ctx = ReportContext::new(
            &school,
            &plan,
            Some(&degree),
            &metrics,
            &summary,
            &dag,
            &term_plan,
        );

        let spec = spec_from_report_context(&ctx, "main");
        assert_eq!(spec.graph_id, "main");
        assert_eq!(spec.nodes.len(), 1);
        assert_eq!(spec.nodes[0].id, "CS101");
        assert!(spec.nodes[0].on_critical_path);
        assert_eq!(spec.terms.len(), 1);
    }

    #[test]
    fn test_select_best_prereq_path_full_match() {
        let dnf = vec![
            vec!["CS101".to_string(), "CS102".to_string()],
            vec!["CS101".to_string()],
        ];
        let plan: HashSet<&str> = ["CS101", "CS102"].iter().copied().collect();
        let result = select_best_prereq_path(&dnf, &plan, &HashMap::new());
        // First path is fully satisfied
        assert_eq!(result.len(), 2);
    }

    #[test]
    fn test_select_best_prereq_path_partial_match() {
        let dnf = vec![
            vec!["CS101".to_string(), "CS102".to_string()],
            vec!["CS103".to_string()],
        ];
        // Only CS101 in plan — partial match for first path (1 of 2)
        // CS103 not in plan — 0 of 1 for second path
        let plan: HashSet<&str> = std::iter::once("CS101").collect();
        let result = select_best_prereq_path(&dnf, &plan, &HashMap::new());
        assert_eq!(result, vec!["CS101"]);
    }

    #[test]
    fn test_select_best_prereq_path_with_equivalences() {
        let dnf = vec![vec!["CS101".to_string()]];
        let plan: HashSet<&str> = std::iter::once("CS101ALT").collect();
        let mut equivs: HashMap<String, HashSet<String>> = HashMap::new();
        let mut s = HashSet::new();
        s.insert("CS101ALT".to_string());
        equivs.insert("CS101".to_string(), s);
        let result = select_best_prereq_path(&dnf, &plan, &equivs);
        assert_eq!(result, vec!["CS101ALT"]);
    }

    #[test]
    fn test_build_edges_corequisite() {
        use crate::core::models::Course;

        let mut school = School::new("T".to_string());
        let c1 = Course::new(
            "CS101".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        );
        let mut c2 = Course::new(
            "CS101L".to_string(),
            "CS".to_string(),
            "101L".to_string(),
            1.0,
        );
        c2.corequisites = vec!["CS101".to_string()];
        school.add_course(c1);
        school.add_course(c2);

        let plan: HashSet<&str> = ["CS101", "CS101L"].iter().copied().collect();
        let edges = build_edges_from_courses(&school, &HashMap::new(), &plan);

        let coreq_edges: Vec<_> = edges
            .iter()
            .filter(|e| e.edge_type == EdgeType::Corequisite)
            .collect();
        assert_eq!(coreq_edges.len(), 1);
        assert_eq!(coreq_edges[0].from, "CS101");
        assert_eq!(coreq_edges[0].to, "CS101L");
    }

    #[test]
    fn test_aggregator_medians_returns_none_for_unseen_course() {
        use crate::core::statistics::aggregator::{AggregatorConfig, MetricsAggregator};
        let agg = MetricsAggregator::new(AggregatorConfig::default());
        // Aggregator present but course never observed → all None.
        assert_eq!(
            aggregator_medians(Some(&agg), "GHOST101"),
            (None, None, None)
        );
        // No aggregator at all → also all None.
        assert_eq!(aggregator_medians(None, "ANY"), (None, None, None));
    }

    #[test]
    fn test_spec_from_scored_plan_threads_aggregator() {
        use crate::core::degree::plan_selector::{PlanScore, ScoredPlan};
        use crate::core::degree::plan_variant::PlanVariant;
        use crate::core::metrics::CourseMetrics;
        use crate::core::models::Course;
        use crate::core::report::term_scheduler::{Term, TermPlan};
        use crate::core::statistics::aggregator::{AggregatorConfig, MetricsAggregator};

        let mut school = School::new("T".to_string());
        school.add_course(Course::new(
            "CS101".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        ));

        let mut course_metrics = HashMap::new();
        course_metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                delay: 2,
                blocking: 4,
                complexity: 6,
                centrality: 0,
                chain_length: 1,
            },
        );

        let mut schedule = TermPlan::new(1, false, 15.0);
        schedule.terms = vec![Term {
            number: 1,
            courses: vec!["CS101".to_string()],
            total_credits: 4.0,
        }];

        let plan = ScoredPlan {
            variant: PlanVariant::from_parts(vec!["CS101".to_string()], HashMap::new(), 4.0),
            score: PlanScore {
                terms_required: 1,
                total_complexity: 6,
                longest_delay: 2,
                longest_delay_chain: vec!["CS101".to_string()],
                is_calc_ready: false,
                avg_chain_length: 1.0,
            },
            schedule,
            course_metrics: course_metrics.clone(),
        };

        // Without an aggregator, medians stay None.
        let spec_no_agg = spec_from_scored_plan(&school, &HashMap::new(), &plan, None, "no-agg");
        assert!(spec_no_agg.nodes[0].median_complexity.is_none());

        // With an aggregator that has seen this course, medians populate.
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
        agg.add_plan(&course_metrics, 60.0);
        let spec_agg = spec_from_scored_plan(&school, &HashMap::new(), &plan, Some(&agg), "agg");
        assert_eq!(spec_agg.nodes[0].median_complexity, Some(6.0));
        assert_eq!(spec_agg.nodes[0].median_delay, Some(2.0));
        assert_eq!(spec_agg.nodes[0].median_blocking, Some(4.0));
    }
}
