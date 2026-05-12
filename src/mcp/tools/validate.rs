//! Degree validation tool
//!
//! Provides the `validate_degree` MCP tool that validates degree YAML content
//! and returns structured feedback.

use crate::core::degree::audit::{detect_lowest_course_level, find_upper_level_without_prereqs};
use crate::core::degree::{parse_degree_yaml, DegreeParseError, RequirementResolver};
use crate::core::models::degree::{FromClause, Requirement, RequirementType};
use crate::core::models::CourseGraph;
use crate::core::{
    validate_degree_program_with_options, DegreeProgram, ValidationError, ValidationOptions,
    ValidationResult, ValidationWarning,
};
use crate::mcp::tools::shared::{
    format_yaml_context, ToolFollowup, TOOL_ANALYZE_DEGREE, TOOL_AUDIT_DEGREE,
    TOOL_GET_DEGREE_SCHEMA,
};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `validate_degree` tool
///
/// Provide exactly one YAML source: `yaml_content` (inline string),
/// `yaml_path` (workspace-relative file), or `degree_id` (stored in the
/// database — requires the `database` feature). Inline content avoids
/// re-pasting the whole YAML on every call once it's stored.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ValidateDegreeRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Filesystem path (workspace-relative) to a YAML file the server will read.
    /// Mutually exclusive with `yaml_content` / `degree_id`.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored `degree_id` (from `store_degree`). Looked up via the database;
    /// the same YAML is then validated. Mutually exclusive with the others.
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

    /// If true, patterns that match no enumerated courses (e.g. external
    /// gen-ed pools like `*:100+`, `POLS:100+`) become warnings instead of
    /// errors. Default false (strict validation).
    #[schemars(
        description = "If true, patterns with no matching courses become warnings instead of errors. Default false."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub allow_unmatched_patterns: Option<bool>,
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
    /// Parse error message when the YAML couldn't be parsed.
    pub parse_error: Option<String>,
    /// 1-indexed line of the parse error, pulled from `serde_yaml::Location`
    /// when available. `None` for non-positional errors (e.g. serialise failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_line: Option<usize>,
    /// 1-indexed column of the parse error, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_column: Option<usize>,
    /// ±3 source-line context window around the parse error with a caret
    /// pointing at the column. Lets callers see the offending statement
    /// without re-opening the YAML.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_context: Option<String>,
    /// List of validation errors
    pub errors: Vec<ValidationErrorInfo>,
    /// List of validation warnings
    pub warnings: Vec<ValidationWarningInfo>,
    /// Context about what's defined in the degree
    pub context: Option<DegreeContext>,
    /// Pool resolution for every requirement that uses a `from` clause.
    /// Lets callers spot patterns that silently match nothing or that resolve
    /// to a smaller set than expected.
    pub resolved_pools: Vec<ResolvedPoolInfo>,
    /// General suggestions for improvement
    pub suggestions: Vec<String>,
    /// Structured hints about the next MCP call worth making, based on the
    /// validation outcome (e.g. unprereqed upper-level courses → run audit).
    pub tool_followups: Vec<ToolFollowup>,
}

/// One requirement's resolved selection pool, surfaced so the caller can
/// confirm patterns matched the expected courses.
#[derive(Debug, Serialize)]
pub struct ResolvedPoolInfo {
    /// Requirement identifier (or dotted path for nested options)
    pub requirement_id: String,
    /// Requirement type (`"select"` or `"one_of"`)
    pub requirement_type: &'static str,
    /// Number of courses matched by the requirement's `from` clause after
    /// applying include/exclude patterns
    pub pool_size: usize,
    /// Up to 10 sample course keys from the resolved pool, alphabetised
    pub sample: Vec<String>,
    /// Set when no courses match the patterns — the requirement is unsatisfiable
    /// as written.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<&'static str>,
}

// ============================================================================
// Tool Implementation
// ============================================================================

/// Execute the `validate_degree` tool
///
/// # Arguments
/// * `yaml_content` - The degree YAML content to validate
/// * `allow_unmatched_patterns` - If true, surface unmatched-pattern errors
///   as warnings instead of errors (for YAMLs that intentionally reference
///   external gen-ed pools)
///
/// # Returns
/// Structured validation response
#[must_use]
pub fn execute(yaml_content: &str, allow_unmatched_patterns: bool) -> ValidationResponse {
    // Try to parse the YAML
    let program = match parse_degree_yaml(yaml_content) {
        Ok(p) => p,
        Err(e) => {
            let (line, column) = match &e {
                DegreeParseError::YamlError { line, column, .. } => (*line, *column),
                DegreeParseError::IoError(_) => (None, None),
            };
            let context = match (line, column) {
                (Some(l), Some(c)) => Some(format_yaml_context(yaml_content, l, c)),
                _ => None,
            };
            return ValidationResponse {
                is_valid: false,
                parse_error: Some(format_parse_error(&e)),
                parse_error_line: line,
                parse_error_column: column,
                parse_error_context: context,
                errors: vec![],
                warnings: vec![],
                context: None,
                resolved_pools: vec![],
                suggestions: vec![
                    "Fix the YAML syntax error first, then re-validate.".to_string(),
                    "Use get_degree_schema to review the expected format.".to_string(),
                ],
                tool_followups: vec![ToolFollowup {
                    tool: TOOL_GET_DEGREE_SCHEMA,
                    reason: "YAML parse error; review the schema before retrying.".to_string(),
                    suggested_args: serde_json::json!({ "section": "quickstart" }),
                }],
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
    let opts = ValidationOptions {
        allow_unmatched_patterns,
    };
    let result = validate_degree_program_with_options(&program, opts);

    // Cheap hint surface: count upper-level courses lacking prereqs, so the
    // caller knows whether to spend an audit_degree round-trip. Audit remains
    // authoritative for the actual list and chain analysis.
    let unprereqed_upper_level = count_upper_level_missing_prereqs(&program);

    // Resolve every `from` clause so the caller can see which courses each
    // pattern actually matched — catches `exclude` patterns that drop nothing
    // and `include` patterns too narrow to be useful.
    let resolved_pools = collect_resolved_pools(&program);

    // Convert validation result to response format
    let errors = convert_validation_errors(&result);
    let warnings = convert_validation_warnings(&result);
    let suggestions = generate_suggestions(&result, &context, unprereqed_upper_level);

    let tool_followups = build_followups(&result, unprereqed_upper_level);

    ValidationResponse {
        is_valid: result.is_valid,
        parse_error: None,
        parse_error_line: None,
        parse_error_column: None,
        parse_error_context: None,
        errors,
        warnings,
        context: Some(context),
        resolved_pools,
        suggestions,
        tool_followups,
    }
}

/// Build follow-up suggestions for a validate response.
fn build_followups(result: &ValidationResult, unprereqed_upper_level: usize) -> Vec<ToolFollowup> {
    let mut followups = Vec::new();
    if unprereqed_upper_level > 0 {
        followups.push(ToolFollowup {
            tool: TOOL_AUDIT_DEGREE,
            reason: format!(
                "{unprereqed_upper_level} upper-level course(s) declare no prerequisites — audit_degree surfaces the list and finds implicit-requirement issues."
            ),
            suggested_args: serde_json::json!({}),
        });
    }
    if result.is_valid && result.errors.is_empty() {
        followups.push(ToolFollowup {
            tool: TOOL_ANALYZE_DEGREE,
            reason: "Validation passed; run analyze_degree to compute plan-level metrics and selected plans.".to_string(),
            suggested_args: serde_json::json!({}),
        });
    }
    followups
}

/// Execute and serialize the result as JSON
///
/// # Arguments
/// * `yaml_content` - The degree YAML content to validate
/// * `allow_unmatched_patterns` - If true, surface unmatched-pattern errors
///   as warnings (see `execute`)
///
/// # Returns
/// JSON string representation of the validation response
#[must_use]
pub fn execute_json(yaml_content: &str, allow_unmatched_patterns: bool) -> String {
    let response = execute(yaml_content, allow_unmatched_patterns);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helper Functions
// ============================================================================

fn format_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError {
            message,
            line,
            column,
        } => {
            // Prefix the structured location ahead of the raw message so the
            // human-readable string still carries the position info — handy
            // for log scraping and for clients that don't parse the JSON.
            match (line, column) {
                (Some(l), Some(c)) => {
                    format!("YAML syntax error at line {l} column {c}: {message}")
                }
                _ => format!("YAML syntax error: {message}"),
            }
        }
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
            ValidationWarning::PatternMatchesNoCoursesAllowed {
                pattern,
                requirement_id,
            } => ValidationWarningInfo {
                warning_type: "PatternMatchesNoCoursesAllowed".to_string(),
                message: format!(
                    "Pattern '{pattern}' in requirement '{requirement_id}' matches no enumerated courses (allowed via allow_unmatched_patterns)"
                ),
            },
        })
        .collect()
}

fn generate_suggestions(
    result: &ValidationResult,
    context: &DegreeContext,
    unprereqed_upper_level: usize,
) -> Vec<String> {
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

    if unprereqed_upper_level > 0 {
        suggestions.push(format!(
            "Note: {unprereqed_upper_level} upper-level course(s) declare no prerequisites; run audit_degree for the list and to surface implicit-requirement issues."
        ));
    }

    suggestions
}

/// Count upper-level courses that declare no prerequisites.
///
/// Mirrors the `audit_degree` probe but returns only the count, used to emit
/// a one-line hint in validate's suggestions without forcing a separate call.
fn count_upper_level_missing_prereqs(program: &DegreeProgram) -> usize {
    let graph_result = CourseGraph::from_degree_program(program);
    let lowest_level = detect_lowest_course_level(program);
    find_upper_level_without_prereqs(&graph_result, lowest_level).len()
}

/// Walk every requirement (including nested `one_of` options) and resolve the
/// `from` clause to its pool of matching courses.
fn collect_resolved_pools(program: &DegreeProgram) -> Vec<ResolvedPoolInfo> {
    let mut resolver = RequirementResolver::new(&program.courses);
    let mut pools = Vec::new();
    for (id, req) in &program.requirements {
        collect_pools_from_req(id, req, &mut resolver, &mut pools);
    }
    // Stable order so callers don't depend on HashMap iteration.
    pools.sort_by(|a, b| a.requirement_id.cmp(&b.requirement_id));
    pools
}

/// Recurse into a requirement, emitting one [`ResolvedPoolInfo`] per `from`
/// clause encountered (including those inside `one_of` options).
fn collect_pools_from_req(
    id: &str,
    req: &Requirement,
    resolver: &mut RequirementResolver<'_>,
    out: &mut Vec<ResolvedPoolInfo>,
) {
    if let Some(from) = &req.from {
        if matches!(
            req.req_type,
            RequirementType::Select | RequirementType::OneOf
        ) {
            out.push(build_pool_info(id, &req.req_type, from, resolver));
        }
    }
    if let Some(options) = &req.options {
        for option in options {
            for nested in &option.requirements {
                let base = format!("{id}.{}", option.id);
                let nested_id = nested
                    .name
                    .as_deref()
                    .map_or_else(|| base.clone(), |n| format!("{base}:{n}"));
                collect_pools_from_req(&nested_id, nested, resolver, out);
            }
        }
    }
}

fn build_pool_info(
    id: &str,
    req_type: &RequirementType,
    from: &FromClause,
    resolver: &mut RequirementResolver<'_>,
) -> ResolvedPoolInfo {
    let pool = resolver.resolve_pool(from);
    let warning = if pool.is_empty() {
        Some("Resolved pool is empty — patterns matched no enumerated courses")
    } else {
        None
    };
    let mut sample: Vec<String> = pool.iter().take(10).cloned().collect();
    sample.sort();
    ResolvedPoolInfo {
        requirement_id: id.to_string(),
        requirement_type: requirement_type_label(req_type),
        pool_size: pool.len(),
        sample,
        warning,
    }
}

const fn requirement_type_label(req_type: &RequirementType) -> &'static str {
    match req_type {
        RequirementType::All => "all",
        RequirementType::Select => "select",
        RequirementType::OneOf => "one_of",
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
        let response = execute(VALID_YAML, false);
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
        let response = execute(INVALID_YAML, false);
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
        let response = execute("not: valid: yaml: {{", false);
        assert!(!response.is_valid);
        assert!(response.parse_error.is_some());
    }

    #[test]
    fn test_parse_error_surfaces_structured_line_column_and_context() {
        // Construct a YAML where line 3 has a clearly broken mapping the
        // serde_yaml parser will pin to that line — used to confirm we
        // surface line, column, and a ±3-line context window with a caret.
        let yaml =
            "degree:\n  id: t\n  total_credits: [not, a, number]\nrequirements: {}\ncourses: {}\n";
        let response = execute(yaml, false);
        assert!(!response.is_valid);
        assert!(response.parse_error.is_some(), "expected a parse error");
        assert!(
            response.parse_error_line.is_some(),
            "parse_error_line must be populated; got response: {response:?}"
        );
        assert!(
            response.parse_error_column.is_some(),
            "parse_error_column must be populated"
        );
        let context = response
            .parse_error_context
            .as_deref()
            .expect("parse_error_context must be populated when line+column are known");
        assert!(
            context.contains("^ here"),
            "context must carry a caret marker; got:\n{context}"
        );
        assert!(
            context.lines().count() >= 2,
            "context should span multiple lines, got:\n{context}"
        );
        // The structured message should also surface the location for log scrapers.
        let msg = response.parse_error.as_deref().unwrap();
        assert!(
            msg.contains("line"),
            "parse_error message should mention 'line': got {msg:?}"
        );
    }

    #[test]
    fn test_context_populated() {
        let response = execute(VALID_YAML, false);
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
        let json_str = execute_json(VALID_YAML, false);
        let parsed: Result<serde_json::Value, _> = serde_json::from_str(&json_str);
        assert!(parsed.is_ok(), "Output should be valid JSON");
        let value = parsed.unwrap();
        assert!(value["is_valid"].as_bool().unwrap());
    }

    #[test]
    fn test_execute_json_with_errors() {
        let json_str = execute_json(INVALID_YAML, false);
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
        let response = execute(yaml, false);
        assert!(!response.is_valid);
        assert!(response
            .errors
            .iter()
            .any(|e| e.error_type == "CircularPrerequisite"));
    }

    #[test]
    fn test_suggestions_for_valid_degree() {
        let response = execute(VALID_YAML, false);
        assert!(response.is_valid);
        assert!(response.suggestions.iter().any(|s| s.contains("valid")));
    }

    #[test]
    fn test_suggestions_for_invalid_degree() {
        let response = execute(INVALID_YAML, false);
        assert!(!response.is_valid);
        assert!(response.suggestions.iter().any(|s| s.contains("Fix")));
    }

    /// YAML with an external gen-ed pool pattern (`POLS:100+`) that matches
    /// no enumerated courses. Used by both flag-state tests below.
    const EXTERNAL_POOL_YAML: &str = r#"
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
    courses: [CS101]

  external_geneds:
    name: Political Science Gen-Ed
    type: select
    category: gened
    count: 1
    from:
      pattern: "POLS:100+"

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4
"#;

    #[test]
    fn test_unmatched_pattern_is_error_by_default() {
        let response = execute(EXTERNAL_POOL_YAML, false);
        assert!(
            !response.is_valid,
            "POLS:100+ should fail strict validation when no POLS courses exist"
        );
        assert!(
            response
                .errors
                .iter()
                .any(|e| e.error_type == "PatternMatchesNoCourses"),
            "expected PatternMatchesNoCourses error, got {:?}",
            response.errors
        );
    }

    #[test]
    fn test_resolved_pools_lists_select_requirements() {
        let yaml = r#"
degree:
  id: test
  institution: T
  program: T
  total_credits: 120
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101]
  cs_electives:
    name: CS Electives
    type: select
    category: major
    count: 1
    from:
      pattern: "CS:300+"

courses:
  CS101:
    title: A
    prefix: CS
    number: "101"
    credits: 4
  CS300:
    title: B
    prefix: CS
    number: "300"
    credits: 4
    prerequisites_raw: "CS101"
  CS400:
    title: C
    prefix: CS
    number: "400"
    credits: 4
    prerequisites_raw: "CS300"
"#;
        let response = execute(yaml, false);
        let pool = response
            .resolved_pools
            .iter()
            .find(|p| p.requirement_id == "cs_electives")
            .expect("cs_electives should be resolved");
        assert_eq!(pool.requirement_type, "select");
        assert!(
            pool.pool_size >= 2,
            "CS:300+ should match at least CS300 and CS400"
        );
        assert!(pool.warning.is_none(), "non-empty pool should not warn");
    }

    #[test]
    fn test_resolved_pools_warns_when_pattern_matches_nothing() {
        // POLS:100+ matches nothing in this YAML — caller wants a single
        // surface to spot that without scanning every error type.
        let response = execute(EXTERNAL_POOL_YAML, true);
        let pool = response
            .resolved_pools
            .iter()
            .find(|p| p.requirement_id == "external_geneds")
            .expect("external_geneds should be in resolved_pools");
        assert_eq!(pool.pool_size, 0);
        assert!(
            pool.warning.is_some(),
            "empty pool must surface a warning string"
        );
    }

    #[test]
    fn test_tool_followups_suggest_audit_when_upper_level_lacks_prereqs() {
        // CS101 anchors the lowest level; CS300 is upper-level without any
        // declared prerequisites — triggers validate's prereq-coverage hint.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 8
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS300]

courses:
  CS101:
    title: Intro CS
    prefix: CS
    number: "101"
    credits: 4
  CS300:
    title: Upper CS
    prefix: CS
    number: "300"
    credits: 4
"#;
        let response = execute(yaml, false);
        assert!(response.is_valid);
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "audit_degree"),
            "unprereqed upper-level course must trigger an audit_degree followup; got {:?}",
            response.tool_followups
        );
    }

    #[test]
    fn test_tool_followups_suggest_analyze_when_validation_passes_clean() {
        let response = execute(VALID_YAML, false);
        assert!(response.is_valid);
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "analyze_degree"),
            "valid YAML must suggest analyze_degree; got {:?}",
            response.tool_followups
        );
    }

    #[test]
    fn test_tool_followups_suggest_schema_on_parse_error() {
        let response = execute("not: valid: yaml: {{", false);
        assert!(!response.is_valid);
        assert!(response.parse_error.is_some());
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "get_degree_schema"),
            "parse-error response must point at get_degree_schema; got {:?}",
            response.tool_followups
        );
    }

    #[test]
    fn test_unmatched_pattern_is_warning_when_allowed() {
        let response = execute(EXTERNAL_POOL_YAML, true);
        assert!(
            response.is_valid,
            "allow_unmatched_patterns=true must keep validation valid; errors: {:?}",
            response.errors
        );
        // The previous error must NOT appear
        assert!(
            !response
                .errors
                .iter()
                .any(|e| e.error_type == "PatternMatchesNoCourses"),
            "PatternMatchesNoCourses must not be raised when flag is on"
        );
        // ...but a corresponding warning should
        assert!(
            response
                .warnings
                .iter()
                .any(|w| w.warning_type == "PatternMatchesNoCoursesAllowed"),
            "expected PatternMatchesNoCoursesAllowed warning, got {:?}",
            response.warnings
        );
    }
}
