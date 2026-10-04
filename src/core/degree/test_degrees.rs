//! Small degrees shared by the unit tests of the analysis modules.

/// CSU's MATH156, `(MATH124 & MATH126) | MATH127` — an OR between a group of courses and
/// one course — with a course, CS999, that uses MATH124 too.
pub const MATH156: &str = r#"degree: {id: t, institution: CSU, program: T, total_credits: 10, gpa_minimum: 2.0}
requirements:
  core: {name: Core, type: all, category: major, courses: [MATH156]}
courses:
  MATH124: {title: Log, prefix: MATH, number: "124", credits: 1}
  MATH126: {title: Trig, prefix: MATH, number: "126", credits: 1}
  MATH127: {title: Precalc, prefix: MATH, number: "127", credits: 4}
  MATH156: {title: Comp Math I, prefix: MATH, number: "156", credits: 4, prerequisites_raw: "(MATH124 & MATH126) | MATH127"}
  CS999: {title: Uses MATH124, prefix: CS, number: "999", credits: 3, prerequisites_raw: "MATH124"}
"#;

/// [`MATH156`]'s prerequisite graph.
pub fn math156_graph() -> crate::core::models::CourseGraph {
    let (program, _) = crate::core::degree::parse_degree_auto(MATH156).expect("parses");
    crate::core::models::CourseGraph::from_degree_program(&program).graph
}
