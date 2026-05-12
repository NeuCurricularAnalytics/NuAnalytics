//! Course-pattern preview tool.
//!
//! Provides the `find_courses_matching` MCP tool: a thin wrapper around
//! [`RequirementResolver::resolve_pool`] that lets callers preview which
//! courses in a YAML match a set of `include` / `exclude` patterns. Useful
//! while sketching a `select` requirement when the model wants to see what
//! the pool will contain before committing to the requirement definition.

use crate::core::degree::audit::extract_course_level;
use crate::core::degree::{parse_degree_yaml, DegreeParseError, RequirementResolver};
use crate::core::models::degree::FromClause;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for `find_courses_matching`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FindCoursesMatchingRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Filesystem path the server will read. Mutually exclusive with the others.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored degree id (DB lookup). Mutually exclusive with the others.
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

    /// Patterns to match against course keys defined in the YAML. Uses the
    /// same grammar as `select` requirement `from.pattern` / `from.include`:
    /// `"CS:3000+"`, `"MATH:300-499"`, `"*:100+"`, etc.
    #[schemars(
        description = "Comma-separated patterns to match (e.g. \"CS:300+,MATH:300-499\"). Same grammar as `select` from.pattern / from.include."
    )]
    pub patterns: String,

    /// Optional patterns or course keys to exclude from the resolved pool.
    #[schemars(
        description = "Comma-separated patterns or course keys to exclude (e.g. \"CS:380-399,CS499\")."
    )]
    pub exclude: Option<String>,
}

/// One resolved course in the preview response.
#[derive(Debug, Serialize)]
pub struct MatchedCourse {
    /// Course key (e.g. `"CS3000"`).
    pub course_id: String,
    /// Course title.
    pub title: Option<String>,
    /// Subject prefix (e.g. `"CS"`).
    pub prefix: Option<String>,
    /// Course number (e.g. `"3000"`).
    pub number: Option<String>,
    /// Detected course level (e.g. 3000).
    pub level: Option<u32>,
}

/// Response for `find_courses_matching`.
#[derive(Debug, Serialize)]
pub struct FindCoursesMatchingResponse {
    /// True when the YAML parsed and the pool was resolved (even if empty).
    pub success: bool,
    /// Error message when `success` is false.
    pub error: Option<String>,
    /// Patterns echoed back from the request for correlation.
    pub patterns: Vec<String>,
    /// Patterns / course keys echoed back from the `exclude` field.
    pub exclude: Vec<String>,
    /// Number of courses that matched after applying excludes.
    pub match_count: usize,
    /// Resolved courses, sorted by key.
    pub matched: Vec<MatchedCourse>,
    /// Surfaced when `match_count == 0` so the caller sees the empty pool
    /// without scanning the array.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<&'static str>,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `find_courses_matching` tool.
#[must_use]
pub fn execute(
    yaml_content: &str,
    patterns: Vec<String>,
    exclude: Vec<String>,
) -> FindCoursesMatchingResponse {
    if patterns.is_empty() {
        return error_response(
            patterns,
            exclude,
            "Provide at least one pattern (e.g. \"CS:300+\").",
        );
    }

    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => return error_response(patterns, exclude, format_parse_error(&e)),
    };

    // Drive the existing resolver via a synthesised FromClause; same path
    // validate_degree's `resolved_pools` walks for each `select` requirement.
    let from = FromClause {
        courses: None,
        pattern: None,
        include: Some(patterns.clone()),
        exclude: (!exclude.is_empty()).then(|| exclude.clone()),
        groups: None,
        groups_required: None,
        per_group: None,
    };
    let mut resolver = RequirementResolver::new(&program.courses);
    let mut pool = resolver.resolve_pool(&from);
    pool.sort();

    let matched: Vec<MatchedCourse> = pool
        .iter()
        .map(|key| {
            let course = program.courses.get(key);
            MatchedCourse {
                course_id: key.clone(),
                title: course.map(|c| c.name.clone()),
                prefix: course.map(|c| c.prefix.clone()),
                number: course.map(|c| c.number.clone()),
                level: extract_course_level(key),
            }
        })
        .collect();

    let match_count = matched.len();
    let note = if match_count == 0 {
        Some("Patterns matched no enumerated courses — either the patterns are too narrow or the YAML doesn't yet list courses that satisfy them.")
    } else {
        None
    };

    FindCoursesMatchingResponse {
        success: true,
        error: None,
        patterns,
        exclude,
        match_count,
        matched,
        note,
    }
}

/// Execute and serialize as JSON.
#[must_use]
pub fn execute_json(yaml_content: &str, patterns: Vec<String>, exclude: Vec<String>) -> String {
    let response = execute(yaml_content, patterns, exclude);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

fn error_response(
    patterns: Vec<String>,
    exclude: Vec<String>,
    error: impl Into<String>,
) -> FindCoursesMatchingResponse {
    FindCoursesMatchingResponse {
        success: false,
        error: Some(error.into()),
        patterns,
        exclude,
        match_count: 0,
        matched: Vec::new(),
        note: None,
    }
}

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError { message, .. } => format!("YAML syntax error: {message}"),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_YAML: &str = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 16
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
  CS200:
    title: Mid CS
    prefix: CS
    number: "200"
    credits: 4
  CS300:
    title: Algorithms
    prefix: CS
    number: "300"
    credits: 4
  CS400:
    title: Senior CS
    prefix: CS
    number: "400"
    credits: 4
  MATH156:
    title: Calculus I
    prefix: MATH
    number: "156"
    credits: 4
"#;

    #[test]
    fn test_pattern_matches_upper_division_courses() {
        let response = execute(TEST_YAML, vec!["CS:300+".to_string()], Vec::new());
        assert!(response.success);
        let ids: Vec<&str> = response
            .matched
            .iter()
            .map(|m| m.course_id.as_str())
            .collect();
        assert!(ids.contains(&"CS300"));
        assert!(ids.contains(&"CS400"));
        assert!(!ids.contains(&"CS101"));
        assert!(!ids.contains(&"MATH156"));
        assert_eq!(response.match_count, ids.len());
        assert!(response.note.is_none());
    }

    #[test]
    fn test_exclude_drops_matching_courses_from_pool() {
        let response = execute(
            TEST_YAML,
            vec!["CS:*".to_string()],
            vec!["CS400".to_string()],
        );
        assert!(response.success);
        let ids: Vec<&str> = response
            .matched
            .iter()
            .map(|m| m.course_id.as_str())
            .collect();
        assert!(ids.contains(&"CS300"));
        assert!(!ids.contains(&"CS400"), "CS400 should be excluded");
    }

    #[test]
    fn test_empty_pool_surfaces_note() {
        let response = execute(TEST_YAML, vec!["POLS:100+".to_string()], Vec::new());
        assert!(response.success);
        assert_eq!(response.match_count, 0);
        assert!(response.note.is_some());
    }

    #[test]
    fn test_no_patterns_returns_error() {
        let response = execute(TEST_YAML, Vec::new(), Vec::new());
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_parse_error_yaml_surfaces_in_error_field() {
        let response = execute("not: valid: yaml: {{", vec!["CS:*".to_string()], Vec::new());
        assert!(!response.success);
        assert!(response.error.is_some());
    }

    #[test]
    fn test_multiple_patterns_combined_with_exclude() {
        // Cross-subject pool (CS:* + MATH:*) minus two specific course keys.
        // Verifies that exclude applies to the union of include patterns and
        // that multiple include patterns merge rather than override.
        let response = execute(
            TEST_YAML,
            vec!["CS:*".to_string(), "MATH:*".to_string()],
            vec!["CS400".to_string(), "MATH156".to_string()],
        );
        assert!(response.success);
        let ids: Vec<&str> = response
            .matched
            .iter()
            .map(|m| m.course_id.as_str())
            .collect();
        // CS pool minus the excluded CS course; MATH pool excluded entirely.
        assert!(ids.contains(&"CS101"));
        assert!(ids.contains(&"CS200"));
        assert!(ids.contains(&"CS300"));
        assert!(!ids.contains(&"CS400"));
        assert!(!ids.contains(&"MATH156"));
        assert_eq!(response.match_count, ids.len());
    }

    #[test]
    fn test_matched_entries_carry_title_and_level() {
        let response = execute(TEST_YAML, vec!["CS:300+".to_string()], Vec::new());
        assert!(response.success);
        let cs300 = response
            .matched
            .iter()
            .find(|m| m.course_id == "CS300")
            .expect("CS300 must be in matched");
        assert_eq!(cs300.title.as_deref(), Some("Algorithms"));
        assert_eq!(cs300.prefix.as_deref(), Some("CS"));
        assert_eq!(cs300.level, Some(300));
    }
}
