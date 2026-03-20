//! Schema documentation content for degree YAML files
//!
//! Loads the degree schema from the embedded asset file (`src/assets/Degree-schema.yaml`)
//! and serves sections on demand. The YAML file is the single source of truth for schema
//! documentation; this module only handles section extraction.

/// The full schema file, embedded at compile time from `src/assets/Degree-schema.yaml`
const SCHEMA_YAML: &str = include_str!("../assets/Degree-schema.yaml");

/// Section delimiter pattern used in the schema file
const SECTION_DELIMITER: &str =
    "# =============================================================================";

/// Get schema content for a given section
///
/// # Arguments
/// * `section` - One of: "all", "degree", "requirements", "courses", "examples"
///
/// # Returns
/// The relevant portion of the schema YAML (with comment-based documentation)
#[must_use]
pub fn get_schema_content(section: &str) -> String {
    match section.to_lowercase().as_str() {
        "degree" => extract_sections(&["DEGREE METADATA"]),
        "requirements" => {
            extract_sections(&["^REQUIREMENTS", "FROM BLOCK", "COURSE REFERENCE SYNTAX"])
        }
        "courses" => extract_sections(&["^COURSES", "PREREQUISITE EXPRESSION SYNTAX"]),
        "examples" => {
            extract_sections(&["COMPLETE EXAMPLE", "GENERAL EDUCATION", "BEST PRACTICES"])
        }
        _ => SCHEMA_YAML.to_string(),
    }
}

/// Extract one or more named sections from the schema file
///
/// Sections are delimited by `# ====...====` lines. A section starts at the
/// delimiter line containing the header and ends just before the next top-level
/// delimiter pair.
fn extract_sections(headers: &[&str]) -> String {
    let sections = parse_all_sections();
    let mut result = String::new();

    for header in headers {
        // A `^` prefix means "starts with" to avoid partial matches
        // (e.g., "^REQUIREMENTS" won't match "DEGREE REQUIREMENTS SCHEMA")
        let (starts_with, pattern) = header.strip_prefix('^').map_or_else(
            || (false, header.to_uppercase()),
            |stripped| (true, stripped.to_uppercase()),
        );

        if let Some(content) = sections.iter().find_map(|(name, body)| {
            let name_upper = name.to_uppercase();
            let matches = if starts_with {
                name_upper.starts_with(&pattern)
            } else {
                name_upper.contains(&pattern)
            };
            if matches {
                Some(body.as_str())
            } else {
                None
            }
        }) {
            if !result.is_empty() {
                result.push('\n');
            }
            result.push_str(content);
        }
    }

    if result.is_empty() {
        SCHEMA_YAML.to_string()
    } else {
        result
    }
}

/// Parse the schema file into named sections
///
/// Returns a list of `(section_name, section_content)` pairs. Each section
/// includes its delimiter header and all content up to the next section.
fn parse_all_sections() -> Vec<(String, String)> {
    let mut sections = Vec::new();
    let lines: Vec<&str> = SCHEMA_YAML.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        // Look for a section: delimiter, then header line, then delimiter
        if lines[i].starts_with(SECTION_DELIMITER) && i + 2 < lines.len() {
            let header_line = lines[i + 1].trim_start_matches('#').trim();

            // Only treat as a section header if the next line is also a delimiter
            if lines[i + 2].starts_with(SECTION_DELIMITER) && !header_line.is_empty() {
                let section_name = header_line.to_string();
                let start = i;
                i += 3; // skip past the header block

                // Collect lines until the next section header
                while i < lines.len() {
                    if lines[i].starts_with(SECTION_DELIMITER)
                        && i + 2 < lines.len()
                        && lines[i + 2].starts_with(SECTION_DELIMITER)
                    {
                        break;
                    }
                    i += 1;
                }

                let content: String = lines[start..i].to_vec().join("\n");

                sections.push((section_name, content));
                continue;
            }
        }
        i += 1;
    }

    sections
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_schema_yaml_is_loaded() {
        assert!(
            !SCHEMA_YAML.is_empty(),
            "Schema YAML should be embedded at compile time"
        );
        assert!(SCHEMA_YAML.contains("DEGREE REQUIREMENTS SCHEMA"));
    }

    #[test]
    fn test_parse_all_sections_finds_sections() {
        let sections = parse_all_sections();
        let names: Vec<&str> = sections.iter().map(|(n, _)| n.as_str()).collect();

        assert!(
            names.iter().any(|n| n.contains("DEGREE METADATA")),
            "Should find DEGREE METADATA section, found: {names:?}"
        );
        assert!(
            names.iter().any(|n| n.contains("REQUIREMENTS")),
            "Should find REQUIREMENTS section"
        );
        assert!(
            names.iter().any(|n| n.contains("COURSES")),
            "Should find COURSES section"
        );
    }

    #[test]
    fn test_get_all_returns_full_schema() {
        let result = get_schema_content("all");
        assert!(result.contains("DEGREE REQUIREMENTS SCHEMA"));
        assert!(result.contains("DEGREE METADATA"));
        assert!(result.contains("REQUIREMENTS"));
        assert!(result.contains("COURSES"));
    }

    #[test]
    fn test_get_degree_section() {
        let result = get_schema_content("degree");
        assert!(result.contains("DEGREE METADATA"));
        assert!(!result.contains("PREREQUISITE EXPRESSION SYNTAX"));
    }

    #[test]
    fn test_get_requirements_section() {
        let result = get_schema_content("requirements");
        assert!(result.contains("REQUIREMENTS"));
        assert!(result.contains("type: all"));
    }

    #[test]
    fn test_get_courses_section() {
        let result = get_schema_content("courses");
        assert!(result.contains("COURSES"));
        assert!(result.contains("prerequisites"));
    }

    #[test]
    fn test_get_examples_section() {
        let result = get_schema_content("examples");
        assert!(result.contains("COMPLETE EXAMPLE"));
        assert!(result.contains("BEST PRACTICES"));
    }

    #[test]
    fn test_unknown_section_returns_all() {
        let result = get_schema_content("unknown_section");
        assert!(result.contains("DEGREE REQUIREMENTS SCHEMA"));
        assert!(result.contains("DEGREE METADATA"));
    }

    #[test]
    fn test_case_insensitive_sections() {
        let upper = get_schema_content("DEGREE");
        let lower = get_schema_content("degree");
        let mixed = get_schema_content("Degree");
        assert_eq!(upper, lower);
        assert_eq!(lower, mixed);
    }
}
