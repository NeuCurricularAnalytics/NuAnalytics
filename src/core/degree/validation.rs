//! Validation framework for degree programs

use super::course_reference::CourseReference;
use crate::core::models::course::Course;
use crate::core::models::dag::DAG;
use crate::core::models::degree::{FromClause, Requirement, RequirementType};
use crate::core::models::DegreeProgram;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

// ============================================================================
// Helper Functions - Common validation operations
// ============================================================================

/// Validate a course reference string and check all referenced courses exist
///
/// Parses the course reference (handling bundles and equivalents) and validates
/// that all referenced courses exist in the course catalog.
///
/// # Arguments
/// * `course_key` - Course reference string to validate
/// * `req_id` - Requirement ID for error reporting
/// * `courses` - Course catalog to check against
/// * `result` - Validation result to add errors to
fn validate_course_reference(
    course_key: &str,
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    match CourseReference::parse(course_key) {
        Ok(course_ref) => {
            for course in course_ref.courses() {
                if !courses.contains_key(course) {
                    result.add_error(ValidationError::MissingCourse {
                        course_key: course.to_string(),
                        requirement_id: req_id.to_string(),
                    });
                }
            }
        }
        Err(err) => {
            result.add_error(ValidationError::InvalidRequirement {
                requirement_id: req_id.to_string(),
                reason: format!("Invalid course reference '{course_key}': {err}"),
            });
        }
    }
}

/// Validate a list of course references
///
/// Iterates over a list of course reference strings and validates each one.
fn validate_course_list(
    course_list: &[String],
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    for course_key in course_list {
        validate_course_reference(course_key, req_id, courses, result);
    }
}

/// Result of validating a degree program
#[derive(Debug, Clone)]
pub struct ValidationResult {
    /// Whether the degree program is valid
    pub is_valid: bool,

    /// List of errors found
    pub errors: Vec<ValidationError>,

    /// List of warnings (non-fatal issues)
    pub warnings: Vec<ValidationWarning>,
}

/// Types of validation errors
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// Circular prerequisite dependency detected
    CircularPrerequisite {
        /// The cycle path
        cycle: Vec<String>,
    },

    /// Course referenced in requirements doesn't exist
    MissingCourse {
        /// Course key that's missing
        course_key: String,
        /// Requirement ID that references it
        requirement_id: String,
    },

    /// Pattern in requirement doesn't match any courses
    PatternMatchesNoCourses {
        /// The pattern that matched nothing
        pattern: String,
        /// Requirement ID containing the pattern
        requirement_id: String,
    },

    /// Malformed pattern syntax
    InvalidPattern {
        /// The invalid pattern
        pattern: String,
        /// Why it's invalid
        reason: String,
        /// Requirement ID containing the pattern
        requirement_id: String,
    },

    /// Course prerequisite references non-existent course
    MissingPrerequisite {
        /// Course that has the prerequisite
        course_key: String,
        /// The missing prerequisite
        prerequisite_key: String,
    },

    /// Course corequisite references non-existent course
    MissingCorequisite {
        /// Course that has the corequisite
        course_key: String,
        /// The missing corequisite
        corequisite_key: String,
    },

    /// Requirement has invalid configuration
    InvalidRequirement {
        /// Requirement ID
        requirement_id: String,
        /// What's wrong with it
        reason: String,
    },

    /// Cross-listing is not bidirectional
    UnidirectionalCrossListing {
        /// Course that lists cross-listing
        course_key: String,
        /// Cross-listed course that doesn't list it back
        cross_listed_key: String,
    },
}

/// Types of validation warnings
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// Course is defined but never referenced in requirements
    UnreferencedCourse {
        /// Course key that's not referenced
        course_key: String,
    },

    /// Cross-listed course doesn't exist
    MissingCrossListedCourse {
        /// Course that has the cross-listing
        course_key: String,
        /// Missing cross-listed course
        cross_listed_key: String,
    },

    /// Pattern could be more specific
    BroadPattern {
        /// The overly broad pattern
        pattern: String,
        /// Requirement ID
        requirement_id: String,
        /// Number of courses matched
        match_count: usize,
    },

    /// Course has no prerequisites (could be intentional)
    IsolatedCourse {
        /// Course with no prerequisites or dependents
        course_key: String,
    },

    /// Course is implicitly required (prerequisite of a required course)
    HiddenRequirement {
        /// The implicit course
        course_key: String,
        /// The required course that depends on it (immediate parent)
        required_by: String,
        /// The chain of courses leading to this requirement (e.g., "A -> B -> C")
        dependency_chain: Vec<String>,
    },

    /// A required course has a prerequisite choice where none of the options are listed in the degree
    HiddenRequirementOption {
        /// The list of options (e.g., `["MATH120", "MATH124"]`)
        options: Vec<String>,
        /// The required course that triggers this choice
        required_by: String,
    },
}

impl ValidationResult {
    /// Create a new validation result
    #[must_use]
    pub const fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
        }
    }

    /// Add an error to the result
    pub fn add_error(&mut self, error: ValidationError) {
        self.is_valid = false;
        self.errors.push(error);
    }

    /// Add a warning to the result
    pub fn add_warning(&mut self, warning: ValidationWarning) {
        self.warnings.push(warning);
    }

    /// Format errors and warnings as a human-readable string
    #[must_use]
    pub fn format_report(&self) -> String {
        let mut report = String::new();

        if self.is_valid {
            report.push_str("✓ Degree program is valid\n");
        } else {
            report.push_str("✗ Degree program has errors\n");
        }

        if !self.errors.is_empty() {
            let _ = write!(report, "\nErrors ({}): \n", self.errors.len());
            for (i, error) in self.errors.iter().enumerate() {
                let _ = writeln!(report, "  {}. {}", i + 1, format_error(error));
            }
        }

        if !self.warnings.is_empty() {
            let _ = write!(report, "\nWarnings ({}): \n", self.warnings.len());

            // Group warnings by type
            let mut unreferenced = Vec::new();
            let mut missing_cross_listed = Vec::new();
            let mut broad_pattern = Vec::new();
            let mut isolated = Vec::new();
            let mut hidden_req = Vec::new();
            let mut hidden_opts = Vec::new();

            for warning in &self.warnings {
                match warning {
                    ValidationWarning::UnreferencedCourse { .. } => unreferenced.push(warning),
                    ValidationWarning::MissingCrossListedCourse { .. } => {
                        missing_cross_listed.push(warning);
                    }
                    ValidationWarning::BroadPattern { .. } => broad_pattern.push(warning),
                    ValidationWarning::IsolatedCourse { .. } => isolated.push(warning),
                    ValidationWarning::HiddenRequirement { .. } => hidden_req.push(warning),
                    ValidationWarning::HiddenRequirementOption { .. } => hidden_opts.push(warning),
                }
            }

            // Helper to print a group
            let mut print_group = |title: &str, warnings: &[&ValidationWarning]| {
                if !warnings.is_empty() {
                    let _ = writeln!(report, "\n  {title}:");
                    for warning in warnings {
                        let _ = writeln!(report, "    - {}", format_warning(warning));
                    }
                }
            };

            print_group("Unreferenced Courses", &unreferenced);
            print_group("Hidden Requirements (Implicitly Required)", &hidden_req);
            print_group("Hidden Requirement Options (Implicit Choice)", &hidden_opts);
            print_group("Missing Cross-Listed Courses", &missing_cross_listed);
            print_group("Broad Patterns", &broad_pattern);
            print_group("Isolated Courses", &isolated);
        }

        report
    }
}

impl Default for ValidationResult {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate a complete degree program
///
/// Performs comprehensive validation including:
/// - Circular prerequisite detection
/// - Course reference validation
/// - Pattern matching validation
/// - Requirement configuration validation
///
/// # Returns
/// A `ValidationResult` containing all errors and warnings found
#[must_use]
pub fn validate_degree_program(program: &DegreeProgram) -> ValidationResult {
    let mut result = ValidationResult::new();

    // Build DAG for prerequisite validation (Strict only for cycle detection)
    let strict_dag = build_strict_dag_from_courses(&program.courses);

    // 1. Check for circular prerequisites
    validate_no_cycles(&strict_dag, &mut result);

    // 2. Validate course references in prerequisites and corequisites
    validate_course_prerequisites(&program.courses, &mut result);

    // 3. Validate requirements reference valid courses
    validate_requirements(&program.requirements, &program.courses, &mut result);

    // 4. Validate cross-listing relationships
    validate_cross_listing(&program.courses, &mut result);

    // 5. Check for unreferenced courses
    check_unreferenced_courses(&program.requirements, &program.courses, &mut result);

    result
}

/// Build a DAG from course prerequisites (Strict dependencies only)
fn build_strict_dag_from_courses(courses: &HashMap<String, Course>) -> DAG {
    let mut dag = DAG::new();

    for (key, course) in courses {
        dag.add_course(key.clone());

        let strict_prereqs = course.prerequisites_raw.as_ref().map_or_else(
            || course.prerequisites.clone(),
            |raw| extract_strict_prerequisites(raw),
        );

        for prereq in strict_prereqs {
            // Only add if prereq exists (to avoid polluting DAG with invalid nodes,
            // though missing prereqs are caught by other checks)
            if courses.contains_key(&prereq) {
                dag.add_prerequisite(key.clone(), &prereq);
            }
        }

        // Corequisites are usually strict dependencies
        for coreq in &course.corequisites {
            if courses.contains_key(coreq) {
                dag.add_corequisite(key.clone(), coreq);
            }
        }

        for coreq in &course.strict_corequisites {
            if courses.contains_key(coreq) {
                dag.add_corequisite(key.clone(), coreq);
            }
        }
    }

    dag
}

/// Extract strictly required courses from a prerequisite string
/// (A | B) & C -> C is strict, A and B are not
fn extract_strict_prerequisites(raw: &str) -> Vec<String> {
    let mut strict = Vec::new();

    // Clean string: remove grades [X], trim
    let cleaned = remove_grade_requirements(raw);

    // Split top-level ANDs
    let parts = split_by_delimiter_at_level(&cleaned, '&');

    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // If part contains top-level OR, it's optional -> skip
        // Check for | at level 0
        if contains_delimiter_at_level(trimmed, '|') {
            continue;
        }

        // If wrapped in parens (A), unwrap and recurse
        if trimmed.starts_with('(') && trimmed.ends_with(')') {
            // Verify parens are matching for the whole string
            if is_wrapped_in_parens(trimmed) {
                let inner = &trimmed[1..trimmed.len() - 1];
                strict.extend(extract_strict_prerequisites(inner));
                continue;
            }
        }

        // It's a single course (or bundle/equivalent which we parse as strict for now)
        // clean up any remaining chars
        let course_key = trimmed.replace(['(', ')'], "").trim().to_string();
        if !course_key.is_empty() {
            strict.push(course_key);
        }
    }

    strict
}

fn remove_grade_requirements(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut in_grade = false;
    for c in s.chars() {
        if c == '[' {
            in_grade = true;
        } else if c == ']' {
            in_grade = false;
        } else if !in_grade {
            result.push(c);
        }
    }
    result
}

fn split_by_delimiter_at_level(s: &str, delimiter: char) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut level = 0;

    for c in s.chars() {
        if c == '(' {
            level += 1;
            current.push(c);
        } else if c == ')' {
            if level > 0 {
                level -= 1;
            }
            current.push(c);
        } else if c == delimiter && level == 0 {
            parts.push(current.clone());
            current.clear();
        } else {
            current.push(c);
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

fn contains_delimiter_at_level(s: &str, delimiter: char) -> bool {
    let mut level = 0;
    for c in s.chars() {
        if c == '(' {
            level += 1;
        } else if c == ')' {
            if level > 0 {
                level -= 1;
            }
        } else if c == delimiter && level == 0 {
            return true;
        }
    }
    false
}

/// Extract top-level OR options from a prerequisite string
/// `(A | B) -> ["A", "B"]`
/// `(A & B) | C -> ["A & B", "C"]` - simplified, we just want to catch single course ORs for now
fn extract_top_level_options(raw: &str) -> Vec<String> {
    let mut options = Vec::new();
    let cleaned = remove_grade_requirements(raw);

    // Split by top-level OR
    let parts = split_by_delimiter_at_level(&cleaned, '|');

    for part in parts {
        let trimmed = part.trim();
        if trimmed.is_empty() {
            continue;
        }

        // Remove surrounding parens if present
        let clean_part = if is_wrapped_in_parens(trimmed) {
            &trimmed[1..trimmed.len() - 1]
        } else {
            trimmed
        };

        // Only return single courses as options for now to keep it simple
        // If an option is a complex expression (A & B), we skip it for this check
        if !clean_part.contains('&') && !clean_part.contains('|') {
            options.push(clean_part.trim().to_string());
        }
    }

    options
}

fn is_wrapped_in_parens(s: &str) -> bool {
    if !s.starts_with('(') || !s.ends_with(')') {
        return false;
    }

    let mut level = 0;
    for (i, c) in s.chars().enumerate() {
        if c == '(' {
            level += 1;
        } else if c == ')' {
            level -= 1;
            if level == 0 && i < s.len() - 1 {
                return false; // Closed before end
            }
        }
    }
    level == 0
}

/// Detect cycles in prerequisite graph
fn validate_no_cycles(dag: &DAG, result: &mut ValidationResult) {
    let mut visited = HashSet::new();
    let mut rec_stack = Vec::new();

    for course in &dag.courses {
        if !visited.contains(course) {
            if let Some(cycle) = detect_cycle(course, dag, &mut visited, &mut rec_stack) {
                result.add_error(ValidationError::CircularPrerequisite { cycle });
            }
        }
    }
}

/// DFS-based cycle detection
fn detect_cycle(
    course: &str,
    dag: &DAG,
    visited: &mut HashSet<String>,
    rec_stack: &mut Vec<String>,
) -> Option<Vec<String>> {
    visited.insert(course.to_string());
    rec_stack.push(course.to_string());

    if let Some(prereqs) = dag.get_prerequisites(course) {
        for prereq in prereqs {
            if !visited.contains(prereq) {
                if let Some(cycle) = detect_cycle(prereq, dag, visited, rec_stack) {
                    return Some(cycle);
                }
            } else if rec_stack.contains(prereq) {
                // Found a cycle - extract it
                let cycle_start = rec_stack.iter().position(|c| c == prereq).unwrap();
                let mut cycle: Vec<String> = rec_stack[cycle_start..].to_vec();
                cycle.push(prereq.clone()); // Close the cycle
                return Some(cycle);
            }
        }
    }

    rec_stack.pop();
    None
}

/// Validate that all prerequisites and corequisites reference existing courses
fn validate_course_prerequisites(courses: &HashMap<String, Course>, result: &mut ValidationResult) {
    for (key, course) in courses {
        // Check prerequisites (Strict only)
        // We only require that STRICT prerequisites exist.
        // Optional prerequisites (in OR clauses) are allowed to be external/missing
        // to support "alternate but equivalent" scenarios without errors.
        let strict_prereqs = course.prerequisites_raw.as_ref().map_or_else(
            || course.prerequisites.clone(),
            |raw| extract_strict_prerequisites(raw),
        );

        for prereq in strict_prereqs {
            if !courses.contains_key(&prereq) {
                result.add_error(ValidationError::MissingPrerequisite {
                    course_key: key.clone(),
                    prerequisite_key: prereq.clone(),
                });
            }
        }

        // Check corequisites
        for coreq in &course.corequisites {
            if !courses.contains_key(coreq) {
                result.add_error(ValidationError::MissingCorequisite {
                    course_key: key.clone(),
                    corequisite_key: coreq.clone(),
                });
            }
        }

        // Check strict corequisites
        for coreq in &course.strict_corequisites {
            if !courses.contains_key(coreq) {
                result.add_error(ValidationError::MissingCorequisite {
                    course_key: key.clone(),
                    corequisite_key: coreq.clone(),
                });
            }
        }
    }
}

/// Validate requirements reference valid courses and patterns
fn validate_requirements(
    requirements: &HashMap<String, Requirement>,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    for (req_id, req) in requirements {
        match req.req_type {
            RequirementType::All => {
                if let Some(course_list) = &req.courses {
                    validate_course_list(course_list, req_id, courses, result);
                }
            }
            RequirementType::Select => {
                validate_select_requirement(req, req_id, courses, result);
            }
            RequirementType::OneOf => {
                validate_oneof_requirement(req, req_id, courses, result);
            }
        }
    }
}

/// Validate a Select requirement's from clause and selection specification
fn validate_select_requirement(
    req: &Requirement,
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    // Only validate if there's a 'from' clause present
    if let Some(from) = &req.from {
        validate_from_clause(from, req_id, courses, result);

        // Validate count, credits, or groups_required are specified when using 'from'
        let has_selection_spec = req.count.is_some()
            || req.credits.is_some()
            || req.credit_range.is_some()
            || from.groups_required.is_some()
            || from.per_group.is_some();

        if !has_selection_spec {
            result.add_error(ValidationError::InvalidRequirement {
                requirement_id: req_id.to_string(),
                reason:
                    "Select requirement with 'from' must specify count, credits, credit_range, groups_required, or per_group"
                        .to_string(),
            });
        }
    }
    // If no 'from' clause, it might be an external requirement or use 'courses' directly
}

/// Validate a `OneOf` requirement's options and nested requirements
fn validate_oneof_requirement(
    req: &Requirement,
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    if let Some(options) = &req.options {
        for option in options {
            // Recursively validate nested requirements
            let nested_reqs: HashMap<String, Requirement> = option
                .requirements
                .iter()
                .enumerate()
                .map(|(i, r)| (format!("{req_id}:{}:{i}", option.id), r.clone()))
                .collect();
            validate_requirements(&nested_reqs, courses, result);
        }
    } else {
        result.add_error(ValidationError::InvalidRequirement {
            requirement_id: req_id.to_string(),
            reason: "OneOf requirement missing 'options'".to_string(),
        });
    }
}

/// Validate a from clause (courses, patterns, groups)
fn validate_from_clause(
    from: &FromClause,
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    // Validate explicit course list
    if let Some(course_list) = &from.courses {
        validate_course_list(course_list, req_id, courses, result);
    }

    // Validate pattern
    if let Some(pattern) = &from.pattern {
        validate_pattern(pattern, req_id, courses, result);
    }

    // Validate included patterns
    if let Some(patterns) = &from.include {
        for pattern in patterns {
            validate_pattern(pattern, req_id, courses, result);
        }
    }

    // Validate groups
    if let Some(groups) = &from.groups {
        for group in groups {
            let group_req_id = format!("{req_id}:group:{}", group.id);
            validate_course_list(&group.courses, &group_req_id, courses, result);
        }
    }
}

/// Validate a course pattern and check if it matches any courses
fn validate_pattern(
    pattern: &str,
    req_id: &str,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    // Parse pattern: "PREFIX:LEVEL" where LEVEL can be "*", "300+", "100-299", etc.
    let parts: Vec<&str> = pattern.split(':').collect();

    if parts.len() != 2 {
        result.add_error(ValidationError::InvalidPattern {
            pattern: pattern.to_string(),
            reason: "Pattern must be in format 'PREFIX:LEVEL' (e.g., 'ICS:400+', 'CS:*')"
                .to_string(),
            requirement_id: req_id.to_string(),
        });
        return;
    }

    let prefix = parts[0];
    let level_spec = parts[1];

    // Allow *:* as a wildcard pattern (matches all courses)
    if prefix == "*" && level_spec == "*" {
        // Valid wildcard - skip validation since it intentionally matches everything
        return;
    }

    // Validate level specification
    if level_spec != "*" && !validate_level_spec(level_spec) {
        result.add_error(ValidationError::InvalidPattern {
            pattern: pattern.to_string(),
            reason: format!(
                "Invalid level specification '{level_spec}'. Valid formats: '*', '300+', '100-299'"
            ),
            requirement_id: req_id.to_string(),
        });
        return;
    }

    // Check if pattern matches any courses
    let matches = match_pattern(pattern, courses);

    if matches.is_empty() {
        result.add_error(ValidationError::PatternMatchesNoCourses {
            pattern: pattern.to_string(),
            requirement_id: req_id.to_string(),
        });
    } else if matches.len() > 50 {
        // Warn if pattern is too broad
        result.add_warning(ValidationWarning::BroadPattern {
            pattern: pattern.to_string(),
            requirement_id: req_id.to_string(),
            match_count: matches.len(),
        });
    }
}

/// Validate level specification format
fn validate_level_spec(spec: &str) -> bool {
    // Check for wildcard
    if spec == "*" {
        return true;
    }

    // Check for "300+" format
    if let Some(num_part) = spec.strip_suffix('+') {
        return num_part.parse::<u32>().is_ok();
    }

    // Check for "100-299" format
    if spec.contains('-') {
        let range: Vec<&str> = spec.split('-').collect();
        if range.len() == 2 {
            return range[0].parse::<u32>().is_ok() && range[1].parse::<u32>().is_ok();
        }
    }

    // Check for exact number
    spec.parse::<u32>().is_ok()
}

/// Match courses against a pattern
fn match_pattern(pattern: &str, courses: &HashMap<String, Course>) -> Vec<String> {
    let parts: Vec<&str> = pattern.split(':').collect();
    if parts.len() != 2 {
        return Vec::new();
    }

    let prefix = parts[0];
    let level_spec = parts[1];

    courses
        .iter()
        .filter(|(_, course)| {
            // Check prefix match
            if course.prefix != prefix {
                return false;
            }

            // Check level match
            match_level(&course.number, level_spec)
        })
        .map(|(key, _)| key.clone())
        .collect()
}

/// Check if a course number matches a level specification
fn match_level(number: &str, level_spec: &str) -> bool {
    // Wildcard matches all
    if level_spec == "*" {
        return true;
    }

    // Parse course number (extract leading digits)
    let course_num = extract_course_number(number);

    // "300+" format
    if let Some(stripped) = level_spec.strip_suffix('+') {
        if let Ok(min_level) = stripped.parse::<u32>() {
            return course_num >= min_level;
        }
    }

    // "100-299" format
    if level_spec.contains('-') {
        let range: Vec<&str> = level_spec.split('-').collect();
        if range.len() == 2 {
            if let (Ok(min), Ok(max)) = (range[0].parse::<u32>(), range[1].parse::<u32>()) {
                return course_num >= min && course_num <= max;
            }
        }
    }

    // Exact number match
    if let Ok(exact) = level_spec.parse::<u32>() {
        return course_num == exact;
    }

    false
}

/// Extract numeric course level from course number
fn extract_course_number(number: &str) -> u32 {
    // Extract leading digits
    let digits: String = number.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().unwrap_or(0)
}

/// Validate cross-listing relationships between courses
///
/// Checks that:
/// 1. Cross-listed courses exist
/// 2. Cross-listing is bidirectional (if A lists B, B should list A)
fn validate_cross_listing(courses: &HashMap<String, Course>, result: &mut ValidationResult) {
    for (course_key, course) in courses {
        if let Some(cross_listed) = &course.cross_listed_as {
            for cross_listed_key in cross_listed {
                // Check if cross-listed course exists
                if !courses.contains_key(cross_listed_key) {
                    result.add_warning(ValidationWarning::MissingCrossListedCourse {
                        course_key: course_key.clone(),
                        cross_listed_key: cross_listed_key.clone(),
                    });
                    continue;
                }

                // Check if cross-listing is bidirectional
                if let Some(other_course) = courses.get(cross_listed_key) {
                    let is_bidirectional = other_course
                        .cross_listed_as
                        .as_ref()
                        .is_some_and(|list| list.contains(course_key));

                    if !is_bidirectional {
                        result.add_error(ValidationError::UnidirectionalCrossListing {
                            course_key: course_key.clone(),
                            cross_listed_key: cross_listed_key.clone(),
                        });
                    }
                }
            }
        }
    }
}

/// Check for courses that are defined but never referenced in requirements
fn check_unreferenced_courses(
    requirements: &HashMap<String, Requirement>,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    let mut explicitly_referenced = HashSet::new();

    // Collect all referenced courses from requirements
    for req in requirements.values() {
        collect_referenced_courses(req, &mut explicitly_referenced);
    }

    // Also include courses with patterns
    let patterns: Vec<String> = extract_patterns(requirements);
    for pattern in &patterns {
        let matches = match_pattern(pattern, courses);
        explicitly_referenced.extend(matches);
    }

    // Now identify implicitly referenced courses (prerequisites of referenced courses)
    // We use two BFS passes:
    // 1. All reachable courses (Weak + Strict) -> To mark as "Referenced" (avoid Unreferenced warning)
    let weakly_reachable = compute_weakly_reachable(&explicitly_referenced, courses);

    // 2. Strictly reachable courses (Strict only) -> To mark as "Hidden Requirement" (Warn if not explicit)
    let (implicit_paths, mut implicit_options) =
        compute_strictly_reachable(&explicitly_referenced, courses, &weakly_reachable);

    // Add warnings (sorted by key for deterministic output)
    let mut sorted_keys: Vec<_> = courses.keys().collect();
    sorted_keys.sort();

    // Add implicit option warnings
    // Sort implicit options for deterministic output
    implicit_options.sort_by(|a, b| a.0.cmp(&b.0));

    for (required_by, options) in implicit_options {
        result.add_warning(ValidationWarning::HiddenRequirementOption {
            required_by,
            options,
        });
    }

    for key in sorted_keys {
        if !weakly_reachable.contains(key) {
            result.add_warning(ValidationWarning::UnreferencedCourse {
                course_key: key.clone(),
            });
        } else if !explicitly_referenced.contains(key) {
            // It IS reachable, but not explicitly referenced.
            // Only warn if it is STRICTLY reachable (Hidden Requirement)
            if let Some(path) = implicit_paths.get(key) {
                // Format path: Root -> Parent -> Key
                // We want to show: Key is required by ...
                // Let's store the full path for the warning
                let parent = if path.len() >= 2 {
                    path[path.len() - 2].clone()
                } else {
                    String::from("Unknown")
                };

                result.add_warning(ValidationWarning::HiddenRequirement {
                    course_key: key.clone(),
                    required_by: parent,
                    dependency_chain: path.clone(),
                });
            }
            // If weakly reachable but not strict, it's an optional alternative.
            // We consider it "Referenced" enough to suppress Unreferenced warning,
            // but "Optional" enough to suppress Hidden Requirement warning.
        }
    }
}

/// Compute all courses reachable from the explicit set (Weak + Strict prerequisites)
fn compute_weakly_reachable(
    explicitly_referenced: &HashSet<String>,
    courses: &HashMap<String, Course>,
) -> HashSet<String> {
    let mut weakly_reachable = explicitly_referenced.clone();
    let mut work_queue: Vec<String> = explicitly_referenced.iter().cloned().collect();

    while let Some(current_key) = work_queue.pop() {
        if let Some(course) = courses.get(&current_key) {
            // Check all prerequisites (including optional)
            for prereq in &course.prerequisites {
                if !weakly_reachable.contains(prereq) {
                    weakly_reachable.insert(prereq.clone());
                    work_queue.push(prereq.clone());
                }
            }
            // Check corequisites
            for coreq in &course.corequisites {
                if !weakly_reachable.contains(coreq) {
                    weakly_reachable.insert(coreq.clone());
                    work_queue.push(coreq.clone());
                }
            }
            // Check strict corequisites
            for coreq in &course.strict_corequisites {
                if !weakly_reachable.contains(coreq) {
                    weakly_reachable.insert(coreq.clone());
                    work_queue.push(coreq.clone());
                }
            }
        }
    }
    weakly_reachable
}

/// Compute strictly reachable courses and identify implicit options
#[allow(clippy::type_complexity)]
fn compute_strictly_reachable(
    explicitly_referenced: &HashSet<String>,
    courses: &HashMap<String, Course>,
    weakly_reachable: &HashSet<String>,
) -> (HashMap<String, Vec<String>>, Vec<(String, Vec<String>)>) {
    let mut strictly_reachable = explicitly_referenced.clone();

    // Queue stores (course_key, path_to_course)
    // For explicit courses, path is just [course_key]
    let mut work_queue: Vec<(String, Vec<String>)> = explicitly_referenced
        .iter()
        .map(|k| (k.clone(), vec![k.clone()]))
        .collect();

    let mut implicit_paths: HashMap<String, Vec<String>> = HashMap::new();
    let mut implicit_options: Vec<(String, Vec<String>)> = Vec::new();

    while let Some((current_key, current_path)) = work_queue.pop() {
        if let Some(course) = courses.get(&current_key) {
            // Check strict prerequisites only
            let strict_prereqs = course.prerequisites_raw.as_ref().map_or_else(
                || course.prerequisites.clone(),
                |raw| extract_strict_prerequisites(raw),
            );

            for prereq in strict_prereqs {
                if courses.contains_key(&prereq) {
                    // Ensure exists
                    if !strictly_reachable.contains(&prereq) {
                        strictly_reachable.insert(prereq.clone());

                        let mut new_path = current_path.clone();
                        new_path.push(prereq.clone());

                        work_queue.push((prereq.clone(), new_path.clone()));
                        implicit_paths.insert(prereq.clone(), new_path);
                    }
                }
            }

            // Detect implicit choices (OR groups)
            // If a required course has prerequisites like (A | B), and neither A nor B is referenced,
            // then we have a "Hidden Option".
            if let Some(raw) = &course.prerequisites_raw {
                let options = extract_top_level_options(raw);
                if options.len() > 1 {
                    // Check if ANY of the options are already reachable/referenced
                    let any_referenced = options.iter().any(|opt| weakly_reachable.contains(opt));

                    if !any_referenced {
                        // None of the options are referenced. This is a hidden choice.
                        // Filter to only include options that actually exist in the course catalog
                        let valid_options: Vec<String> = options
                            .into_iter()
                            .filter(|opt| courses.contains_key(opt))
                            .collect();

                        if !valid_options.is_empty() {
                            implicit_options.push((current_key.clone(), valid_options));
                        }
                    }
                }
            }

            // Corequisites are usually strict
            for coreq in &course.corequisites {
                if !strictly_reachable.contains(coreq) {
                    strictly_reachable.insert(coreq.clone());

                    let mut new_path = current_path.clone();
                    new_path.push(coreq.clone());

                    work_queue.push((coreq.clone(), new_path.clone()));
                    implicit_paths.insert(coreq.clone(), new_path);
                }
            }
            for coreq in &course.strict_corequisites {
                if !strictly_reachable.contains(coreq) {
                    strictly_reachable.insert(coreq.clone());

                    let mut new_path = current_path.clone();
                    new_path.push(coreq.clone());

                    work_queue.push((coreq.clone(), new_path.clone()));
                    implicit_paths.insert(coreq.clone(), new_path);
                }
            }
        }
    }

    (implicit_paths, implicit_options)
}

/// Recursively collect all course references from a requirement
fn collect_referenced_courses(req: &Requirement, referenced: &mut HashSet<String>) {
    // Collect from direct course list
    collect_courses_from_list(req.courses.as_ref(), referenced);

    // Collect from 'from' section
    if let Some(from) = &req.from {
        collect_courses_from_list(from.courses.as_ref(), referenced);
        collect_courses_from_groups(from.groups.as_ref(), referenced);
    }

    // Recursively collect from options
    collect_courses_from_options(req.options.as_ref(), referenced);
}

/// Collect courses from a course list
fn collect_courses_from_list(course_list: Option<&Vec<String>>, referenced: &mut HashSet<String>) {
    if let Some(courses) = course_list {
        for course_key in courses {
            if let Ok(course_ref) = CourseReference::parse(course_key) {
                for course in course_ref.courses() {
                    referenced.insert(course.to_string());
                }
            }
        }
    }
}

/// Collect courses from requirement groups
fn collect_courses_from_groups(
    groups: Option<&Vec<crate::core::models::degree::CourseGroup>>,
    referenced: &mut HashSet<String>,
) {
    if let Some(group_list) = groups {
        for group in group_list {
            collect_courses_from_list(Some(&group.courses), referenced);
        }
    }
}

/// Collect courses from requirement options (recursive)
fn collect_courses_from_options(
    options: Option<&Vec<crate::core::models::degree::RequirementOption>>,
    referenced: &mut HashSet<String>,
) {
    if let Some(option_list) = options {
        for option in option_list {
            for nested_req in &option.requirements {
                collect_referenced_courses(nested_req, referenced);
            }
        }
    }
}

/// Extract all patterns from requirements
fn extract_patterns(requirements: &HashMap<String, Requirement>) -> Vec<String> {
    let mut patterns = Vec::new();

    for req in requirements.values() {
        extract_patterns_from_requirement(req, &mut patterns);
    }

    patterns
}

/// Recursively extract patterns from a requirement
fn extract_patterns_from_requirement(req: &Requirement, patterns: &mut Vec<String>) {
    if let Some(from) = &req.from {
        if let Some(pattern) = &from.pattern {
            patterns.push(pattern.clone());
        }
        if let Some(include) = &from.include {
            patterns.extend(include.clone());
        }
    }

    if let Some(options) = &req.options {
        for option in options {
            for nested_req in &option.requirements {
                extract_patterns_from_requirement(nested_req, patterns);
            }
        }
    }
}

/// Format an error as a human-readable string
fn format_error(error: &ValidationError) -> String {
    match error {
        ValidationError::CircularPrerequisite { cycle } => {
            format!("Circular prerequisite: {}", cycle.join(" → "))
        }
        ValidationError::MissingCourse {
            course_key,
            requirement_id,
        } => {
            format!(
                "Course '{course_key}' referenced in requirement '{requirement_id}' does not exist"
            )
        }
        ValidationError::PatternMatchesNoCourses {
            pattern,
            requirement_id,
        } => {
            format!("Pattern '{pattern}' in requirement '{requirement_id}' matches no courses")
        }
        ValidationError::InvalidPattern {
            pattern,
            reason,
            requirement_id,
        } => {
            format!("Invalid pattern '{pattern}' in requirement '{requirement_id}': {reason}")
        }
        ValidationError::MissingPrerequisite {
            course_key,
            prerequisite_key,
        } => {
            format!(
                "Course '{course_key}' has prerequisite '{prerequisite_key}' which does not exist"
            )
        }
        ValidationError::MissingCorequisite {
            course_key,
            corequisite_key,
        } => {
            format!(
                "Course '{course_key}' has corequisite '{corequisite_key}' which does not exist"
            )
        }
        ValidationError::InvalidRequirement {
            requirement_id,
            reason,
        } => {
            format!("Requirement '{requirement_id}' is invalid: {reason}")
        }
        ValidationError::UnidirectionalCrossListing {
            course_key,
            cross_listed_key,
        } => {
            format!(
                "Course '{course_key}' lists '{cross_listed_key}' as cross-listed, but '{cross_listed_key}' does not list '{course_key}' back"
            )
        }
    }
}

/// Format a warning as a human-readable string
fn format_warning(warning: &ValidationWarning) -> String {
    match warning {
        ValidationWarning::UnreferencedCourse { course_key } => {
            format!("Course '{course_key}' is defined but never referenced in requirements")
        }
        ValidationWarning::BroadPattern {
            pattern,
            requirement_id,
            match_count,
        } => {
            format!(
                "Pattern '{pattern}' in requirement '{requirement_id}' matches {match_count} courses (very broad)"
            )
        }
        ValidationWarning::IsolatedCourse { course_key } => {
            format!("Course '{course_key}' has no prerequisites and no courses depend on it")
        }
        ValidationWarning::MissingCrossListedCourse {
            course_key,
            cross_listed_key,
        } => {
            format!(
                "Course '{course_key}' lists '{cross_listed_key}' as cross-listed, but '{cross_listed_key}' does not exist"
            )
        }
        ValidationWarning::HiddenRequirement {
            course_key,
            required_by: _,
            dependency_chain,
        } => {
            if dependency_chain.is_empty() {
                format!(
                    "Course '{course_key}' is implicitly required but not listed in requirements"
                )
            } else {
                format!(
                    "Course '{course_key}' is implicitly required: {}",
                    dependency_chain.join(" -> ")
                )
            }
        }
        ValidationWarning::HiddenRequirementOption {
            options,
            required_by,
        } => {
            format!(
                "One of [{}] is implicitly required by '{}' (prerequisite choice)",
                options.join(", "),
                required_by
            )
        }
    }
}
