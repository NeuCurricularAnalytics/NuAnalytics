//! `get_reference`: the material to read before writing a degree or a query.
//!
//! One tool with a `topic` rather than a tool per document, so the tool list stays short
//! and a model looking for "the format" finds one place. Everything is compiled in, so it
//! answers with no database and no filesystem.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

use crate::mcp::tools::{json_schema, schema};

/// What to look up.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ReferenceTopic {
    /// The degree format, as written in YAML: fields, requirement types, examples.
    DegreeYaml,
    /// The JSON Schema (2020-12) a unified degree JSON validates against.
    DegreeJsonSchema,
    /// The database's tables and columns, and how they join — for `query_sql`.
    Database,
}

/// Request parameters for `get_reference`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetReferenceRequest {
    /// What to look up.
    #[schemars(
        description = "\"degree-yaml\" (the degree format), \"degree-json-schema\" (the JSON Schema for unified degree JSON) or \"database\" (tables, columns and how they join)"
    )]
    pub topic: ReferenceTopic,
    /// `degree-yaml` only: one section of the format.
    #[schemars(
        description = "degree-yaml only: one section, e.g. \"quickstart\" or \"requirements\""
    )]
    pub section: Option<String>,
    /// `database` only: one table in full.
    #[schemars(description = "database only: one table's full definition, e.g. \"analysis_runs\"")]
    pub table: Option<String>,
}

/// Answer a reference request.
#[must_use]
pub fn execute(req: &GetReferenceRequest) -> String {
    match req.topic {
        ReferenceTopic::DegreeYaml => schema::execute(req.section.as_deref()),
        ReferenceTopic::DegreeJsonSchema => json_schema::execute(),
        ReferenceTopic::Database => crate::core::json::to_json_pretty(
            &crate::core::query::schema_doc::describe(req.table.as_deref()),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(topic: &str, extra: &serde_json::Value) -> String {
        let mut req = serde_json::json!({ "topic": topic });
        if let (Some(obj), Some(more)) = (req.as_object_mut(), extra.as_object()) {
            obj.extend(more.clone());
        }
        execute(&serde_json::from_value(req).expect("request decodes"))
    }

    #[test]
    fn each_topic_answers_from_what_is_compiled_in() {
        assert!(ask("degree-yaml", &serde_json::json!({})).contains("requirements"));
        let schema: serde_json::Value =
            serde_json::from_str(&ask("degree-json-schema", &serde_json::json!({}))).expect("json");
        assert!(schema.get("$schema").is_some() || schema.get("properties").is_some());
        let db: serde_json::Value =
            serde_json::from_str(&ask("database", &serde_json::json!({"table": "programs"})))
                .expect("json");
        assert_eq!(db["name"], "programs");
    }

    #[test]
    fn an_unknown_topic_is_refused_at_decoding() {
        let bad: Result<GetReferenceRequest, _> =
            serde_json::from_value(serde_json::json!({ "topic": "nope" }));
        assert!(bad.is_err());
    }
}
