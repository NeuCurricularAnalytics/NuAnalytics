//! CSV parser for curriculum data
//!
//! This module handles parsing CSV files from [CurricularAnalytics.org](https://curricularanalytics.org/)
//! format into the internal `School` data model. The parsing happens in three passes:
//!
//! 1. **First pass**: Load all courses and build ID-to-key mappings
//! 2. **Second pass**: Determine storage keys (handling duplicates)
//! 3. **Third pass**: Add prerequisites, corequisites using resolved keys

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

/// Intermediate data structure for first-pass course parsing
///
/// Tracks all the mappings needed to handle duplicate course keys
/// and convert CSV course IDs to storage keys.
struct CourseParseContext {
    /// Maps Course ID to natural key (e.g., "1" -> "CS101")
    course_id_to_natural_key: HashMap<String, String>,
    /// Maps natural key to all Course IDs with that key
    natural_key_to_ids: HashMap<String, Vec<String>>,
    /// Maps Course ID to parsed Course object
    courses_by_id: HashMap<String, Course>,
    /// Preserves original order of course IDs for deterministic output
    course_ids_in_order: Vec<String>,
}

impl CourseParseContext {
    /// Creates a new empty parse context
    fn new() -> Self {
        Self {
            course_id_to_natural_key: HashMap::new(),
            natural_key_to_ids: HashMap::new(),
            courses_by_id: HashMap::new(),
            course_ids_in_order: Vec::new(),
        }
    }

    /// Adds a course to the context, tracking all necessary mappings
    ///
    /// # Arguments
    /// * `course_id` - The CSV Course ID field
    /// * `course` - The parsed Course object
    fn add_course(&mut self, course_id: String, mut course: Course) {
        let natural_key = course.key();
        course.csv_id = Some(course_id.clone());

        self.course_id_to_natural_key
            .insert(course_id.clone(), natural_key.clone());
        self.natural_key_to_ids
            .entry(natural_key)
            .or_default()
            .push(course_id.clone());
        self.courses_by_id.insert(course_id.clone(), course);
        self.course_ids_in_order.push(course_id);
    }

    /// Computes final storage keys for all courses
    ///
    /// If multiple courses share the same natural key, appends the CSV ID
    /// to make the storage key unique (e.g., `CS101_2`).
    ///
    /// # Errors
    /// Returns an error if there's an internal consistency problem
    fn compute_storage_keys(&self) -> Result<HashMap<String, String>, Box<dyn Error>> {
        let mut course_id_to_storage_key = HashMap::new();

        for (course_id, natural_key) in &self.course_id_to_natural_key {
            let ids_with_same_key = self.natural_key_to_ids.get(natural_key).ok_or_else(|| {
                format!(
                    "Internal consistency error: natural key '{natural_key}' not found in key mapping"
                )
            })?;

            let storage_key = if ids_with_same_key.len() > 1 {
                format!("{natural_key}_{course_id}")
            } else {
                natural_key.clone()
            };

            course_id_to_storage_key.insert(course_id.clone(), storage_key);
        }

        Ok(course_id_to_storage_key)
    }
}

/// Parse a curriculum CSV file and return a School object with all courses and degrees
///
/// The parsing happens in three passes:
/// 1. Load all courses and build ID-to-key mappings
/// 2. Determine storage keys (handling duplicate natural keys)
/// 3. Add prerequisites and corequisites using resolved keys
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

    // Parse metadata and create school structure
    let metadata = parse_metadata(&lines)?;
    let mut school = create_school_from_metadata(&metadata);

    // Find and validate courses section
    let (courses_start, headers) = find_courses_section(&lines)?;

    // First pass: Load all courses and build mappings
    let mut ctx = CourseParseContext::new();
    first_pass_load_courses(&lines, courses_start, &headers, &mut ctx);

    // Second pass: Compute final storage keys
    let storage_keys = ctx.compute_storage_keys()?;

    // Third pass: Add prerequisites and corequisites
    third_pass_add_dependencies(&lines, courses_start, &headers, &mut ctx, &storage_keys);

    // Build the final school structure
    finalize_school(&mut school, ctx, &storage_keys, &metadata.name)?;

    Ok(school)
}

/// Creates a School and Degree from parsed metadata
fn create_school_from_metadata(metadata: &CurriculumMetadata) -> School {
    let mut school = School::new(metadata.institution.clone());
    let degree = Degree::new(
        metadata.name.clone(),
        metadata.degree_type.clone(),
        metadata.cip_code.clone(),
        metadata.system_type.clone(),
    );
    school.add_degree(degree);
    school
}

/// Finds the courses section and extracts headers
///
/// # Returns
/// Tuple of (start index, headers vector)
///
/// # Errors
/// Returns error if courses section is not found or has no header
fn find_courses_section(lines: &[&str]) -> Result<(usize, Vec<String>), Box<dyn Error>> {
    let courses_start = lines
        .iter()
        .position(|line| line.to_lowercase().contains("courses"))
        .ok_or("No 'Courses' section found in CSV")?;

    if courses_start + 1 >= lines.len() {
        return Err("No course header found".into());
    }

    let header_line = lines[courses_start + 1];
    let headers = parse_csv_line(header_line);

    Ok((courses_start, headers))
}

/// First pass: Load all courses and build ID-to-key mappings
fn first_pass_load_courses(
    lines: &[&str],
    courses_start: usize,
    headers: &[String],
    ctx: &mut CourseParseContext,
) {
    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        if let Ok(course) = parse_course_line(line, headers) {
            if let Some(course_id) = get_field(line, "Course ID", headers) {
                ctx.add_course(course_id, course);
            }
        }
    }
}

/// Third pass: Add prerequisites and corequisites using resolved storage keys
fn third_pass_add_dependencies(
    lines: &[&str],
    courses_start: usize,
    headers: &[String],
    ctx: &mut CourseParseContext,
    storage_keys: &HashMap<String, String>,
) {
    for line in lines.iter().skip(courses_start + 2) {
        if line.trim().is_empty() {
            continue;
        }

        let Some(course_id) = get_field(line, "Course ID", headers) else {
            continue;
        };

        let Some(course) = ctx.courses_by_id.get_mut(&course_id) else {
            continue;
        };

        // Parse and add prerequisites
        if let Some(prereq_str) = get_field(line, "Prerequisites", headers) {
            if !prereq_str.trim().is_empty() {
                add_prerequisites_with_mapping(course, &prereq_str, storage_keys);
            }
        }

        // Parse and add corequisites
        if let Some(coreq_str) = get_field(line, "Corequisites", headers) {
            if !coreq_str.trim().is_empty() {
                add_corequisites_with_mapping(course, &coreq_str, storage_keys);
            }
        }

        // Parse and add strict corequisites
        if let Some(strict_coreq_str) = get_field(line, "Strict-Corequisites", headers) {
            if !strict_coreq_str.trim().is_empty() {
                add_strict_corequisites_with_mapping(course, &strict_coreq_str, storage_keys);
            }
        }
    }
}

/// Finalizes the school structure with courses and a default plan
///
/// # Errors
/// Returns error if no degree was created
fn finalize_school(
    school: &mut School,
    mut ctx: CourseParseContext,
    storage_keys: &HashMap<String, String>,
    curriculum_name: &str,
) -> Result<(), Box<dyn Error>> {
    // Add all courses to school using their storage keys
    for course_id in &ctx.course_ids_in_order {
        if let Some(course) = ctx.courses_by_id.remove(course_id) {
            if let Some(storage_key) = storage_keys.get(course_id) {
                school.add_course_with_key(storage_key.clone(), course);
            }
        }
    }

    // Create a default plan with all courses
    let mut plan = Plan::new(
        curriculum_name.to_string(),
        school.degrees.first().ok_or("No degree found")?.id(),
    );
    plan.institution = Some(school.name.clone());

    // Add all courses to the plan in order
    for course_id in &ctx.course_ids_in_order {
        if let Some(storage_key) = storage_keys.get(course_id) {
            plan.add_course(storage_key.clone());
        }
    }

    school.add_plan(plan);
    Ok(())
}

/// Normalizes a raw CSV field by stripping problematic characters
///
/// Removes:
/// - Leading/trailing whitespace
/// - Double quotes from quoted fields
/// - Byte Order Mark (BOM) character `\u{feff}`
/// - Zero-width space `\u{200b}`
fn clean_field(field: &str) -> String {
    field
        .trim_matches(|c: char| c.is_whitespace() || c == '"' || c == '\u{feff}' || c == '\u{200b}')
        .to_string()
}

/// Parses curriculum metadata from the header section of the CSV
///
/// Reads the first 10 lines looking for key-value pairs like:
/// - `Curriculum,My Program Name`
/// - `Institution,University Name`
/// - `Degree Type,BS`
/// - `System Type,semester`
/// - `CIP,11.0701`
///
/// # Errors
/// Returns an error if required fields (Curriculum, Institution) are missing
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

/// Splits a CSV line into individual cleaned fields
///
/// Simple comma-based splitting with field cleanup. Does not handle
/// quoted fields containing commas (use proper CSV parser for complex files).
fn parse_csv_line(line: &str) -> Vec<String> {
    line.split(',').map(clean_field).collect()
}

/// Parses a single course line from the CSV into a Course object
///
/// Extracts Course Name, Prefix, Number, Credit Hours, and Canonical Name
/// from the CSV fields using the provided headers for column mapping.
///
/// # Errors
/// Returns an error if required fields (Prefix, Number) are missing
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

/// Retrieves a field value from a CSV line by header name
///
/// Performs case-insensitive matching against the headers array
/// to find the column index, then returns the corresponding field value.
///
/// # Arguments
/// * `line` - The raw CSV line
/// * `header_name` - The column header to look up
/// * `headers` - Array of parsed header names
///
/// # Returns
/// `Some(value)` if the header exists and the field has a value, `None` otherwise
fn get_field(line: &str, header_name: &str, headers: &[String]) -> Option<String> {
    let fields = parse_csv_line(line);

    headers
        .iter()
        .position(|h| h.eq_ignore_ascii_case(header_name))
        .and_then(|idx| fields.get(idx))
        .cloned()
}

/// Adds prerequisites from a semicolon-separated string to a course
///
/// Converts CSV Course IDs to storage keys using the provided mapping.
/// Falls back to normalizing the string as a course key if not found in mapping.
///
/// # Arguments
/// * `course` - The course to add prerequisites to
/// * `prereq_str` - Semicolon-separated list of prerequisite IDs (e.g., "1;2;5")
/// * `course_id_to_key` - Mapping from CSV Course ID to storage key
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

/// Adds corequisites from a semicolon-separated string to a course
///
/// Converts CSV Course IDs to storage keys. Unlike prerequisites,
/// missing mappings are silently skipped since corequisites may reference
/// optional or elective courses not included in this curriculum.
///
/// # Arguments
/// * `course` - The course to add corequisites to
/// * `coreq_str` - Semicolon-separated list of corequisite IDs
/// * `course_id_to_key` - Mapping from CSV Course ID to storage key
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

/// Adds strict corequisites from a semicolon-separated string to a course
///
/// Strict corequisites must be taken in the same term as the course.
/// Like regular corequisites, missing mappings are silently skipped.
///
/// # Arguments
/// * `course` - The course to add strict corequisites to
/// * `coreq_str` - Semicolon-separated list of strict corequisite IDs
/// * `course_id_to_key` - Mapping from CSV Course ID to storage key
fn add_strict_corequisites_with_mapping(
    course: &mut Course,
    coreq_str: &str,
    course_id_to_key: &HashMap<String, String>,
) {
    for coreq in coreq_str.split(';') {
        let trimmed = coreq.trim();
        if !trimmed.is_empty() {
            if let Some(key) = course_id_to_key.get(trimmed) {
                course.add_strict_corequisite(key.clone());
            }
        }
    }
}

/// Normalizes a course key to `PREFIXNUMBER` format
///
/// Handles various input formats:
/// - `"CS 1800"` → `"CS1800"`
/// - `"CS1800"` → `"CS1800"`
/// - `"CS 1800 (or coreq)"` → `"CS1800"` (strips parenthetical notes)
/// - `"  PHYS  1151  "` → `"PHYS1151"` (handles extra whitespace)
///
/// # Arguments
/// * `input` - Raw course key string from CSV
///
/// # Returns
/// Normalized course key, or empty string if input is empty
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
    fn test_normalize_course_key_empty() {
        assert_eq!(normalize_course_key(""), "");
        assert_eq!(normalize_course_key("   "), "");
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

    #[test]
    fn test_parse_csv_line_with_quotes() {
        let line = "1,\"Course With, Comma\",CS,101,,,3.0";
        let fields = parse_csv_line(line);
        // Note: Simple comma split doesn't handle quoted commas properly
        // This documents expected behavior
        assert!(fields.len() >= 7);
    }

    #[test]
    fn test_clean_field_removes_bom() {
        let with_bom = "\u{feff}Curriculum";
        assert_eq!(clean_field(with_bom), "Curriculum");
    }

    #[test]
    fn test_clean_field_removes_quotes() {
        assert_eq!(clean_field("\"quoted\""), "quoted");
        assert_eq!(clean_field("  \"spaced\"  "), "spaced");
    }

    #[test]
    fn test_clean_field_removes_zero_width_space() {
        // Zero-width space at beginning is trimmed
        assert_eq!(clean_field("\u{200b}Test"), "Test");
    }

    #[test]
    fn test_get_field_case_insensitive() {
        let headers = vec![
            "Course ID".to_string(),
            "Course Name".to_string(),
            "Credit Hours".to_string(),
        ];
        let line = "1,Intro to CS,3.0";

        assert_eq!(
            get_field(line, "course id", &headers),
            Some("1".to_string())
        );
        assert_eq!(
            get_field(line, "COURSE NAME", &headers),
            Some("Intro to CS".to_string())
        );
        assert_eq!(
            get_field(line, "Credit hours", &headers),
            Some("3.0".to_string())
        );
    }

    #[test]
    fn test_get_field_missing_header() {
        let headers = vec!["Course ID".to_string()];
        let line = "1";

        assert_eq!(get_field(line, "Missing Header", &headers), None);
    }

    #[test]
    fn test_parse_metadata_valid() {
        let lines = vec![
            "Curriculum,Test Program",
            "Institution,Test University",
            "Degree Type,BS",
            "System Type,semester",
            "CIP,11.0701",
        ];

        let metadata = parse_metadata(&lines).unwrap();
        assert_eq!(metadata.name, "Test Program");
        assert_eq!(metadata.institution, "Test University");
        assert_eq!(metadata.degree_type, "BS");
        assert_eq!(metadata.system_type, "semester");
        assert_eq!(metadata.cip_code, "11.0701");
    }

    #[test]
    fn test_parse_metadata_handles_typo() {
        // "Insitution" is a common typo in curriculum databases
        let lines = vec!["Curriculum,Test Program", "Insitution,Test University"];

        let metadata = parse_metadata(&lines).unwrap();
        assert_eq!(metadata.institution, "Test University");
    }

    #[test]
    fn test_parse_metadata_missing_curriculum() {
        let lines = vec!["Institution,Test University"];

        let result = parse_metadata(&lines);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Curriculum"));
    }

    #[test]
    fn test_parse_metadata_missing_institution() {
        let lines = vec!["Curriculum,Test Program"];

        let result = parse_metadata(&lines);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("Institution"));
    }

    #[test]
    fn test_course_parse_context_add_course() {
        let mut ctx = CourseParseContext::new();
        let course = Course::new(
            "Intro to CS".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );

        ctx.add_course("1".to_string(), course);

        assert!(ctx.courses_by_id.contains_key("1"));
        assert_eq!(ctx.course_ids_in_order, vec!["1"]);
        assert_eq!(
            ctx.course_id_to_natural_key.get("1"),
            Some(&"CS101".to_string())
        );
    }

    #[test]
    fn test_course_parse_context_duplicate_keys() {
        let mut ctx = CourseParseContext::new();

        // Two courses with same prefix+number
        let course1 = Course::new(
            "Intro A".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );
        let course2 = Course::new(
            "Intro B".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );

        ctx.add_course("1".to_string(), course1);
        ctx.add_course("2".to_string(), course2);

        let storage_keys = ctx.compute_storage_keys().unwrap();

        // Should append IDs to make unique
        assert_eq!(storage_keys.get("1"), Some(&"CS101_1".to_string()));
        assert_eq!(storage_keys.get("2"), Some(&"CS101_2".to_string()));
    }

    #[test]
    fn test_course_parse_context_unique_keys() {
        let mut ctx = CourseParseContext::new();

        let course1 = Course::new(
            "Intro".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );
        let course2 = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "201".to_string(),
            4.0,
        );

        ctx.add_course("1".to_string(), course1);
        ctx.add_course("2".to_string(), course2);

        let storage_keys = ctx.compute_storage_keys().unwrap();

        // Should use natural keys when unique
        assert_eq!(storage_keys.get("1"), Some(&"CS101".to_string()));
        assert_eq!(storage_keys.get("2"), Some(&"CS201".to_string()));
    }
}
