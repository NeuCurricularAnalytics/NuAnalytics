//! CSV parser for curriculum data

use crate::core::models::{Course, Degree, Plan, School};
use std::collections::HashMap;
use std::error::Error;
use std::fs;
use std::path::Path;

/// Represents parsed curriculum metadata from CSV header
#[derive(Debug, Clone)]
pub struct CurriculumMetadata {
    /// Curriculum name
    pub name: String,
    /// Institution name
    pub institution: String,
    /// Degree type (BS, BA, etc.)
    pub degree_type: String,
    /// System type (semester, quarter, etc.)
    pub system_type: String,
    /// CIP code for the degree
    pub cip_code: String,
}

/// Parse a curriculum CSV file and return a School object with all courses and degrees
///
/// # Arguments
/// * `path` - Path to the CSV file
///
/// # Returns
/// A `School` object populated with courses and degrees from the file
///
/// # Errors
/// Returns an error if file cannot be read or parsed
///
/// # Panics
/// Panics if a course's natural key is not found in the `natural_key_to_ids` map during deduplication
pub fn parse_curriculum_csv<P: AsRef<Path>>(path: P) -> Result<School, Box<dyn Error>> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    // Parse metadata section
    let metadata = parse_metadata(&lines)?;

    // Extract metadata values to avoid redundant clones
    let curriculum_name = metadata.name;
    let institution_name = metadata.institution;

    // Create school and degree
    let mut school = School::new(institution_name);
    let degree = Degree::new(
        curriculum_name.clone(),
        metadata.degree_type.clone(),
        metadata.cip_code,
    );
    school.add_degree(degree);

    // Find the courses section
    let courses_start = lines
        .iter()
        .position(|line| line.to_lowercase().contains("courses"))
        .ok_or("No 'Courses' section found in CSV")?;

    if courses_start + 1 >= lines.len() {
        return Err("No course header found".into());
    }

    let header_line = lines[courses_start + 1];
    let headers = parse_csv_line(header_line);

    // First pass: Load all courses with their IDs and natural keys
    // Build a mapping from Course ID -> natural key to detect duplicates
    let mut course_id_to_natural_key: HashMap<String, String> = HashMap::new();
    let mut natural_key_to_ids: HashMap<String, Vec<String>> = HashMap::new();
    let mut courses_by_id: HashMap<String, Course> = HashMap::new();
    let mut course_ids_in_order: Vec<String> = Vec::new();

    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(mut course) = parse_course_line(line, &headers) {
            let natural_key = course.key(); // e.g., "XXXX"

            if let Some(course_id) = get_field(line, "Course ID", &headers) {
                course.csv_id = Some(course_id.clone());

                // Track this course ID
                course_id_to_natural_key.insert(course_id.clone(), natural_key.clone());
                natural_key_to_ids
                    .entry(natural_key)
                    .or_default()
                    .push(course_id.clone());
                courses_by_id.insert(course_id.clone(), course);
                course_ids_in_order.push(course_id);
            }
        }
    }

    // Second pass: Determine final storage keys
    // If a natural key has multiple IDs, append the ID to make it unique
    let mut course_id_to_storage_key: HashMap<String, String> = HashMap::new();

    for (course_id, natural_key) in &course_id_to_natural_key {
        let ids_with_same_key = natural_key_to_ids.get(natural_key).unwrap();
        let storage_key = if ids_with_same_key.len() > 1 {
            // Conflict - append ID
            format!("{natural_key}_{course_id}")
        } else {
            // No conflict - use natural key
            natural_key.clone()
        };

        course_id_to_storage_key.insert(course_id.clone(), storage_key);
    }

    // Third pass: Add prerequisites using course IDs, which will be converted to storage keys
    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        if let Some(course_id) = get_field(line, "Course ID", &headers) {
            if let Some(course) = courses_by_id.get_mut(&course_id) {
                // Parse prerequisites
                if let Some(prereq_str) = get_field(line, "Prerequisites", &headers) {
                    if !prereq_str.trim().is_empty() {
                        add_prerequisites_with_mapping(
                            course,
                            &prereq_str,
                            &course_id_to_storage_key,
                        );
                    }
                }

                // Parse corequisites
                if let Some(coreq_str) = get_field(line, "Corequisites", &headers) {
                    if !coreq_str.trim().is_empty() {
                        add_corequisites_with_mapping(
                            course,
                            &coreq_str,
                            &course_id_to_storage_key,
                        );
                    }
                }
            }
        }
    }

    // Add all courses to school using their storage keys
    for course_id in &course_ids_in_order {
        if let Some(course) = courses_by_id.remove(course_id) {
            if let Some(storage_key) = course_id_to_storage_key.get(course_id) {
                school.add_course_with_key(storage_key.clone(), course);
            }
        }
    }

    // Create a default plan with all courses
    let mut plan = Plan::new(
        curriculum_name,
        school.degrees.first().ok_or("No degree found")?.id(),
    );
    plan.institution = Some(school.name.clone());

    // Add all courses to the plan using their storage keys in order
    for course_id in &course_ids_in_order {
        if let Some(storage_key) = course_id_to_storage_key.get(course_id) {
            plan.add_course(storage_key.clone());
        }
    }

    school.add_plan(plan);

    Ok(school)
}

/// Normalize a raw CSV field by stripping whitespace, quotes, BOM, and zero-width characters
fn clean_field(field: &str) -> String {
    field
        .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\u{feff}' || c == '\u{200b}')
        .to_string()
}

/// Parse curriculum metadata from the header section
fn parse_metadata(lines: &[&str]) -> Result<CurriculumMetadata, Box<dyn Error>> {
    let mut metadata = CurriculumMetadata {
        name: String::new(),
        institution: String::new(),
        degree_type: String::new(),
        system_type: String::new(),
        cip_code: String::new(),
    };

    for line in lines.iter().take(10) {
        let parts = parse_csv_line(line);
        if parts.len() < 2 {
            continue;
        }

        let key = parts[0].to_lowercase();
        let value = parts[1].clone();

        match key.as_str() {
            "curriculum" => metadata.name = value,
            "institution" | "insitution" => metadata.institution = value, // or because of the common typo in the plan database
            "degree type" => metadata.degree_type = value,
            "system type" => metadata.system_type = value,
            "cip" => metadata.cip_code = value,
            _ => {}
        }
    }

    // Validate required fields
    if metadata.name.is_empty() {
        return Err("Missing Curriculum name".into());
    }
    if metadata.institution.is_empty() {
        return Err("Missing Institution".into());
    }

    Ok(metadata)
}

/// Parse a CSV line into fields
fn parse_csv_line(line: &str) -> Vec<String> {
    line.split(',').map(clean_field).collect()
}

/// Parse a single course line from the CSV
fn parse_course_line(line: &str, headers: &[String]) -> Result<Course, Box<dyn Error>> {
    let _fields = parse_csv_line(line);

    let name = get_field(line, "Course Name", headers).unwrap_or_default();
    let prefix = get_field(line, "Prefix", headers).unwrap_or_default();
    let number = get_field(line, "Number", headers).unwrap_or_default();

    let credit_hours_str =
        get_field(line, "Credit Hours", headers).unwrap_or_else(|| "0".to_string());
    let credit_hours = credit_hours_str.parse::<f32>().unwrap_or(0.0);

    if prefix.is_empty() || number.is_empty() {
        return Err("Missing prefix or number".into());
    }

    let mut course = Course::new(name, prefix, number, credit_hours);

    // Set optional fields
    if let Some(canonical) = get_field(line, "Canonical Name", headers) {
        if !canonical.is_empty() {
            course.set_canonical_name(canonical);
        }
    }

    Ok(course)
}

/// Get a field value from a CSV line by header name
fn get_field(line: &str, header_name: &str, headers: &[String]) -> Option<String> {
    let fields = parse_csv_line(line);

    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(header_name))
        .and_then(|idx| fields.get(idx))
        .cloned()
}

/// Add prerequisites from a semicolon-separated string, converting course IDs to keys
fn add_prerequisites_with_mapping(
    course: &mut Course,
    prereq_str: &str,
    course_id_to_key: &HashMap<String, String>,
) {
    for prereq in prereq_str.split(';') {
        let trimmed = prereq.trim();
        if !trimmed.is_empty() {
            // Try to map course ID to key, otherwise use as-is
            if let Some(key) = course_id_to_key.get(trimmed) {
                course.add_prerequisite(key.clone());
            } else {
                // Fall back to normalizing as course key
                let normalized = normalize_course_key(trimmed);
                if !normalized.is_empty() {
                    course.add_prerequisite(normalized);
                }
            }
        }
    }
}

/// Add corequisites from a semicolon-separated string, converting course IDs to keys
/// Note: Corequisites are optional and should not create hard dependencies
fn add_corequisites_with_mapping(
    course: &mut Course,
    coreq_str: &str,
    course_id_to_key: &HashMap<String, String>,
) {
    for coreq in coreq_str.split(';') {
        let trimmed = coreq.trim();
        if !trimmed.is_empty() {
            // Try to map course ID to key; skip if mapping not found
            // (corequisites may be optional or electives that don't have explicit mappings)
            if let Some(key) = course_id_to_key.get(trimmed) {
                course.add_corequisite(key.clone());
            }
        }
    }
}

/// Normalize a course key to PREFIXNUMBER format
/// Handles cases like "CS 1800", "CS1800", "CS 1800 (or coreq)"
fn normalize_course_key(input: &str) -> String {
    // Remove parentheses and anything after them
    let cleaned = input.split('(').next().unwrap_or(input).trim();

    // Split by space and rejoin without space
    let parts: Vec<&str> = cleaned.split_whitespace().collect();

    if parts.len() >= 2 {
        format!("{}{}", parts[0], parts[1])
    } else if parts.len() == 1 {
        parts[0].to_string()
    } else {
        String::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_course_key() {
        assert_eq!(normalize_course_key("CS 1800"), "CS1800");
        assert_eq!(normalize_course_key("CS1800"), "CS1800");
        assert_eq!(normalize_course_key("MATH 1342"), "MATH1342");
        assert_eq!(normalize_course_key("CS 1800 (or coreq)"), "CS1800");
        assert_eq!(normalize_course_key("  PHYS  1151  "), "PHYS1151");
    }

    #[test]
    fn test_parse_csv_line() {
        let line = "CS1800,Discrete Structures,CS,1800,CS1700,CS1801,false,4.0,";
        let fields = parse_csv_line(line);

        assert_eq!(fields.len(), 9);
        assert_eq!(fields[0], "CS1800");
        assert_eq!(fields[1], "Discrete Structures");
        assert_eq!(fields[2], "CS");
        assert_eq!(fields[3], "1800");
    }
}
