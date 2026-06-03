//! Degree trim tool
//!
//! Exposes [`crate::core::degree::trim_program`] over MCP. The trimmed YAML
//! is returned inline; an optional `output_path` writes it to disk. Every
//! successful call also stores the trimmed body in the process-wide
//! [`crate::mcp::cache::YAML_CACHE`] and surfaces its handle as
//! `trimmed_cache_id` so the caller can chain `validate_degree` /
//! `audit_degree` against the result without re-serialising the YAML.

use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::core::degree::{
    parse_degree_auto, save_degree_to_yaml, serialize_degree_yaml, trim_program, DegreeParseError,
    TrimOptions, TrimReport,
};
use crate::mcp::cache::YAML_CACHE;
use crate::mcp::tools::shared::{
    format_degree_parse_error, format_yaml_context, ToolFollowup, TOOL_AUDIT_DEGREE,
    TOOL_VALIDATE_DEGREE,
};

// ============================================================================
// Request / Response Types
// ============================================================================

/// Request parameters for the `trim_degree` tool.
///
/// Provide exactly one YAML source: `yaml_content`, `yaml_path`, or
/// `degree_id` (the latter accepts `cache:<hash>` handles from prior tool
/// calls). The trim semantics match the CLI's `degree trim` subcommand —
/// alternatives collapse to a single shortest entry path, except inside
/// protected subjects.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TrimDegreeRequest {
    /// Inline YAML body. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Workspace-relative file path. Mutually exclusive with the others.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored degree id (DB or `cache:<hash>` handle). Mutually exclusive.
    #[schemars(
        description = "Stored degree id (cache:<hash> handle or DB row). Mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

    /// Extra subject prefixes to protect from trimming, in addition to the
    /// degree's declared `major_subjects`. Case-insensitive.
    #[schemars(
        description = "Subject prefixes to protect in addition to the degree's `major_subjects`. Case-insensitive."
    )]
    pub keep_all: Option<Vec<String>>,

    /// Course keys to pin as winners at any choice point listing them.
    /// Overrides both the shortest-path metric and the prefer-protected rule.
    #[schemars(
        description = "Course keys to pin as winners at any choice point that lists them. Overrides the shortest-path metric."
    )]
    pub include: Option<Vec<String>>,

    /// Optional disk write. Primary output is always inline; this just adds a
    /// side-effect file when set. The handler refuses to overwrite a
    /// `yaml_path` input.
    #[schemars(
        description = "Optional path to write the trimmed YAML to. The trimmed content is also returned inline regardless."
    )]
    pub output_path: Option<String>,
}

/// Summary of what the trim did. Mirrors [`TrimReport`] in a serializable
/// shape ready to ship over the wire.
#[derive(Debug, Serialize)]
pub struct TrimReportInfo {
    /// Subject prefixes that were treated as protected.
    pub protected_subjects: Vec<String>,
    /// `true` when `protected_subjects` was derived from requirement content
    /// because the source YAML omitted `major_subjects`.
    pub protected_subjects_derived: bool,
    /// Course keys removed from `program.courses` because no remaining
    /// requirement or prereq references them.
    pub orphan_courses_removed: Vec<String>,
}

impl From<TrimReport> for TrimReportInfo {
    fn from(r: TrimReport) -> Self {
        Self {
            protected_subjects: r.protected_subjects,
            protected_subjects_derived: r.protected_subjects_derived,
            orphan_courses_removed: r.orphan_courses_removed,
        }
    }
}

/// Response body for the `trim_degree` tool.
#[derive(Debug, Serialize)]
pub struct TrimResponse {
    /// Whether the trim succeeded. `false` for parse errors, refused
    /// overwrites, and I/O failures on the optional disk write.
    pub success: bool,

    /// YAML parse error message, populated when the input failed to load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error: Option<String>,
    /// 1-indexed line of the parse error, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_line: Option<usize>,
    /// 1-indexed column of the parse error, when available.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_column: Option<usize>,
    /// ±3 source-line context window around the parse error with a caret
    /// pointing at the column.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parse_error_context: Option<String>,

    /// Non-parse error message (e.g. refused overwrite, write I/O failure).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,

    /// Trimmed YAML serialised back from the modified program. Present
    /// whenever `success == true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trimmed_yaml: Option<String>,

    /// `cache:<hash>` handle for the trimmed YAML. Pass as `degree_id` to
    /// any follow-up tool to avoid re-pasting the body. Always issued on a
    /// successful trim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trimmed_cache_id: Option<String>,

    /// Side-effect path actually written to disk, when `output_path` was set
    /// and the write succeeded.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output_path: Option<String>,

    /// Structured summary of the trim's protected-subject decision and the
    /// orphans it pruned.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub report: Option<TrimReportInfo>,

    /// Hints about the next MCP call worth making.
    pub tool_followups: Vec<ToolFollowup>,
}

// ============================================================================
// Tool implementation
// ============================================================================

/// Trim a degree YAML and assemble a structured response. Pure function —
/// the only side effects are an optional disk write and a YAML-cache
/// insertion, both of which the caller opts into.
#[must_use]
pub fn execute(
    yaml_content: &str,
    keep_all: &[String],
    include: &[String],
    output_path: Option<&str>,
    source_path: Option<&str>,
) -> TrimResponse {
    let program = match parse_degree_auto(yaml_content) {
        Ok((p, _warnings)) => p,
        Err(e) => return parse_error_response(&e, yaml_content),
    };

    let opts = TrimOptions {
        keep_all_subjects: keep_all.iter().map(|s| s.to_uppercase()).collect(),
        include_courses: include.iter().cloned().collect::<HashSet<_>>(),
    };
    let (trimmed, report) = trim_program(&program, &opts);

    let yaml = match serialize_degree_yaml(&trimmed) {
        Ok(s) => s,
        Err(e) => {
            return TrimResponse {
                success: false,
                error: Some(format!("Failed to serialise trimmed YAML: {e}")),
                ..empty_response()
            };
        }
    };

    // Optional side-effect: write the trimmed body to disk. We refuse to
    // overwrite the input file even when the caller asked us to, to keep
    // accidental destruction out of the MCP surface (mirrors the CLI guard).
    let written_path = match output_path {
        None => None,
        Some(out) => {
            if let Some(src) = source_path {
                if std::path::Path::new(out) == std::path::Path::new(src) {
                    return TrimResponse {
                        success: false,
                        error: Some(format!(
                            "refusing to overwrite input file {out}; choose a different output_path"
                        )),
                        ..empty_response()
                    };
                }
            }
            if let Err(e) = save_degree_to_yaml(&trimmed, out) {
                return TrimResponse {
                    success: false,
                    error: Some(format!("Failed to write {out}: {e}")),
                    ..empty_response()
                };
            }
            Some(out.to_string())
        }
    };

    let cache_id = YAML_CACHE.lock().map(|mut c| c.insert(yaml.clone())).ok();
    let followups = build_followups(cache_id.as_deref());

    TrimResponse {
        success: true,
        parse_error: None,
        parse_error_line: None,
        parse_error_column: None,
        parse_error_context: None,
        error: None,
        trimmed_yaml: Some(yaml),
        trimmed_cache_id: cache_id,
        output_path: written_path,
        report: Some(report.into()),
        tool_followups: followups,
    }
}

/// Execute and JSON-serialise the response. Wired up by `server.rs`.
#[must_use]
pub fn execute_json(
    yaml_content: &str,
    keep_all: &[String],
    include: &[String],
    output_path: Option<&str>,
    source_path: Option<&str>,
) -> String {
    let response = execute(yaml_content, keep_all, include, output_path, source_path);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

/// Skeleton response — all fields zeroed except the ones the caller will set.
/// Centralised so the various error branches don't drift apart.
const fn empty_response() -> TrimResponse {
    TrimResponse {
        success: false,
        parse_error: None,
        parse_error_line: None,
        parse_error_column: None,
        parse_error_context: None,
        error: None,
        trimmed_yaml: None,
        trimmed_cache_id: None,
        output_path: None,
        report: None,
        tool_followups: Vec::new(),
    }
}

fn parse_error_response(e: &DegreeParseError, yaml: &str) -> TrimResponse {
    let (line, column) = match e {
        DegreeParseError::YamlError { line, column, .. } => (*line, *column),
        DegreeParseError::IoError(_) | DegreeParseError::JsonError(_) => (None, None),
    };
    let context = match (line, column) {
        (Some(l), Some(c)) => Some(format_yaml_context(yaml, l, c)),
        _ => None,
    };
    TrimResponse {
        success: false,
        parse_error: Some(format_degree_parse_error(e)),
        parse_error_line: line,
        parse_error_column: column,
        parse_error_context: context,
        ..empty_response()
    }
}

fn build_followups(trimmed_cache_id: Option<&str>) -> Vec<ToolFollowup> {
    let Some(cache_id) = trimmed_cache_id else {
        return Vec::new();
    };
    vec![
        ToolFollowup {
            tool: TOOL_VALIDATE_DEGREE,
            reason:
                "Confirm the trimmed YAML still validates — comments are dropped on serialisation."
                    .to_string(),
            suggested_args: serde_json::json!({ "degree_id": cache_id }),
        },
        ToolFollowup {
            tool: TOOL_AUDIT_DEGREE,
            reason: "Audit the trimmed plan for hidden prereqs / deep chains after collapse."
                .to_string(),
            suggested_args: serde_json::json!({ "degree_id": cache_id }),
        },
    ]
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_YAML: &str = r#"
degree:
  name: Test BS
  id: test
  institution: Test U
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  allow_double_counting: false
  major_subjects: [CS]

requirements:
  core:
    type: all
    courses:
      - CS300
      - "{MATH215, MATH241}"

courses:
  CS300:
    title: Programming
    prefix: CS
    number: "300"
    credits: 4
    prerequisites: "MATH241"
  MATH215:
    title: Applied Calculus
    prefix: MATH
    number: "215"
    credits: 4
  MATH241:
    title: Calculus I
    prefix: MATH
    number: "241"
    credits: 4
"#;

    #[test]
    fn trim_happy_path_returns_yaml_report_and_cache_id() {
        let response = execute(SAMPLE_YAML, &[], &[], None, None);
        assert!(
            response.success,
            "error: {:?} | parse_error: {:?}",
            response.error, response.parse_error
        );
        assert!(response.parse_error.is_none());
        let yaml = response
            .trimmed_yaml
            .as_ref()
            .expect("trimmed_yaml must be present on success");
        assert!(yaml.contains("MATH215"), "MATH215 must survive: {yaml}");
        assert!(
            !yaml.contains("MATH241"),
            "MATH241 must be pruned via the equivalents collapse: {yaml}"
        );
        let cache_id = response
            .trimmed_cache_id
            .as_ref()
            .expect("a successful trim must publish a cache handle");
        assert!(cache_id.starts_with("cache:"));
        let report = response.report.as_ref().expect("report present");
        assert!(report.protected_subjects.contains(&"CS".to_string()));
        assert!(report
            .orphan_courses_removed
            .contains(&"MATH241".to_string()));
    }

    #[test]
    fn trim_followups_target_the_trimmed_cache_handle() {
        // The whole point of `trimmed_cache_id` is to let the model chain
        // validate/audit without re-pasting the trimmed YAML; verify the
        // suggested args carry the freshly-issued handle.
        let response = execute(SAMPLE_YAML, &[], &[], None, None);
        let cache_id = response.trimmed_cache_id.as_ref().unwrap();
        let tools: Vec<&str> = response.tool_followups.iter().map(|f| f.tool).collect();
        assert_eq!(tools, vec![TOOL_VALIDATE_DEGREE, TOOL_AUDIT_DEGREE]);
        for f in &response.tool_followups {
            assert_eq!(
                f.suggested_args["degree_id"].as_str(),
                Some(cache_id.as_str())
            );
        }
    }

    #[test]
    fn trim_keep_all_preserves_extra_subject() {
        let response = execute(SAMPLE_YAML, &["MATH".to_string()], &[], None, None);
        assert!(response.success);
        let yaml = response.trimmed_yaml.unwrap();
        assert!(
            yaml.contains("MATH215") && yaml.contains("MATH241"),
            "both MATH equivalents should survive with --keep-all MATH: {yaml}"
        );
    }

    #[test]
    fn trim_include_overrides_default_canonical() {
        let response = execute(SAMPLE_YAML, &[], &["MATH241".to_string()], None, None);
        assert!(response.success);
        let yaml = response.trimmed_yaml.unwrap();
        assert!(
            yaml.contains("MATH241") && !yaml.contains("MATH215"),
            "--include must force MATH241 as the canonical: {yaml}"
        );
    }

    #[test]
    fn trim_refuses_to_overwrite_input_path() {
        let tmp = tempfile::NamedTempFile::new().expect("tempfile");
        std::fs::write(tmp.path(), SAMPLE_YAML).expect("write");
        let path = tmp.path().to_string_lossy().to_string();
        let response = execute(SAMPLE_YAML, &[], &[], Some(&path), Some(&path));
        assert!(!response.success);
        let err = response.error.unwrap();
        assert!(
            err.contains("refusing to overwrite"),
            "unexpected error: {err}"
        );
    }

    #[test]
    fn trim_writes_output_path_when_provided() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        let out = tmp.path().join("trimmed.yaml");
        let response = execute(SAMPLE_YAML, &[], &[], Some(out.to_str().unwrap()), None);
        assert!(response.success, "error: {:?}", response.error);
        assert!(out.exists(), "output file must exist");
        assert_eq!(response.output_path.as_deref(), out.to_str());
    }

    #[test]
    fn trim_emits_parse_error_for_malformed_yaml() {
        let response = execute("not: valid: yaml: [", &[], &[], None, None);
        assert!(!response.success);
        assert!(response.parse_error.is_some());
        assert!(response.trimmed_yaml.is_none());
        assert!(response.trimmed_cache_id.is_none());
    }

    #[test]
    fn trim_execute_json_returns_valid_parseable_json() {
        let json_str = execute_json(SAMPLE_YAML, &[], &[], None, None);
        let value: serde_json::Value =
            serde_json::from_str(&json_str).expect("execute_json must emit valid JSON");
        assert_eq!(value["success"], serde_json::json!(true));
        assert!(value["trimmed_yaml"].is_string());
        assert!(value["trimmed_cache_id"]
            .as_str()
            .unwrap()
            .starts_with("cache:"));
    }

    #[test]
    fn trim_parse_error_populates_line_and_column() {
        // A YAML where serde_yaml can pin a precise location — a list where a
        // scalar is expected. The exact (line, column) varies across versions;
        // we just need *some* position to be reported.
        let bad_yaml =
            "degree:\n  total_credits: [not, a, number]\nrequirements: {}\ncourses: {}\n";
        let response = execute(bad_yaml, &[], &[], None, None);
        assert!(!response.success);
        assert!(response.parse_error.is_some());
        assert!(
            response.parse_error_line.is_some(),
            "serde_yaml reports a line for this error class — must propagate"
        );
        assert!(response.parse_error_column.is_some());
    }

    #[test]
    fn trim_parse_error_context_renders_window_with_caret() {
        let bad_yaml =
            "degree:\n  total_credits: [not, a, number]\nrequirements: {}\ncourses: {}\n";
        let response = execute(bad_yaml, &[], &[], None, None);
        let context = response
            .parse_error_context
            .as_ref()
            .expect("context window must be populated when line/column are known");
        assert!(
            context.contains('^'),
            "context must include a caret pointing at the column; got: {context}"
        );
    }

    #[test]
    fn trim_cache_handle_is_resolvable_in_yaml_cache() {
        // The handle returned in `trimmed_cache_id` must round-trip through
        // YAML_CACHE so callers really can use it as a `degree_id` argument
        // on the next tool call.
        let response = execute(SAMPLE_YAML, &[], &[], None, None);
        let cache_id = response.trimmed_cache_id.unwrap();
        let body = {
            let cache = YAML_CACHE.lock().unwrap();
            cache
                .get(&cache_id)
                .expect("trimmed yaml must be retrievable from YAML_CACHE")
                .0
        };
        assert!(body.contains("Test BS"));
    }
}
