//! YAML degree parser
//!
//! Handles parsing degree program definitions from YAML strings or files.
//! Supports both file-based loading and string parsing for network sources.

use crate::core::models::DegreeProgram;
use std::path::Path;

/// Error type for degree YAML parsing.
#[derive(Debug)]
pub enum DegreeParseError {
    /// File I/O error.
    IoError(String),

    /// YAML parse error. `line` and `column` carry the structured location
    /// pulled from `serde_yaml::Error::location()` when available so callers
    /// can render source-context windows without re-grepping the message.
    /// Both are `None` for serialisation failures or whole-document errors
    /// the parser couldn't pin to a position.
    YamlError {
        /// Human-readable error message from `serde_yaml`.
        message: String,
        /// 1-indexed source line where the error was detected, when known.
        line: Option<usize>,
        /// 1-indexed source column, when known.
        column: Option<usize>,
    },

    /// JSON parse/serialize error (unified JSON or ai-landscape input).
    JsonError(String),
}

impl DegreeParseError {
    /// Construct a `YamlError` with no location info — used for serialise
    /// failures and synthetic errors that don't carry a source position.
    #[must_use]
    pub fn yaml_message(message: impl Into<String>) -> Self {
        Self::YamlError {
            message: message.into(),
            line: None,
            column: None,
        }
    }

    /// Construct a `JsonError` from a message.
    #[must_use]
    pub fn json_message(message: impl Into<String>) -> Self {
        Self::JsonError(message.into())
    }
}

impl std::fmt::Display for DegreeParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::IoError(msg) => write!(f, "IO Error: {msg}"),
            Self::YamlError { message, .. } => write!(f, "YAML Parse Error: {message}"),
            Self::JsonError(msg) => write!(f, "JSON Parse Error: {msg}"),
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
/// A parsed `DegreeProgram` on success, or `DegreeParseError` on failure
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
///   name: BS Computer Science
///   degree_type: BS
///   system_type: semester
///   id: test-degree
///   institution: Test University
///   catalog_year: "2024-2025"
///   total_credits: 120
///   gpa_minimum: 2.0
///   allow_double_counting: false
/// requirements: {}
/// courses: {}
/// "#;
///
/// let program = parse_degree_yaml(yaml).unwrap();
/// assert_eq!(program.degree.id.unwrap(), "test-degree");
/// ```
pub fn parse_degree_yaml(yaml_content: &str) -> Result<DegreeProgram, DegreeParseError> {
    let mut program = serde_yaml::from_str::<DegreeProgram>(yaml_content).map_err(|e| {
        let location = e.location();
        DegreeParseError::YamlError {
            message: format!("Failed to parse YAML: {e}"),
            // serde_yaml's Location reports 1-indexed positions; surface them
            // unchanged so the caller-facing message matches what editors show.
            line: location.as_ref().map(serde_yaml::Location::line),
            column: location.as_ref().map(serde_yaml::Location::column),
        }
    })?;

    resolve_prerequisites(&mut program);

    Ok(program)
}

/// Helper to populate `prerequisites` vector from `prerequisites_raw` string.
///
/// Shared by the JSON parser so YAML and JSON inputs run the same
/// preprocessing stage.
pub(crate) fn resolve_prerequisites(program: &mut DegreeProgram) {
    for course in program.courses.values_mut() {
        if let Some(raw) = &course.prerequisites_raw {
            if course.prerequisites.is_empty() {
                // Parse raw string
                // 1. Replace operators with spaces
                let cleaned = raw.replace(['(', ')', '&', '|'], " ");

                // 2. Split by whitespace
                for part in cleaned.split_whitespace() {
                    // 3. Handle grade requirements [X]
                    let key = part.find('[').map_or(part, |idx| &part[..idx]);

                    if !key.is_empty() {
                        let key_string = key.to_string();
                        if !course.prerequisites.contains(&key_string) {
                            course.prerequisites.push(key_string);
                        }
                    }
                }
            }
        }
    }
}

/// Serialize a degree program to a YAML string
///
/// Use this function to export a degree program back to YAML for storage
/// or interchange.
///
/// # Errors
/// Returns an error if serialization fails
pub fn serialize_degree_yaml(program: &DegreeProgram) -> Result<String, DegreeParseError> {
    serde_yaml::to_string(program)
        .map_err(|e| DegreeParseError::yaml_message(format!("Failed to serialize YAML: {e}")))
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
/// A parsed `DegreeProgram` on success, or `DegreeParseError` on failure
///
/// # Errors
/// Returns an error if the file cannot be read or the YAML is invalid
///
/// # Example
/// ```no_run
/// use nu_analytics::core::degree::load_degree_from_yaml;
///
/// let program = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml").unwrap();
/// println!("Loaded: {}", program.degree.id.unwrap());
/// ```
pub fn load_degree_from_yaml<P: AsRef<Path>>(path: P) -> Result<DegreeProgram, DegreeParseError> {
    let path = path.as_ref();

    // Read file
    let contents = std::fs::read_to_string(path).map_err(|e| {
        DegreeParseError::IoError(format!("Failed to read {}: {e}", path.display()))
    })?;

    // Parse YAML using the string parser
    parse_degree_yaml(&contents)
}

/// Save a degree program to a YAML file
///
/// # Errors
/// Returns an error if the file cannot be written or serialization fails
pub fn save_degree_to_yaml<P: AsRef<Path>>(
    program: &DegreeProgram,
    path: P,
) -> Result<(), DegreeParseError> {
    let path = path.as_ref();
    let yaml = serialize_degree_yaml(program)?;
    std::fs::write(path, yaml)
        .map_err(|e| DegreeParseError::IoError(format!("Failed to write {}: {e}", path.display())))
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
  name: Bachelor of Science in Test
  degree_type: BS
  system_type: semester
  id: test-degree
  institution: Test University
  catalog_year: "2024-2025"
  total_credits: 120
  gpa_minimum: 2.0
  allow_double_counting: false

requirements: {}
courses: {}
"#;

        let result = parse_degree_yaml(yaml_content);
        assert!(result.is_ok());

        let program = result.unwrap();
        assert_eq!(program.degree.id, Some("test-degree".to_string()));
        assert_eq!(
            program.degree.institution,
            Some("Test University".to_string())
        );
        assert_eq!(program.degree.total_credits, Some(120));
    }

    #[test]
    fn test_parse_degree_yaml_captures_line_column_from_serde_yaml() {
        // Force a syntax error on a known line so we can assert the structured
        // location survives the wrap into DegreeParseError. Line 3 has the
        // broken mapping (`total_credits: [list, where, number, expected]`).
        let yaml = "degree:\n  id: t\n  total_credits: [list, instead, of, number]\nrequirements: {}\ncourses: {}\n";
        let err = parse_degree_yaml(yaml).expect_err("malformed YAML must error");
        match err {
            DegreeParseError::YamlError { line, column, .. } => {
                assert!(
                    line.is_some(),
                    "line must be populated by serde_yaml::Error::location()"
                );
                // serde_yaml reports a positive column for in-line mapping errors;
                // accept anything non-zero so the assertion isn't brittle.
                assert!(column.is_some());
            }
            DegreeParseError::IoError(msg) => panic!("expected YamlError, got IoError({msg})"),
            DegreeParseError::JsonError(msg) => panic!("expected YamlError, got JsonError({msg})"),
        }
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
  name: BS Test
  degree_type: BS
  system_type: semester
  id: test-degree
  institution: Test University
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
    name: Intro to CS
    prefix: CS
    number: "101"
    credit_hours: 3
  CS102:
    name: Programming
    prefix: CS
    number: "102"
    credit_hours: 4
    prerequisites_raw: "CS101"
"#;

        let result = parse_degree_yaml(yaml_content);
        if let Err(ref e) = result {
            eprintln!("Parse error: {e:?}");
        }
        assert!(result.is_ok());

        let program = result.unwrap();
        assert_eq!(program.courses.len(), 2);
        assert!(program.courses.contains_key("CS101"));
        assert!(program.courses.contains_key("CS102"));

        let cs102 = program.courses.get("CS102").unwrap();
        assert_eq!(cs102.prerequisites_raw, Some("CS101".to_string()));
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
  name: BS File Test
  degree_type: BS
  system_type: semester
  id: file-test-degree
  institution: File Test University
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

        let program = result.unwrap();
        assert_eq!(program.degree.id, Some("file-test-degree".to_string()));

        Ok(())
    }
}
