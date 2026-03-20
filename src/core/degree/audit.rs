//! Shared audit helper functions for degree program analysis
//!
//! This module contains the core audit logic used by both the CLI `degree audit`
//! command and the MCP `audit_degree` tool. By centralising these functions we
//! avoid duplicating prerequisite-chain analysis, course-level detection, and
//! requirement-course collection across the two entry points.

use crate::core::models::degree::Requirement;
use crate::core::models::CourseGraphResult;
use crate::core::DegreeProgram;
use std::collections::HashSet;

/// Extract the numeric course level from a course key.
///
/// The level is determined by extracting all digit characters from the key and
/// rounding down to the nearest thousand (for 4-digit numbers) or hundred
/// (for 3-digit numbers).
///
/// # Examples
/// ```
/// use nu_analytics::core::degree::audit::extract_course_level;
///
/// assert_eq!(extract_course_level("CS1000"), Some(1000));
/// assert_eq!(extract_course_level("CS2510"), Some(2000));
/// assert_eq!(extract_course_level("MATH156"), Some(100));
/// assert_eq!(extract_course_level("CS101"), Some(100));
/// ```
pub fn extract_course_level(key: &str) -> Option<u32> {
    let digits: String = key.chars().filter(char::is_ascii_digit).collect();
    let num: u32 = digits.parse().ok()?;
    if num >= 1000 {
        Some((num / 1000) * 1000)
    } else {
        Some((num / 100) * 100)
    }
}

/// Detect the lowest course level present in a degree program.
///
/// Iterates over all course keys, extracts their levels via
/// [`extract_course_level`], and returns the minimum. Falls back to `100` when
/// no levels can be detected.
#[must_use]
pub fn detect_lowest_course_level(program: &DegreeProgram) -> u32 {
    program
        .courses
        .keys()
        .filter_map(|k| extract_course_level(k))
        .min()
        .unwrap_or(100)
}

/// Find upper-level courses that have no prerequisites defined.
///
/// A course is considered "upper-level" if its detected level is strictly
/// greater than `lowest_level`. Returns a sorted list of `(course_key, level)`
/// tuples, ordered by level ascending then course key alphabetically.
#[must_use]
pub fn find_upper_level_without_prereqs(
    graph_result: &CourseGraphResult,
    lowest_level: u32,
) -> Vec<(String, u32)> {
    let mut missing = Vec::new();

    for key in graph_result.graph.course_keys() {
        if let Some(level) = extract_course_level(key) {
            if level <= lowest_level {
                continue;
            }
            if let Some(node) = graph_result.graph.get(key) {
                if node.prerequisites.is_empty() {
                    missing.push((key.to_string(), level));
                }
            }
        }
    }

    missing.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    missing
}

/// Find courses with deep prerequisite chains (at or above `threshold`).
///
/// Only courses that are "in scope" (matching major subjects or appearing in
/// requirements) are considered. Returns a sorted list of
/// `(course_key, formatted_branch_lengths, formatted_chain)` tuples, ordered
/// by maximum chain length descending then course key alphabetically.
#[must_use]
pub fn find_deep_chains(
    program: &DegreeProgram,
    graph_result: &CourseGraphResult,
    threshold: usize,
) -> Vec<(String, String, String)> {
    let major_subjects = program.degree.major_subjects.as_ref();
    let requirement_courses = collect_requirement_courses(program);
    let mut deep = Vec::new();

    for key in graph_result.graph.course_keys() {
        if !is_course_in_scope(key, major_subjects, &requirement_courses) {
            continue;
        }

        if let Some(chain) = graph_result.graph.structured_prerequisite_chain(key) {
            let max_branch_len = chain.branch_lengths().into_iter().max().unwrap_or(0);
            if max_branch_len >= threshold {
                deep.push((key.to_string(), chain.format_lengths(), chain.format()));
            }
        }
    }

    deep.sort_by(|a, b| {
        let a_max = parse_max_chain_length(&a.1);
        let b_max = parse_max_chain_length(&b.1);
        b_max.cmp(&a_max).then_with(|| a.0.cmp(&b.0))
    });
    deep
}

/// Collect all course keys referenced in the program's requirements.
///
/// Recursively walks every requirement (including `from` clauses, groups, and
/// nested `options`) and returns the set of all course keys found.
#[must_use]
pub fn collect_requirement_courses(program: &DegreeProgram) -> HashSet<String> {
    let mut courses = HashSet::new();
    for req in program.requirements.values() {
        collect_from_requirement(req, &mut courses);
    }
    courses
}

/// Recursively collect course keys from a single requirement.
pub fn collect_from_requirement<S: ::std::hash::BuildHasher>(
    req: &Requirement,
    courses: &mut HashSet<String, S>,
) {
    if let Some(req_courses) = &req.courses {
        courses.extend(req_courses.iter().cloned());
    }
    if let Some(from) = &req.from {
        if let Some(from_courses) = &from.courses {
            courses.extend(from_courses.iter().cloned());
        }
        if let Some(groups) = &from.groups {
            for group in groups {
                courses.extend(group.courses.iter().cloned());
            }
        }
    }
    if let Some(options) = &req.options {
        for option in options {
            for nested_req in &option.requirements {
                collect_from_requirement(nested_req, courses);
            }
        }
    }
}

/// Check whether a course is in scope for audit analysis.
///
/// A course is in scope if:
/// - `major_subjects` is `Some` and the course's subject prefix matches one of
///   the listed subjects (case-insensitive), **or**
/// - `major_subjects` is `None` and the course key appears in
///   `requirement_courses`.
#[must_use]
pub fn is_course_in_scope<S: ::std::hash::BuildHasher>(
    course_key: &str,
    major_subjects: Option<&Vec<String>>,
    requirement_courses: &HashSet<String, S>,
) -> bool {
    if let Some(subjects) = major_subjects {
        let digit_pos = course_key.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
        if digit_pos > 0 {
            let subject = &course_key[..digit_pos];
            if subjects.iter().any(|s| s.eq_ignore_ascii_case(subject)) {
                return true;
            }
        }
    }

    if major_subjects.is_none() {
        return requirement_courses.contains(course_key);
    }

    false
}

/// Parse the maximum chain length from a formatted length string (e.g., "5, 3" -> 5).
fn parse_max_chain_length(lengths_str: &str) -> usize {
    lengths_str
        .split(", ")
        .filter_map(|n| n.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::degree::parse_degree_yaml;

    #[test]
    fn test_extract_course_level_four_digit() {
        assert_eq!(extract_course_level("CS1000"), Some(1000));
        assert_eq!(extract_course_level("CS2510"), Some(2000));
        assert_eq!(extract_course_level("CS4500"), Some(4000));
    }

    #[test]
    fn test_extract_course_level_three_digit() {
        assert_eq!(extract_course_level("MATH156"), Some(100));
        assert_eq!(extract_course_level("CS101"), Some(100));
        assert_eq!(extract_course_level("CS301"), Some(300));
    }

    #[test]
    fn test_extract_course_level_no_digits() {
        assert_eq!(extract_course_level("COOP"), None);
    }

    #[test]
    fn test_detect_lowest_course_level_basic() {
        let yaml = r#"
degree:
  id: test
  institution: Test
  program: Test
  total_credits: 120
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses:
      - CS1000

courses:
  CS1000:
    title: Intro
    prefix: CS
    number: "1000"
    credits: 4
  CS2000:
    title: Advanced
    prefix: CS
    number: "2000"
    credits: 4
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        assert_eq!(detect_lowest_course_level(&program), 1000);
    }

    #[test]
    fn test_detect_lowest_course_level_empty() {
        let yaml = r"
degree:
  id: test
  institution: Test
  program: Test
  total_credits: 120
  gpa_minimum: 2.0

requirements: {}

courses: {}
";
        let program = parse_degree_yaml(yaml).unwrap();
        assert_eq!(detect_lowest_course_level(&program), 100);
    }

    #[test]
    fn test_is_course_in_scope_with_major_subjects() {
        let subjects = vec!["CS".to_string(), "DS".to_string()];
        let req_courses = HashSet::new();

        assert!(is_course_in_scope("CS2500", Some(&subjects), &req_courses));
        assert!(is_course_in_scope("DS3000", Some(&subjects), &req_courses));
        assert!(!is_course_in_scope(
            "MATH1341",
            Some(&subjects),
            &req_courses
        ));
    }

    #[test]
    fn test_is_course_in_scope_without_major_subjects() {
        let mut req_courses = HashSet::new();
        req_courses.insert("CS2500".to_string());
        req_courses.insert("MATH1341".to_string());

        assert!(is_course_in_scope("CS2500", None, &req_courses));
        assert!(is_course_in_scope("MATH1341", None, &req_courses));
        assert!(!is_course_in_scope("PHYS1000", None, &req_courses));
    }

    #[test]
    fn test_collect_requirement_courses_basic() {
        let yaml = r#"
degree:
  id: test
  institution: Test
  program: Test
  total_credits: 120
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses:
      - CS1000
      - CS2000
  elective:
    name: Elective
    type: all
    category: major
    courses:
      - MATH1341

courses:
  CS1000:
    title: Intro
    prefix: CS
    number: "1000"
    credits: 4
  CS2000:
    title: Advanced
    prefix: CS
    number: "2000"
    credits: 4
  MATH1341:
    title: Calculus
    prefix: MATH
    number: "1341"
    credits: 4
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        let courses = collect_requirement_courses(&program);
        assert!(courses.contains("CS1000"));
        assert!(courses.contains("CS2000"));
        assert!(courses.contains("MATH1341"));
        assert_eq!(courses.len(), 3);
    }
}
