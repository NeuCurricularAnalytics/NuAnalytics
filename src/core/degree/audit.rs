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

/// One entry returned by [`find_deep_chains`]: the course key plus both the
/// pretty-printed forms (kept for CLI output) and the structured branch list
/// (used by the MCP layer to surface per-branch arrays).
#[derive(Debug, Clone)]
pub struct DeepChainResult {
    /// Course key whose prerequisite chain met the threshold
    pub course: String,
    /// Branch lengths formatted for display (e.g. `"5, 3"`)
    pub branch_lengths: String,
    /// Full chain formatted for display (e.g. `(A → B → C) & (D → E)`)
    pub chain: String,
    /// Structured branches; each branch is leaf-to-immediate-prereq order
    pub branches: Vec<Vec<String>>,
}

/// Find courses with deep prerequisite chains (at or above `threshold`).
///
/// Only courses that are "in scope" (matching major subjects or appearing in
/// requirements) are considered. Returns a list ordered by maximum chain
/// length descending then course key alphabetically.
#[must_use]
pub fn find_deep_chains(
    program: &DegreeProgram,
    graph_result: &CourseGraphResult,
    threshold: usize,
) -> Vec<DeepChainResult> {
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
                deep.push(DeepChainResult {
                    course: key.to_string(),
                    branch_lengths: chain.format_lengths(),
                    chain: chain.format(),
                    branches: chain.branches.clone(),
                });
            }
        }
    }

    deep.sort_by(|a, b| {
        let a_max = parse_max_chain_length(&a.branch_lengths);
        let b_max = parse_max_chain_length(&b.branch_lengths);
        b_max.cmp(&a_max).then_with(|| a.course.cmp(&b.course))
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

/// Split a course key like `"CS150B"` into `("CS", 150)`.
///
/// The prefix is the leading letter run; the number is the contiguous digit
/// run that follows. Suffix characters past the digit block (`B` in
/// `"CS150B"`) are ignored — they distinguish lab/lecture pairings but
/// don't change the numeric ordering used by the intermediate-prereq
/// heuristic. Returns `None` when either piece is missing.
fn parse_subject_and_number(key: &str) -> Option<(&str, u32)> {
    let digit_start = key.find(|c: char| c.is_ascii_digit())?;
    if digit_start == 0 {
        return None;
    }
    let prefix = &key[..digit_start];
    let digit_end = key[digit_start..]
        .find(|c: char| !c.is_ascii_digit())
        .map_or(key.len(), |off| digit_start + off);
    let number: u32 = key[digit_start..digit_end].parse().ok()?;
    Some((prefix, number))
}

/// One missing-intermediate-prerequisite finding.
///
/// Reports that course `course_id` declares `declared_prereq` directly,
/// while `suggested_intermediate` — a same-subject course numerically
/// between them — also depends on `declared_prereq` and *might* belong
/// in `course_id`'s prereq chain. Heuristic, never an error.
#[derive(Debug, Clone)]
pub struct MissingIntermediateFinding {
    /// Course that may be skipping an intermediary (e.g. `"CS165"`).
    pub course_id: String,
    /// Prereq the course declares directly (e.g. `"CS150B"`).
    pub declared_prereq: String,
    /// Same-subject candidate course that sits numerically between
    /// `declared_prereq` and `course_id`, and also has `declared_prereq`
    /// among its prerequisites (e.g. `"CS164"`).
    pub suggested_intermediate: String,
    /// Human-readable summary suitable for surfacing in tool output.
    pub rationale: String,
}

/// Detect courses that may be skipping a same-subject intermediate prereq.
///
/// **Heuristic, not a hard error.** A finding fires when:
///
/// 1. Course `C` declares `A` as a prerequisite (`C` and `A` parse cleanly and
///    share the same subject prefix).
/// 2. Some other course `B` in the program has the same subject as `C`,
///    lists `A` among its own prerequisites, and `B.number` falls strictly
///    between `A.number` and `C.number`.
/// 3. `B` is **not** already among `C`'s prerequisites (i.e. there is an
///    actual gap, not a redundant report).
///
/// Cross-subject prereqs (e.g. `CS165` requires `MATH121`) are skipped
/// because the numeric ordering loses meaning across departments. The
/// `suffix` portion of keys like `CS150A` / `CS150B` is ignored — these
/// share `number=150` and won't slot one as a missing intermediate of the
/// other.
///
/// Results are sorted by `(course_id, declared_prereq, suggested_intermediate)`
/// so output is stable across runs.
#[must_use]
pub fn find_missing_intermediate_prereqs(
    program: &DegreeProgram,
) -> Vec<MissingIntermediateFinding> {
    let mut findings = Vec::new();

    // Sort the course iteration order so the finding list is deterministic.
    let mut keys: Vec<&String> = program.courses.keys().collect();
    keys.sort();

    for c_key in &keys {
        let c_key_str = c_key.as_str();
        let Some((c_subject, c_number)) = parse_subject_and_number(c_key_str) else {
            continue;
        };
        let Some(c_course) = program.courses.get(*c_key) else {
            continue;
        };
        if c_course.prerequisites.is_empty() {
            continue;
        }

        for a_key in &c_course.prerequisites {
            let Some((a_subject, a_number)) = parse_subject_and_number(a_key) else {
                continue;
            };
            // Heuristic only applies within a subject — comparing CS165's
            // number against MATH1341's number tells us nothing useful.
            if !c_subject.eq_ignore_ascii_case(a_subject) {
                continue;
            }
            // `A` must be a real course in the program (otherwise validate's
            // MissingPrerequisite check already flags it).
            if !program.courses.contains_key(a_key) {
                continue;
            }

            for b_key in &keys {
                let b_key_str = b_key.as_str();
                if b_key_str == c_key_str || b_key_str == a_key.as_str() {
                    continue;
                }
                let Some((b_subject, b_number)) = parse_subject_and_number(b_key_str) else {
                    continue;
                };
                if !b_subject.eq_ignore_ascii_case(c_subject) {
                    continue;
                }
                if !(a_number < b_number && b_number < c_number) {
                    continue;
                }
                let Some(b_course) = program.courses.get(*b_key) else {
                    continue;
                };
                if !b_course.prerequisites.iter().any(|p| p == a_key) {
                    continue;
                }
                // If C already lists B as a prereq, there's no gap to report.
                if c_course.prerequisites.iter().any(|p| p == b_key_str) {
                    continue;
                }
                findings.push(MissingIntermediateFinding {
                    course_id: c_key_str.to_string(),
                    declared_prereq: a_key.clone(),
                    suggested_intermediate: b_key_str.to_string(),
                    rationale: format!(
                        "{c_key_str} declares {a_key} as a prerequisite, but {b_key_str} (same subject, also depends on {a_key}) sits numerically between them. Verify {b_key_str} is not a missing intermediate in {c_key_str}'s prereq chain."
                    ),
                });
            }
        }
    }

    findings.sort_by(|a, b| {
        a.course_id
            .cmp(&b.course_id)
            .then_with(|| a.declared_prereq.cmp(&b.declared_prereq))
            .then_with(|| a.suggested_intermediate.cmp(&b.suggested_intermediate))
    });
    findings
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
    fn test_parse_subject_and_number_handles_letter_suffix() {
        assert_eq!(parse_subject_and_number("CS150B"), Some(("CS", 150)));
        assert_eq!(parse_subject_and_number("CS165"), Some(("CS", 165)));
        assert_eq!(parse_subject_and_number("MATH1341"), Some(("MATH", 1341)));
        assert_eq!(parse_subject_and_number("CSci2200"), Some(("CSci", 2200)));
        assert_eq!(parse_subject_and_number("123CS"), None);
        assert_eq!(parse_subject_and_number("CS"), None);
    }

    #[test]
    fn test_missing_intermediate_flags_csu_style_chain_gap() {
        // CSU CS150B → CS164 → CS165 (where CS165 wrongly points at CS150B).
        let yaml = r#"
degree:
  id: chain-gap
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS150B, CS164, CS165]

courses:
  CS150B:
    title: Intro CS
    prefix: CS
    number: "150"
    credits: 4
  CS164:
    title: Data Structures
    prefix: CS
    number: "164"
    credits: 4
    prerequisites_raw: "CS150B"
  CS165:
    title: Algorithms
    prefix: CS
    number: "165"
    credits: 4
    prerequisites_raw: "CS150B"
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        let findings = find_missing_intermediate_prereqs(&program);
        assert_eq!(
            findings.len(),
            1,
            "exactly one gap finding expected; got {findings:?}"
        );
        let f = &findings[0];
        assert_eq!(f.course_id, "CS165");
        assert_eq!(f.declared_prereq, "CS150B");
        assert_eq!(f.suggested_intermediate, "CS164");
    }

    #[test]
    fn test_missing_intermediate_silent_when_chain_is_correct() {
        // CS165 already points at CS164 → no gap to report.
        let yaml = r#"
degree:
  id: chain-ok
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS150B, CS164, CS165]

courses:
  CS150B:
    title: Intro CS
    prefix: CS
    number: "150"
    credits: 4
  CS164:
    title: Data Structures
    prefix: CS
    number: "164"
    credits: 4
    prerequisites_raw: "CS150B"
  CS165:
    title: Algorithms
    prefix: CS
    number: "165"
    credits: 4
    prerequisites_raw: "CS150B & CS164"
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        let findings = find_missing_intermediate_prereqs(&program);
        assert!(
            findings.is_empty(),
            "well-formed chain must not produce findings; got {findings:?}"
        );
    }

    #[test]
    fn test_missing_intermediate_skips_cross_subject_prereqs() {
        // CS165's prereq is MATH121 (cross-subject); MATH125 sits "between"
        // numerically but the heuristic doesn't apply across subjects.
        let yaml = r#"
degree:
  id: cross-subject
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [MATH121, MATH125, CS165]

courses:
  MATH121:
    title: Calc
    prefix: MATH
    number: "121"
    credits: 4
  MATH125:
    title: Calc II
    prefix: MATH
    number: "125"
    credits: 4
    prerequisites_raw: "MATH121"
  CS165:
    title: Algos
    prefix: CS
    number: "165"
    credits: 4
    prerequisites_raw: "MATH121"
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        let findings = find_missing_intermediate_prereqs(&program);
        assert!(
            findings.is_empty(),
            "cross-subject prereq must not trigger same-subject heuristic; got {findings:?}"
        );
    }

    #[test]
    fn test_missing_intermediate_skips_equal_number_letter_variants() {
        // CS150A and CS150B share number 150 — the strict `a < b < c`
        // ordering rules out flagging one as a missing intermediate for the
        // other. CS200 declares CS150A; sibling CS150B is not "between".
        let yaml = r#"
degree:
  id: lab-pair
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS150A, CS150B, CS200]

courses:
  CS150A:
    title: Lab
    prefix: CS
    number: "150"
    credits: 1
  CS150B:
    title: Lecture
    prefix: CS
    number: "150"
    credits: 3
    prerequisites_raw: "CS150A"
  CS200:
    title: Followup
    prefix: CS
    number: "200"
    credits: 4
    prerequisites_raw: "CS150A"
"#;
        let program = parse_degree_yaml(yaml).unwrap();
        let findings = find_missing_intermediate_prereqs(&program);
        assert!(
            findings.is_empty(),
            "letter-suffix sibling courses must not slot as intermediates; got {findings:?}"
        );
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
