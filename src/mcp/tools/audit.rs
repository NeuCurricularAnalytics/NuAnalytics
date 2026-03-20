//! Degree audit tool
//!
//! Provides the `audit_degree` MCP tool that performs a comprehensive audit
//! of a degree YAML: validation, missing prerequisites detection, and deep
//! prerequisite chain analysis.

use crate::core::degree::{parse_degree_yaml, DegreeParseError};
use crate::core::models::{CourseGraph, CourseGraphResult};
use crate::core::{validate_degree_program, DegreeProgram};
use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `audit_degree` tool
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AuditDegreeRequest {
    /// The complete degree YAML content as a string
    #[schemars(description = "Complete degree program YAML content to audit")]
    pub yaml_content: String,

    /// Prerequisite chain depth threshold (default: 3)
    #[schemars(description = "Minimum chain length to flag as deep (default: 3)")]
    pub chain_threshold: Option<usize>,
}

/// A course missing expected prerequisites
#[derive(Debug, Serialize)]
pub struct MissingPrereqInfo {
    /// Course key
    pub course: String,
    /// Detected course level (e.g., 2000, 300)
    pub level: u32,
}

/// A course with a deep prerequisite chain
#[derive(Debug, Serialize)]
pub struct DeepChainInfo {
    /// Course key
    pub course: String,
    /// Maximum chain branch length
    pub max_depth: usize,
    /// Formatted branch lengths (e.g., "5, 3")
    pub branch_lengths: String,
    /// Formatted chain representation
    pub chain: String,
}

/// Complete audit response
#[derive(Debug, Serialize)]
pub struct AuditResponse {
    /// Whether the degree has no critical issues
    pub passed: bool,
    /// Parse error if YAML couldn't be parsed
    pub parse_error: Option<String>,

    /// Validation errors count
    pub validation_errors: usize,
    /// Validation warnings count
    pub validation_warnings: usize,
    /// Formatted validation report
    pub validation_report: String,

    /// Upper-level courses without prerequisites
    pub missing_prerequisites: Vec<MissingPrereqInfo>,

    /// Courses with deep prerequisite chains
    pub deep_chains: Vec<DeepChainInfo>,
    /// Threshold used for deep chain detection
    pub chain_threshold: usize,

    /// Degree context (if parseable)
    pub degree_name: Option<String>,
    /// Institution name
    pub institution: Option<String>,
    /// Total courses defined
    pub total_courses: usize,
}

// ============================================================================
// Tool Implementation
// ============================================================================

const DEFAULT_CHAIN_THRESHOLD: usize = 3;

/// Execute the `audit_degree` tool
#[must_use]
pub fn execute(yaml_content: &str, chain_threshold: Option<usize>) -> AuditResponse {
    let threshold = chain_threshold.unwrap_or(DEFAULT_CHAIN_THRESHOLD);

    // Try to parse the YAML
    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => {
            return AuditResponse {
                passed: false,
                parse_error: Some(format_parse_error(&e)),
                validation_errors: 0,
                validation_warnings: 0,
                validation_report: String::new(),
                missing_prerequisites: vec![],
                deep_chains: vec![],
                chain_threshold: threshold,
                degree_name: None,
                institution: None,
                total_courses: 0,
            };
        }
    };

    // Run validation
    let validation = validate_degree_program(&program);

    // Build course graph
    let graph_result = CourseGraph::from_degree_program(&program);

    // Find upper-level courses missing prerequisites
    let lowest_level = detect_lowest_course_level(&program);
    let missing_prereqs = find_upper_level_without_prereqs(&graph_result, lowest_level);

    // Find deep prerequisite chains
    let deep_chains = find_deep_chains(&program, &graph_result, threshold);

    let passed =
        validation.errors.is_empty() && missing_prereqs.is_empty() && deep_chains.is_empty();

    AuditResponse {
        passed,
        parse_error: None,
        validation_errors: validation.errors.len(),
        validation_warnings: validation.warnings.len(),
        validation_report: validation.format_report(),
        missing_prerequisites: missing_prereqs,
        deep_chains,
        chain_threshold: threshold,
        degree_name: Some(program.degree.name.clone()),
        institution: program.degree.institution.clone(),
        total_courses: program.courses.len(),
    }
}

/// Execute and serialize the result as JSON
#[must_use]
pub fn execute_json(yaml_content: &str, chain_threshold: Option<usize>) -> String {
    let response = execute(yaml_content, chain_threshold);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Audit Helpers
// ============================================================================

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError(msg) => format!("YAML syntax error: {msg}"),
    }
}

/// Detect the lowest course level in the degree program
fn detect_lowest_course_level(program: &DegreeProgram) -> u32 {
    program
        .courses
        .keys()
        .filter_map(|k| extract_course_level(k))
        .min()
        .unwrap_or(100)
}

/// Extract numeric course level from a course key (e.g., CS1000 -> 1000, MATH156 -> 100)
fn extract_course_level(key: &str) -> Option<u32> {
    let digits: String = key.chars().filter(char::is_ascii_digit).collect();
    let num: u32 = digits.parse().ok()?;
    if num >= 1000 {
        Some((num / 1000) * 1000)
    } else {
        Some((num / 100) * 100)
    }
}

/// Find upper-level courses that have no prerequisites defined
fn find_upper_level_without_prereqs(
    graph_result: &CourseGraphResult,
    lowest_level: u32,
) -> Vec<MissingPrereqInfo> {
    let mut missing = Vec::new();

    for key in graph_result.graph.course_keys() {
        if let Some(level) = extract_course_level(key) {
            if level <= lowest_level {
                continue;
            }
            if let Some(node) = graph_result.graph.get(key) {
                if node.prerequisites.is_empty() {
                    missing.push(MissingPrereqInfo {
                        course: key.to_string(),
                        level,
                    });
                }
            }
        }
    }

    missing.sort_by(|a, b| a.level.cmp(&b.level).then_with(|| a.course.cmp(&b.course)));
    missing
}

/// Find courses with deep prerequisite chains
fn find_deep_chains(
    program: &DegreeProgram,
    graph_result: &CourseGraphResult,
    threshold: usize,
) -> Vec<DeepChainInfo> {
    let major_subjects = program.degree.major_subjects.as_ref();
    let requirement_courses = collect_requirement_courses(program);
    let mut deep = Vec::new();

    for key in graph_result.graph.course_keys() {
        if !is_course_in_scope(key, major_subjects, &requirement_courses) {
            continue;
        }

        if let Some(chain) = graph_result.graph.structured_prerequisite_chain(key) {
            let max_depth = chain.branch_lengths().into_iter().max().unwrap_or(0);
            if max_depth >= threshold {
                deep.push(DeepChainInfo {
                    course: key.to_string(),
                    max_depth,
                    branch_lengths: chain.format_lengths(),
                    chain: chain.format(),
                });
            }
        }
    }

    deep.sort_by(|a, b| {
        b.max_depth
            .cmp(&a.max_depth)
            .then_with(|| a.course.cmp(&b.course))
    });
    deep
}

/// Collect all course keys referenced in requirements
fn collect_requirement_courses(program: &DegreeProgram) -> HashSet<String> {
    let mut courses = HashSet::new();
    for req in program.requirements.values() {
        collect_from_requirement(req, &mut courses);
    }
    courses
}

/// Recursively collect course keys from a requirement
fn collect_from_requirement(
    req: &crate::core::models::degree::Requirement,
    courses: &mut HashSet<String>,
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

/// Check if a course is in scope for audit (matches major subjects or is in requirements)
fn is_course_in_scope(
    course_key: &str,
    major_subjects: Option<&Vec<String>>,
    requirement_courses: &HashSet<String>,
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

#[cfg(test)]
mod tests {
    use super::*;

    const VALID_YAML: &str = r#"
degree:
  id: test-degree
  institution: Test University
  program: Test Program
  total_credits: 120
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
    fn test_audit_valid_degree() {
        let response = execute(VALID_YAML, None);
        assert!(response.parse_error.is_none());
        assert_eq!(response.total_courses, 2);
        assert_eq!(response.degree_name, Some("Test Program".to_string()));
    }

    #[test]
    fn test_audit_malformed_yaml() {
        let response = execute("not: valid: yaml: {{", None);
        assert!(!response.passed);
        assert!(response.parse_error.is_some());
    }

    #[test]
    fn test_audit_json_output() {
        let json = execute_json(VALID_YAML, None);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert!(parsed["total_courses"].as_u64().unwrap() > 0);
    }

    #[test]
    fn test_extract_course_level() {
        assert_eq!(super::extract_course_level("CS1000"), Some(1000));
        assert_eq!(super::extract_course_level("CS2510"), Some(2000));
        assert_eq!(super::extract_course_level("MATH156"), Some(100));
        assert_eq!(super::extract_course_level("CS101"), Some(100));
    }

    #[test]
    fn test_audit_with_custom_threshold() {
        let response = execute(VALID_YAML, Some(1));
        // With threshold 1, even CS201 (1 prereq) might be flagged
        assert!(response.parse_error.is_none());
        assert_eq!(response.chain_threshold, 1);
    }

    #[test]
    fn test_audit_detects_missing_course() {
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
      - CS101
      - CS999

courses:
  CS101:
    title: Intro
    prefix: CS
    number: "101"
    credits: 4
"#;
        let response = execute(yaml, None);
        assert!(!response.passed);
        assert!(response.validation_errors > 0);
    }
}
