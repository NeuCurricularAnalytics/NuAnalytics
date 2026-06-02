//! JSON degree parser (unified format) with ai-landscape auto-conversion.
//!
//! The unified JSON is the serialized [`DegreeProgram`] (the same model YAML
//! produces) with prerequisites carried as the symmetric tagged structure
//! (`{"and"|"or": [...]}`, bare string = leaf). On load we also accept a raw
//! ai-landscape program file and convert it on the fly (see
//! [`crate::core::degree::landscape_convert`]).

use std::path::Path;

use serde_json::Value;

use super::landscape_convert::convert_landscape_str;
use super::yaml_parser::{resolve_prerequisites, DegreeParseError};
use crate::core::models::DegreeProgram;
use crate::core::prerequisite_parser::parse_to_ast;

/// Parse a unified degree JSON string (auto-converting ai-landscape files).
///
/// # Errors
/// Returns an error if the JSON is invalid or matches neither shape.
pub fn parse_degree_json(json: &str) -> Result<DegreeProgram, DegreeParseError> {
    let (program, warnings) = parse_degree_json_with_warnings(json)?;
    for w in &warnings {
        eprintln!("warning: {w}");
    }
    Ok(program)
}

/// Parse a unified degree JSON string, returning conversion warnings (if any).
///
/// # Errors
/// Returns an error if the JSON is invalid or matches neither shape.
pub fn parse_degree_json_with_warnings(
    json: &str,
) -> Result<(DegreeProgram, Vec<String>), DegreeParseError> {
    let value: Value = serde_json::from_str(json)
        .map_err(|e| DegreeParseError::json_message(format!("Failed to parse JSON: {e}")))?;

    let (mut program, warnings) = if looks_like_landscape(&value) {
        let result = convert_landscape_str(json).map_err(DegreeParseError::json_message)?;
        (result.program, result.warnings)
    } else {
        let program: DegreeProgram = serde_json::from_value(value).map_err(|e| {
            DegreeParseError::json_message(format!(
                "unrecognized JSON ({e}). Expected a unified degree (top-level `degree`) or an \
                 ai-landscape program (a `courses` category map). Cluster pipeline files nest \
                 programs under `course_scraper.<program>.results` / `course_verifier...` — run \
                 `degree convert` on those (it expands each program)."
            ))
        })?;
        (program, Vec::new())
    };

    resolve_prerequisites(&mut program);
    Ok((program, warnings))
}

/// Heuristic: an ai-landscape file has a `courses` object whose values are
/// arrays (category -> list), whereas the unified format maps course keys to
/// course objects.
fn looks_like_landscape(value: &Value) -> bool {
    value
        .get("courses")
        .and_then(Value::as_object)
        .is_some_and(|courses| courses.values().next().is_some_and(Value::is_array))
}

/// Load a unified degree JSON file (auto-converting ai-landscape files).
///
/// # Errors
/// Returns an error if the file cannot be read or parsed.
pub fn load_degree_from_json<P: AsRef<Path>>(path: P) -> Result<DegreeProgram, DegreeParseError> {
    let path = path.as_ref();
    let contents = std::fs::read_to_string(path).map_err(|e| {
        DegreeParseError::IoError(format!("Failed to read {}: {e}", path.display()))
    })?;
    let (program, warnings) = parse_degree_json_with_warnings(&contents)?;
    for w in &warnings {
        eprintln!("warning ({}): {w}", path.display());
    }
    Ok(program)
}

/// Serialize a degree program to unified JSON, emitting prerequisites in the
/// structured tagged form. Set `pretty` for human-readable output.
///
/// # Errors
/// Returns an error if serialization fails.
pub fn serialize_degree_json(
    program: &DegreeProgram,
    pretty: bool,
) -> Result<String, DegreeParseError> {
    let value = to_unified_value(program)?;
    let out = if pretty {
        serde_json::to_string_pretty(&value)
    } else {
        serde_json::to_string(&value)
    };
    out.map_err(|e| DegreeParseError::json_message(format!("Failed to serialize JSON: {e}")))
}

/// Build the unified-JSON `Value` for a program.
///
/// Serializes the model, then rewrites each course's `prerequisites_raw`
/// boolean string into the structured `{"and"|"or": ...}` tagged form under the
/// `prerequisites` key.
///
/// # Errors
/// Returns an error if model serialization fails.
pub fn to_unified_value(program: &DegreeProgram) -> Result<Value, DegreeParseError> {
    let mut value = serde_json::to_value(program)
        .map_err(|e| DegreeParseError::json_message(format!("Failed to serialize program: {e}")))?;
    structurize_prereqs(&mut value);
    Ok(value)
}

/// Replace each course's `prerequisites_raw` string with the structured tagged
/// `prerequisites` object. Courses without a parseable expression are left
/// without a prerequisites field.
fn structurize_prereqs(value: &mut Value) {
    let Some(courses) = value.get_mut("courses").and_then(Value::as_object_mut) else {
        return;
    };
    for course in courses.values_mut() {
        let Some(obj) = course.as_object_mut() else {
            continue;
        };
        if let Some(raw) = obj.remove("prerequisites_raw") {
            if let Some(expr) = raw.as_str().and_then(parse_to_ast) {
                if let Ok(structured) = serde_json::to_value(&expr) {
                    obj.insert("prerequisites".to_string(), structured);
                }
            }
        }
    }
}

/// Save a degree program to a unified JSON file (pretty-printed).
///
/// # Errors
/// Returns an error if the file cannot be written or serialization fails.
pub fn save_degree_to_json<P: AsRef<Path>>(
    program: &DegreeProgram,
    path: P,
) -> Result<(), DegreeParseError> {
    let path = path.as_ref();
    let json = serialize_degree_json(program, true)?;
    std::fs::write(path, json)
        .map_err(|e| DegreeParseError::IoError(format!("Failed to write {}: {e}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_unified_json_structured_prereqs() {
        let landscape = r#"{
            "university": "Test U",
            "degree": "Bachelor's of Science Computer Science",
            "ai_program": null,
            "courses": {
                "cs_course_core": [
                    {"course_code":"CS 101","title":"Intro","course_hours":"4","prerequisites":[]},
                    {"course_code":"CS 201","title":"DS","course_hours":"4","prerequisites":[["CS 101"]]}
                ]
            }
        }"#;

        // Auto-convert from landscape shape.
        let (program, _warn) = parse_degree_json_with_warnings(landscape).unwrap();
        assert!(program.courses.contains_key("CS201"));

        // Serialize to unified JSON: prereqs become the structured tagged form.
        let unified = serialize_degree_json(&program, false).unwrap();
        assert!(unified.contains("\"prerequisites\""));
        assert!(!unified.contains("prerequisites_raw"));

        // Re-parse the unified JSON (now the non-landscape branch) and confirm
        // the prerequisite survives the round-trip.
        let reparsed = parse_degree_json(&unified).unwrap();
        assert_eq!(
            reparsed.courses["CS201"].prerequisites_raw.as_deref(),
            Some("CS101")
        );
    }

    #[test]
    fn test_parse_degree_json_invalid_json_errors() {
        assert!(parse_degree_json_with_warnings("{ not json ").is_err());
    }

    #[test]
    fn test_parse_degree_json_cluster_shape_errors_with_guidance() {
        // A cluster pipeline file is neither landscape nor a unified degree, so
        // the loader errors — and the message steers the user to `degree convert`.
        let cluster = r#"{"course_verifier":{"CS BS":{"results":{
            "university":"U","degree":"BS","courses":{
                "cs_course_core":[{"course_code":"CS 101","title":"Intro","course_hours":"3","prerequisites":[]}]}}}}}"#;
        let msg = parse_degree_json_with_warnings(cluster)
            .unwrap_err()
            .to_string();
        assert!(
            msg.contains("course_scraper") || msg.contains("degree convert"),
            "cluster files should be steered to `degree convert`; got: {msg}"
        );
    }

    #[test]
    fn test_parse_degree_json_neither_shape_errors() {
        // Valid JSON, `courses` maps a key to an object (so not landscape), but
        // the value isn't a valid Course -> unified branch must surface an error.
        let bad = r#"{"courses":{"CS1":{"credit_hours":"not-a-number"}}}"#;
        assert!(parse_degree_json_with_warnings(bad).is_err());
    }

    #[test]
    fn test_load_and_save_degree_json_file_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let landscape = dir.path().join("prog.json");
        std::fs::write(
            &landscape,
            r#"{"university":"U","degree":"BS CS","ai_program":null,
                "courses":{"cs_course_core":[
                    {"course_code":"CS 201","title":"DS","course_hours":"4",
                     "prerequisites":[["CS 101"]]}]}}"#,
        )
        .unwrap();

        // Load auto-converts the landscape file.
        let program = load_degree_from_json(&landscape).unwrap();
        assert!(program.courses.contains_key("CS201"));

        // Save to unified JSON and reload through the file path.
        let unified = dir.path().join("prog.unified.json");
        save_degree_to_json(&program, &unified).unwrap();
        let reloaded = load_degree_from_json(&unified).unwrap();
        assert_eq!(
            reloaded.courses["CS201"].prerequisites_raw.as_deref(),
            Some("CS101")
        );
    }

    #[test]
    fn test_detects_landscape_vs_unified() {
        let landscape: Value =
            serde_json::from_str(r#"{"courses":{"cs_course_core":[]}}"#).unwrap();
        // empty arrays -> first value is array
        assert!(looks_like_landscape(
            &serde_json::from_str(r#"{"courses":{"core":[{"course_code":"CS1"}]}}"#).unwrap()
        ));
        // unified: course keys map to objects
        assert!(!looks_like_landscape(
            &serde_json::from_str(r#"{"courses":{"CS1":{"name":"x"}}}"#).unwrap()
        ));
        let _ = landscape;
    }
}
