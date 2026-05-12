//! Combined degree pipeline tool.
//!
//! Provides the `degree_pipeline` MCP tool that runs validate → audit →
//! analyze in a single call. Short-circuits on YAML parse failures so the
//! caller doesn't burn round-trips on a YAML that can't even be parsed.
//!
//! Composes the existing sub-tools (`validate::execute`, `audit::execute`,
//! `analyze::execute`) — no new core logic.

use crate::mcp::tools::{analyze, audit, validate};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for the `degree_pipeline` tool.
///
/// Accepts the standard yaml-source trio plus pass-through knobs for the
/// three sub-tools and per-stage skip flags so callers can stop at validate
/// (or validate + audit) when they don't need full analysis.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct DegreePipelineRequest {
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

    /// Forwarded to `validate_degree`: when true, patterns that match no
    /// enumerated courses become warnings instead of errors.
    #[schemars(
        description = "If true, validate treats unmatched patterns as warnings instead of errors. Default false."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub allow_unmatched_patterns: Option<bool>,

    /// Forwarded to `audit_degree`: minimum chain length to flag as deep.
    #[schemars(description = "Minimum prereq chain length to flag as deep (default 3)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub chain_threshold: Option<usize>,

    /// Forwarded to `analyze_degree`: cap on plans generated.
    #[schemars(description = "Maximum plans to generate (default 500)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub max_plans: Option<usize>,

    /// Forwarded to `analyze_degree`: comma-separated course codes that
    /// every generated plan must include.
    #[schemars(
        description = "Comma-separated course codes that every generated plan must include (e.g. \"CS150B,MATH156\")"
    )]
    pub include_courses: Option<String>,

    /// Skip the audit stage. The response will carry `audit: null`.
    /// Useful when the caller only wants validate + analyze.
    #[schemars(description = "Skip audit_degree (default false)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub skip_audit: Option<bool>,

    /// Skip the analyze stage. The response will carry `analyze: null`.
    /// Useful when the caller only wants validate (+ audit).
    #[schemars(description = "Skip analyze_degree (default false)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub skip_analyze: Option<bool>,
}

/// Combined response carrying each sub-tool's full output.
#[derive(Debug, Serialize)]
pub struct DegreePipelineResponse {
    /// True when the YAML parsed (validate did not return a `parse_error`).
    /// Sub-tool-level pass/fail still lives in each nested response — read
    /// `validate.is_valid`, `audit.passed`, `analyze.success` for that.
    pub success: bool,
    /// Parse error string when validate could not parse the YAML. When set,
    /// `audit` and `analyze` are guaranteed to be `null` (the pipeline
    /// short-circuits).
    pub parse_error: Option<String>,
    /// `validate_degree` output. Always populated.
    pub validate: validate::ValidationResponse,
    /// `audit_degree` output. `None` when `skip_audit=true` or when validate
    /// short-circuited on a parse error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub audit: Option<audit::AuditResponse>,
    /// `analyze_degree` output. `None` when `skip_analyze=true` or when
    /// validate short-circuited on a parse error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub analyze: Option<analyze::AnalysisResponse>,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `degree_pipeline` tool.
///
/// The argument count crosses clippy's default ceiling because every option
/// the sub-tools accept must flow through; grouping them into a struct would
/// just move the same data without reducing the caller's burden.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute(
    yaml_content: &str,
    allow_unmatched_patterns: bool,
    chain_threshold: Option<usize>,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    skip_audit: bool,
    skip_analyze: bool,
) -> DegreePipelineResponse {
    // Default to surfacing hidden-prereq warnings on the bundled validate
    // response so the pipeline caller sees them without an extra flag.
    let validate_response = validate::execute(yaml_content, allow_unmatched_patterns, true);

    // Short-circuit on a hard parse failure — there's no point running audit
    // or analyze against a YAML the parser already rejected.
    if let Some(parse_error) = validate_response.parse_error.clone() {
        return DegreePipelineResponse {
            success: false,
            parse_error: Some(parse_error),
            validate: validate_response,
            audit: None,
            analyze: None,
        };
    }

    let audit_response = if skip_audit {
        None
    } else {
        // Default to surfacing missing-intermediate prereq findings on the
        // bundled audit response so the pipeline caller sees them without an
        // extra flag.
        Some(audit::execute(yaml_content, chain_threshold, true))
    };

    let analyze_response = if skip_analyze {
        None
    } else {
        Some(analyze::execute(
            yaml_content,
            max_plans,
            include_courses,
            false,
            None,
            false,
            false,
            None,
        ))
    };

    DegreePipelineResponse {
        success: true,
        parse_error: None,
        validate: validate_response,
        audit: audit_response,
        analyze: analyze_response,
    }
}

/// Execute and serialize as JSON.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute_json(
    yaml_content: &str,
    allow_unmatched_patterns: bool,
    chain_threshold: Option<usize>,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    skip_audit: bool,
    skip_analyze: bool,
) -> String {
    let response = execute(
        yaml_content,
        allow_unmatched_patterns,
        chain_threshold,
        max_plans,
        include_courses,
        skip_audit,
        skip_analyze,
    );
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_YAML: &str = r#"
degree:
  id: test-degree
  institution: Test University
  program: Test Program
  total_credits: 16
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Introduction
    type: all
    category: major
    courses: [CS101, CS201]

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
    fn test_pipeline_runs_all_three_stages_on_valid_yaml() {
        let response = execute(TEST_YAML, false, None, Some(10), None, false, false);
        assert!(response.success);
        assert!(response.parse_error.is_none());
        assert!(response.validate.is_valid);
        let audit = response.audit.expect("audit must run by default");
        assert_eq!(audit.total_courses, 2);
        let analyze = response.analyze.expect("analyze must run by default");
        assert!(analyze.success);
        assert!(analyze.plans_analyzed > 0);
    }

    #[test]
    fn test_pipeline_short_circuits_on_parse_error() {
        let response = execute(
            "not: valid: yaml: {{",
            false,
            None,
            None,
            None,
            false,
            false,
        );
        assert!(!response.success);
        assert!(response.parse_error.is_some());
        assert!(
            response.audit.is_none(),
            "audit must be None when validate parse-errors"
        );
        assert!(
            response.analyze.is_none(),
            "analyze must be None when validate parse-errors"
        );
    }

    #[test]
    fn test_pipeline_skip_audit_returns_validate_and_analyze() {
        let response = execute(TEST_YAML, false, None, Some(10), None, true, false);
        assert!(response.success);
        assert!(response.audit.is_none());
        assert!(response.analyze.is_some());
    }

    #[test]
    fn test_pipeline_skip_analyze_returns_validate_and_audit() {
        let response = execute(TEST_YAML, false, None, None, None, false, true);
        assert!(response.success);
        assert!(response.audit.is_some());
        assert!(response.analyze.is_none());
    }

    #[test]
    fn test_pipeline_skip_both_returns_validate_only() {
        let response = execute(TEST_YAML, false, None, None, None, true, true);
        assert!(response.success);
        assert!(response.audit.is_none());
        assert!(response.analyze.is_none());
        assert!(response.validate.is_valid);
    }

    #[test]
    fn test_execute_json_serializes_with_expected_keys() {
        let json = execute_json(TEST_YAML, false, None, Some(10), None, false, false);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["success"].as_bool(), Some(true));
        assert!(parsed["validate"].is_object());
        assert!(parsed["audit"].is_object());
        assert!(parsed["analyze"].is_object());
    }
}
