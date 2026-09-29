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
use crate::core::report::report_stats::ReportStats;
use crate::core::report::term_scheduler::{course_credits_with_fallback, TermPlan};
use crate::core::report::ReportContext;

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
/// is built with [`ReportStats`] — from a live aggregator or a stored run; they
/// are `None` for single-plan reports where neither exists.
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
    aggregator: Option<&ReportStats>,
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

    edges.extend(build_edges_from_dag(dag, &plan_courses));

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
    aggregator: Option<&ReportStats>,
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

    // Term placement decides *which* option of an OR-group gets the edge when several
    // are in the plan. Without it the first-listed option wins even when it is scheduled
    // alongside the dependent and another option sits an earlier term back, drawing an
    // edge the schedule appears to violate. Placement itself is untouched: `nodes` and
    // `terms` above come from the schedule and never consult `edges`.
    let term_of = term_index(&plan.schedule);
    let edges = build_edges_from_courses(school, equivalences, &plan_courses, &term_of);

    CurriculumGraphSpec {
        graph_id: graph_id.to_string(),
        nodes,
        edges,
        terms,
        critical_path_ids,
    }
}

/// Map each scheduled course to its term number.
fn term_index(schedule: &crate::core::report::term_scheduler::TermPlan) -> HashMap<String, usize> {
    let mut index = HashMap::new();
    for term in &schedule.terms {
        for course in &term.courses {
            // First placement wins; a course should appear once, and if it somehow
            // appears twice the earlier term is the one a dependent must clear.
            index.entry(course.clone()).or_insert(term.number);
        }
    }
    index
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
    aggregator: Option<&ReportStats>,
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
    aggregator: Option<&ReportStats>,
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
    aggregator: Option<&ReportStats>,
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

/// Build edges from an already-computed `DAG`, keeping only courses in the plan.
///
/// Targets are walked in sorted order because `DAG::dependencies` and
/// `DAG::corequisites` are `HashMap`s: an unordered walk emits the same edges in a
/// different order on every run, and this list is serialised into the report. The inner
/// `Vec`s are already ordered by `core::degree::plan_dag`.
fn build_edges_from_dag(dag: &DAG, plan_courses: &HashSet<&str>) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for (map, edge_type) in [
        (&dag.dependencies, EdgeType::Prerequisite),
        (&dag.corequisites, EdgeType::Corequisite),
    ] {
        let mut targets: Vec<&String> = map.keys().collect();
        targets.sort_unstable();
        for course in targets {
            if !plan_courses.contains(course.as_str()) {
                continue;
            }
            for source in &map[course] {
                if plan_courses.contains(source.as_str()) {
                    edges.push(GraphEdge {
                        from: source.clone(),
                        to: course.clone(),
                        edge_type: edge_type.clone(),
                    });
                }
            }
        }
    }
    edges
}

/// Build edges for a plan by re-parsing course prerequisites and resolving
/// equivalences.
fn build_edges_from_courses(
    school: &School,
    equivalences: &HashMap<String, HashSet<String>>,
    plan_courses: &HashSet<&str>,
    term_of: &HashMap<String, usize>,
) -> Vec<GraphEdge> {
    use crate::core::prerequisite_parser::parse_to_dnf;

    let mut edges = Vec::new();

    // Sorted: `plan_courses` is a `HashSet`, so an unordered walk emits the same edges in
    // a different order on every run, and this list is serialised into the report.
    let mut ordered: Vec<&str> = plan_courses.iter().copied().collect();
    ordered.sort_unstable();

    for course_key in ordered {
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
            let selected = select_best_prereq_path(
                &dnf_paths,
                plan_courses,
                equivalences,
                term_of,
                term_of.get(course_key).copied(),
            );
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
/// Prefers a complete path (all prereqs in the plan) **that the schedule actually
/// satisfies** — every course in it placed strictly before `dependent_term` — then any
/// complete path, then the longest partial match. Resolves each prerequisite through
/// equivalences when the direct course is not in the plan.
///
/// The term check is what stops the picture contradicting the schedule. `CS430` requires
/// `CS314 | CS370`; with both in the plan the first-listed option won on source order
/// alone, so the graph drew `CS314 → CS430` while the scheduler had satisfied the group
/// with `CS370` a term earlier and placed `CS430` alongside `CS314`. Measured over the
/// stored corpus, an option scheduled early enough was available but unchosen for 2,033
/// course-instances across 179 programs.
///
/// It is only ever a tie-break: with one complete path, no term information, or no
/// complete path that precedes the dependent, the result is exactly what it was before.
/// Nothing here influences *placement* — the caller derives terms from the schedule.
fn select_best_prereq_path<'a>(
    dnf_paths: &'a [Vec<String>],
    plan_courses: &HashSet<&str>,
    equivalences: &HashMap<String, HashSet<String>>,
    term_of: &HashMap<String, usize>,
    dependent_term: Option<usize>,
) -> Vec<String> {
    let resolve = |p: &'a String| -> Option<String> {
        if plan_courses.contains(p.as_str()) {
            return Some(p.clone());
        }
        // Shared with the metrics DAG rather than reimplemented: it takes the
        // lexicographic minimum, where a first-hit lookup over the `HashSet` drew a
        // different edge on every run of the same plan.
        //
        // The *scope* still differs on purpose. `plan_dag` substitutes an equivalent only
        // for a `Required` prerequisite — an OR-group option missing from the plan is left
        // alone (`an_or_group_does_not_fall_back_to_the_equivalence_table`). Here every
        // member of the chosen DNF path is resolved, OR-alternatives included, so the
        // picture can draw an equivalence the metrics did not.
        crate::core::degree::plan_dag::equivalent_in_plan(p, equivalences, plan_courses)
            .map(str::to_string)
    };

    // First pass: complete paths, in source order. An empty DNF path counts as complete
    // (it resolves to nothing and satisfies trivially), matching the previous
    // `resolved.len() == path.len()` test rather than being filtered out.
    let complete: Vec<Vec<String>> = dnf_paths
        .iter()
        .filter_map(|path| {
            let resolved: Vec<String> = path.iter().filter_map(resolve).collect();
            (resolved.len() == path.len()).then_some(resolved)
        })
        .collect();

    if let Some(term) = dependent_term {
        // Prefer a complete path the schedule genuinely clears. Still source order
        // among those, so the choice stays stable.
        if let Some(satisfied) = complete.iter().find(|path| {
            path.iter()
                .all(|c| term_of.get(c).is_some_and(|t| *t < term))
        }) {
            return satisfied.clone();
        }
    }
    if let Some(first) = complete.first() {
        return first.clone();
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
    fn edges_from_a_dag_are_emitted_in_sorted_target_order() {
        // `DAG::dependencies` is a `HashMap`, so an unsorted walk serialises the same
        // edges in a different order on every run of the same plan. Rebuilt each
        // iteration on purpose: a map allocated once keeps one iteration order for the
        // life of the process, so hoisting it would sample a single order.
        let expected = vec![
            ("AAA100".to_string(), "BBB200".to_string()),
            ("BBB200".to_string(), "CCC300".to_string()),
            ("AAA100".to_string(), "DDD400".to_string()),
            ("BBB200".to_string(), "EEE500".to_string()),
        ];
        for i in 0..50 {
            let mut dag = DAG::new();
            for (course, prereq) in [
                ("DDD400", "AAA100"),
                ("BBB200", "AAA100"),
                ("EEE500", "BBB200"),
                ("CCC300", "BBB200"),
            ] {
                dag.add_prerequisite(course.to_string(), prereq);
            }
            let plan: HashSet<&str> = ["AAA100", "BBB200", "CCC300", "DDD400", "EEE500"]
                .into_iter()
                .collect();
            let got: Vec<(String, String)> = build_edges_from_dag(&dag, &plan)
                .into_iter()
                .map(|e| (e.from, e.to))
                .collect();
            assert_eq!(
                got, expected,
                "build {i}: edges must follow sorted target order, not hash order"
            );
        }
    }

    #[test]
    fn edges_from_courses_are_emitted_in_sorted_course_order() {
        // Same property for the other edge builder, which walks the plan `HashSet`.
        // The literal order is pinned rather than only run-to-run equality: an
        // equality-only check would also pass for a consistently wrong ordering.
        use crate::core::models::Course;

        let mut school = School::new("T".to_string());
        for (prefix, number, prereq) in [
            ("AAA", "100", None),
            ("BBB", "200", Some("AAA100")),
            ("CCC", "300", Some("BBB200")),
            ("DDD", "400", Some("AAA100")),
            ("EEE", "500", Some("BBB200")),
        ] {
            let mut c = Course::new(
                format!("{prefix} {number}"),
                prefix.to_string(),
                number.to_string(),
                3.0,
            );
            c.prerequisites_raw = prereq.map(str::to_string);
            school.add_course(c);
        }
        let expected = vec![
            ("AAA100".to_string(), "BBB200".to_string()),
            ("BBB200".to_string(), "CCC300".to_string()),
            ("AAA100".to_string(), "DDD400".to_string()),
            ("BBB200".to_string(), "EEE500".to_string()),
        ];
        for i in 0..50 {
            let plan: HashSet<&str> = ["AAA100", "BBB200", "CCC300", "DDD400", "EEE500"]
                .into_iter()
                .collect();
            let got: Vec<(String, String)> =
                build_edges_from_courses(&school, &HashMap::new(), &plan, &HashMap::new())
                    .into_iter()
                    .map(|e| (e.from, e.to))
                    .collect();
            assert_eq!(
                got, expected,
                "build {i}: edges must follow sorted course order, not the plan set's hash order"
            );
        }
    }

    #[test]
    fn select_best_prereq_path_resolves_an_equivalence_deterministically() {
        // `equivalences` values are `HashSet`s, so taking the first hit drew a different
        // edge on each run for the same plan. Rebuilt inside the loop on purpose: a set
        // allocated once keeps one iteration order for the life of the process, so
        // hoisting it would sample a single order and a first-hit implementation could
        // pass by luck.
        let dnf = vec![vec!["MATH140".to_string()]];
        let plan: HashSet<&str> = ["MATH152", "MATH241", "MATH999"].into_iter().collect();
        for _ in 0..50 {
            let mut equivalences = HashMap::new();
            equivalences.insert(
                "MATH140".to_string(),
                ["MATH152", "MATH241", "MATH999"]
                    .into_iter()
                    .map(String::from)
                    .collect::<HashSet<_>>(),
            );
            assert_eq!(
                select_best_prereq_path(&dnf, &plan, &equivalences, &HashMap::new(), None),
                vec!["MATH152".to_string()],
                "the lexicographic minimum, not whichever the hash order yielded"
            );
        }
    }

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

        let stats = ReportStats::from_aggregator(&agg);
        let spec = spec_from_components(
            &school,
            &dag,
            &term_plan,
            &metrics,
            &[],
            Some(&stats),
            "agg",
        );

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
        let result = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &HashMap::new(), None);
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
        let result = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &HashMap::new(), None);
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
        let result = select_best_prereq_path(&dnf, &plan, &equivs, &HashMap::new(), None);
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
        let edges = build_edges_from_courses(&school, &HashMap::new(), &plan, &HashMap::new());

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
            aggregator_medians(Some(&ReportStats::from_aggregator(&agg)), "GHOST101"),
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
        let stats = ReportStats::from_aggregator(&agg);
        let spec_agg = spec_from_scored_plan(&school, &HashMap::new(), &plan, Some(&stats), "agg");
        assert_eq!(spec_agg.nodes[0].median_complexity, Some(6.0));
        assert_eq!(spec_agg.nodes[0].median_delay, Some(2.0));
        assert_eq!(spec_agg.nodes[0].median_blocking, Some(4.0));
    }
    // --- term-aware OR-option selection -------------------------------------

    fn terms(pairs: &[(&str, usize)]) -> HashMap<String, usize> {
        pairs.iter().map(|(c, t)| ((*c).to_string(), *t)).collect()
    }

    #[test]
    fn an_or_group_picks_the_option_the_schedule_actually_clears() {
        // The CS430 case: prerequisites `CS314 | CS370`, both in the plan, CS370 a term
        // earlier and CS314 alongside the dependent. Source order alone chose CS314 and
        // drew an edge the schedule appeared to violate.
        let dnf = vec![vec!["CS314".to_string()], vec!["CS370".to_string()]];
        let plan: HashSet<&str> = ["CS314", "CS370", "CS430"].into_iter().collect();
        let term_of = terms(&[("CS370", 4), ("CS314", 5), ("CS430", 5)]);
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &term_of, Some(5));
        assert_eq!(picked, ["CS370"], "chose an option not scheduled before");
    }

    #[test]
    fn the_first_listed_option_still_wins_when_it_precedes_the_dependent() {
        // The tie-break must not reorder anything it does not have to.
        let dnf = vec![vec!["CS314".to_string()], vec!["CS370".to_string()]];
        let plan: HashSet<&str> = ["CS314", "CS370", "CS430"].into_iter().collect();
        let term_of = terms(&[("CS314", 3), ("CS370", 4), ("CS430", 5)]);
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &term_of, Some(5));
        assert_eq!(picked, ["CS314"], "source order lost for no reason");
    }

    #[test]
    fn with_no_term_information_the_old_choice_is_kept() {
        // Callers without a schedule (`spec_from_components`) must be unaffected.
        let dnf = vec![vec!["CS314".to_string()], vec!["CS370".to_string()]];
        let plan: HashSet<&str> = ["CS314", "CS370"].into_iter().collect();
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &HashMap::new(), None);
        assert_eq!(picked, ["CS314"]);
    }

    #[test]
    fn when_no_option_precedes_the_dependent_the_first_complete_path_is_kept() {
        // 378 stored instances look like this. Dropping the edge entirely would hide a
        // real prerequisite, so the previous answer stands.
        let dnf = vec![vec!["CS314".to_string()], vec!["CS370".to_string()]];
        let plan: HashSet<&str> = ["CS314", "CS370", "CS430"].into_iter().collect();
        let term_of = terms(&[("CS314", 5), ("CS370", 6), ("CS430", 5)]);
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &term_of, Some(5));
        assert_eq!(picked, ["CS314"]);
    }

    #[test]
    fn a_multi_course_path_must_have_every_member_scheduled_early_enough() {
        // AND-of-ORs: `(A & B) | C`. A is late, so the pair cannot be the satisfying
        // route even though both are in the plan.
        let dnf = vec![
            vec!["A".to_string(), "B".to_string()],
            vec!["C".to_string()],
        ];
        let plan: HashSet<&str> = ["A", "B", "C", "D"].into_iter().collect();
        let term_of = terms(&[("A", 5), ("B", 1), ("C", 2), ("D", 5)]);
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &term_of, Some(5));
        assert_eq!(picked, ["C"], "a late member did not disqualify its path");
    }

    #[test]
    fn a_partial_match_is_still_the_fallback_when_nothing_is_complete() {
        let dnf = vec![vec!["X".to_string(), "Y".to_string()]];
        let plan: HashSet<&str> = ["Y", "Z"].into_iter().collect();
        let term_of = terms(&[("Y", 1), ("Z", 4)]);
        let picked = select_best_prereq_path(&dnf, &plan, &HashMap::new(), &term_of, Some(4));
        assert_eq!(picked, ["Y"]);
    }

    #[test]
    fn term_index_maps_each_scheduled_course_to_its_term() {
        use crate::core::report::term_scheduler::{Term, TermPlan};
        let plan = TermPlan {
            terms: vec![
                Term {
                    number: 1,
                    courses: vec!["A".to_string(), "B".to_string()],
                    total_credits: 6.0,
                },
                Term {
                    number: 2,
                    courses: vec!["C".to_string()],
                    total_credits: 3.0,
                },
            ],
            is_quarter_system: false,
            target_credits: 15.0,
            unscheduled: Vec::new(),
        };
        let index = term_index(&plan);
        assert_eq!(index.get("A"), Some(&1));
        assert_eq!(index.get("B"), Some(&1));
        assert_eq!(index.get("C"), Some(&2));
        assert_eq!(index.get("NOPE"), None);
    }
}
