//! Shared utilities for MCP tool implementations.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::core::degree::DegreeParseError;
use crate::core::json::{coded_error, error_code};

// ─── DegreeParseError formatting ─────────────────────────────────────────────

/// Render a [`DegreeParseError`] into a human-readable `parse_error` string.
///
/// Shared between `validate_degree` and `trim_degree` so the wording stays
/// consistent and a single edit propagates to both.
#[must_use]
pub fn format_degree_parse_error(e: &DegreeParseError) -> String {
    match e {
        DegreeParseError::IoError(msg) => format!("File error: {msg}"),
        DegreeParseError::YamlError {
            message,
            line,
            column,
        } => match (line, column) {
            // Prefix the structured location so log scrapers and JSON-blind
            // clients can still see the position.
            (Some(l), Some(c)) => format!("YAML syntax error at line {l} column {c}: {message}"),
            _ => format!("YAML syntax error: {message}"),
        },
        DegreeParseError::JsonError(msg) => format!("JSON syntax error: {msg}"),
    }
}

// ─── Tool-name constants ─────────────────────────────────────────────────────
// Used by the `tool_followups` builders so a rename of the actual MCP handler
// in server.rs surfaces as a compile-time grep instead of silently breaking
// follow-up suggestions.

/// MCP tool name: reference material (the degree format, its JSON Schema, the database).
pub const TOOL_GET_REFERENCE: &str = "get_reference";
/// MCP tool name: degree validation.
pub const TOOL_VALIDATE_DEGREE: &str = "validate_degree";
/// MCP tool name: degree audit (deep prereq chains + missing prereqs).
pub const TOOL_AUDIT_DEGREE: &str = "audit_degree";
/// MCP tool name: full degree analysis (plan generation + aggregate metrics).
pub const TOOL_ANALYZE_DEGREE: &str = "analyze_degree";
/// MCP tool name: per-course detail view.
pub const TOOL_GET_COURSE_DETAIL: &str = "get_course_detail";
/// MCP tool name: one-call plan-graph rendering.
pub const TOOL_RENDER_PLAN_GRAPH: &str = "render_plan_graph";
/// MCP tool name: the HTML degree report, computed afresh.
pub const TOOL_RENDER_DEGREE_REPORT: &str = "render_degree_report";

/// Every tool a follow-up or hint can name, so a test can check each is one the server
/// serves.
pub const FOLLOWUP_TOOLS: [&str; 7] = [
    TOOL_GET_REFERENCE,
    TOOL_VALIDATE_DEGREE,
    TOOL_AUDIT_DEGREE,
    TOOL_ANALYZE_DEGREE,
    TOOL_GET_COURSE_DETAIL,
    TOOL_RENDER_PLAN_GRAPH,
    TOOL_RENDER_DEGREE_REPORT,
];

/// Hint about the next MCP call a tool's response suggests the caller make.
///
/// Tools attach a `tool_followups: Vec<ToolFollowup>` array when their output
/// implies an obvious next step — `analyze_degree` flagging a sample with
/// `was_truncated=true` suggesting a rerun with higher `max_plans`,
/// `audit_degree` finding deep chains suggesting `render_plan_graph` to
/// visualise them, etc. The default is an empty vector when the response
/// state doesn't warrant a follow-up — no token cost for happy-path calls.
#[derive(Debug, Serialize)]
pub struct ToolFollowup {
    /// MCP tool name the caller should consider invoking next
    /// (e.g. `"audit_degree"`, `"render_plan_graph"`).
    pub tool: &'static str,
    /// Short human-readable explanation of why this follow-up is suggested.
    pub reason: String,
    /// JSON object the caller can plug straight into the suggested tool's
    /// request. Always emitted as a JSON object even when empty so callers
    /// can spread it without a type check.
    pub suggested_args: serde_json::Value,
}

/// Where a tool's degree comes from: exactly one of three fields.
///
/// Flattened into every request that takes a degree, so the three fields sit at the top
/// level of each tool's parameters. Three fields rather than one sniffed string: a path
/// that does not exist and a `program_key` that does not match look the same as a bare
/// string, and the error would have to guess which was meant.
#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
pub struct DegreeSourceArgs {
    /// A degree by reference.
    #[schemars(
        description = "A degree by reference: \"sample:<key>\" (see list_sample_degrees), \"cache:<hash>\" (the source.handle an earlier call returned), or a stored program's program_key or degree_id"
    )]
    pub degree: Option<String>,
    /// The degree inline.
    #[schemars(description = "The degree inline, as YAML or unified JSON")]
    pub content: Option<String>,
    /// A degree file on the server.
    #[schemars(description = "Path to a degree file (YAML or JSON) on the server's filesystem")]
    pub path: Option<String>,
}

/// A refusal of the arguments themselves, coded so the MCP error envelope says so.
#[must_use]
pub fn bad_arguments(message: impl std::fmt::Display) -> String {
    coded_error(error_code::BAD_ARGUMENTS, message).to_string()
}

/// A degree source, once exactly one was given.
#[derive(Debug, PartialEq, Eq)]
pub enum DegreeSource {
    /// Inline YAML or JSON.
    Content(String),
    /// A file on the server's filesystem.
    Path(String),
    /// `sample:<key>`, `cache:<hash>`, or a stored program.
    Reference(String),
}

impl DegreeSourceArgs {
    /// The one source given.
    ///
    /// # Errors
    /// A JSON error string when none or more than one was given, or when `content` is an
    /// `@`-path reference rather than a degree.
    pub fn into_source(self) -> Result<DegreeSource, String> {
        match (self.degree, self.content, self.path) {
            (Some(d), None, None) => Ok(DegreeSource::Reference(d)),
            (None, Some(c), None) => {
                // A leading `@` is never valid YAML or JSON (it is a reserved YAML
                // indicator) and almost always means an at-path reference. Refuse it by
                // name rather than hand it to the parser.
                if c.trim_start().starts_with('@') {
                    return Err(bad_arguments(
                        "content must be the degree itself, not a path reference (it starts with '@'). Use path for a file on the server, or degree for a sample:, cache: or stored reference.",
                    ));
                }
                Ok(DegreeSource::Content(c))
            }
            (None, None, Some(p)) => Ok(DegreeSource::Path(p)),
            (None, None, None) => Err(bad_arguments(
                "Must provide exactly one of: degree, content, or path",
            )),
            _ => Err(bad_arguments(
                "Provide exactly one of: degree, content, or path (not several)",
            )),
        }
    }
}

/// Why [`write_output`] wrote nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteRefusal {
    /// [`error_code::BAD_ARGUMENTS`] when the file exists (the caller's to decide),
    /// [`error_code::WRITE_FAILED`] when a directory or the file could not be written.
    pub code: &'static str,
    /// What happened, naming the path.
    pub message: String,
}

impl WriteRefusal {
    /// The failure payload, coded by cause.
    #[must_use]
    pub fn to_json(&self) -> String {
        coded_error(self.code, &self.message).to_string()
    }
}

/// Write a tool's output to `path`, refusing to replace an existing file unless asked.
///
/// The rule every tool that writes follows: a file that exists is not replaced unless the
/// caller passed `overwrite=true`, so a mistyped path cannot destroy something. Parent
/// directories are created.
///
/// # Errors
/// A [`WriteRefusal`] naming the path when it exists and `overwrite` is false, or when a
/// directory or the file cannot be written.
pub fn write_output(path: &str, content: &str, overwrite: bool) -> Result<(), WriteRefusal> {
    let target = std::path::Path::new(path);
    if target.exists() && !overwrite {
        return Err(WriteRefusal {
            code: error_code::BAD_ARGUMENTS,
            message: format!("{path} already exists; pass overwrite=true to replace it"),
        });
    }
    let failed = |message: String| WriteRefusal {
        code: error_code::WRITE_FAILED,
        message,
    };
    if let Some(parent) = target.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| failed(format!("cannot create {}: {e}", parent.display())))?;
    }
    std::fs::write(target, content).map_err(|e| failed(format!("cannot write {path}: {e}")))
}

/// A tool's output, returned inline or written to `path`: `(inline, written_path)`, with
/// exactly one of the two set.
///
/// # Errors
/// [`write_output`]'s refusal.
pub fn deliver_output(
    content: String,
    path: Option<&str>,
    overwrite: bool,
) -> Result<(Option<String>, Option<String>), WriteRefusal> {
    match path {
        Some(path) => {
            write_output(path, &content, overwrite).map(|()| (None, Some(path.to_string())))
        }
        None => Ok((Some(content), None)),
    }
}

/// Read a degree file named by a tool's `path` argument.
///
/// # Errors
/// A failure payload naming the path, coded [`error_code::SOURCE_NOT_FOUND`] when there is
/// no such file.
pub fn read_degree_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| {
        let mut payload = serde_json::json!({
            "error": format!("cannot read degree file {path}: {e}"),
            "path": path,
        });
        if e.kind() == std::io::ErrorKind::NotFound {
            payload["code"] = error_code::SOURCE_NOT_FOUND.into();
        }
        payload.to_string()
    })
}

/// Render a ±3-line context window around a 1-indexed `line` in `yaml`.
///
/// A caret points at `column` under the offending line. Used to give
/// `validate_degree` (and any future tool that surfaces parse errors) an
/// editor-style snippet that pins down which YAML statement broke.
///
/// Lines are 1-indexed to match `serde_yaml::Location`; the function clamps
/// to `[1, total_lines]` so an out-of-range location still emits a valid
/// window. Returns an empty string when `yaml` is empty.
#[must_use]
pub fn format_yaml_context(yaml: &str, line: usize, column: usize) -> String {
    use std::fmt::Write as _;

    if yaml.is_empty() {
        return String::new();
    }
    let lines: Vec<&str> = yaml.lines().collect();
    let total = lines.len();
    if total == 0 {
        return String::new();
    }
    let target = line.clamp(1, total);
    let start = target.saturating_sub(3).max(1);
    let end = (target + 3).min(total);

    // Pad the line-number column to the widest number we'll print so the
    // caret-column alignment is preserved regardless of digit count.
    let gutter_width = end.to_string().len();

    let mut out = String::new();
    for (idx, content) in lines.iter().enumerate().take(end).skip(start - 1) {
        let n = idx + 1;
        let _ = writeln!(out, "{n:>gutter_width$}: {content}");
        if n == target {
            // Build the caret line: same gutter padding + ": " + spaces up
            // to (column - 1) + a caret. Column is 1-indexed; column 0 is
            // treated as 1 so we never produce a negative offset.
            let caret_col = column.max(1) - 1;
            let pad = " ".repeat(gutter_width + 2 + caret_col);
            out.push_str(&pad);
            out.push_str("^ here\n");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(degree: Option<&str>, content: Option<&str>, path: Option<&str>) -> DegreeSourceArgs {
        DegreeSourceArgs {
            degree: degree.map(str::to_string),
            content: content.map(str::to_string),
            path: path.map(str::to_string),
        }
    }

    #[test]
    fn at_prefixed_content_is_refused_and_pointed_at_path() {
        // A leading `@` means the caller mistook content for an at-path reference — fail
        // fast with a directive error instead of stalling the YAML parser.
        let err = args(None, Some("@/path/to/degree.yaml"), None)
            .into_source()
            .expect_err("@-prefixed content must be rejected");
        assert!(err.contains("path") && err.contains('@'), "{err}");
        assert!(args(None, Some("   @foo"), None).into_source().is_err());
    }

    #[test]
    fn exactly_one_source_is_required() {
        let none = args(None, None, None).into_source().unwrap_err();
        assert!(none.contains("exactly one of"), "{none}");
        let several = args(Some("sample:csu"), None, Some("/tmp/x.yaml"))
            .into_source()
            .unwrap_err();
        assert!(several.contains("not several"), "{several}");
    }

    #[test]
    fn each_field_resolves_to_its_own_kind_of_source() {
        assert_eq!(
            args(None, Some("degree:\n  id: x\n"), None).into_source(),
            Ok(DegreeSource::Content("degree:\n  id: x\n".into()))
        );
        assert_eq!(
            args(None, None, Some("/tmp/x.yaml")).into_source(),
            Ok(DegreeSource::Path("/tmp/x.yaml".into()))
        );
        assert_eq!(
            args(Some("prog:1"), None, None).into_source(),
            Ok(DegreeSource::Reference("prog:1".into()))
        );
    }

    #[test]
    fn the_source_fields_sit_at_the_top_level_of_a_flattened_request() {
        #[derive(Deserialize, schemars::JsonSchema)]
        struct Probe {
            #[serde(flatten)]
            source: DegreeSourceArgs,
            other: Option<i32>,
        }
        let schema = serde_json::to_value(schemars::schema_for!(Probe)).expect("schema");
        for field in ["degree", "content", "path", "other"] {
            assert!(
                schema["properties"].get(field).is_some(),
                "{field} not top-level: {schema}"
            );
        }
        let probe: Probe =
            serde_json::from_value(serde_json::json!({"degree": "sample:csu", "other": 3}))
                .expect("decodes");
        assert_eq!(probe.source.degree.as_deref(), Some("sample:csu"));
        assert_eq!(probe.other, Some(3));
    }

    #[test]
    fn test_format_yaml_context_includes_caret_and_surrounding_lines() {
        let yaml = "one\ntwo\nthree\nfour\nfive\nsix\nseven\n";
        // Line 4, column 3 → "four", caret should sit under the 'u'.
        let ctx = format_yaml_context(yaml, 4, 3);
        assert!(ctx.contains("four"), "context must include offending line");
        assert!(ctx.contains("^ here"));
        // ±3 lines means we should see lines 1..=7 (clamped to total).
        assert!(ctx.contains("one"));
        assert!(ctx.contains("seven"));
    }

    #[test]
    fn test_format_yaml_context_clamps_to_file_bounds() {
        let yaml = "first\nsecond\nthird\n";
        // Line 99 is well past EOF — should clamp to the last line.
        let ctx = format_yaml_context(yaml, 99, 1);
        assert!(ctx.contains("third"));
        assert!(ctx.contains("^ here"));
    }

    #[test]
    fn test_format_yaml_context_returns_empty_for_empty_input() {
        assert!(format_yaml_context("", 1, 1).is_empty());
    }

    #[test]
    fn test_format_yaml_context_caret_aligns_with_column() {
        // Use a known-width gutter (single-digit line numbers) so we can
        // count the caret offset precisely. Line 3 column 5 means the caret
        // should sit 4 spaces past the ": " separator (column-1 spaces).
        let yaml = "alpha\nbeta\ngamma-x\ndelta\nepsilon\n";
        let ctx = format_yaml_context(yaml, 3, 5);
        // Find the caret line and check its leading spaces.
        let caret_line = ctx
            .lines()
            .find(|l| l.contains("^ here"))
            .expect("caret line must exist");
        // Gutter is "3: " (3 chars), then 4 spaces to reach column 5.
        // So the caret sits at index 3 + 4 = 7.
        let caret_pos = caret_line.find('^').expect("caret present");
        assert_eq!(caret_pos, 7, "caret offset mismatch in: {caret_line:?}");
    }

    #[test]
    fn test_write_output_refuses_codes_and_replaces_on_request() {
        let dir = std::env::temp_dir().join(format!("nuanalytics-write-{}", std::process::id()));
        let path = dir.join("a/b.txt");
        let path = path.to_str().unwrap();

        assert_eq!(write_output(path, "one", false), Ok(()), "creates parents");
        let refused = write_output(path, "two", false).unwrap_err();
        assert_eq!(refused.code, error_code::BAD_ARGUMENTS);
        assert!(
            refused.message.contains("overwrite=true"),
            "{}",
            refused.message
        );
        assert_eq!(
            std::fs::read_to_string(path).unwrap(),
            "one",
            "refusal wrote nothing"
        );

        assert_eq!(write_output(path, "two", true), Ok(()));
        assert_eq!(std::fs::read_to_string(path).unwrap(), "two");

        // A parent that is a file cannot be created: an I/O failure, not a bad argument.
        let under_file = format!("{path}/c.txt");
        assert_eq!(
            write_output(&under_file, "x", false).unwrap_err().code,
            error_code::WRITE_FAILED
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_deliver_output_is_inline_or_written_never_both() {
        assert_eq!(
            deliver_output("body".into(), None, false),
            Ok((Some("body".into()), None))
        );
        let path =
            std::env::temp_dir().join(format!("nuanalytics-deliver-{}.txt", std::process::id()));
        let path = path.to_str().unwrap();
        assert_eq!(
            deliver_output("body".into(), Some(path), true),
            Ok((None, Some(path.to_string())))
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn test_read_degree_file_reads_or_says_not_found() {
        let path =
            std::env::temp_dir().join(format!("nuanalytics-read-{}.yaml", std::process::id()));
        std::fs::write(&path, "degree: {}\n").unwrap();
        assert_eq!(
            read_degree_file(path.to_str().unwrap()).unwrap(),
            "degree: {}\n"
        );
        std::fs::remove_file(&path).ok();

        let missing: serde_json::Value =
            serde_json::from_str(&read_degree_file("/nonexistent/nu/x.yaml").unwrap_err()).unwrap();
        assert_eq!(missing["code"], error_code::SOURCE_NOT_FOUND);
        assert!(missing["error"]
            .as_str()
            .unwrap()
            .contains("/nonexistent/nu/x.yaml"));
    }
}
