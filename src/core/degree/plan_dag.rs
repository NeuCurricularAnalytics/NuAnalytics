//! Build the prerequisite DAG for one generated plan.
//!
//! Every structural metric — delay, blocking, complexity, chain length — is derived from
//! this graph, so the rules here decide the numbers. It exists once, called by both the
//! CLI and the MCP server, because it previously existed twice and the copies disagreed:
//! the MCP version added an edge for *every* in-plan option of an OR-group, turning
//! "either of these satisfies me" into "both of these are required". Spurious edges
//! lengthen paths and widen fan-out, so the two paths reported complexity roughly 19%
//! apart on one degree.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::core::models::course_graph::CourseGraph;
use crate::core::models::DAG;

/// Build the DAG induced by `courses`, keeping only edges between courses in the plan.
///
/// **An OR-group contributes at most one edge** — one when any of its options is in the
/// plan, none otherwise — chosen by `select_or_group_option`. An `Optional` edge carrying
/// no group id is dropped: there is nothing to choose between.
///
/// Corequisites contribute nothing: they are taken alongside, not before, so they impose
/// no ordering. A required prerequisite missing from the plan is matched through
/// `equivalences` before being dropped.
///
/// `include_courses` are courses the caller forced into every plan (`--include`); an
/// OR-group containing one of them resolves to it, since that is demonstrably the branch
/// the student is taking. Pass an empty set when there is no such preference.
///
/// Every caller uses the default hasher; generalising over `BuildHasher` would need
/// three type parameters here and add noise at every call site for no benefit.
#[allow(clippy::implicit_hasher)]
#[must_use]
pub fn build_plan_dag(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
    include_courses: &HashSet<String>,
) -> DAG {
    let plan: HashSet<&str> = courses.iter().map(String::as_str).collect();
    let mut dag = DAG::new();

    for course_key in courses {
        // Unconditional, so a course with no edges — an elective placeholder, or one
        // absent from the graph — still counts towards the plan the metrics cover.
        dag.add_course(course_key.clone());
        let Some(node) = graph.get(course_key) else {
            continue;
        };

        for prereq in node.required_prerequisites() {
            if plan.contains(prereq) {
                dag.add_prerequisite(course_key.clone(), prereq);
            } else if let Some(equiv) = equivalent_in_plan(prereq, equivalences, &plan) {
                dag.add_prerequisite(course_key.clone(), equiv);
            }
        }

        // `BTreeMap` so the walk does not depend on hash order. Groups are independent,
        // so the *set* of edges is the same either way, but `DAG` stores dependencies as
        // a `Vec` and appends unseen courses in insertion order, so an unordered walk
        // makes the stored ordering vary between runs.
        let or_groups: BTreeMap<usize, Vec<&str>> =
            node.optional_prerequisite_groups().into_iter().collect();
        for options in or_groups.values() {
            if let Some(chosen) =
                select_or_group_option(options, &plan, include_courses, graph, course_key)
            {
                dag.add_prerequisite(course_key.clone(), chosen);
            }
        }
    }
    dag
}

/// Choose the single option that satisfies an OR-group for this plan.
///
/// **At most one edge, never several** — several is the defect this function exists to
/// prevent.
///
/// When more than one option is in the plan the choice is made in a fixed order: a course
/// the caller forced into the plan (lexicographic minimum if several were forced), else
/// whichever option the most other courses in the plan *reference*, else the
/// lexicographic minimum. Every tier ends in a name comparison so the answer cannot vary
/// between runs.
fn select_or_group_option<'a>(
    options: &[&'a str],
    plan: &HashSet<&'a str>,
    include_courses: &HashSet<String>,
    graph: &CourseGraph,
    course_key: &str,
) -> Option<&'a str> {
    let in_plan: Vec<&'a str> = options
        .iter()
        .filter(|opt| plan.contains(*opt))
        .copied()
        .collect();
    match in_plan.len() {
        0 => return None,
        1 => return Some(in_plan[0]),
        _ => {}
    }

    // A course the caller pinned into every plan is the branch actually being taken.
    if let Some(forced) = in_plan
        .iter()
        .filter(|opt| include_courses.contains(**opt))
        .min()
    {
        return Some(*forced);
    }

    // Otherwise the option the rest of the plan leans on hardest, so the DAG reflects the
    // route a student would really follow.
    in_plan
        .iter()
        .min_by_key(|opt| {
            (
                std::cmp::Reverse(in_plan_references(graph, opt, plan, course_key)),
                **opt,
            )
        })
        .copied()
}

/// How many courses in the plan, other than `excluding`, name `course` as a prerequisite.
///
/// References of every kind count, not just required ones — that is the *counting rule*
/// the CLI used, and tightening it would move edge selection, so it is left alone here
/// and tracked in `docs/clean-up-analysis-todo.md`.
///
/// `CourseNode::dependents` is the reverse index the graph already maintains;
/// `break_cycles` prunes it alongside the forward edges, so it cannot name an edge that
/// was removed. That `Vec` is itself built in hash order — safe only because this returns
/// a count and never picks an element out of it. Do not reach for `.next()` or `.find()`
/// here without sorting first.
fn in_plan_references(
    graph: &CourseGraph,
    course: &str,
    plan: &HashSet<&str>,
    excluding: &str,
) -> usize {
    graph.get(course).map_or(0, |n| {
        n.dependents
            .iter()
            .filter(|d| d.as_str() != excluding && plan.contains(d.as_str()))
            .count()
    })
}

/// Find an equivalent of `course` that is in the plan.
///
/// Lexicographic minimum rather than the first hit: `equivalences` values are `HashSet`s,
/// so `find_map` would pick whichever the per-process hash order yielded. That choice
/// becomes a DAG edge, shifts delay factors, and moves the scheduled term — an observed
/// source of run-to-run variation.
fn equivalent_in_plan<'a>(
    course: &str,
    equivalences: &HashMap<String, HashSet<String>>,
    plan: &HashSet<&'a str>,
) -> Option<&'a str> {
    equivalences.get(course).and_then(|equivs| {
        equivs
            .iter()
            .filter_map(|eq| plan.get(eq.as_str()).copied())
            .min()
    })
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::course_graph::{CourseNode, PrerequisiteEdge, PrerequisiteType};

    /// A course with the given prerequisite edges.
    fn node(key: &str, edges: Vec<(&str, PrerequisiteType, Option<usize>)>) -> CourseNode {
        CourseNode {
            key: key.to_string(),
            has_course_data: true,
            prerequisites: edges
                .into_iter()
                .map(|(p, t, g)| PrerequisiteEdge {
                    prerequisite: p.to_string(),
                    prereq_type: t,
                    or_group: g,
                })
                .collect(),
            prerequisite_paths: Vec::new(),
            dependents: Vec::new(),
            credits: 3.0,
            title: String::new(),
            prerequisites_raw: None,
        }
    }

    fn graph_of(nodes: Vec<CourseNode>) -> CourseGraph {
        CourseGraph::from_nodes(nodes)
    }

    fn plan(courses: &[&str]) -> Vec<String> {
        courses.iter().map(|c| (*c).to_string()).collect()
    }

    fn deps<'a>(dag: &'a DAG, course: &str) -> Vec<&'a str> {
        let mut d = ordered_deps(dag, course);
        d.sort_unstable();
        d
    }

    /// Dependencies as stored, order included.
    fn ordered_deps<'a>(dag: &'a DAG, course: &str) -> Vec<&'a str> {
        dag.dependencies
            .get(course)
            .map(|v| v.iter().map(String::as_str).collect())
            .unwrap_or_default()
    }

    #[test]
    fn an_or_group_contributes_exactly_one_edge() {
        // The defect this module exists to prevent. Both alternatives are in the plan;
        // the degree says either satisfies CS201. Adding both states that CS201 requires
        // both — an AND where the format said OR — which lengthens paths and inflates
        // every structural metric derived from this graph.
        let g = graph_of(vec![
            node("CS101", vec![]),
            node("CS102", vec![]),
            node(
                "CS201",
                vec![
                    ("CS101", PrerequisiteType::Optional, Some(0)),
                    ("CS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["CS101", "CS102", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(
            deps(&dag, "CS201").len(),
            1,
            "an OR-group must yield one prerequisite, got {:?}",
            deps(&dag, "CS201")
        );
    }

    #[test]
    fn separate_or_groups_each_contribute_one() {
        let g = graph_of(vec![
            node("A1", vec![]),
            node("A2", vec![]),
            node("B1", vec![]),
            node("B2", vec![]),
            node(
                "TARGET",
                vec![
                    ("A1", PrerequisiteType::Optional, Some(0)),
                    ("A2", PrerequisiteType::Optional, Some(0)),
                    ("B1", PrerequisiteType::Optional, Some(1)),
                    ("B2", PrerequisiteType::Optional, Some(1)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["A1", "A2", "B1", "B2", "TARGET"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(
            deps(&dag, "TARGET").len(),
            2,
            "one per group, not one total"
        );
    }

    #[test]
    fn required_prerequisites_are_all_kept() {
        // Only OR-groups collapse to one. A plain requirement is a requirement.
        let g = graph_of(vec![
            node("M1", vec![]),
            node("M2", vec![]),
            node(
                "CS300",
                vec![
                    ("M1", PrerequisiteType::Required, None),
                    ("M2", PrerequisiteType::Required, None),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["M1", "M2", "CS300"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS300"), ["M1", "M2"]);
    }

    #[test]
    fn a_forced_course_wins_the_or_group() {
        // `--include` means the student is demonstrably taking that branch.
        let g = graph_of(vec![
            node("CS101", vec![]),
            node("CS102", vec![]),
            node(
                "CS201",
                vec![
                    ("CS101", PrerequisiteType::Optional, Some(0)),
                    ("CS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let forced: HashSet<String> = std::iter::once("CS102".to_string()).collect();
        let dag = build_plan_dag(
            &plan(&["CS101", "CS102", "CS201"]),
            &g,
            &HashMap::new(),
            &forced,
        );
        assert_eq!(deps(&dag, "CS201"), ["CS102"]);
    }

    #[test]
    fn the_option_other_courses_depend_on_is_preferred() {
        // With no forced course, follow the route the rest of the plan already takes —
        // ZCS101 gates CS250, so it is the branch a student is really on. Named so it
        // loses the lexicographic tiebreak: passing on that tier would prove nothing.
        let g = graph_of(vec![
            node("ZCS101", vec![]),
            node("ACS102", vec![]),
            node("CS250", vec![("ZCS101", PrerequisiteType::Required, None)]),
            node(
                "CS201",
                vec![
                    ("ZCS101", PrerequisiteType::Optional, Some(0)),
                    ("ACS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["ZCS101", "ACS102", "CS250", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS201"), ["ZCS101"]);
    }

    #[test]
    fn an_or_group_with_nothing_in_the_plan_adds_no_edge() {
        let g = graph_of(vec![node(
            "CS201",
            vec![
                ("CS101", PrerequisiteType::Optional, Some(0)),
                ("CS102", PrerequisiteType::Optional, Some(0)),
            ],
        )]);
        let dag = build_plan_dag(&plan(&["CS201"]), &g, &HashMap::new(), &HashSet::new());
        assert!(deps(&dag, "CS201").is_empty());
    }

    #[test]
    fn corequisites_impose_no_ordering() {
        // Taken alongside, not before.
        let g = graph_of(vec![
            node("LAB101", vec![]),
            node(
                "PHY101",
                vec![("LAB101", PrerequisiteType::Corequisite, None)],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["LAB101", "PHY101"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(deps(&dag, "PHY101").is_empty());
    }

    #[test]
    fn a_required_prerequisite_is_matched_through_an_equivalence() {
        let g = graph_of(vec![
            node("MATH241", vec![]),
            node("CS201", vec![("MATH140", PrerequisiteType::Required, None)]),
        ]);
        let mut equivs = HashMap::new();
        equivs.insert(
            "MATH140".to_string(),
            std::iter::once("MATH241".to_string()).collect::<HashSet<_>>(),
        );
        let dag = build_plan_dag(&plan(&["MATH241", "CS201"]), &g, &equivs, &HashSet::new());
        assert_eq!(deps(&dag, "CS201"), ["MATH241"]);
    }

    #[test]
    fn the_same_plan_always_yields_the_same_dag() {
        // OR-groups are collected into a `HashMap`, so both which option is chosen and
        // the order the chosen edges are appended in must be pinned. A different edge
        // changes delay factors and the scheduled term; a different order changes the
        // stored dependency list for no reason at all.
        let g = graph_of(vec![
            node("A1", vec![]),
            node("A2", vec![]),
            node("A3", vec![]),
            node("B1", vec![]),
            node("B2", vec![]),
            node(
                "T",
                vec![
                    ("A1", PrerequisiteType::Optional, Some(0)),
                    ("A2", PrerequisiteType::Optional, Some(0)),
                    ("A3", PrerequisiteType::Optional, Some(0)),
                    ("B1", PrerequisiteType::Optional, Some(1)),
                    ("B2", PrerequisiteType::Optional, Some(1)),
                ],
            ),
        ]);
        let courses = plan(&["A1", "A2", "A3", "B1", "B2", "T"]);
        let first = build_plan_dag(&courses, &g, &HashMap::new(), &HashSet::new());
        for _ in 0..50 {
            let again = build_plan_dag(&courses, &g, &HashMap::new(), &HashSet::new());
            assert_eq!(ordered_deps(&first, "T"), ordered_deps(&again, "T"));
        }
        assert_eq!(first.courses, courses, "courses stay in plan order");
    }

    #[test]
    fn every_prerequisite_type_has_a_decided_role() {
        // Compile-time guard: adding a `PrerequisiteType` variant fails to compile here,
        // which is the prompt to decide whether it belongs in the graph the metrics are
        // derived from. The classification itself lives on `CourseNode`; this pins that
        // no variant is silently unaccounted for, and covers `StrictCorequisite`, which
        // no other test reaches.
        const fn expected_edges(t: PrerequisiteType) -> usize {
            match t {
                PrerequisiteType::Required | PrerequisiteType::Optional => 1,
                PrerequisiteType::Corequisite | PrerequisiteType::StrictCorequisite => 0,
            }
        }
        for t in [
            PrerequisiteType::Required,
            PrerequisiteType::Optional,
            PrerequisiteType::Corequisite,
            PrerequisiteType::StrictCorequisite,
        ] {
            let group = (t == PrerequisiteType::Optional).then_some(0);
            let g = graph_of(vec![node("P", vec![]), node("C", vec![("P", t, group)])]);
            let dag = build_plan_dag(&plan(&["P", "C"]), &g, &HashMap::new(), &HashSet::new());
            assert_eq!(
                deps(&dag, "C").len(),
                expected_edges(t),
                "{t:?} produced the wrong number of edges"
            );
        }
    }

    #[test]
    fn an_optional_edge_with_no_or_group_is_dropped() {
        // The parser only marks an edge optional as part of a group, so this shape should
        // not occur; if it ever does there is nothing to choose between, and inventing a
        // prerequisite would be worse than omitting one.
        let g = graph_of(vec![
            node("CS101", vec![]),
            node("CS201", vec![("CS101", PrerequisiteType::Optional, None)]),
        ]);
        let dag = build_plan_dag(
            &plan(&["CS101", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(deps(&dag, "CS201").is_empty());
    }

    #[test]
    fn a_tie_on_the_earlier_tiers_falls_to_the_lexicographic_minimum() {
        // Nothing forced and nothing referenced, so only the last tier can decide. Without
        // it the answer would follow hash order and the DAG would differ between runs.
        let g = graph_of(vec![
            node("ZCS101", vec![]),
            node("ACS102", vec![]),
            node(
                "CS201",
                vec![
                    ("ZCS101", PrerequisiteType::Optional, Some(0)),
                    ("ACS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["ZCS101", "ACS102", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS201"), ["ACS102"]);
    }

    #[test]
    fn two_forced_options_in_one_group_resolve_to_the_lexicographic_minimum() {
        // `--include` can pin both branches. CS250 makes ZCS101 the most-referenced
        // option, so the reference tier would choose it — only the forced tier's own
        // name comparison yields ACS102. Without that, this test would pass on tier 3
        // and prove nothing about tier 1.
        let g = graph_of(vec![
            node("ZCS101", vec![]),
            node("ACS102", vec![]),
            node("CS250", vec![("ZCS101", PrerequisiteType::Required, None)]),
            node(
                "CS201",
                vec![
                    ("ZCS101", PrerequisiteType::Optional, Some(0)),
                    ("ACS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let forced: HashSet<String> = ["ZCS101", "ACS102"].into_iter().map(String::from).collect();
        let dag = build_plan_dag(
            &plan(&["ZCS101", "ACS102", "CS250", "CS201"]),
            &g,
            &HashMap::new(),
            &forced,
        );
        assert_eq!(deps(&dag, "CS201"), ["ACS102"]);
    }

    #[test]
    fn a_forced_course_outside_the_plan_does_not_win_the_group() {
        // Forcing only expresses a preference among options the plan actually contains.
        // Honouring it otherwise would wire an edge to a course nobody is taking.
        let g = graph_of(vec![
            node("CS101", vec![]),
            node(
                "CS201",
                vec![
                    ("CS101", PrerequisiteType::Optional, Some(0)),
                    ("CS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let forced: HashSet<String> = std::iter::once("CS102".to_string()).collect();
        let dag = build_plan_dag(&plan(&["CS101", "CS201"]), &g, &HashMap::new(), &forced);
        assert_eq!(deps(&dag, "CS201"), ["CS101"]);
    }

    #[test]
    fn an_or_group_does_not_fall_back_to_the_equivalence_table() {
        // Deliberate asymmetry: a required prerequisite missing from the plan is matched
        // through `equivalences`, an OR-group option is not. A group already offers its
        // own alternatives, so reaching further would add a branch the degree never gave.
        let g = graph_of(vec![
            node("MATH241", vec![]),
            node(
                "CS201",
                vec![
                    ("MATH140", PrerequisiteType::Optional, Some(0)),
                    ("MATH150", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let mut equivs = HashMap::new();
        equivs.insert(
            "MATH140".to_string(),
            std::iter::once("MATH241".to_string()).collect::<HashSet<_>>(),
        );
        let dag = build_plan_dag(&plan(&["MATH241", "CS201"]), &g, &equivs, &HashSet::new());
        assert!(deps(&dag, "CS201").is_empty());
    }

    #[test]
    fn the_lexicographically_smallest_equivalent_in_the_plan_is_chosen() {
        // `equivalences` values are `HashSet`s. Taking the first hit would pick whichever
        // the per-process hash order yielded, and that choice becomes a DAG edge. Three
        // candidates over 50 builds, so a wrong pick cannot stay lucky.
        let g = graph_of(vec![
            node("MATH241", vec![]),
            node("MATH152", vec![]),
            node("MATH999", vec![]),
            node("CS201", vec![("MATH140", PrerequisiteType::Required, None)]),
        ]);
        let courses = plan(&["MATH241", "MATH152", "MATH999", "CS201"]);
        for _ in 0..50 {
            // Rebuilt every iteration on purpose. A `HashSet` allocated once keeps one
            // iteration order for the life of the process, so hoisting this would sample
            // a single order 50 times and a first-hit implementation could pass by luck.
            let mut equivs = HashMap::new();
            equivs.insert(
                "MATH140".to_string(),
                ["MATH241", "MATH152", "MATH999"]
                    .into_iter()
                    .map(String::from)
                    .collect::<HashSet<_>>(),
            );
            let dag = build_plan_dag(&courses, &g, &equivs, &HashSet::new());
            assert_eq!(deps(&dag, "CS201"), ["MATH152"]);
        }
    }

    #[test]
    fn a_prerequisite_in_the_plan_wins_over_its_equivalent() {
        // The equivalence is a fallback, not an alternative: if the course the degree
        // actually names is there, that is the edge.
        let g = graph_of(vec![
            node("MATH140", vec![]),
            node("MATH241", vec![]),
            node("CS201", vec![("MATH140", PrerequisiteType::Required, None)]),
        ]);
        let mut equivs = HashMap::new();
        equivs.insert(
            "MATH140".to_string(),
            std::iter::once("MATH241".to_string()).collect::<HashSet<_>>(),
        );
        let dag = build_plan_dag(
            &plan(&["MATH140", "MATH241", "CS201"]),
            &g,
            &equivs,
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS201"), ["MATH140"]);
    }

    #[test]
    fn a_prerequisite_with_no_match_and_no_equivalent_adds_no_edge() {
        let g = graph_of(vec![
            node("CS101", vec![]),
            node("CS201", vec![("MATH140", PrerequisiteType::Required, None)]),
        ]);
        let mut equivs = HashMap::new();
        equivs.insert(
            "MATH140".to_string(),
            std::iter::once("MATH241".to_string()).collect::<HashSet<_>>(),
        );
        for e in [&equivs, &HashMap::new()] {
            let dag = build_plan_dag(&plan(&["CS101", "CS201"]), &g, e, &HashSet::new());
            assert!(deps(&dag, "CS201").is_empty());
        }
    }

    #[test]
    fn every_plan_course_reaches_the_dag_even_with_no_graph_node() {
        // Elective placeholders arrive here with no node. Dropping them would shorten the
        // plan the metrics are computed over.
        let g = graph_of(vec![node("CS101", vec![])]);
        let dag = build_plan_dag(
            &plan(&["CS101", "ELEC_01"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert!(dag.courses.contains(&"ELEC_01".to_string()));
        assert!(deps(&dag, "ELEC_01").is_empty());
    }

    #[test]
    fn edges_are_stored_required_first_then_groups_in_ascending_group_order() {
        // `deps` sorts, so this is the only test that can see the order
        // `DAG::dependencies` actually holds — the property the `BTreeMap` exists for.
        // Names are chosen to lose under any sort, so a reordering cannot accidentally
        // match the expectation.
        let g = graph_of(vec![
            node("ZREQ", vec![]),
            node("AREQ", vec![]),
            node("BOPT", vec![]),
            node("AOPT", vec![]),
            node(
                "T",
                vec![
                    ("ZREQ", PrerequisiteType::Required, None),
                    ("AREQ", PrerequisiteType::Required, None),
                    ("BOPT", PrerequisiteType::Optional, Some(1)),
                    ("AOPT", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let courses = plan(&["ZREQ", "AREQ", "BOPT", "AOPT", "T"]);
        let dag = build_plan_dag(&courses, &g, &HashMap::new(), &HashSet::new());
        assert_eq!(
            ordered_deps(&dag, "T"),
            ["ZREQ", "AREQ", "AOPT", "BOPT"],
            "required edges in declaration order, then one per group in ascending group id"
        );
        assert_eq!(dag.courses, courses, "courses appended in plan order");
    }

    #[test]
    fn any_kind_of_reference_counts_towards_the_or_group_preference() {
        // Deliberate, and documented on `in_plan_references`: the count is references of
        // every kind. CS250 names ZCS101 only as a corequisite, and that is still enough
        // to make it the branch the plan leans on. Restricting the count to required
        // edges would drop ZCS101 to zero and the name tier would pick ACS102, so this
        // test can only pass for the stated reason.
        let g = graph_of(vec![
            node("ZCS101", vec![]),
            node("ACS102", vec![]),
            node(
                "CS250",
                vec![("ZCS101", PrerequisiteType::Corequisite, None)],
            ),
            node(
                "CS201",
                vec![
                    ("ZCS101", PrerequisiteType::Optional, Some(0)),
                    ("ACS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["ZCS101", "ACS102", "CS250", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS201"), ["ZCS101"]);
    }

    #[test]
    fn references_from_outside_the_plan_do_not_decide_the_group() {
        // The tier follows the route *this* plan takes. OMIT900 requires ZCS101 but is
        // not in the plan, so it is not evidence about this student; discounting it
        // leaves nothing to separate the options and the name decides.
        let g = graph_of(vec![
            node("ZCS101", vec![]),
            node("ACS102", vec![]),
            node(
                "OMIT900",
                vec![("ZCS101", PrerequisiteType::Required, None)],
            ),
            node(
                "CS201",
                vec![
                    ("ZCS101", PrerequisiteType::Optional, Some(0)),
                    ("ACS102", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["ZCS101", "ACS102", "CS201"]),
            &g,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(deps(&dag, "CS201"), ["ACS102"]);
    }

    #[test]
    fn an_or_group_option_absent_from_the_graph_counts_as_unreferenced() {
        // No node means nothing is known about it, which is zero references — not
        // "unknown, therefore best". Both directions, because a wrong default and a
        // wrong self-count fail in opposite ones.
        let referenced = graph_of(vec![
            node("ZOPT", vec![]),
            node("CS250", vec![("ZOPT", PrerequisiteType::Required, None)]),
            node(
                "CS201",
                vec![
                    ("AOPT", PrerequisiteType::Optional, Some(0)),
                    ("ZOPT", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["AOPT", "ZOPT", "CS250", "CS201"]),
            &referenced,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(
            deps(&dag, "CS201"),
            ["ZOPT"],
            "a real reference beats a course with no node"
        );

        let unreferenced = graph_of(vec![
            node("ZOPT", vec![]),
            node(
                "CS201",
                vec![
                    ("AOPT", PrerequisiteType::Optional, Some(0)),
                    ("ZOPT", PrerequisiteType::Optional, Some(0)),
                ],
            ),
        ]);
        let dag = build_plan_dag(
            &plan(&["AOPT", "ZOPT", "CS201"]),
            &unreferenced,
            &HashMap::new(),
            &HashSet::new(),
        );
        assert_eq!(
            deps(&dag, "CS201"),
            ["AOPT"],
            "the requesting course is not a reference, so the name decides"
        );
    }
}
