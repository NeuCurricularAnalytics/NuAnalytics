//! Shared utilities for MCP tool implementations.

use serde::Serialize;

use crate::core::degree::DegreeParseError;
use crate::core::json::error_json;

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

/// MCP tool name: schema documentation.
pub const TOOL_GET_DEGREE_SCHEMA: &str = "get_degree_schema";
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

/// How a degree YAML was supplied to validate/audit/analyze.
///
/// Exactly one source is required. `Path` is read from the filesystem at the
/// MCP server's working directory; `DegreeId` is fetched from the configured
/// database (requires the `database` feature).
#[derive(Debug)]
pub enum YamlSource {
    /// Inline YAML body passed by the caller.
    Content(String),
    /// Path to a YAML file on the MCP server's filesystem.
    Path(String),
    /// Stored degree id; the server fetches the YAML from the database.
    DegreeId(String),
}

/// Pick a [`YamlSource`] from the three optional input fields.
///
/// Returns a JSON error string when the caller supplied none or more than one,
/// so the handler can return it directly.
///
/// # Errors
/// Returns a JSON error string when zero or more than one source is provided.
pub fn parse_yaml_source(
    yaml_content: Option<String>,
    yaml_path: Option<String>,
    degree_id: Option<String>,
) -> Result<YamlSource, String> {
    let count = u8::from(yaml_content.is_some())
        + u8::from(yaml_path.is_some())
        + u8::from(degree_id.is_some());
    if count == 0 {
        return Err(error_json(
            "Must provide exactly one of: yaml_content, yaml_path, or degree_id",
        ));
    }
    if count > 1 {
        return Err(error_json(
            "Provide exactly one of: yaml_content, yaml_path, or degree_id (not multiple)",
        ));
    }
    if let Some(c) = yaml_content {
        // A leading `@` is never valid degree YAML/JSON (it's a reserved YAML
        // indicator) and almost always means the caller meant an at-path
        // reference. Fail fast with a directive error rather than handing it to
        // the parser, which previously stalled the whole tool call.
        if c.trim_start().starts_with('@') {
            return Err(error_json(
                "yaml_content must be inline YAML/JSON, not a path reference (it starts with '@'). Use yaml_path for a file on the server, or degree_id for a cache:<hash> / stored program.",
            ));
        }
        return Ok(YamlSource::Content(c));
    }
    if let Some(p) = yaml_path {
        return Ok(YamlSource::Path(p));
    }
    Ok(YamlSource::DegreeId(degree_id.unwrap_or_default()))
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

// ============================================================================
// Lenient option deserializers
// ============================================================================
//
// Some MCP clients (notably Cowork / the Claude Agent SDK) serialize
// numeric and boolean parameters as JSON strings (e.g. `"2023"` rather
// than `2023`). Default serde rejects those when the field is typed as
// `Option<i32>` etc., so requests fail before reaching tool logic.
//
// Those helpers now live in `crate::core::json`, so the query engines and the CLI reach
// them without the `mcp` feature. Apply via
// `#[serde(default, deserialize_with = "crate::core::json::deserialize_opt_<T>")]`.
// The `default` attribute is required so an absent field stays `None` instead of routing
// through the deserializer.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_yaml_source_rejects_at_prefixed_content() {
        // A leading `@` means the caller mistook yaml_content for an at-path
        // reference — fail fast with a directive error instead of stalling the
        // YAML parser (the field-report hang).
        let err = parse_yaml_source(Some("@/path/to/degree.yaml".to_string()), None, None)
            .expect_err("@-prefixed yaml_content must be rejected");
        assert!(
            err.contains("yaml_path") && err.contains('@'),
            "error must redirect to yaml_path and name the '@': {err}"
        );
        // Leading whitespace before the `@` is still caught.
        assert!(parse_yaml_source(Some("   @foo".to_string()), None, None).is_err());
    }

    #[test]
    fn test_parse_yaml_source_accepts_normal_content() {
        let src = parse_yaml_source(Some("degree:\n  id: x\n".to_string()), None, None)
            .expect("normal yaml_content must be accepted");
        assert!(matches!(src, YamlSource::Content(_)));
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

    // ─── parse_comma_list_usize ─────────────────────────────────────────────

    // ─── parse_yaml_source ──────────────────────────────────────────────────

    #[test]
    fn test_parse_yaml_source_zero_sources_errors() {
        let err = parse_yaml_source(None, None, None).unwrap_err();
        assert!(err.contains("Must provide exactly one of"));
    }

    #[test]
    fn test_parse_yaml_source_multiple_sources_errors() {
        let err = parse_yaml_source(
            Some("inline".to_string()),
            Some("/tmp/x.yaml".to_string()),
            None,
        )
        .unwrap_err();
        assert!(err.contains("not multiple"));
    }

    #[test]
    fn test_parse_yaml_source_single_content_resolves_to_content() {
        let src = parse_yaml_source(Some("body".to_string()), None, None).unwrap();
        assert!(matches!(src, YamlSource::Content(s) if s == "body"));
    }

    #[test]
    fn test_parse_yaml_source_single_path_resolves_to_path() {
        let src = parse_yaml_source(None, Some("/tmp/x.yaml".to_string()), None).unwrap();
        assert!(matches!(src, YamlSource::Path(s) if s == "/tmp/x.yaml"));
    }

    #[test]
    fn test_parse_yaml_source_single_degree_id_resolves_to_id() {
        let src = parse_yaml_source(None, None, Some("deg-1".to_string())).unwrap();
        assert!(matches!(src, YamlSource::DegreeId(s) if s == "deg-1"));
    }

    // ─── read_yaml_file ─────────────────────────────────────────────────────

    // ─── Lenient option deserializers ──────────────────────────────────────
}
