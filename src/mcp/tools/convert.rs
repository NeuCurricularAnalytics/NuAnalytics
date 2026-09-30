//! Degree format conversion: `convert_degree`.
//!
//! Any degree — a stored program, a sample, a cached or inline body, a file — out as the
//! unified degree JSON or as YAML. The input may be YAML, unified JSON, or an ai-landscape
//! program JSON; a multi-program ai-landscape *cluster* file is inventoried, and `program`
//! picks one out of it. This is also the export path for a stored program: pass its
//! `program_key` as `degree` and `output_path` to write it to disk.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::core::degree::{
    convert_landscape, extract_cluster_programs, parse_degree_auto,
    parse_degree_json_with_warnings, serialize_degree_yaml, to_unified_value,
    unified_value_to_string, DegreeParseError,
};
use crate::core::json::{deserialize_opt_bool, to_json_pretty};
use crate::core::DegreeProgram;
use crate::mcp::cache::yaml_cache;
use crate::mcp::tools::shared::{
    deliver_output, DegreeSourceArgs, ToolFollowup, TOOL_ANALYZE_DEGREE, TOOL_VALIDATE_DEGREE,
};

/// Format a converted degree is written in.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormat {
    /// The unified degree JSON — what `db import` stores and every tool accepts.
    #[default]
    Json,
    /// The YAML format degrees are authored in.
    Yaml,
}

impl OutputFormat {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Json => "json",
            Self::Yaml => "yaml",
        }
    }
}

/// Request parameters for the `convert_degree` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ConvertDegreeRequest {
    /// Where the degree comes from: exactly one of `degree`, `content`, `path`.
    #[serde(flatten)]
    pub source: DegreeSourceArgs,
    /// Output format.
    #[schemars(description = "Output format: \"json\" (unified degree JSON, default) or \"yaml\"")]
    #[serde(default)]
    pub format: Option<OutputFormat>,
    /// Program to convert out of an ai-landscape cluster file.
    #[schemars(
        description = "For an ai-landscape cluster file: the program to convert. Omit to list the programs."
    )]
    pub program: Option<String>,
    /// Write the result here instead of returning it inline.
    #[schemars(description = "Write the result to this file instead of returning it inline")]
    pub output_path: Option<String>,
    /// Replace `output_path` if it already exists.
    #[schemars(description = "Replace output_path if it already exists (default false)")]
    #[serde(default, deserialize_with = "deserialize_opt_bool")]
    pub overwrite: Option<bool>,
    /// Pretty-print JSON output (default true).
    #[serde(default, deserialize_with = "deserialize_opt_bool")]
    #[schemars(description = "Pretty-print JSON output (default true)")]
    pub pretty: Option<bool>,
}

/// How to convert, apart from the degree itself.
#[derive(Debug, Default)]
pub struct ConvertOptions<'a> {
    /// Program to pick out of a cluster file.
    pub program: Option<&'a str>,
    /// Output format.
    pub format: OutputFormat,
    /// Pretty-print JSON.
    pub pretty: bool,
    /// File to write instead of returning the result inline.
    pub output_path: Option<&'a str>,
    /// Replace `output_path` if it exists.
    pub overwrite: bool,
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
    /// Why, when the cause is known: see [`crate::core::json::error_code`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<&'static str>,
    /// Shape of the input: `"single"` program or `"cluster"` pipeline file.
    pub input_kind: &'static str,
    /// Number of programs found in the input (1 for a single program).
    pub program_count: usize,
    /// `json` or `yaml`.
    pub format: &'static str,
    /// The converted degree, when returned inline.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    /// The file written, when `output_path` was given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Data-quality warnings raised during conversion (defaulted credits, etc.).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub conversion_warnings: Vec<String>,
    /// Inventory of programs (populated for a cluster file with no `program`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub programs: Vec<ClusterProgramInfo>,
    /// `cache:` handle for the converted degree; pass as `degree` to chain into
    /// `validate_degree` / `analyze_degree` without re-sending it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_id: Option<String>,
    /// Free-text guidance (e.g. how to convert a cluster program).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Suggested next MCP calls.
    pub tool_followups: Vec<ToolFollowup>,
}

impl ConvertResponse {
    /// A failed response carrying `msg` as the error.
    fn error(msg: impl std::fmt::Display) -> Self {
        Self {
            success: false,
            error: Some(msg.to_string()),
            code: None,
            input_kind: "single",
            program_count: 0,
            format: OutputFormat::default().as_str(),
            content: None,
            path: None,
            conversion_warnings: Vec::new(),
            programs: Vec::new(),
            cache_id: None,
            note: None,
            tool_followups: Vec::new(),
        }
    }
}

/// Serialize a degree program to the unified JSON string.
fn to_unified_json(program: &DegreeProgram, pretty: bool) -> Result<String, DegreeParseError> {
    let value = to_unified_value(program)?;
    unified_value_to_string(&value, pretty).map_err(|e| {
        DegreeParseError::json_message(format!("Failed to serialize unified JSON: {e}"))
    })
}

/// What a degree's text turned out to hold.
enum Parsed {
    /// One program.
    Program {
        /// The program, parsed or converted.
        program: Box<DegreeProgram>,
        /// What conversion had to guess or drop.
        warnings: Vec<String>,
        /// `"single"` or `"cluster"`, as [`ConvertResponse::input_kind`].
        input_kind: &'static str,
        /// Programs the input held.
        program_count: usize,
    },
    /// A cluster file's inventory, returned as is.
    Inventory(ConvertResponse),
}

/// Parse `text` — YAML, unified JSON, an ai-landscape program or cluster file.
fn parse(text: &str, program: Option<&str>) -> Result<Parsed, String> {
    if !text.trim_start().starts_with('{') {
        let (prog, warnings) = parse_degree_auto(text).map_err(|e| e.to_string())?;
        return Ok(Parsed::single(prog, warnings));
    }
    let value: serde_json::Value =
        serde_json::from_str(text).map_err(|e| format!("Invalid JSON: {e}"))?;
    if let Some(programs) = extract_cluster_programs(&value) {
        return pick_cluster_program(&programs, program);
    }
    let (prog, warnings) = parse_degree_json_with_warnings(text).map_err(|e| e.to_string())?;
    Ok(Parsed::single(prog, warnings))
}

impl Parsed {
    /// A file holding one program.
    fn single(program: DegreeProgram, warnings: Vec<String>) -> Self {
        Self::Program {
            program: Box::new(program),
            warnings,
            input_kind: "single",
            program_count: 1,
        }
    }
}

/// Convert a degree's text as `opts` says.
#[must_use]
pub fn execute(text: &str, opts: &ConvertOptions<'_>) -> ConvertResponse {
    let (program, warnings, kind, program_count) = match parse(text, opts.program) {
        Ok(Parsed::Program {
            program,
            warnings,
            input_kind,
            program_count,
        }) => (program, warnings, input_kind, program_count),
        Ok(Parsed::Inventory(response)) => return response,
        Err(e) => return ConvertResponse::error(e),
    };
    let rendered = match opts.format {
        OutputFormat::Json => to_unified_json(&program, opts.pretty),
        OutputFormat::Yaml => serialize_degree_yaml(&program),
    };
    let rendered = match rendered {
        Ok(r) => r,
        Err(e) => return ConvertResponse::error(e),
    };
    // Cached in either format: every degree tool reads YAML and JSON alike.
    let cache_id = Some(yaml_cache().insert(rendered.clone()));
    let (content, path) = match deliver_output(rendered, opts.output_path, opts.overwrite) {
        Ok(delivered) => delivered,
        Err(refusal) => {
            return ConvertResponse {
                code: Some(refusal.code),
                ..ConvertResponse::error(refusal.message)
            }
        }
    };
    let followups = cache_id.as_ref().map_or_else(Vec::new, |id| {
        vec![
            ToolFollowup {
                tool: TOOL_VALIDATE_DEGREE,
                reason: "Validate the converted degree before analyzing.".to_string(),
                suggested_args: serde_json::json!({ "degree": id }),
            },
            ToolFollowup {
                tool: TOOL_ANALYZE_DEGREE,
                reason: "Analyze the converted degree (plans + metrics).".to_string(),
                suggested_args: serde_json::json!({ "degree": id }),
            },
        ]
    });
    ConvertResponse {
        success: true,
        error: None,
        code: None,
        input_kind: kind,
        program_count,
        format: opts.format.as_str(),
        content,
        path,
        conversion_warnings: warnings,
        programs: Vec::new(),
        cache_id,
        note: None,
        tool_followups: followups,
    }
}

/// One program out of a cluster file, or the file's inventory when none was named.
///
/// The name matches exactly, then case-insensitively; with no match the error lists the
/// names available.
fn pick_cluster_program(
    programs: &[(String, crate::core::degree::LandscapeProgram)],
    program: Option<&str>,
) -> Result<Parsed, String> {
    let Some(sel) = program else {
        let inventory: Vec<ClusterProgramInfo> = programs
            .iter()
            .map(|(name, prog)| ClusterProgramInfo {
                name: name.clone(),
                university: prog.university.clone(),
                degree: prog.degree.clone(),
            })
            .collect();
        return Ok(Parsed::Inventory(ConvertResponse::inventory(inventory)));
    };
    let Some((_, prog)) = programs.iter().find(|(name, _)| name == sel).or_else(|| {
        programs
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(sel))
    }) else {
        let available: Vec<&str> = programs.iter().map(|(n, _)| n.as_str()).collect();
        return Err(format!(
            "Program {sel:?} not found in cluster. Available: {}",
            available.join(", ")
        ));
    };
    let result = convert_landscape(prog);
    Ok(Parsed::Program {
        program: Box::new(result.program),
        warnings: result.warnings,
        input_kind: "cluster",
        program_count: programs.len(),
    })
}

impl ConvertResponse {
    /// A cluster file's inventory: which programs it holds, for a call naming one.
    fn inventory(programs: Vec<ClusterProgramInfo>) -> Self {
        Self {
            success: true,
            error: None,
            input_kind: "cluster",
            program_count: programs.len(),
            programs,
            note: Some(
                "Multi-program cluster file. Re-call with `program` set to one of the listed \
                 names to convert it, or use the CLI `degree convert` to expand all programs to \
                 files."
                    .to_string(),
            ),
            ..Self::error("")
        }
    }
}

/// Execute `convert_degree` and serialize the response to a JSON string.
#[must_use]
pub fn execute_json(text: &str, opts: &ConvertOptions<'_>) -> String {
    to_json_pretty(&execute(text, opts))
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

    fn json_opts() -> ConvertOptions<'static> {
        ConvertOptions {
            pretty: true,
            ..ConvertOptions::default()
        }
    }

    const CLUSTER: &str = r#"{
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

    #[test]
    fn convert_single_ai_landscape_returns_unified_json() {
        let r = execute(AI_LANDSCAPE, &json_opts());
        assert!(r.success, "expected success, got {:?}", r.error);
        assert_eq!(
            (r.input_kind, r.program_count, r.format),
            ("single", 1, "json")
        );
        let v: serde_json::Value = serde_json::from_str(&r.content.expect("inline")).unwrap();
        assert!(v.get("degree").is_some() && v.get("courses").is_some());
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
        let r = execute(unified_in, &json_opts());
        assert!(
            r.success,
            "unified input should normalize through: {:?}",
            r.error
        );
        assert!(r.content.unwrap().contains("\"degree\""));
    }

    #[test]
    fn a_degree_round_trips_between_yaml_and_json() {
        let yaml = include_str!("../../../samples/degrees/csu-cs-bscs-general.yaml");
        let as_json = execute(yaml, &json_opts());
        assert!(as_json.success, "{:?}", as_json.error);
        let back = execute(
            &as_json.content.expect("json"),
            &ConvertOptions {
                format: OutputFormat::Yaml,
                ..json_opts()
            },
        );
        assert!(back.success, "{:?}", back.error);
        assert_eq!(back.format, "yaml");
        let (program, _) = parse_degree_auto(&back.content.expect("yaml")).expect("parses");
        let (original, _) = parse_degree_auto(yaml).expect("parses");
        assert_eq!(program.courses.len(), original.courses.len());
    }

    #[test]
    fn output_path_writes_the_file_and_refuses_to_replace_one_unasked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let target = dir.path().join("out.json");
        let target = target.to_str().expect("utf8");
        let opts = ConvertOptions {
            output_path: Some(target),
            ..json_opts()
        };
        let written = execute(AI_LANDSCAPE, &opts);
        assert!(written.success, "{:?}", written.error);
        assert_eq!(written.path.as_deref(), Some(target));
        assert!(written.content.is_none(), "written, not returned inline");
        let again = execute(AI_LANDSCAPE, &opts);
        assert!(
            !again.success,
            "an existing file is not replaced by default"
        );
        assert!(again.error.unwrap().contains("overwrite"));
        let replaced = execute(
            AI_LANDSCAPE,
            &ConvertOptions {
                overwrite: true,
                ..opts
            },
        );
        assert!(replaced.success, "{:?}", replaced.error);
    }

    #[test]
    fn convert_malformed_json_reports_error() {
        let r = execute("{ not valid json", &json_opts());
        assert!(!r.success);
        assert!(r.error.unwrap().contains("Invalid JSON"));
    }

    #[test]
    fn convert_cluster_without_program_returns_inventory() {
        let r = execute(CLUSTER, &json_opts());
        assert!(r.success, "{:?}", r.error);
        assert_eq!(r.input_kind, "cluster");
        assert!(!r.programs.is_empty(), "inventory should list the program");
        assert!(r.content.is_none(), "no single program selected");
        assert!(r.note.is_some());
    }

    #[test]
    fn convert_cluster_with_program_converts_it() {
        let r = execute(
            CLUSTER,
            &ConvertOptions {
                program: Some("Computer Science BS"),
                ..json_opts()
            },
        );
        assert!(r.success, "expected conversion, got {:?}", r.error);
        assert_eq!(r.input_kind, "cluster");
        assert!(r.content.unwrap().contains("\"degree\""));
    }

    #[test]
    fn convert_cluster_program_not_found_lists_available() {
        let r = execute(
            CLUSTER,
            &ConvertOptions {
                program: Some("No Such Program"),
                ..json_opts()
            },
        );
        assert!(!r.success);
        let err = r.error.unwrap();
        assert!(err.contains("not found"));
        assert!(
            err.contains("Computer Science BS"),
            "available names listed: {err}"
        );
    }

    #[test]
    fn execute_json_emits_parseable_json() {
        let v: serde_json::Value =
            serde_json::from_str(&execute_json(AI_LANDSCAPE, &json_opts())).unwrap();
        assert_eq!(v["success"], serde_json::json!(true));
    }
}
