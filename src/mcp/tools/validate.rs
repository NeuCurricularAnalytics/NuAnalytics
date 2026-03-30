//! Degree validation tool
//!
//! Provides the `validate_degree` MCP tool that validates degree YAML content
//! and returns structured feedback.

use crate::core::degree::{parse_degree_yaml, DegreeParseError};
use crate::core::{validate_degree_program, ValidationError, ValidationResult, ValidationWarning};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `validate_degree` tool
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ValidateDegreeRequest {
    /// The complete degree YAML content as a string
    #[schemars(description = "Complete degree program YAML content to validate")]
    pub yaml_content: String,
}

/// Structured error information returned by validation
#[derive(Debug, Serialize)]
pub struct ValidationErrorInfo {
    /// Error type identifier
    pub error_type: String,
    /// Human-readable error message
    pub message: String,
    /// Suggestion for fixing the error
    pub suggestion: Option<String>,
}

/// Structured warning information returned by validation
#[derive(Debug, Serialize)]
pub struct ValidationWarningInfo {
    /// Warning type identifier
    pub warning_type: String,
    /// Human-readable warning message
    pub message: String,
}

/// Context about the degree being validated
#[derive(Debug, Serialize)]
pub struct DegreeContext {
    /// Degree program name (if parseable)
    pub degree_name: Option<String>,
    /// Institution name (if parseable)
    pub institution: Option<String>,
    /// Total number of courses defined
    pub total_courses: usize,
    /// Total number of requirements defined
    pub total_requirements: usize,
    /// List of defined course keys
    pub defined_courses: Vec<String>,
    /// List of defined requirement IDs
    pub defined_requirements: Vec<String>,
}

/// Complete validation response
#[derive(Debug, Serialize)]
pub struct ValidationResponse {
    /// Whether the degree program is valid
    pub is_valid: bool,
    /// Parse error if YAML couldn't be parsed
    pub parse_error: Option<String>,
    /// List of validation errors
    pub errors: Vec<ValidationErrorInfo>,
    /// List of validation warnings
    pub warnings: Vec<ValidationWarningInfo>,
    /// Context about what's defined in the degree
    pub context: Option<DegreeContext>,
    /// General suggestions for improvement
    pub suggestions: Vec<String>,
}

// ============================================================================
// Tool Implementation
// ============================================================================

/// Execute the `validate_degree` tool
///
/// # Arguments
/// * `yaml_content` - The degree YAML content to validate
///
/// # Returns
/// Structured validation response
#[must_use]
pub fn execute(yaml_content: &str) -> ValidationResponse {
    // Try to parse the YAML
    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => {
            return ValidationResponse {
                is_valid: false,
                parse_error: Some(format_parse_error(&e)),
                errors: vec![],
                warnings: vec![],
                context: None,
                suggestions: vec![
                    "Fix the YAML syntax error first, then re-validate.".to_string(),
                    "Use get_degree_schema to review the expected format.".to_string(),
                ],
            };
        }
    };

    // Build context from successfully parsed program
    let mut defined_courses: Vec<String> = program.courses.keys().cloned().collect();
    defined_courses.sort();

    let mut defined_requirements: Vec<String> = program.requirements.keys().cloned().collect();
    defined_requirements.sort();

    let context = DegreeContext {
        degree_name: Some(program.degree.name.clone()),
        institution: program.degree.institution.clone(),
        total_courses: program.courses.len(),
        total_requirements: program.requirements.len(),
        defined_courses,
        defined_requirements,
    };

    // Run validation
    let result = validate_degree_program(&program);

    // Convert validation result to response format
    let errors = convert_validation_errors(&result);
    let warnings = convert_validation_warnings(&result);
    let suggestions = generate_suggestions(&result, &context);

    ValidationResponse {
        is_valid: result.is_valid,
        parse_error: None,
        errors,
        warnings,
        context: Some(context),
        suggestions,
    }
}

/// Execute and serialize the result as JSON
///
/// # Arguments
/// * `yaml_content` - The degree YAML content to validate
///
/// # Returns
/// JSON string representation of the validation response
#[must_use]
pub fn execute_json(yaml_content: &str) -> String {
    let response = execute(yaml_content);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helper Functions
// ============================================================================

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError(msg) => format!("YAML syntax error: {msg}"),
    }
}

fn convert_validation_errors(result: &ValidationResult) -> Vec<ValidationErrorInfo> {
    result
        .errors
        .iter()
        .map(|e| match e {
            ValidationError::CircularPrerequisite { cycle } => ValidationErrorInfo {
                error_type: "CircularPrerequisite".to_string(),
                message: format!("Circular prerequisite chain detected: {}", cycle.join(" → ")),
                suggestion: Some(
                    "Remove one of the prerequisites to break the cycle.".to_string(),
                ),
            },
            ValidationError::MissingCourse {
                course_key,
                requirement_id,
            } => ValidationErrorInfo {
                error_type: "MissingCourse".to_string(),
                message: format!(
                    "Course '{course_key}' referenced in requirement '{requirement_id}' but not defined"
                ),
                suggestion: Some(format!(
                    "Add '{course_key}' to the courses section, or remove it from requirement '{requirement_id}'."
                )),
            },
            ValidationError::PatternMatchesNoCourses {
                pattern,
                requirement_id,
            } => ValidationErrorInfo {
                error_type: "PatternMatchesNoCourses".to_string(),
                message: format!(
                    "Pattern '{pattern}' in requirement '{requirement_id}' doesn't match any courses"
                ),
                suggestion: Some(
                    "Check the pattern syntax or add courses that match the pattern.".to_string(),
                ),
            },
            ValidationError::InvalidPattern {
                pattern,
                reason,
                requirement_id,
            } => ValidationErrorInfo {
                error_type: "InvalidPattern".to_string(),
                message: format!(
                    "Invalid pattern '{pattern}' in requirement '{requirement_id}': {reason}"
                ),
                suggestion: Some("Use format like 'CS:3000+' or 'MATH:300-499'.".to_string()),
            },
            ValidationError::MissingPrerequisite {
                course_key,
                prerequisite_key,
            } => ValidationErrorInfo {
                error_type: "MissingPrerequisite".to_string(),
                message: format!(
                    "Course '{course_key}' lists prerequisite '{prerequisite_key}' which is not defined"
                ),
                suggestion: Some(format!(
                    "Add '{prerequisite_key}' to the courses section, or fix the prerequisite for '{course_key}'."
                )),
            },
            ValidationError::MissingCorequisite {
                course_key,
                corequisite_key,
            } => ValidationErrorInfo {
                error_type: "MissingCorequisite".to_string(),
                message: format!(
                    "Course '{course_key}' lists corequisite '{corequisite_key}' which is not defined"
                ),
                suggestion: Some(format!("Add '{corequisite_key}' to the courses section.")),
            },
            ValidationError::InvalidRequirement {
                requirement_id,
                reason,
            } => ValidationErrorInfo {
                error_type: "InvalidRequirement".to_string(),
                message: format!("Invalid requirement '{requirement_id}': {reason}"),
                suggestion: None,
            },
            ValidationError::UnidirectionalCrossListing {
                course_key,
                cross_listed_key,
            } => ValidationErrorInfo {
                error_type: "UnidirectionalCrossListing".to_string(),
                message: format!(
                    "Course '{course_key}' is cross-listed with '{cross_listed_key}', but '{cross_listed_key}' doesn't list it back"
                ),
                suggestion: Some(format!(
                    "Add 'cross_listed_as: [{course_key}]' to course '{cross_listed_key}'."
                )),
            },
        })
        .collect()
}

fn convert_validation_warnings(result: &ValidationResult) -> Vec<ValidationWarningInfo> {
    result
        .warnings
        .iter()
        .map(|w| match w {
            ValidationWarning::UnreferencedCourse { course_key } => ValidationWarningInfo {
                warning_type: "UnreferencedCourse".to_string(),
                message: format!(
                    "Course '{course_key}' is defined but never referenced in any requirement"
                ),
            },
            ValidationWarning::MissingCrossListedCourse {
                course_key,
                cross_listed_key,
            } => ValidationWarningInfo {
                warning_type: "MissingCrossListedCourse".to_string(),
                message: format!(
                    "Course '{course_key}' is cross-listed with '{cross_listed_key}', but '{cross_listed_key}' is not defined"
                ),
            },
            ValidationWarning::BroadPattern {
                pattern,
                requirement_id,
                match_count,
            } => ValidationWarningInfo {
                warning_type: "BroadPattern".to_string(),
                message: format!(
                    "Pattern '{pattern}' in requirement '{requirement_id}' matches {match_count} courses - consider being more specific"
                ),
            },
            ValidationWarning::IsolatedCourse { course_key } => ValidationWarningInfo {
                warning_type: "IsolatedCourse".to_string(),
                message: format!(
                    "Course '{course_key}' has no prerequisites and nothing depends on it"
                ),
            },
            ValidationWarning::HiddenRequirement {
                course_key,
                required_by,
                dependency_chain,
            } => ValidationWarningInfo {
                warning_type: "HiddenRequirement".to_string(),
                message: format!(
                    "Course '{course_key}' is implicitly required (prerequisite of '{required_by}' via chain: {})",
                    dependency_chain.join(" → ")
                ),
            },
            ValidationWarning::HiddenRequirementOption { options, required_by } => {
                ValidationWarningInfo {
                    warning_type: "HiddenRequirementOption".to_string(),
                    message: format!(
                        "Course '{required_by}' requires one of [{}], but none are listed in degree requirements",
                        options.join(", ")
                    ),
                }
            }
        })
        .collect()
}

fn generate_suggestions(result: &ValidationResult, context: &DegreeContext) -> Vec<String> {
    let mut suggestions = Vec::new();

    if result.is_valid {
        suggestions.push("✓ Degree program is valid!".to_string());
        if !result.warnings.is_empty() {
            suggestions.push(
                "Consider addressing the warnings above to improve the degree definition."
                    .to_string(),
            );
        }
    } else {
        suggestions.push("Fix the errors above and re-validate.".to_string());

        // Add specific suggestions based on error patterns
        let has_missing_courses = result
            .errors
            .iter()
            .any(|e| matches!(e, ValidationError::MissingCourse { .. }));
        if has_missing_courses {
            suggestions.push(format!(
                "Currently defined courses: {}",
                if context.defined_courses.len() > 10 {
                    format!(
                        "{} courses (use validate_degree to see full list)",
                        context.defined_courses.len()
                    )
                } else {
                    context.defined_courses.join(", ")
                }
            ));
        }
    }

    suggestions
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

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4
"#;

    const INVALID_YAML: &str = r#"
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
      - CS102

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4
"#;

    #[test]
    fn test_valid_degree() {
        let response = execute(VALID_YAML);
        assert!(
            response.is_valid,
            "Expected valid but got errors: {:?}",
            response.errors
        );
        assert!(response.parse_error.is_none());
        assert!(response.errors.is_empty());
        assert!(response.context.is_some());
    }

    #[test]
    fn test_invalid_degree_missing_course() {
        let response = execute(INVALID_YAML);
        assert!(!response.is_valid);
        assert!(
            response.parse_error.is_none(),
            "Got parse error: {:?}",
            response.parse_error
        );
        assert!(!response.errors.is_empty());
        assert!(response
            .errors
            .iter()
            .any(|e| e.error_type == "MissingCourse"));
    }

    #[test]
    fn test_malformed_yaml() {
        let response = execute("not: valid: yaml: {{");
        assert!(!response.is_valid);
        assert!(response.parse_error.is_some());
    }

    #[test]
    fn test_context_populated() {
        let response = execute(VALID_YAML);
        assert!(
            response.is_valid,
            "Expected valid but got errors: {:?}",
            response.errors
        );
        let context = response.context.unwrap();
        assert_eq!(context.total_courses, 1);
        assert_eq!(context.total_requirements, 1);
        assert!(context.defined_courses.contains(&"CS101".to_string()));
    }

    #[test]
    fn test_execute_json_returns_valid_json() {
        let json_str = execute_json(VALID_YAML);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        assert!(parsed.is_ok(), "Output should be valid JSON");
        let value = parsed.unwrap();
        assert!(value["is_valid"].as_bool().unwrap());
    }

    #[test]
    fn test_execute_json_with_errors() {
        let json_str = execute_json(INVALID_YAML);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        assert!(parsed.is_ok(), "Output should be valid JSON");
        let value = parsed.unwrap();
        assert!(!value["is_valid"].as_bool().unwrap());
        assert!(!value["errors"].as_array().unwrap().is_empty());
    }

    #[test]
    fn test_circular_prerequisite_detection() {
        let yaml = r#"
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

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4
    prerequisites_raw: "CS102"
  CS102:
    title: Data Structures
    prefix: CS
    number: "102"
    credits: 4
    prerequisites_raw: "CS101"
"#;
        let response = execute(yaml);
        assert!(!response.is_valid);
        assert!(response
            .errors
            .iter()
            .any(|e| e.error_type == "CircularPrerequisite"));
    }

    #[test]
    fn test_suggestions_for_valid_degree() {
        let response = execute(VALID_YAML);
        assert!(response.is_valid);
        assert!(response.suggestions.iter().any(|s| s.contains("valid")));
    }

    #[test]
    fn test_suggestions_for_invalid_degree() {
        let response = execute(INVALID_YAML);
        assert!(!response.is_valid);
        assert!(response.suggestions.iter().any(|s| s.contains("Fix")));
    }
}
