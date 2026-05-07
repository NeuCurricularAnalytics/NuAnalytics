//! Degree audit tool
//!
//! Provides the `audit_degree` MCP tool that performs a comprehensive audit
//! of a degree YAML: validation, missing prerequisites detection, and deep
//! prerequisite chain analysis.

use crate::core::degree::audit::{
    detect_lowest_course_level, find_deep_chains, find_upper_level_without_prereqs,
};
use crate::core::degree::{parse_degree_yaml, DegreeParseError};
use crate::core::models::CourseGraph;
use crate::core::validate_degree_program;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

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
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
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
    let missing_prereqs: Vec<MissingPrereqInfo> =
        find_upper_level_without_prereqs(&graph_result, lowest_level)
            .into_iter()
            .map(|(course, level)| MissingPrereqInfo { course, level })
            .collect();

    // Find deep prerequisite chains
    let deep_chains: Vec<DeepChainInfo> = find_deep_chains(&program, &graph_result, threshold)
        .into_iter()
        .map(|(course, branch_lengths, chain)| {
            let max_depth = branch_lengths
                .split(", ")
                .filter_map(|n| n.parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            DeepChainInfo {
                course,
                max_depth,
                branch_lengths,
                chain,
            }
        })
        .collect();

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
        use crate::core::degree::audit::extract_course_level;
        assert_eq!(extract_course_level("CS1000"), Some(1000));
        assert_eq!(extract_course_level("CS2510"), Some(2000));
        assert_eq!(extract_course_level("MATH156"), Some(100));
        assert_eq!(extract_course_level("CS101"), Some(100));
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
