//! Degree trim tool
//!
//! Exposes [`crate::core::degree::trim_program`] over MCP. The trimmed YAML is returned
//! inline; `output_path` also writes it to disk. Every successful call caches the trimmed
//! body and returns its handle as `trimmed_degree`, which any degree tool takes as
//! `degree`, so `validate_degree` and `audit_degree` can follow without resending it.

use rmcp::schemars;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

use crate::core::degree::{
    parse_degree_auto, serialize_degree_yaml, trim_program, DegreeParseError, TrimOptions,
    TrimReport,
};
use crate::mcp::cache::yaml_cache;
use crate::mcp::tools::shared::{
    format_degree_parse_error, format_yaml_context, write_output, DegreeSourceArgs, ToolFollowup,
    TOOL_AUDIT_DEGREE, TOOL_VALIDATE_DEGREE,
};

// ============================================================================
// Request / Response Types
// ============================================================================

/// Request parameters for the `trim_degree` tool.
///
/// Takes the degree as `source` — exactly one of `degree` (which accepts `cache:<hash>`
/// handles from prior tool calls), `content` or `path`. The trim semantics match the CLI's `degree trim` subcommand —
/// alternatives collapse to a single shortest entry path, except inside
/// protected subjects.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct TrimDegreeRequest {
    /// Where the degree comes from: exactly one of `degree`, `content`, `path`.
    #[serde(flatten)]
    pub source: DegreeSourceArgs,

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

    /// Also write the trimmed YAML to this file. Never the input file.
    #[schemars(
        description = "Also write the trimmed YAML to this file (it is returned inline regardless). Never replaces the input; replaces another existing file only with overwrite=true."
    )]
    pub output_path: Option<String>,

    /// Replace `output_path` when it already exists.
    #[schemars(description = "Replace output_path if it already exists (default false)")]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_bool")]
    pub overwrite: Option<bool>,
}

/// Where the trimmed degree is written, if anywhere, and what it came from.
#[derive(Debug, Default, Clone, Copy)]
pub struct TrimOutput<'a> {
    /// File to also write the trimmed YAML to.
    pub path: Option<&'a str>,
    /// Replace `path` when it exists. The input file is never replaced.
    pub overwrite: bool,
    /// The input's file, when it was read from one.
    pub source_path: Option<&'a str>,
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
    /// Why, when the cause is known: see [`crate::core::json::error_code`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,

    /// Trimmed YAML serialised back from the modified program. Present
    /// whenever `success == true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trimmed_yaml: Option<String>,

    /// The trimmed degree as a `cache:` reference, which any degree tool takes as `degree`.
    /// Issued on every successful trim.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trimmed_degree: Option<String>,

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
    output: &TrimOutput<'_>,
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

    let written_path = match output.path.map(|out| write_trimmed(out, &yaml, output)) {
        None => None,
        Some(Ok(out)) => Some(out),
        Some(Err(refusal)) => {
            return TrimResponse {
                success: false,
                error: Some(refusal.message),
                code: Some(refusal.code),
                ..empty_response()
            }
        }
    };

    let handle = yaml_cache().insert(yaml.clone());
    let followups = build_followups(&handle);

    TrimResponse {
        success: true,
        parse_error: None,
        parse_error_line: None,
        parse_error_column: None,
        parse_error_context: None,
        error: None,
        code: None,
        trimmed_yaml: Some(yaml),
        trimmed_degree: Some(handle),
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
    output: &TrimOutput<'_>,
) -> String {
    crate::core::json::to_json_pretty(&execute(yaml_content, keep_all, include, output))
}

/// Write the trimmed YAML to `out`, never over the input file.
///
/// The input is compared by canonical path, so another spelling of it (`sub/../in.yaml`)
/// is still refused, with or without `overwrite`.
fn write_trimmed(
    out: &str,
    yaml: &str,
    output: &TrimOutput<'_>,
) -> Result<String, crate::mcp::tools::shared::WriteRefusal> {
    if output.source_path.is_some_and(|src| same_file(out, src)) {
        return Err(crate::mcp::tools::shared::WriteRefusal {
            code: crate::core::json::error_code::BAD_ARGUMENTS,
            message: format!(
                "refusing to overwrite the input file {out}; choose a different output_path"
            ),
        });
    }
    write_output(out, yaml, output.overwrite).map(|()| out.to_string())
}

/// Whether two paths name the same existing file. A path that does not exist names no
/// file, so it is never the (existing) input.
fn same_file(a: &str, b: &str) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    }
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
        code: None,
        trimmed_degree: None,
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

fn build_followups(cache_id: &str) -> Vec<ToolFollowup> {
    vec![
        ToolFollowup {
            tool: TOOL_VALIDATE_DEGREE,
            reason:
                "Confirm the trimmed YAML still validates — comments are dropped on serialisation."
                    .to_string(),
            suggested_args: serde_json::json!({ "degree": cache_id }),
        },
        ToolFollowup {
            tool: TOOL_AUDIT_DEGREE,
            reason: "Audit the trimmed plan for hidden prereqs / deep chains after collapse."
                .to_string(),
            suggested_args: serde_json::json!({ "degree": cache_id }),
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
        let response = execute(SAMPLE_YAML, &[], &[], &TrimOutput::default());
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
            .trimmed_degree
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
        // The whole point of `trimmed_degree` is to let the model chain
        // validate/audit without re-pasting the trimmed YAML; verify the
        // suggested args carry the freshly-issued handle.
        let response = execute(SAMPLE_YAML, &[], &[], &TrimOutput::default());
        let cache_id = response.trimmed_degree.as_ref().unwrap();
        let tools: Vec<&str> = response.tool_followups.iter().map(|f| f.tool).collect();
        assert_eq!(tools, vec![TOOL_VALIDATE_DEGREE, TOOL_AUDIT_DEGREE]);
        for f in &response.tool_followups {
            assert_eq!(f.suggested_args["degree"].as_str(), Some(cache_id.as_str()));
        }
    }

    #[test]
    fn trim_keep_all_preserves_extra_subject() {
        let response = execute(
            SAMPLE_YAML,
            &["MATH".to_string()],
            &[],
            &TrimOutput::default(),
        );
        assert!(response.success);
        let yaml = response.trimmed_yaml.unwrap();
        assert!(
            yaml.contains("MATH215") && yaml.contains("MATH241"),
            "both MATH equivalents should survive with --keep-all MATH: {yaml}"
        );
    }

    #[test]
    fn trim_include_overrides_default_canonical() {
        let response = execute(
            SAMPLE_YAML,
            &[],
            &["MATH241".to_string()],
            &TrimOutput::default(),
        );
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
        let response = execute(
            SAMPLE_YAML,
            &[],
            &[],
            &TrimOutput {
                path: Some(&path),
                overwrite: true,
                source_path: Some(&path),
            },
        );
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
        let response = execute(
            SAMPLE_YAML,
            &[],
            &[],
            &TrimOutput {
                path: out.to_str(),
                ..TrimOutput::default()
            },
        );
        assert!(response.success, "error: {:?}", response.error);
        assert!(out.exists(), "output file must exist");
        assert_eq!(response.output_path.as_deref(), out.to_str());
    }

    #[test]
    fn trim_refuses_the_input_under_another_spelling_of_its_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir(dir.path().join("sub")).expect("mkdir");
        let input = dir.path().join("in.yaml");
        std::fs::write(&input, SAMPLE_YAML).expect("write");
        let alias = dir.path().join("sub").join("..").join("in.yaml");
        let response = execute(
            SAMPLE_YAML,
            &[],
            &[],
            &TrimOutput {
                path: alias.to_str(),
                overwrite: true,
                source_path: input.to_str(),
            },
        );
        assert!(
            !response.success,
            "input overwritten via {}",
            alias.display()
        );
        assert_eq!(std::fs::read_to_string(&input).unwrap(), SAMPLE_YAML);
    }

    #[test]
    fn trim_replaces_an_existing_output_file_only_with_overwrite() {
        let dir = tempfile::tempdir().expect("tempdir");
        let out = dir.path().join("keep.yaml");
        std::fs::write(&out, "someone else's file").expect("write");
        let output = |overwrite| TrimOutput {
            path: out.to_str(),
            overwrite,
            source_path: None,
        };
        let refused = execute(SAMPLE_YAML, &[], &[], &output(false));
        assert!(
            !refused.success,
            "an existing file was replaced without overwrite"
        );
        assert_eq!(
            refused.code,
            Some(crate::core::json::error_code::BAD_ARGUMENTS)
        );
        assert_eq!(
            std::fs::read_to_string(&out).unwrap(),
            "someone else's file"
        );

        let replaced = execute(SAMPLE_YAML, &[], &[], &output(true));
        assert!(replaced.success, "error: {:?}", replaced.error);
        assert_ne!(
            std::fs::read_to_string(&out).unwrap(),
            "someone else's file"
        );
    }

    #[test]
    fn trim_emits_parse_error_for_malformed_yaml() {
        let response = execute("not: valid: yaml: [", &[], &[], &TrimOutput::default());
        assert!(!response.success);
        assert!(response.parse_error.is_some());
        assert!(response.trimmed_yaml.is_none());
        assert!(response.trimmed_degree.is_none());
    }

    #[test]
    fn trim_execute_json_returns_valid_parseable_json() {
        let json_str = execute_json(SAMPLE_YAML, &[], &[], &TrimOutput::default());
        let value: serde_json::Value =
            serde_json::from_str(&json_str).expect("execute_json must emit valid JSON");
        assert_eq!(value["success"], serde_json::json!(true));
        assert!(value["trimmed_yaml"].is_string());
        assert!(value["trimmed_degree"]
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
        let response = execute(bad_yaml, &[], &[], &TrimOutput::default());
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
        let response = execute(bad_yaml, &[], &[], &TrimOutput::default());
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
        // The handle returned in `trimmed_degree` must round-trip through the cache so
        // callers really can use it as a `degree` argument on the next tool call.
        let response = execute(SAMPLE_YAML, &[], &[], &TrimOutput::default());
        let cache_id = response.trimmed_degree.unwrap();
        let body = {
            let cache = yaml_cache();
            cache
                .get(&cache_id)
                .expect("trimmed yaml must be retrievable from the cache")
                .0
        };
        assert!(body.contains("Test BS"));
    }
}
