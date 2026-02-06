//! Validation framework for degree programs

use super::degree::{FromClause, Requirement, RequirementType};
use super::{Course, DAG, DegreeProgram};
use std::collections::{HashMap, HashSet};

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
}

/// Types of validation warnings
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationWarning {
    /// Course is defined but never referenced in requirements
    UnreferencedCourse {
        /// Course key that's not referenced
        course_key: String,
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
}

impl ValidationResult {
    /// Create a new validation result
    #[must_use]
    pub fn new() -> Self {
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
            report.push_str(&format!("\nErrors ({}): \n", self.errors.len()));
            for (i, error) in self.errors.iter().enumerate() {
                report.push_str(&format!("  {}. {}\n", i + 1, format_error(error)));
            }
        }

        if !self.warnings.is_empty() {
            report.push_str(&format!("\nWarnings ({}): \n", self.warnings.len()));
            for (i, warning) in self.warnings.iter().enumerate() {
                report.push_str(&format!("  {}. {}\n", i + 1, format_warning(warning)));
            }
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

    // Build DAG for prerequisite validation
    let dag = build_dag_from_courses(&program.courses);

    // 1. Check for circular prerequisites
    validate_no_cycles(&dag, &mut result);

    // 2. Validate course references in prerequisites and corequisites
    validate_course_prerequisites(&program.courses, &mut result);

    // 3. Validate requirements reference valid courses
    validate_requirements(&program.requirements, &program.courses, &mut result);

    // 4. Check for unreferenced courses
    check_unreferenced_courses(&program.requirements, &program.courses, &mut result);

    result
}

/// Build a DAG from course prerequisites
fn build_dag_from_courses(courses: &HashMap<String, Course>) -> DAG {
    let mut dag = DAG::new();

    for (key, course) in courses {
        dag.add_course(key.clone());

        // Add prerequisites
        for prereq in &course.prerequisites {
            dag.add_prerequisite(key.clone(), prereq);
        }

        // Add corequisites
        for coreq in &course.corequisites {
            dag.add_corequisite(key.clone(), coreq);
        }

        // Add strict corequisites
        for coreq in &course.strict_corequisites {
            dag.add_corequisite(key.clone(), coreq);
        }
    }

    dag
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
fn validate_course_prerequisites(
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    for (key, course) in courses {
        // Check prerequisites
        for prereq in &course.prerequisites {
            if !courses.contains_key(prereq) {
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
                    for course_key in course_list {
                        // Skip validation for course choice sets like "[AA100, AA101]" or "{CS201, PHIL201}"
                        if course_key.starts_with('[') || course_key.starts_with('{') {
                            continue;
                        }

                        if !courses.contains_key(course_key) {
                            result.add_error(ValidationError::MissingCourse {
                                course_key: course_key.clone(),
                                requirement_id: req_id.clone(),
                            });
                        }
                    }
                }
            }
            RequirementType::Select => {
                // Only validate if there's a 'from' clause present
                if let Some(from) = &req.from {
                    validate_from_clause(from, req_id, courses, result);

                    // Validate count, credits, or groups_required are specified when using 'from'
                    // Note: Some requirements use groups_required or per_group instead
                    let has_selection_spec = req.count.is_some()
                        || req.credits.is_some()
                        || req.credit_range.is_some()
                        || from.groups_required.is_some()
                        || from.per_group.is_some();

                    if !has_selection_spec {
                        result.add_error(ValidationError::InvalidRequirement {
                            requirement_id: req_id.clone(),
                            reason:
                                "Select requirement with 'from' must specify count, credits, credit_range, groups_required, or per_group"
                                    .to_string(),
                        });
                    }
                }
                // If no 'from' clause, it might be an external requirement or use 'courses' directly
                // These are special cases and we skip validation for now
            }
            RequirementType::OneOf => {
                if let Some(options) = &req.options {
                    for option in options {
                        // Recursively validate nested requirements
                        let nested_reqs: HashMap<String, Requirement> = option
                            .requirements
                            .iter()
                            .enumerate()
                            .map(|(i, r)| (format!("{}:{}:{}", req_id, option.id, i), r.clone()))
                            .collect();
                        validate_requirements(&nested_reqs, courses, result);
                    }
                } else {
                    result.add_error(ValidationError::InvalidRequirement {
                        requirement_id: req_id.clone(),
                        reason: "OneOf requirement missing 'options'".to_string(),
                    });
                }
            }
        }
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
        for course_key in course_list {
            // Skip validation for course choice sets like "[AA100, AA101]" or "{CS201, PHIL201}"
            // These represent paired courses or choice sets and should be validated separately
            if course_key.starts_with('[') || course_key.starts_with('{') {
                // TODO: Parse and validate course choice sets properly
                continue;
            }

            if !courses.contains_key(course_key) {
                result.add_error(ValidationError::MissingCourse {
                    course_key: course_key.clone(),
                    requirement_id: req_id.to_string(),
                });
            }
        }
    }

    // Validate pattern
    if let Some(pattern) = &from.pattern {
        validate_pattern(pattern, req_id, courses, result);
    }

    // Validate groups
    if let Some(groups) = &from.groups {
        for group in groups {
            for course_key in &group.courses {
                if !courses.contains_key(course_key) {
                    result.add_error(ValidationError::MissingCourse {
                        course_key: course_key.clone(),
                        requirement_id: format!("{req_id}:group:{}", group.id),
                    });
                }
            }
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
                "Invalid level specification '{}'. Valid formats: '*', '300+', '100-299'",
                level_spec
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
    if spec.ends_with('+') {
        let num_part = &spec[..spec.len() - 1];
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
    if level_spec.ends_with('+') {
        if let Ok(min_level) = level_spec[..level_spec.len() - 1].parse::<u32>() {
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
    let digits: String = number.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().unwrap_or(0)
}

/// Check for courses that are defined but never referenced in requirements
fn check_unreferenced_courses(
    requirements: &HashMap<String, Requirement>,
    courses: &HashMap<String, Course>,
    result: &mut ValidationResult,
) {
    let mut referenced = HashSet::new();

    // Collect all referenced courses
    for req in requirements.values() {
        collect_referenced_courses(req, &mut referenced);
    }

    // Also include courses with patterns
    let patterns: Vec<String> = extract_patterns(requirements);
    for pattern in &patterns {
        let matches = match_pattern(pattern, courses);
        referenced.extend(matches);
    }

    // Check each course
    for key in courses.keys() {
        if !referenced.contains(key) {
            result.add_warning(ValidationWarning::UnreferencedCourse {
                course_key: key.clone(),
            });
        }
    }
}

/// Recursively collect all course references from a requirement
fn collect_referenced_courses(req: &Requirement, referenced: &mut HashSet<String>) {
    if let Some(course_list) = &req.courses {
        referenced.extend(course_list.iter().cloned());
    }

    if let Some(from) = &req.from {
        if let Some(course_list) = &from.courses {
            referenced.extend(course_list.iter().cloned());
        }

        if let Some(groups) = &from.groups {
            for group in groups {
                referenced.extend(group.courses.iter().cloned());
            }
        }
    }

    if let Some(options) = &req.options {
        for option in options {
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
                "Course '{}' referenced in requirement '{}' does not exist",
                course_key, requirement_id
            )
        }
        ValidationError::PatternMatchesNoCourses {
            pattern,
            requirement_id,
        } => {
            format!(
                "Pattern '{}' in requirement '{}' matches no courses",
                pattern, requirement_id
            )
        }
        ValidationError::InvalidPattern {
            pattern,
            reason,
            requirement_id,
        } => {
            format!(
                "Invalid pattern '{}' in requirement '{}': {}",
                pattern, requirement_id, reason
            )
        }
        ValidationError::MissingPrerequisite {
            course_key,
            prerequisite_key,
        } => {
            format!(
                "Course '{}' has prerequisite '{}' which does not exist",
                course_key, prerequisite_key
            )
        }
        ValidationError::MissingCorequisite {
            course_key,
            corequisite_key,
        } => {
            format!(
                "Course '{}' has corequisite '{}' which does not exist",
                course_key, corequisite_key
            )
        }
        ValidationError::InvalidRequirement {
            requirement_id,
            reason,
        } => {
            format!("Requirement '{}' is invalid: {}", requirement_id, reason)
        }
    }
}

/// Format a warning as a human-readable string
fn format_warning(warning: &ValidationWarning) -> String {
    match warning {
        ValidationWarning::UnreferencedCourse { course_key } => {
            format!(
                "Course '{}' is defined but never referenced in requirements",
                course_key
            )
        }
        ValidationWarning::BroadPattern {
            pattern,
            requirement_id,
            match_count,
        } => {
            format!(
                "Pattern '{}' in requirement '{}' matches {} courses (very broad)",
                pattern, requirement_id, match_count
            )
        }
        ValidationWarning::IsolatedCourse { course_key } => {
            format!(
                "Course '{}' has no prerequisites and no courses depend on it",
                course_key
            )
        }
    }
}
