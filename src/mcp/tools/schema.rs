//! Schema documentation tool
//!
//! Provides the `get_degree_schema` MCP tool that returns documentation
//! about the degree YAML format.

use crate::mcp::schema_content::get_schema_content;
use rmcp::schemars;
use serde::Deserialize;

/// Request parameters for the `get_degree_schema` tool
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetSchemaRequest {
    /// Optional section filter: "all", "degree", "requirements", "courses", or "examples"
    #[schemars(
        description = "Section to return: 'all' (default), 'degree', 'requirements', 'courses', or 'examples'"
    )]
    pub section: Option<String>,
}

/// Execute the `get_degree_schema` tool
///
/// # Arguments
/// * `section` - Optional section filter
///
/// # Returns
/// Schema documentation sourced from the embedded `Degree-schema.yaml` asset
#[must_use]
pub fn execute(section: Option<&str>) -> String {
    get_schema_content(section.unwrap_or("all"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_all_schema() {
        let result = execute(None);
        assert!(result.contains("DEGREE REQUIREMENTS SCHEMA"));
        assert!(result.contains("DEGREE METADATA"));
        assert!(result.contains("REQUIREMENTS"));
        assert!(result.contains("COURSES"));
    }

    #[test]
    fn test_get_degree_section() {
        let result = execute(Some("degree"));
        assert!(result.contains("DEGREE METADATA"));
        assert!(!result.contains("PREREQUISITE EXPRESSION SYNTAX"));
    }

    #[test]
    fn test_get_requirements_section() {
        let result = execute(Some("requirements"));
        assert!(result.contains("REQUIREMENTS"));
        assert!(result.contains("type: all"));
    }

    #[test]
    fn test_get_courses_section() {
        let result = execute(Some("courses"));
        assert!(result.contains("COURSES"));
        assert!(result.contains("prerequisites"));
    }

    #[test]
    fn test_get_examples_section() {
        let result = execute(Some("examples"));
        assert!(result.contains("COMPLETE EXAMPLE"));
        assert!(result.contains("BEST PRACTICES"));
    }

    #[test]
    fn test_unknown_section_returns_all() {
        let result = execute(Some("unknown_section"));
        assert!(result.contains("DEGREE REQUIREMENTS SCHEMA"));
        assert!(result.contains("DEGREE METADATA"));
    }

    #[test]
    fn test_case_insensitive_sections() {
        let upper = execute(Some("DEGREE"));
        let lower = execute(Some("degree"));
        let mixed = execute(Some("Degree"));
        assert_eq!(upper, lower);
        assert_eq!(lower, mixed);
    }
}
