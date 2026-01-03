//! CSV parser for curriculum data

use crate::core::models::{Course, Degree, School};
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
pub fn parse_curriculum_csv<P: AsRef<Path>>(path: P) -> Result<School, Box<dyn Error>> {
    let content = fs::read_to_string(path)?;
    let lines: Vec<&str> = content.lines().collect();

    // Parse metadata section
    let metadata = parse_metadata(&lines)?;

    // Create school and degree
    let mut school = School::new(metadata.institution.clone());
    let degree = Degree::new(
        metadata.name.clone(),
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

    // First pass: create all courses without prerequisites
    let mut courses_by_key: HashMap<String, Course> = HashMap::new();
    let mut course_id_to_key: HashMap<String, String> = HashMap::new();

    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(course) = parse_course_line(line, &headers) {
            let key = course.key();
            if let Some(course_id) = get_field(line, "Course ID", &headers) {
                course_id_to_key.insert(course_id.to_string(), key.clone());
            }
            courses_by_key.insert(key, course);
        }
    }

    // Second pass: add prerequisites and corequisites, converting Course IDs to course keys
    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(course_key) = extract_course_key(line, &headers) {
            if let Some(course) = courses_by_key.get_mut(&course_key) {
                // Parse prerequisites
                if let Some(prereq_str) = get_field(line, "Prerequisites", &headers) {
                    if !prereq_str.trim().is_empty() {
                        add_prerequisites_with_mapping(course, prereq_str, &course_id_to_key);
                    }
                }

                // Parse corequisites
                if let Some(coreq_str) = get_field(line, "Corequisites", &headers) {
                    if !coreq_str.trim().is_empty() {
                        add_corequisites_with_mapping(course, coreq_str, &course_id_to_key);
                    }
                }
            }
        }
    }

    // Add all courses to school
    for course in courses_by_key.into_values() {
        school.add_course(course);
    }

    Ok(school)
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
        let parts: Vec<&str> = line.split(',').map(str::trim).collect();
        if parts.len() < 2 {
            continue;
        }

        let key = parts[0].to_lowercase();
        let value = parts[1].to_string();

        match key.as_str() {
            "curriculum" => metadata.name = value,
            "institution" => metadata.institution = value,
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
    line.split(',')
        .map(str::trim)
        .map(std::string::ToString::to_string)
        .collect()
}

/// Parse a single course line from the CSV
fn parse_course_line(line: &str, headers: &[String]) -> Result<Course, Box<dyn Error>> {
    let _fields = parse_csv_line(line);

    let name = get_field(line, "Course Name", headers)
        .unwrap_or_default()
        .to_string();
    let prefix = get_field(line, "Prefix", headers)
        .unwrap_or_default()
        .to_string();
    let number = get_field(line, "Number", headers)
        .unwrap_or_default()
        .to_string();

    let credit_hours_str = get_field(line, "Credit Hours", headers).unwrap_or("0");
    let credit_hours = credit_hours_str.parse::<f32>().unwrap_or(0.0);

    if prefix.is_empty() || number.is_empty() {
        return Err("Missing prefix or number".into());
    }

    let mut course = Course::new(name, prefix, number, credit_hours);

    // Set optional fields
    if let Some(canonical) = get_field(line, "Canonical Name", headers) {
        if !canonical.is_empty() {
            course.set_canonical_name(canonical.to_string());
        }
    }

    Ok(course)
}

/// Extract the course key (PREFIXNUMBER) from a course line
fn extract_course_key(line: &str, headers: &[String]) -> Result<String, Box<dyn Error>> {
    let prefix = get_field(line, "Prefix", headers).ok_or("Missing Prefix")?;
    let number = get_field(line, "Number", headers).ok_or("Missing Number")?;

    Ok(format!("{prefix}{number}"))
}

/// Get a field value from a CSV line by header name
fn get_field<'a>(line: &'a str, header_name: &str, headers: &[String]) -> Option<&'a str> {
    let fields: Vec<&str> = line.split(',').map(str::trim).collect();

    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(header_name))
        .and_then(|idx| fields.get(idx))
        .copied()
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
fn add_corequisites_with_mapping(
    course: &mut Course,
    coreq_str: &str,
    course_id_to_key: &HashMap<String, String>,
) {
    for coreq in coreq_str.split(';') {
        let trimmed = coreq.trim();
        if !trimmed.is_empty() {
            // Try to map course ID to key, otherwise use as-is
            if let Some(key) = course_id_to_key.get(trimmed) {
                course.add_corequisite(key.clone());
            } else {
                // Fall back to normalizing as course key
                let normalized = normalize_course_key(trimmed);
                if !normalized.is_empty() {
                    course.add_corequisite(normalized);
                }
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
