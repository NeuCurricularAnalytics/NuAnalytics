//! Shared utilities for MCP tool implementations.

use serde::{de, Deserialize, Deserializer, Serialize};

#[cfg(feature = "database")]
use std::sync::Arc;

use crate::core::degree::DegreeParseError;

#[cfg(feature = "database")]
use crate::core::database::{tables, DbClient, QueryFilters};

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

/// Read a YAML file from `path`. Errors are returned as JSON strings ready to
/// surface to the MCP client.
///
/// # Errors
/// Returns a JSON error string if the file cannot be opened or read.
pub fn read_yaml_file(path: &str) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| {
        serde_json::json!({
            "error": format!("Failed to read yaml_path: {e}"),
            "path": path,
        })
        .to_string()
    })
}

/// Fetch a stored degree YAML by `degree_id`. Returns the YAML content on
/// success; on failure returns a JSON error string ready to surface.
///
/// # Errors
/// Returns a JSON error string if the database query fails or the row is missing.
#[cfg(feature = "database")]
pub async fn fetch_yaml_by_degree_id(
    client: &Arc<DbClient>,
    degree_id: &str,
) -> Result<String, String> {
    let filters = QueryFilters::new().eq("degree_id", Some(degree_id));
    let result = client
        .select(tables::DEGREES, "yaml_content", &filters, Some(1))
        .await
        .map_err(error_json)?;
    result
        .as_array()
        .and_then(|a| a.first())
        .and_then(|item| item.get("yaml_content"))
        .and_then(serde_json::Value::as_str)
        .map(String::from)
        .ok_or_else(|| {
            serde_json::json!({
                "error": "degree_id not found",
                "degree_id": degree_id,
            })
            .to_string()
        })
}

/// Serialize a JSON error response string from a display-able error value.
pub fn error_json(msg: impl std::fmt::Display) -> String {
    serde_json::json!({ "error": msg.to_string() }).to_string()
}

/// Deserialize a `serde_json::Value` JSON array into a typed `Vec<T>`.
///
/// Items that fail to deserialize are silently skipped.
#[must_use]
pub fn parse_json_array<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Vec<T> {
    value
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|item| serde_json::from_value(item.clone()).ok())
                .collect()
        })
        .unwrap_or_default()
}

/// Deserialize the first element of a JSON array into `T`.
///
/// Returns `None` when the value is not an array, the array is empty,
/// or the first element fails to deserialize. The helper does *not*
/// scan ahead to find a successfully-deserializing item — `parse_first`
/// means the first, not the first that happens to deserialize.
#[must_use]
pub fn parse_first<T: serde::de::DeserializeOwned>(value: &serde_json::Value) -> Option<T> {
    value
        .as_array()?
        .first()
        .and_then(|item| serde_json::from_value(item.clone()).ok())
}

/// Parse a comma-separated list of trimmed, non-empty strings.
///
/// `"11.0101, 11.0701, "` → `["11.0101", "11.0701"]`
#[must_use]
pub fn parse_comma_list(s: &str) -> Vec<String> {
    s.split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .map(String::from)
        .collect()
}

/// Parse a comma-separated list of `usize` values, silently dropping any
/// entry that fails to parse (negative, non-numeric, etc.).
///
/// `"0, 2, abc, 5"` → `[0, 2, 5]`. Used for tool parameters like
/// `analyze_degree`'s `plan_indices`, where invalid entries should be
/// ignored rather than rejecting the whole request.
#[must_use]
pub fn parse_comma_list_usize(s: &str) -> Vec<usize> {
    s.split(',')
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .filter_map(|c| c.parse::<usize>().ok())
        .collect()
}

/// Serialize a value to a pretty-printed JSON string, falling back to an error JSON on failure.
pub fn to_json_pretty(value: &impl serde::Serialize) -> String {
    serde_json::to_string_pretty(value)
        .unwrap_or_else(|e| error_json(format!("Serialization failed: {e}")))
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
// These helpers accept either native or string-encoded values and
// resolve to `None` for missing/null/empty-string inputs. Apply via
// `#[serde(default, deserialize_with = "shared::deserialize_opt_<T>")]`.
// The `default` attribute is required so an absent field stays `None`
// instead of routing through the deserializer.

/// Coerce a JSON value into `Option<T>` accepting native, stringified, or null.
fn coerce_opt<T>(value: serde_json::Value) -> Result<Option<T>, String>
where
    T: serde::de::DeserializeOwned + std::str::FromStr,
    <T as std::str::FromStr>::Err: std::fmt::Display,
{
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) if s.is_empty() => Ok(None),
        serde_json::Value::String(s) => s.parse::<T>().map(Some).map_err(|e| e.to_string()),
        other => serde_json::from_value::<T>(other)
            .map(Some)
            .map_err(|e| e.to_string()),
    }
}

/// Deserialize `Option<i32>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native integer nor a parseable numeric string.
pub fn deserialize_opt_i32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i32>, D::Error> {
    coerce_opt::<i32>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<i64>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native integer nor a parseable numeric string.
pub fn deserialize_opt_i64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    coerce_opt::<i64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<usize>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is negative,
/// not an integer, or otherwise unparseable.
pub fn deserialize_opt_usize<'de, D: Deserializer<'de>>(d: D) -> Result<Option<usize>, D::Error> {
    coerce_opt::<usize>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<u64>` accepting `42`, `"42"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is negative,
/// not an integer, or otherwise unparseable.
pub fn deserialize_opt_u64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    coerce_opt::<u64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<f32>` accepting `1.5`, `"1.5"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native float nor a parseable numeric string.
pub fn deserialize_opt_f32<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f32>, D::Error> {
    coerce_opt::<f32>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<f64>` accepting `1.5`, `"1.5"`, `null`, or `""`.
///
/// # Errors
/// Returns the underlying deserializer error if the input is neither a
/// valid native float nor a parseable numeric string.
pub fn deserialize_opt_f64<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    coerce_opt::<f64>(serde_json::Value::deserialize(d)?).map_err(de::Error::custom)
}

/// Deserialize `Option<bool>` accepting `true`/`false`, `"true"`/`"false"`, `"1"`/`"0"`, or null.
///
/// # Errors
/// Returns the underlying deserializer error if the input is not a boolean,
/// a recognized boolean string (`true`/`false`/`yes`/`no`/`1`/`0`), or `0`/`1`
/// as a JSON number.
pub fn deserialize_opt_bool<'de, D: Deserializer<'de>>(d: D) -> Result<Option<bool>, D::Error> {
    let value = serde_json::Value::deserialize(d)?;
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Bool(b) => Ok(Some(b)),
        serde_json::Value::String(s) => {
            let trimmed = s.trim();
            if trimmed.is_empty() {
                return Ok(None);
            }
            match trimmed.to_ascii_lowercase().as_str() {
                "true" | "1" | "yes" => Ok(Some(true)),
                "false" | "0" | "no" => Ok(Some(false)),
                other => Err(de::Error::custom(format!(
                    "expected boolean (true/false/1/0/yes/no), got {other:?}"
                ))),
            }
        }
        // Accept numeric 1/0 as well (some clients send JSON numbers for booleans)
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(0) => Ok(Some(false)),
            Some(1) => Ok(Some(true)),
            _ => Err(de::Error::custom(format!(
                "expected boolean number 0 or 1, got {n}"
            ))),
        },
        other => Err(de::Error::custom(format!(
            "expected boolean, got {other:?}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[test]
    fn test_error_json_formats_message() {
        let out = error_json("something went wrong");
        assert_eq!(out, r#"{"error":"something went wrong"}"#);
    }

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

    #[test]
    fn test_parse_json_array_valid() {
        #[derive(Deserialize)]
        struct Item {
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"id": 2}]);
        let items: Vec<Item> = parse_json_array(&json);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].id, 1);
    }

    #[test]
    fn test_parse_json_array_skips_bad_entries() {
        #[derive(Deserialize)]
        struct Item {
            #[allow(dead_code)] // only used by serde, not read directly in test
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"name": "no id"}, {"id": 3}]);
        let items: Vec<Item> = parse_json_array(&json);
        assert_eq!(items.len(), 2);
    }

    #[test]
    fn test_parse_json_array_non_array_returns_empty() {
        let json = serde_json::json!({"not": "an array"});
        let items: Vec<serde_json::Value> = parse_json_array(&json);
        assert!(items.is_empty());
    }

    #[test]
    fn test_parse_first_returns_first_element() {
        #[derive(Deserialize)]
        struct Item {
            id: i32,
        }
        let json = serde_json::json!([{"id": 1}, {"id": 2}]);
        let item: Option<Item> = parse_first(&json);
        assert_eq!(item.unwrap().id, 1);
    }

    #[test]
    fn test_parse_first_empty_array_returns_none() {
        let json = serde_json::json!([]);
        let item: Option<serde_json::Value> = parse_first(&json);
        assert!(item.is_none());
    }

    #[test]
    fn test_parse_first_non_array_returns_none() {
        let json = serde_json::json!({"not": "an array"});
        let item: Option<serde_json::Value> = parse_first(&json);
        assert!(item.is_none());
    }

    #[test]
    fn test_parse_first_failed_deserialize_returns_none() {
        #[derive(Deserialize)]
        struct Item {
            #[allow(dead_code)] // only used by serde, not read directly in test
            id: i32,
        }
        // First element is missing the required `id` field; the helper must
        // return None rather than scanning ahead to the second element.
        let json = serde_json::json!([{"name": "no id"}, {"id": 1}]);
        let item: Option<Item> = parse_first(&json);
        assert!(
            item.is_none(),
            "parse_first must not skip ahead past a failed first element"
        );
    }

    #[test]
    fn test_parse_comma_list_basic() {
        assert_eq!(
            parse_comma_list("11.0101,11.0701"),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_empty_string() {
        assert!(parse_comma_list("").is_empty());
    }

    #[test]
    fn test_parse_comma_list_trims_whitespace() {
        assert_eq!(
            parse_comma_list("  11.0101  ,  11.0701  "),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_filters_empty_entries() {
        // Leading/trailing/consecutive commas produce no empty strings
        assert_eq!(
            parse_comma_list(",11.0101,,11.0701,"),
            vec!["11.0101", "11.0701"]
        );
    }

    #[test]
    fn test_parse_comma_list_only_commas_returns_empty() {
        assert!(parse_comma_list(",,,").is_empty());
    }

    #[test]
    fn test_parse_comma_list_single_entry() {
        assert_eq!(parse_comma_list("11.0101"), vec!["11.0101"]);
    }

    // ─── parse_comma_list_usize ─────────────────────────────────────────────

    #[test]
    fn test_parse_comma_list_usize_empty_returns_empty() {
        assert!(parse_comma_list_usize("").is_empty());
        assert!(parse_comma_list_usize(",,,").is_empty());
    }

    #[test]
    fn test_parse_comma_list_usize_single_value() {
        assert_eq!(parse_comma_list_usize("5"), vec![5]);
    }

    #[test]
    fn test_parse_comma_list_usize_multiple_values_with_whitespace() {
        assert_eq!(parse_comma_list_usize("1, 3, 5, 10"), vec![1, 3, 5, 10]);
    }

    #[test]
    fn test_parse_comma_list_usize_silently_drops_invalid_entries() {
        // Negatives and non-numeric tokens are dropped without aborting the
        // parse — callers want best-effort filtering, not all-or-nothing.
        assert_eq!(parse_comma_list_usize("1, abc, 3, -5, 7"), vec![1, 3, 7]);
    }

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

    #[test]
    fn test_read_yaml_file_returns_content_for_existing_file() {
        // Build a unique path under the OS temp dir so concurrent runs don't
        // collide. PID + nanosecond time keeps the test hermetic without
        // depending on unstable ThreadId APIs.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let path = std::env::temp_dir().join(format!(
            "nuanalytics-shared-{}-{nanos}.yaml",
            std::process::id()
        ));
        let body = "degree:\n  id: ok\n";
        std::fs::write(&path, body).expect("temp write");
        let read = read_yaml_file(path.to_str().unwrap()).expect("read");
        assert_eq!(read, body);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn test_read_yaml_file_missing_path_errors_with_context() {
        let err = read_yaml_file("/nonexistent/nuanalytics/should-not-exist.yaml").unwrap_err();
        assert!(err.contains("Failed to read yaml_path"));
        assert!(err.contains("/nonexistent/nuanalytics/should-not-exist.yaml"));
    }

    #[test]
    fn test_to_json_pretty_roundtrip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct Foo {
            x: i32,
        }
        let foo = Foo { x: 42 };
        let s = to_json_pretty(&foo);
        let back: Foo = serde_json::from_str(&s).unwrap();
        assert_eq!(back, foo);
    }

    // ─── Lenient option deserializers ──────────────────────────────────────

    #[test]
    fn test_deserialize_opt_i32_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_i32")]
            x: Option<i32>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 42})).unwrap(), Some(42));
        assert_eq!(parse(serde_json::json!({"x": "42"})).unwrap(), Some(42));
        assert_eq!(parse(serde_json::json!({"x": -7})).unwrap(), Some(-7));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({"x": ""})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "not a number"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_i64_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_i64")]
            x: Option<i64>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(
            parse(serde_json::json!({"x": 1_000_000_i64})).unwrap(),
            Some(1_000_000)
        );
        assert_eq!(
            parse(serde_json::json!({"x": "1000000"})).unwrap(),
            Some(1_000_000)
        );
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "abc"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_usize_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_usize")]
            x: Option<usize>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 25})).unwrap(), Some(25));
        assert_eq!(parse(serde_json::json!({"x": "25"})).unwrap(), Some(25));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        // Negatives can't fit in usize via either path
        assert!(parse(serde_json::json!({"x": -1})).is_err());
        assert!(parse(serde_json::json!({"x": "-1"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_f32_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_f32")]
            x: Option<f32>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 1.5})).unwrap(), Some(1.5));
        assert_eq!(parse(serde_json::json!({"x": "1.5"})).unwrap(), Some(1.5));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert!(parse(serde_json::json!({"x": "nope"})).is_err());
    }

    #[test]
    fn test_deserialize_opt_f64_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_f64")]
            x: Option<f64>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        assert_eq!(parse(serde_json::json!({"x": 2.5})).unwrap(), Some(2.5));
        assert_eq!(parse(serde_json::json!({"x": "2.5"})).unwrap(), Some(2.5));
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
    }

    #[test]
    fn test_deserialize_opt_bool_accepts_all_input_shapes() {
        #[derive(Deserialize)]
        struct R {
            #[serde(default, deserialize_with = "deserialize_opt_bool")]
            x: Option<bool>,
        }
        let parse = |v| serde_json::from_value::<R>(v).map(|r| r.x);
        // Native booleans
        assert_eq!(parse(serde_json::json!({"x": true})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": false})).unwrap(), Some(false));
        // String forms (case-insensitive)
        assert_eq!(parse(serde_json::json!({"x": "true"})).unwrap(), Some(true));
        assert_eq!(
            parse(serde_json::json!({"x": "FALSE"})).unwrap(),
            Some(false)
        );
        assert_eq!(parse(serde_json::json!({"x": "yes"})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": "no"})).unwrap(), Some(false));
        // Numeric forms (string and native)
        assert_eq!(parse(serde_json::json!({"x": "1"})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": "0"})).unwrap(), Some(false));
        assert_eq!(parse(serde_json::json!({"x": 1})).unwrap(), Some(true));
        assert_eq!(parse(serde_json::json!({"x": 0})).unwrap(), Some(false));
        // Empty / null / missing → None
        assert_eq!(parse(serde_json::json!({"x": null})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({"x": ""})).unwrap(), None);
        assert_eq!(parse(serde_json::json!({})).unwrap(), None);
        // Bad inputs
        assert!(parse(serde_json::json!({"x": "maybe"})).is_err());
        assert!(parse(serde_json::json!({"x": 2})).is_err());
    }
}
