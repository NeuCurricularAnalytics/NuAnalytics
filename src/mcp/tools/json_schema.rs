//! The unified degree JSON Schema: `get_reference(topic="degree-json-schema")`.
//!
//! The checked-in `degree.schema.json` the CLI's `degree schema` command also emits, for
//! validating a converted or authored unified degree, or reading the format — wildcard
//! `from` pools included — programmatically. The `degree-yaml` topic is the human-readable
//! reference instead.

/// The unified-degree JSON Schema, embedded at compile time so it ships with
/// the binary (single source of truth shared with the CLI).
const UNIFIED_SCHEMA: &str = include_str!("../../assets/degree.schema.json");

/// Return the unified-degree JSON Schema (JSON Schema 2020-12) as a string,
/// ready to hand to a validator.
#[must_use]
pub fn execute() -> String {
    UNIFIED_SCHEMA.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_valid_json_with_unified_shape() {
        let schema = execute();
        let v: serde_json::Value = serde_json::from_str(&schema).expect("schema is valid JSON");
        // The unified degree requires these three top-level keys.
        let names: Vec<&str> = v
            .get("required")
            .and_then(serde_json::Value::as_array)
            .expect("required array")
            .iter()
            .filter_map(serde_json::Value::as_str)
            .collect();
        assert!(names.contains(&"degree"));
        assert!(names.contains(&"requirements"));
        assert!(names.contains(&"courses"));
    }

    #[test]
    fn schema_documents_wildcard_from_pools() {
        // The fix that prompted this tool: `from` must document pattern/courses
        // so wildcard pools are discoverable, not an untyped object.
        let v: serde_json::Value = serde_json::from_str(&execute()).unwrap();
        let from_clause = v
            .get("$defs")
            .and_then(|d| d.get("fromClause"))
            .expect("fromClause def present");
        let props = from_clause
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .expect("fromClause properties");
        assert!(props.contains_key("courses"));
        assert!(props.contains_key("pattern"));
        assert!(props.contains_key("include"));
    }
}
