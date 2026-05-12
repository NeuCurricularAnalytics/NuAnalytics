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
use crate::core::DegreeProgram;
use crate::mcp::tools::shared::{
    ToolFollowup, TOOL_ANALYZE_DEGREE, TOOL_GET_COURSE_DETAIL, TOOL_RENDER_PLAN_GRAPH,
    TOOL_VALIDATE_DEGREE,
};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request/Response Types
// ============================================================================

/// Request parameters for the `audit_degree` tool
///
/// Provide exactly one YAML source: `yaml_content` (inline), `yaml_path`
/// (workspace-relative file), or `degree_id` (stored in the database —
/// requires the `database` feature).
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct AuditDegreeRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Filesystem path the server will read. Mutually exclusive with the others.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored `degree_id` (DB lookup). Mutually exclusive with the others.
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

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
    /// Whether this course belongs to the degree's `major_subjects` set.
    /// `"internal_missing_prereq"` for courses in the program's own subjects
    /// (the actionable signal); `"external_missing_prereq"` for cross-listed
    /// or supporting-department courses that legitimately don't have CS
    /// prereqs (typically noise). `"unknown_scope"` when `major_subjects`
    /// is not declared.
    pub kind: &'static str,
}

/// One branch of a deep prerequisite chain. Lets callers consume the chain
/// as structured data instead of parsing the legacy `chain` string.
#[derive(Debug, Serialize)]
pub struct DeepChainBranch {
    /// Number of courses in this branch
    pub length: usize,
    /// Courses in this branch, in dependency order (leaf → immediate prereq)
    pub path: Vec<String>,
}

/// A course with a deep prerequisite chain
#[derive(Debug, Serialize)]
pub struct DeepChainInfo {
    /// Course key
    pub course: String,
    /// Maximum chain branch length
    pub max_depth: usize,
    /// Formatted branch lengths (e.g., "5, 3"). Kept for display; for
    /// programmatic use prefer `branches`.
    pub branch_lengths: String,
    /// Formatted chain representation. Kept for display; for programmatic
    /// use prefer `branches`.
    pub chain: String,
    /// Structured branches: each entry has `length` and the ordered course
    /// `path`. Mirrors the data behind `chain` / `branch_lengths`.
    pub branches: Vec<DeepChainBranch>,
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
    /// Structured hints about the next MCP call worth making, based on the
    /// audit outcome (deep chains → render the worst plan, etc.).
    pub tool_followups: Vec<ToolFollowup>,
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
                tool_followups: vec![ToolFollowup {
                    tool: TOOL_VALIDATE_DEGREE,
                    reason: "audit_degree couldn't parse the YAML; validate_degree surfaces the parse error in a more structured form.".to_string(),
                    suggested_args: serde_json::json!({}),
                }],
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
            .map(|(course, level)| MissingPrereqInfo {
                kind: classify_prereq_scope(&course, &program),
                course,
                level,
            })
            .collect();

    // Find deep prerequisite chains
    let deep_chains: Vec<DeepChainInfo> = find_deep_chains(&program, &graph_result, threshold)
        .into_iter()
        .map(|entry| {
            let max_depth = entry.branches.iter().map(Vec::len).max().unwrap_or(0);
            let branches = entry
                .branches
                .into_iter()
                .map(|path| DeepChainBranch {
                    length: path.len(),
                    path,
                })
                .collect();
            DeepChainInfo {
                course: entry.course,
                max_depth,
                branch_lengths: entry.branch_lengths,
                chain: entry.chain,
                branches,
            }
        })
        .collect();

    let passed =
        validation.errors.is_empty() && missing_prereqs.is_empty() && deep_chains.is_empty();

    let tool_followups = build_audit_followups(passed, &missing_prereqs, &deep_chains);

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
        tool_followups,
    }
}

/// Build follow-up suggestions for an audit response.
fn build_audit_followups(
    passed: bool,
    missing_prereqs: &[MissingPrereqInfo],
    deep_chains: &[DeepChainInfo],
) -> Vec<ToolFollowup> {
    let mut followups = Vec::new();
    if let Some(worst) = deep_chains.iter().max_by_key(|d| d.max_depth) {
        followups.push(ToolFollowup {
            tool: TOOL_RENDER_PLAN_GRAPH,
            reason: format!(
                "Deepest chain found on {} ({} steps); the longest path graph shows the chain in context.",
                worst.course, worst.max_depth,
            ),
            suggested_args: serde_json::json!({ "plan_category": "longest" }),
        });
    }
    let internal_missing = missing_prereqs
        .iter()
        .filter(|m| m.kind == "internal_missing_prereq")
        .count();
    if internal_missing > 0 {
        followups.push(ToolFollowup {
            tool: TOOL_GET_COURSE_DETAIL,
            reason: format!(
                "{internal_missing} internal upper-level course(s) lack prerequisites — get_course_detail returns each course's requirement references + dependents so you can decide whether to add them."
            ),
            suggested_args: serde_json::json!({}),
        });
    }
    if passed {
        followups.push(ToolFollowup {
            tool: TOOL_ANALYZE_DEGREE,
            reason: "Audit passed; analyze_degree computes plan-level metrics + selected plans for the report.".to_string(),
            suggested_args: serde_json::json!({}),
        });
    }
    followups
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
        DegreeParseError::YamlError { message, .. } => format!("YAML syntax error: {message}"),
    }
}

/// Tag a missing-prereq finding as internal (subject in `major_subjects`),
/// external (subject not in `major_subjects`), or unknown when the program
/// did not declare its `major_subjects`. Lets callers filter the noisy
/// external-department findings (e.g. cross-listed JTC/MGT courses that
/// legitimately don't carry CS prereqs) without losing internal coverage.
fn classify_prereq_scope(course_key: &str, program: &DegreeProgram) -> &'static str {
    let Some(subjects) = program.degree.major_subjects.as_ref() else {
        return "unknown_scope";
    };
    let digit_pos = course_key.find(|c: char| c.is_ascii_digit()).unwrap_or(0);
    if digit_pos == 0 {
        return "unknown_scope";
    }
    let prefix = &course_key[..digit_pos];
    if subjects.iter().any(|s| s.eq_ignore_ascii_case(prefix)) {
        "internal_missing_prereq"
    } else {
        "external_missing_prereq"
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
    fn test_classify_prereq_scope_uses_major_subjects() {
        // CS101 anchors the lowest level so CS300/MGT340 register as
        // upper-level missing-prereq findings.
        let yaml = r#"
degree:
  id: test
  institution: T
  program: T
  total_credits: 120
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS300, MGT340]

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
  MGT340:
    title: External
    prefix: MGT
    number: "340"
    credits: 3
"#;
        let response = execute(yaml, None);
        let cs = response
            .missing_prerequisites
            .iter()
            .find(|m| m.course == "CS300")
            .expect("CS300 should be in missing_prerequisites");
        let mgt = response
            .missing_prerequisites
            .iter()
            .find(|m| m.course == "MGT340")
            .expect("MGT340 should be in missing_prerequisites");
        assert_eq!(cs.kind, "internal_missing_prereq");
        assert_eq!(mgt.kind, "external_missing_prereq");
    }

    #[test]
    fn test_classify_prereq_scope_unknown_when_major_subjects_absent() {
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
        let response = execute(yaml, None);
        let cs = response
            .missing_prerequisites
            .iter()
            .find(|m| m.course == "CS300")
            .expect("CS300 should be in missing_prerequisites");
        assert_eq!(cs.kind, "unknown_scope");
    }

    #[test]
    fn test_deep_chain_branches_array_present() {
        // A 4-deep chain should appear in deep_chains (threshold defaults to 3)
        // with a structured branches array carrying the path.
        let yaml = r#"
degree:
  id: test
  institution: T
  program: T
  total_credits: 120
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS400]

courses:
  CS100:
    title: A
    prefix: CS
    number: "100"
    credits: 3
  CS200:
    title: B
    prefix: CS
    number: "200"
    credits: 3
    prerequisites_raw: "CS100"
  CS300:
    title: C
    prefix: CS
    number: "300"
    credits: 3
    prerequisites_raw: "CS200"
  CS400:
    title: D
    prefix: CS
    number: "400"
    credits: 3
    prerequisites_raw: "CS300"
"#;
        let response = execute(yaml, Some(3));
        let cs400 = response
            .deep_chains
            .iter()
            .find(|d| d.course == "CS400")
            .expect("CS400 should be flagged as a deep chain");
        assert!(
            !cs400.branches.is_empty(),
            "deep_chain entry must include a structured branches array"
        );
        let branch = &cs400.branches[0];
        assert_eq!(branch.length, branch.path.len());
        assert!(branch.length >= 3);
        assert!(
            branch.path.iter().any(|c| c == "CS100"),
            "path should include CS100 leaf"
        );
    }

    #[test]
    fn test_tool_followups_suggest_render_plan_graph_on_deep_chains() {
        // 4-deep chain CS100 → CS200 → CS300 → CS400 triggers the audit
        // deep-chain finder; the response should propose visualising it.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS400]

courses:
  CS100:
    title: A
    prefix: CS
    number: "100"
    credits: 3
  CS200:
    title: B
    prefix: CS
    number: "200"
    credits: 3
    prerequisites_raw: "CS100"
  CS300:
    title: C
    prefix: CS
    number: "300"
    credits: 3
    prerequisites_raw: "CS200"
  CS400:
    title: D
    prefix: CS
    number: "400"
    credits: 3
    prerequisites_raw: "CS300"
"#;
        let response = execute(yaml, Some(3));
        assert!(!response.deep_chains.is_empty());
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "render_plan_graph"),
            "deep chains must trigger a render_plan_graph followup; got {:?}",
            response.tool_followups
        );
    }

    #[test]
    fn test_tool_followups_suggest_course_detail_on_internal_missing_prereq() {
        // CS300 is upper-level + in major_subjects but declares no prereqs.
        // Audit tags it as internal_missing_prereq → followup should point at
        // get_course_detail so the caller can inspect requirement references.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 8
  gpa_minimum: 2.0
  major_subjects: ["CS"]

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
        let response = execute(yaml, None);
        assert!(response
            .missing_prerequisites
            .iter()
            .any(|m| m.kind == "internal_missing_prereq"));
        assert!(
            response
                .tool_followups
                .iter()
                .any(|f| f.tool == "get_course_detail"),
            "internal missing prereqs must trigger get_course_detail; got {:?}",
            response.tool_followups
        );
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
