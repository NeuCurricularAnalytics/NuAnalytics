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
/// Markdown-formatted schema documentation
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
        assert!(result.contains("Degree Program YAML Schema"));
        assert!(result.contains("Degree Metadata Section"));
        assert!(result.contains("Requirements Section"));
        assert!(result.contains("Courses Section"));
    }

    #[test]
    fn test_get_degree_section() {
        let result = execute(Some("degree"));
        assert!(result.contains("Degree Metadata Section"));
        assert!(!result.contains("Requirements Section"));
    }

    #[test]
    fn test_get_requirements_section() {
        let result = execute(Some("requirements"));
        assert!(result.contains("Requirements Section"));
        assert!(result.contains("type: all"));
    }

    #[test]
    fn test_get_courses_section() {
        let result = execute(Some("courses"));
        assert!(result.contains("Courses Section"));
        assert!(result.contains("prerequisites_raw"));
    }

    #[test]
    fn test_get_examples_section() {
        let result = execute(Some("examples"));
        assert!(result.contains("Complete Example"));
        assert!(result.contains("Common Patterns"));
    }

    #[test]
    fn test_unknown_section_returns_all() {
        let result = execute(Some("unknown_section"));
        // Unknown sections should return all content
        assert!(result.contains("Degree Program YAML Schema"));
        assert!(result.contains("Degree Metadata Section"));
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
