//! ai-landscape → unified degree JSON conversion tool.
//!
//! Provides the `convert_degree` MCP tool: turn an ai-landscape program JSON
//! into the unified `NuAnalytics` degree JSON (the same format the YAML degrees
//! produce), surfacing conversion warnings. A cluster pipeline file (many
//! programs) is inventoried; pass `program` to convert one out of it.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::core::degree::{
    convert_landscape, extract_cluster_programs, parse_degree_json_with_warnings, to_unified_value,
    unified_value_to_string,
};
use crate::mcp::cache::YAML_CACHE;
use crate::mcp::tools::shared::{
    deserialize_opt_bool, read_yaml_file, to_json_pretty, ToolFollowup, TOOL_ANALYZE_DEGREE,
    TOOL_VALIDATE_DEGREE,
};

/// Request parameters for the `convert_degree` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConvertDegreeRequest {
    /// Inline ai-landscape program JSON (or an already-unified degree JSON,
    /// which is normalized through). Provide exactly one of this or `json_path`.
    #[schemars(description = "Inline ai-landscape (or unified) program JSON")]
    pub json_content: Option<String>,
    /// Path to an ai-landscape JSON file on the MCP server's filesystem.
    #[schemars(description = "Path to an ai-landscape JSON file on the server")]
    pub json_path: Option<String>,
    /// When the input is a multi-program *cluster* pipeline file, the program
    /// name to convert. Omit to get the cluster's program inventory instead.
    #[schemars(description = "Program name to convert out of a cluster file")]
    pub program: Option<String>,
    /// Pretty-print the unified JSON (default true).
    #[serde(default, deserialize_with = "deserialize_opt_bool")]
    #[schemars(description = "Pretty-print the unified JSON output (default true)")]
    pub pretty: Option<bool>,
}

/// One program entry in a cluster file's inventory.
#[derive(Debug, Serialize)]
pub struct ClusterProgramInfo {
    /// Program name (the cluster key; pass back as `program`).
    pub name: String,
    /// Institution the program belongs to.
    pub university: String,
    /// Degree title as scraped.
    pub degree: String,
}

/// Response from `convert_degree`.
#[derive(Debug, Serialize)]
pub struct ConvertResponse {
    /// Whether conversion (or inventory) succeeded.
    pub success: bool,
    /// Error message when `success` is false.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Shape of the input: `"single"` program or `"cluster"` pipeline file.
    pub kind: &'static str,
    /// Number of programs found in the input (1 for a single program).
    pub program_count: usize,
    /// The converted unified degree JSON (absent for a cluster inventory).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unified_json: Option<String>,
    /// Data-quality warnings raised during conversion (defaulted credits, etc.).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conversion_warnings: Vec<String>,
    /// Inventory of programs (populated for a cluster file with no `program`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<ClusterProgramInfo>,
    /// Cache handle for the converted JSON; pass as `degree_id` to chain into
    /// `validate_degree` / `analyze_degree` without re-pasting.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_id: Option<String>,
    /// Free-text guidance (e.g. how to convert a cluster program).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Suggested next MCP calls.
    pub tool_followups: Vec<ToolFollowup>,
}

impl ConvertResponse {
    /// Build a failed response carrying `msg` as the error.
    fn error(msg: impl std::fmt::Display) -> Self {
        Self {
            success: false,
            error: Some(msg.to_string()),
            kind: "single",
            program_count: 0,
            unified_json: None,
            conversion_warnings: Vec::new(),
            programs: Vec::new(),
            cache_id: None,
            note: None,
            tool_followups: Vec::new(),
        }
    }
}

/// Build a converted-degree response from a unified JSON string + warnings.
fn converted(
    kind: &'static str,
    program_count: usize,
    unified: String,
    warnings: Vec<String>,
) -> ConvertResponse {
    // Cache the unified body so callers can chain by `degree_id` (the degree
    // tools auto-detect JSON via parse_degree_auto). `.ok()`: a poisoned cache
    // must not fail the conversion — we just omit the chaining handle.
    let cache_id = YAML_CACHE
        .lock()
        .ok()
        .map(|mut cache| cache.insert(unified.clone()));
    let followups = cache_id.as_ref().map_or_else(Vec::new, |id| {
        vec![
            ToolFollowup {
                tool: TOOL_VALIDATE_DEGREE,
                reason: "Validate the converted degree before analyzing.".to_string(),
                suggested_args: serde_json::json!({ "degree_id": id }),
            },
            ToolFollowup {
                tool: TOOL_ANALYZE_DEGREE,
                reason: "Analyze the converted degree (plans + metrics).".to_string(),
                suggested_args: serde_json::json!({ "degree_id": id }),
            },
        ]
    });
    ConvertResponse {
        success: true,
        error: None,
        kind,
        program_count,
        unified_json: Some(unified),
        conversion_warnings: warnings,
        programs: Vec::new(),
        cache_id,
        note: None,
        tool_followups: followups,
    }
}

/// Serialize a degree program to the degree-first unified JSON string, mapping
/// any serializer failure to a `DegreeParseError`. Shared by the single-program
/// and cluster paths.
fn to_unified_json(
    program: &crate::core::DegreeProgram,
    pretty: bool,
) -> Result<String, crate::core::degree::DegreeParseError> {
    let value = to_unified_value(program)?;
    unified_value_to_string(&value, pretty).map_err(|e| {
        crate::core::degree::DegreeParseError::json_message(format!(
            "Failed to serialize unified JSON: {e}"
        ))
    })
}

/// Convert an ai-landscape (or unified) program JSON to the unified format.
///
/// Resolves exactly one of `json_content` / `json_path`, then: for a cluster
/// pipeline file, returns the program inventory (or converts the one named by
/// `program`); for a single program, auto-converts (ai-landscape) or normalizes
/// (already-unified) to the unified JSON.
#[must_use]
pub fn execute(
    json_content: Option<String>,
    json_path: Option<String>,
    program: Option<&str>,
    pretty: bool,
) -> ConvertResponse {
    let content = match (json_content, json_path) {
        (Some(_), Some(_)) => {
            return ConvertResponse::error(
                "Provide exactly one of json_content or json_path (not both)",
            )
        }
        (None, None) => {
            return ConvertResponse::error("Must provide exactly one of: json_content or json_path")
        }
        (Some(c), None) => c,
        (None, Some(p)) => match read_yaml_file(&p) {
            Ok(c) => c,
            Err(e) => return ConvertResponse::error(e),
        },
    };

    let value: serde_json::Value = match serde_json::from_str(&content) {
        Ok(v) => v,
        Err(e) => return ConvertResponse::error(format!("Invalid JSON: {e}")),
    };

    // Cluster pipeline file → inventory or single-program extraction.
    if let Some(programs) = extract_cluster_programs(&value) {
        return convert_cluster(&programs, program, pretty);
    }

    // Single program (raw ai-landscape or already-unified).
    match parse_degree_json_with_warnings(&content) {
        Ok((prog, warnings)) => match to_unified_json(&prog, pretty) {
            Ok(unified) => converted("single", 1, unified, warnings),
            Err(e) => ConvertResponse::error(e),
        },
        Err(e) => ConvertResponse::error(e),
    }
}

/// Handle a cluster pipeline file's already-extracted programs.
///
/// With no `program` selector, returns a bounded inventory of every program
/// (name + institution + degree) instead of dumping them all. With a selector,
/// converts that one program (exact match, then case-insensitive fallback) and
/// errors with the available names if none match.
fn convert_cluster(
    programs: &[(String, crate::core::degree::LandscapeProgram)],
    program: Option<&str>,
    pretty: bool,
) -> ConvertResponse {
    let Some(sel) = program else {
        // No selector → return the inventory.
        let inventory: Vec<ClusterProgramInfo> = programs
            .iter()
            .map(|(name, prog)| ClusterProgramInfo {
                name: name.clone(),
                university: prog.university.clone(),
                degree: prog.degree.clone(),
            })
            .collect();
        return ConvertResponse {
            success: true,
            error: None,
            kind: "cluster",
            program_count: inventory.len(),
            unified_json: None,
            conversion_warnings: Vec::new(),
            programs: inventory,
            cache_id: None,
            note: Some(
                "Multi-program cluster file. Re-call with `program` set to one of the listed \
                 names to convert it, or use the CLI `degree convert` to expand all programs to \
                 files."
                    .to_string(),
            ),
            tool_followups: Vec::new(),
        };
    };

    let Some((_, prog)) = programs.iter().find(|(name, _)| name == sel).or_else(|| {
        programs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(sel))
    }) else {
        let available: Vec<&str> = programs.iter().map(|(n, _)| n.as_str()).collect();
        return ConvertResponse::error(format!(
            "Program {sel:?} not found in cluster. Available: {}",
            available.join(", ")
        ));
    };

    let result = convert_landscape(prog);
    match to_unified_json(&result.program, pretty) {
        Ok(unified) => converted("cluster", programs.len(), unified, result.warnings),
        Err(e) => ConvertResponse::error(e),
    }
}

/// Execute `convert_degree` and serialize the response to a JSON string.
#[must_use]
pub fn execute_json(
    json_content: Option<String>,
    json_path: Option<String>,
    program: Option<&str>,
    pretty: bool,
) -> String {
    to_json_pretty(&execute(json_content, json_path, program, pretty))
}

#[cfg(test)]
mod tests {
    use super::*;

    const AI_LANDSCAPE: &str = r#"{
        "university": "Test University",
        "degree": "BS in Computer Science",
        "ai_program": null,
        "courses": {
            "cs_course_core": [
                {"course_code": "CS 101", "title": "Intro", "course_hours": "3",
                 "picklist": [], "prerequisites": [], "corequisites": [], "strict_corequisites": []}
            ]
        }
    }"#;

    fn first_arg(content: &str) -> ConvertResponse {
        execute(Some(content.to_string()), None, None, true)
    }

    #[test]
    fn convert_single_ai_landscape_returns_unified_json() {
        let r = first_arg(AI_LANDSCAPE);
        assert!(r.success, "expected success, got {:?}", r.error);
        assert_eq!(r.kind, "single");
        assert_eq!(r.program_count, 1);
        let unified = r.unified_json.expect("unified json present");
        // Degree-first, parses as a unified degree.
        let v: serde_json::Value = serde_json::from_str(&unified).unwrap();
        assert!(
            v.get("degree").is_some(),
            "unified output has a degree block"
        );
        assert!(v.get("courses").is_some());
        assert!(
            r.cache_id.is_some(),
            "converted body is cached for chaining"
        );
    }

    #[test]
    fn convert_already_unified_passes_through() {
        let unified_in = r#"{"degree":{"name":"X","degree_type":"BS","system_type":"semester"},
            "requirements":{"core":{"type":"all","courses":["CS101"]}},
            "courses":{"CS101":{"name":"Intro","prefix":"CS","number":"101","credit_hours":3.0}}}"#;
        let r = first_arg(unified_in);
        assert!(
            r.success,
            "unified input should normalize through: {:?}",
            r.error
        );
        assert!(r.unified_json.unwrap().contains("\"degree\""));
    }

    #[test]
    fn convert_malformed_json_reports_error() {
        let r = first_arg("{ not valid json");
        assert!(!r.success);
        assert!(r.error.unwrap().contains("Invalid JSON"));
    }

    #[test]
    fn convert_requires_exactly_one_source() {
        let both = execute(
            Some("{}".to_string()),
            Some("/x.json".to_string()),
            None,
            true,
        );
        assert!(!both.success);
        let neither = execute(None, None, None, true);
        assert!(!neither.success);
    }

    #[test]
    fn convert_cluster_without_program_returns_inventory() {
        let cluster = r#"{
            "course_verifier": {
                "Computer Science BS": { "results": {
                    "university": "Test U", "degree": "BS CS",
                    "courses": {"cs_course_core": [
                        {"course_code":"CS 101","title":"Intro","course_hours":"3",
                         "picklist":[],"prerequisites":[],"corequisites":[],"strict_corequisites":[]}
                    ]}
                }}
            }
        }"#;
        let r = execute(Some(cluster.to_string()), None, None, true);
        assert!(r.success);
        assert_eq!(r.kind, "cluster");
        assert!(!r.programs.is_empty(), "inventory should list the program");
        assert!(r.unified_json.is_none(), "no single program selected");
        assert!(r.note.is_some());
    }

    #[test]
    fn convert_cluster_with_program_converts_it() {
        let cluster = r#"{
            "course_verifier": {
                "Computer Science BS": { "results": {
                    "university": "Test U", "degree": "BS CS",
                    "courses": {"cs_course_core": [
                        {"course_code":"CS 101","title":"Intro","course_hours":"3",
                         "picklist":[],"prerequisites":[],"corequisites":[],"strict_corequisites":[]}
                    ]}
                }}
            }
        }"#;
        let r = execute(
            Some(cluster.to_string()),
            None,
            Some("Computer Science BS"),
            true,
        );
        assert!(r.success, "expected conversion, got {:?}", r.error);
        assert_eq!(r.kind, "cluster");
        assert!(r.unified_json.unwrap().contains("\"degree\""));
    }

    #[test]
    fn execute_json_emits_parseable_json() {
        let s = execute_json(Some(AI_LANDSCAPE.to_string()), None, None, true);
        let v: serde_json::Value = serde_json::from_str(&s).unwrap();
        assert_eq!(v["success"], serde_json::json!(true));
    }

    #[test]
    fn convert_json_path_nonexistent_file_surfaces_read_error() {
        let r = execute(
            None,
            Some("/nonexistent/nuanalytics/degree.json".to_string()),
            None,
            true,
        );
        assert!(!r.success);
        assert!(
            r.error.unwrap().contains("Failed to read"),
            "missing-file read error should be surfaced"
        );
    }

    #[test]
    fn convert_cluster_program_not_found_lists_available() {
        let cluster = r#"{
            "course_verifier": {
                "Computer Science BS": { "results": {
                    "university": "Test U", "degree": "BS CS",
                    "courses": {"cs_course_core": [
                        {"course_code":"CS 101","title":"Intro","course_hours":"3",
                         "picklist":[],"prerequisites":[],"corequisites":[],"strict_corequisites":[]}
                    ]}
                }}
            }
        }"#;
        let r = execute(
            Some(cluster.to_string()),
            None,
            Some("No Such Program"),
            true,
        );
        assert!(!r.success);
        let err = r.error.unwrap();
        assert!(err.contains("not found"));
        assert!(
            err.contains("Computer Science BS"),
            "available names listed: {err}"
        );
    }
}
