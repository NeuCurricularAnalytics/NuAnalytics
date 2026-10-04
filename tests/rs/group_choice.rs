//! `{[A, B], [C]}` in an `all` list, end to end: plan generation, analysis, trimming and the
//! MCP tools see the `one_of` the parser expands it into, and it behaves exactly as the same
//! degree written with that `one_of` by hand.

use std::collections::BTreeSet;

use nu_analytics::core::degree::{
    parse_degree_auto, trim_program, validate_degree_program, PlanGenerator, PlanGeneratorConfig,
    TrimOptions,
};

const COURSES: &str = r#"
courses:
  CS101: {title: Intro, prefix: CS, number: "101", credits: 3}
  CS201: {title: Data Structures, prefix: CS, number: "201", credits: 3, prerequisites_raw: "CS101 & (MATH142 | MATH155)"}
  MATH141: {title: Calculus I, prefix: MATH, number: "141", credits: 4}
  MATH142: {title: Calculus II, prefix: MATH, number: "142", credits: 4, prerequisites_raw: "MATH141"}
  MATH155: {title: Accelerated Calculus, prefix: MATH, number: "155", credits: 5}
"#;
const HEAD: &str =
    "degree: {id: gc, institution: T, program: T, total_credits: 14, gpa_minimum: 2.0}\n";

/// The degree written with the new syntax.
fn with_syntax() -> String {
    format!(
        "{HEAD}requirements:\n  core: {{name: Core, type: all, category: major, courses: [CS101, CS201, \"{{[MATH141, MATH142], [MATH155]}}\"]}}\n{COURSES}"
    )
}

/// The same degree, written as the `one_of` it means.
fn by_hand() -> String {
    format!(
        r"{HEAD}requirements:
  core:
    name: Core
    type: one_of
    category: major
    options:
      - {{id: two, name: Two, requirements: [{{name: Core, type: all, category: major, courses: [CS101, CS201, MATH141, MATH142]}}]}}
      - {{id: one, name: One, requirements: [{{name: Core, type: all, category: major, courses: [CS101, CS201, MATH155]}}]}}
{COURSES}"
    )
}

/// The course sets every generated plan draws, as sorted lists.
fn plan_course_sets(text: &str) -> BTreeSet<Vec<String>> {
    let (program, _) = parse_degree_auto(text).expect("parses");
    let generator = PlanGenerator::new(
        &program.requirements,
        &program.courses,
        PlanGeneratorConfig::default(),
    );
    generator
        .generate()
        .map(|v| {
            let mut courses: Vec<String> = v
                .courses
                .into_iter()
                .filter(|c| {
                    !c.starts_with(nu_analytics::core::degree::placeholder::ELECTIVE_PREFIX)
                })
                .collect();
            courses.sort();
            courses
        })
        .collect()
}

#[test]
fn every_plan_takes_exactly_one_alternative_and_both_occur() {
    let sets = plan_course_sets(&with_syntax());
    let two =
        |s: &Vec<String>| s.contains(&"MATH141".to_string()) && s.contains(&"MATH142".to_string());
    let one = |s: &Vec<String>| s.contains(&"MATH155".to_string());
    assert!(
        sets.iter().all(|s| two(s) != one(s)),
        "each plan takes one alternative: {sets:?}"
    );
    assert!(
        sets.iter().any(two) && sets.iter().any(one),
        "both occur: {sets:?}"
    );
}

#[test]
fn the_syntax_plans_exactly_as_the_hand_written_one_of() {
    assert_eq!(
        plan_course_sets(&with_syntax()),
        plan_course_sets(&by_hand())
    );
    let (program, _) = parse_degree_auto(&with_syntax()).expect("parses");
    assert!(
        validate_degree_program(&program).errors.is_empty(),
        "{:?}",
        validate_degree_program(&program).errors
    );
}

#[test]
fn trimming_an_expanded_degree_works_and_still_validates() {
    let (program, _) = parse_degree_auto(&with_syntax()).expect("parses");
    let (trimmed, _) = trim_program(&program, &TrimOptions::default());
    assert!(
        validate_degree_program(&trimmed).errors.is_empty(),
        "{:?}",
        validate_degree_program(&trimmed).errors
    );
}

/// Analysis figures of the two spellings agree. Both populations are enumerated in full
/// (two plans), so the sample seed — which differs, being derived from the text — plays no
/// part.
#[cfg(feature = "mcp")]
#[test]
fn the_syntax_analyzes_exactly_as_the_hand_written_one_of() {
    use nu_analytics::mcp::tools::analyze::{execute, AnalyzeOptions};
    let a = execute(&with_syntax(), &AnalyzeOptions::default());
    let b = execute(&by_hand(), &AnalyzeOptions::default());
    assert!(a.success && b.success, "{:?} {:?}", a.error, b.error);
    assert!(a.is_full_population && b.is_full_population);
    assert_eq!(a.plans_analyzed, b.plans_analyzed);
    let json = |r: &nu_analytics::mcp::tools::analyze::AnalysisResponse| {
        serde_json::json!({
            "complexity": r.complexity,
            "longest_delay": r.longest_delay,
            "total_credits": r.total_credits,
        })
    };
    assert_eq!(json(&a), json(&b));
}

/// Through the MCP tools, with the degree passed inline.
#[cfg(feature = "mcp")]
#[test]
fn the_mcp_degree_tools_accept_the_syntax_inline() {
    use nu_analytics::mcp::tools::validate;
    let v: serde_json::Value =
        serde_json::from_str(&validate::execute_json(&with_syntax(), false, true)).unwrap();
    assert_eq!(v["is_valid"], true, "{v}");
}
