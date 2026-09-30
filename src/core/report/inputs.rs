//! The structural inputs a degree report is drawn from: courses, prerequisite graph,
//! equivalences.
//!
//! Built from a degree alone, with no analysis — which is what lets a report be rendered
//! from a stored run (`db report`, the MCP `render_stored_report`) as well as from a fresh
//! one. The CLI's analysis pipeline builds the same things through these functions.

use std::collections::{HashMap, HashSet};

use crate::core::models::degree::Requirement;
use crate::core::models::{CourseGraph, School, DAG};

/// Every course's equivalents, from the `{A, B, C}` groups the requirements name.
///
/// Each course in a group maps to the whole group, itself included. Groups are read from
/// every requirement's course list and pool, and from its options' requirements at any
/// depth.
#[must_use]
pub fn build_equivalence_map<S: std::hash::BuildHasher>(
    requirements: &HashMap<String, Requirement, S>,
) -> HashMap<String, HashSet<String>> {
    let mut equivalences = HashMap::new();
    for req in requirements.values() {
        collect_equivalences(req, &mut equivalences);
    }
    equivalences
}

/// Add the equivalence groups `req` names, recursing into its options.
fn collect_equivalences(req: &Requirement, out: &mut HashMap<String, HashSet<String>>) {
    let pool = req.from.as_ref().and_then(|f| f.courses.as_ref());
    for course_ref in req.courses.iter().chain(pool).flatten() {
        let Some(group) = parse_equivalent_courses(course_ref) else {
            continue;
        };
        for course in &group {
            out.entry(course.clone())
                .or_default()
                .extend(group.iter().cloned());
        }
    }
    for option in req.options.iter().flatten() {
        for nested in &option.requirements {
            collect_equivalences(nested, out);
        }
    }
}

/// Build the structural inputs a report needs from a degree program alone.
///
/// The same graph construction `analyze_program` does, including breaking prerequisite
/// cycles, without which the DAG would not be acyclic. Needs no analysis, so a stored
/// run's report can be drawn from its `document` alone.
#[must_use]
pub fn build_report_inputs(
    program: &crate::core::DegreeProgram,
) -> (School, DAG, HashMap<String, HashSet<String>>) {
    let mut graph_result = CourseGraph::from_degree_program(program);
    if !graph_result.cycles.is_empty() {
        graph_result.graph.break_cycles(&graph_result.cycles);
    }
    (
        build_school_from_program(program),
        build_dag_from_graph(&graph_result.graph),
        build_equivalence_map(&program.requirements),
    )
}

/// The degree's courses as a [`School`], keyed by their document keys.
#[must_use]
pub fn build_school_from_program(program: &crate::core::DegreeProgram) -> School {
    let mut school = School::new(
        program
            .degree
            .institution
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );

    for (key, course) in &program.courses {
        let mut school_course = crate::core::models::Course::new(
            course.name.clone(),
            course.prefix.clone(),
            course.number.clone(),
            course.credit_hours,
        );
        school_course.canonical_name = Some(key.clone());
        school_course
            .prerequisites_raw
            .clone_from(&course.prerequisites_raw);
        if let Some(raw) = &course.prerequisites_raw {
            school_course.prerequisites = parse_prerequisites_from_raw(raw);
        }
        school_course.corequisites.clone_from(&course.corequisites);
        school_course
            .typically_offered
            .clone_from(&course.typically_offered);
        school_course
            .gen_ed_attributes
            .clone_from(&course.gen_ed_attributes);

        // Keyed by the document key, not `prefix + number`: every lookup downstream is
        // by the plan's course id, which is the document key. A lab entry sharing its
        // lecture's prefix and number (`CHEM1410` / `CHEM1410L`, both `CHEM` `1410`)
        // would otherwise overwrite it in hash order, and a key that is not
        // `prefix + number` (`COMSW3137`) would miss and be credited as a placeholder.
        school.add_course_with_key(key.clone(), school_course);
    }

    school
}

/// The course keys a raw prerequisite expression names, in order, each once.
///
/// Operators, grouping and grade suffixes are dropped; a key is letters, digits and `_`
/// (`CS1_HONORS`, `CSE_12`). Examples:
/// - `"CS165[C]"` → `["CS165"]`
/// - `"(CS220[C] & CS165[C])"` → `["CS220", "CS165"]`
/// - `"CS162[C] | CS163[C] | CS164[C]"` → `["CS162", "CS163", "CS164"]`
pub fn parse_prerequisites_from_raw(raw: &str) -> Vec<String> {
    let mut prereqs = Vec::new();
    let cleaned = raw.replace(['(', ')', '&', '|', '[', ']'], " ");

    for part in cleaned.split_whitespace() {
        // A grade left by the bracket removal: `B`, `C+`, `B-`.
        if part.len() <= 2
            && part
                .chars()
                .all(|c| c.is_alphabetic() || c == '-' || c == '+')
        {
            continue;
        }

        if part.chars().next().is_some_and(char::is_alphabetic) {
            let key = part
                .find(|c: char| !(c.is_alphanumeric() || c == '_'))
                .map_or(part, |idx| &part[..idx]);
            if !key.is_empty() && !prereqs.contains(&key.to_string()) {
                prereqs.push(key.to_string());
            }
        }
    }

    prereqs
}

/// The report's DAG: each course's first prerequisite path, plus its required edges.
///
/// The first path of the DNF form is the simplest alternative, which is the one a report
/// draws; required prerequisites are added whichever alternative that is. A path course
/// counts only while its edge remains: [`CourseGraph::break_cycles`] removes the edge but
/// leaves the DNF paths as parsed, and reading them unfiltered put the cycle back.
///
/// Drawing only — plan metrics are computed on each plan's own DAG.
#[must_use]
pub fn build_dag_from_graph(graph: &CourseGraph) -> DAG {
    let mut dag = DAG::new();
    for key in graph.course_keys() {
        let Some(node) = graph.get(key) else {
            continue;
        };
        dag.add_course(key.to_string());
        let has_edge = |p: &str| node.prerequisites.iter().any(|e| e.prerequisite == p);
        let first_path = node
            .prerequisite_paths
            .first()
            .into_iter()
            .flatten()
            .map(String::as_str)
            .filter(|p| has_edge(p));
        for prereq in first_path.chain(node.required_prerequisites()) {
            dag.add_prerequisite(key.to_string(), prereq);
        }
    }
    dag
}

/// Parse equivalent courses from `{A, B, C}` syntax
///
/// Returns `Some(set)` if the input is an equivalent group, `None` otherwise.
fn parse_equivalent_courses(course_ref: &str) -> Option<HashSet<String>> {
    if course_ref.starts_with('{') && course_ref.ends_with('}') {
        let inner = &course_ref[1..course_ref.len() - 1];
        let courses: HashSet<String> = inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if courses.len() > 1 {
            return Some(courses);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(xs: &[&str]) -> HashSet<String> {
        xs.iter().map(|s| (*s).to_string()).collect()
    }

    #[test]
    fn test_parse_prerequisites_from_raw_extracts_every_expression_form() {
        let cases: &[(&str, &[&str])] = &[
            ("CS165[C]", &["CS165"]),
            ("(CS220[C] & CS165[C])", &["CS220", "CS165"]),
            (
                "CS162[C] | CS163[C] | CS164[C]",
                &["CS162", "CS163", "CS164"],
            ),
            (
                "(MATH215 | MATH241) & ICS111[B-]",
                &["MATH215", "MATH241", "ICS111"],
            ),
            ("CS101&CS102|CS103", &["CS101", "CS102", "CS103"]),
            ("CS101 | (CS101 & CS102)", &["CS101", "CS102"]),
            ("MATH1310G[C+]", &["MATH1310G"]),
            ("CSE_12 & CS1_HONORS", &["CSE_12", "CS1_HONORS"]),
            ("", &[]),
            ("   ", &[]),
            ("[C] & ()", &[]),
        ];
        for (raw, expected) in cases {
            assert_eq!(
                parse_prerequisites_from_raw(raw),
                *expected,
                "raw = {raw:?}"
            );
        }
    }

    #[test]
    fn test_parse_equivalent_courses_needs_a_braced_group_of_two_or_more() {
        assert_eq!(
            parse_equivalent_courses("{MATH215, MATH241, MATH251A}"),
            Some(set(&["MATH215", "MATH241", "MATH251A"]))
        );
        assert_eq!(
            parse_equivalent_courses("{CS201,PHIL201}"),
            Some(set(&["CS201", "PHIL201"]))
        );
        assert_eq!(
            parse_equivalent_courses("{A101, , B101,}"),
            Some(set(&["A101", "B101"]))
        );
        for s in [
            "CS101",
            "{CS101}",
            "{}",
            "{CS101, CS101}",
            "[CHEM161, CHEM161L]",
            "{CS101, CS102",
        ] {
            assert_eq!(parse_equivalent_courses(s), None, "{s:?}");
        }
    }

    #[test]
    fn test_build_equivalence_map_reads_courses_pools_and_options_at_any_depth() {
        let reqs: HashMap<String, Requirement> = serde_yaml::from_str(
            r#"
calculus: {name: C, type: all, category: major, courses: ["{MATH215, MATH241}", "CS101", "{SOLO}"]}
electives: {name: E, type: select, category: major, count: 1, from: {courses: ["{CS201, PHIL201}"]}}
track:
  name: T
  type: one_of
  category: major
  options:
    - id: a
      name: A
      requirements:
        - {name: A1, type: all, courses: ["{STAT301, MATH371}"]}
        - {name: A2, type: select, count: 1, from: {courses: ["{ECE101, EE101}"]}}
"#,
        )
        .expect("requirements parse");
        let eq = build_equivalence_map(&reqs);
        assert_eq!(eq["MATH241"], set(&["MATH215", "MATH241"]));
        assert_eq!(eq["PHIL201"], set(&["CS201", "PHIL201"]));
        assert_eq!(eq["STAT301"], set(&["STAT301", "MATH371"]));
        assert_eq!(
            eq["EE101"],
            set(&["ECE101", "EE101"]),
            "an option's pool counts too"
        );
        assert!(!eq.contains_key("CS101") && !eq.contains_key("SOLO"));
        assert_eq!(eq.len(), 8);
    }

    #[test]
    fn test_build_report_inputs_breaks_a_prerequisite_cycle() {
        let yaml = r#"
degree: {id: cyc, institution: T, program: T, total_credits: 6, gpa_minimum: 2.0}
requirements:
  core: {name: Core, type: all, category: major, courses: [A101, B101]}
courses:
  A101: {title: A, prefix: A, number: "101", credits: 3, prerequisites_raw: "B101"}
  B101: {title: B, prefix: B, number: "101", credits: 3, prerequisites_raw: "A101"}
"#;
        let (program, _) = crate::core::degree::parse_degree_auto(yaml).expect("parses");
        let (school, dag, _) = build_report_inputs(&program);
        let has = |c: &str, p: &str| {
            dag.get_prerequisites(c)
                .is_some_and(|ps| ps.iter().any(|x| x == p))
        };
        assert!(
            !(has("A101", "B101") && has("B101", "A101")),
            "the cycle survived into the DAG"
        );
        assert!(
            has("A101", "B101") || has("B101", "A101"),
            "breaking the cycle removed both edges"
        );
        assert!(school.get_course("A101").is_some() && school.get_course("B101").is_some());
    }

    /// Two entries sharing `prefix + number` and a key that is not `prefix + number`
    /// must each be found under their own key with their own credits. Keying by
    /// `Course::key()` lost the lecture or the lab at random and credited
    /// `COMSW3137` as a 3-credit placeholder.
    #[test]
    fn test_build_school_from_program_keys_by_document_key() {
        let json = r#"{
            "degree": {"name": "T", "institution": "T", "total_credits": 8},
            "requirements": {"core": {"type": "all", "category": "major",
                "courses": ["CHEM1410", "CHEM1410L", "COMSW3137"]}},
            "courses": {
                "CHEM1410":  {"name": "Chem",     "prefix": "CHEM", "number": "1410", "credit_hours": 3.0},
                "CHEM1410L": {"name": "Chem Lab", "prefix": "CHEM", "number": "1410", "credit_hours": 1.0},
                "COMSW3137": {"name": "Data Str", "prefix": "COMS", "number": "3137", "credit_hours": 4.0}
            }
        }"#;
        let program = crate::core::degree::parse_degree_json(json)
            .expect("inline degree fixture should parse");
        let school = build_school_from_program(&program);
        for (key, credits) in [("CHEM1410", 3.0), ("CHEM1410L", 1.0), ("COMSW3137", 4.0)] {
            let course = school
                .get_course(key)
                .unwrap_or_else(|| panic!("{key} missing from School"));
            assert!(
                (course.credit_hours - credits).abs() < f32::EPSILON,
                "{key}: {} credits, expected {credits}",
                course.credit_hours
            );
        }
    }
}
