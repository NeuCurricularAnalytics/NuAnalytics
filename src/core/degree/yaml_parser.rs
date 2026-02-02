//! YAML degree parser
//!
//! Handles parsing degree program definitions from YAML strings or files.
//! Supports both file-based loading and string parsing for network sources.

use super::models::YamlDegree;
use std::path::Path;

/// Error type for degree YAML parsing
#[derive(Debug)]
pub enum DegreeParseError {
    /// File I/O error
    IoError(String),

    /// YAML parse error
    YamlError(String),
}

impl std::fmt::Display for DegreeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(msg) => write!(f, "IO Error: {msg}"),
            Self::YamlError(msg) => write!(f, "YAML Parse Error: {msg}"),
        }
    }
}

impl std::error::Error for DegreeParseError {}

/// Parse a degree program from a YAML string
///
/// Use this function when you have YAML content from any source
/// (file, network, database, etc.)
///
/// # Arguments
/// * `yaml_content` - YAML string containing the degree definition
///
/// # Returns
/// A parsed `YamlDegree` on success, or `DegreeParseError` on failure
///
/// # Errors
/// Returns an error if the YAML is invalid or doesn't match the expected schema
///
/// # Example
/// ```no_run
/// use nu_analytics::core::degree::parse_degree_yaml;
///
/// let yaml = r#"
/// degree:
///   id: test-degree
///   institution: Test University
///   program: BS Test
///   catalog_year: "2024-2025"
///   total_credits: 120
///   gpa_minimum: 2.0
///   allow_double_counting: false
/// requirements: {}
/// courses: {}
/// "#;
///
/// let degree = parse_degree_yaml(yaml).unwrap();
/// assert_eq!(degree.degree.id, "test-degree");
/// ```
pub fn parse_degree_yaml(yaml_content: &str) -> Result<YamlDegree, DegreeParseError> {
    serde_yaml::from_str::<YamlDegree>(yaml_content)
        .map_err(|e| DegreeParseError::YamlError(format!("Failed to parse YAML: {e}")))
}

/// Load a degree program from a YAML file
///
/// Convenience function that reads a file and parses it as YAML.
/// For loading from other sources (network, etc.), use `parse_degree_yaml` directly.
///
/// # Arguments
/// * `path` - Path to the YAML file containing the degree definition
///
/// # Returns
/// A parsed `YamlDegree` on success, or `DegreeParseError` on failure
///
/// # Errors
/// Returns an error if the file cannot be read or the YAML is invalid
///
/// # Example
/// ```no_run
/// use nu_analytics::core::degree::load_degree_from_yaml;
///
/// let degree = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml").unwrap();
/// println!("Loaded: {}", degree.degree.id);
/// ```
pub fn load_degree_from_yaml<P: AsRef<Path>>(path: P) -> Result<YamlDegree, DegreeParseError> {
    let path = path.as_ref();

    // Read file
    let contents = std::fs::read_to_string(path).map_err(|e| {
        DegreeParseError::IoError(format!("Failed to read {}: {e}", path.display()))
    })?;

    // Parse YAML using the string parser
    parse_degree_yaml(&contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_parse_degree_yaml_valid() {
        let yaml_content = r#"
degree:
  id: test-degree
  institution: Test University
  program: Bachelor of Science in Test
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  allow_double_counting: false

requirements: {}
courses: {}
"#;

        let result = parse_degree_yaml(yaml_content);
        assert!(result.is_ok());

        let degree = result.unwrap();
        assert_eq!(degree.degree.id, "test-degree");
        assert_eq!(degree.degree.institution, "Test University");
        assert_eq!(degree.degree.total_credits, 120);
    }

    #[test]
    fn test_parse_degree_yaml_invalid() {
        let yaml_content = "invalid: yaml: content: [";

        let result = parse_degree_yaml(yaml_content);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_degree_yaml_with_courses() {
        let yaml_content = r#"
degree:
  id: test-degree
  institution: Test University
  program: BS Test
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  allow_double_counting: false

requirements:
  intro:
    name: Introduction
    type: all
    category: major
    courses:
      - CS101
      - CS102

courses:
  CS101:
    subject: CS
    number: "101"
    title: Intro to CS
    credits: 3
  CS102:
    subject: CS
    number: "102"
    title: Programming
    credits: 4
    prerequisites: "CS101"
"#;

        let result = parse_degree_yaml(yaml_content);
        assert!(result.is_ok());

        let degree = result.unwrap();
        assert_eq!(degree.courses.len(), 2);
        assert!(degree.courses.contains_key("CS101"));
        assert!(degree.courses.contains_key("CS102"));

        let cs102 = degree.courses.get("CS102").unwrap();
        assert_eq!(cs102.prerequisites, Some("CS101".to_string()));
    }

    #[test]
    fn test_load_degree_file_not_found() {
        let result = load_degree_from_yaml("/nonexistent/path/degree.yaml");
        assert!(result.is_err());
    }

    #[test]
    fn test_load_degree_from_file() -> Result<(), Box<dyn std::error::Error>> {
        let temp_dir = TempDir::new()?;
        let yaml_file = temp_dir.path().join("degree.yaml");

        let yaml_content = r#"
degree:
  id: file-test-degree
  institution: File Test University
  program: BS File Test
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  allow_double_counting: false

requirements: {}
courses: {}
"#;

        fs::write(&yaml_file, yaml_content)?;

        let result = load_degree_from_yaml(&yaml_file);
        assert!(result.is_ok());

        let degree = result.unwrap();
        assert_eq!(degree.degree.id, "file-test-degree");

        Ok(())
    }
}
