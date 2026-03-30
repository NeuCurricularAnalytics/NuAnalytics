//! Plan validation for generated degree plans
//!
//! Validates that generated plans are complete and correct:
//! - All prerequisites are included
//! - Target credits are met
//! - Required course categories have sufficient courses
//! - No "Unknown" or improperly named courses

use crate::core::models::course::Course;
use crate::core::models::CourseGraph;
use std::collections::{HashMap, HashSet};
use std::fmt::Write;

use super::plan_variant::PlanVariant;

// ============================================================================
// Validation Result Types
// ============================================================================

/// Result of validating a plan
#[derive(Debug, Clone, Default)]
pub struct PlanValidationResult {
    /// Whether the plan is valid
    pub is_valid: bool,

    /// List of validation errors
    pub errors: Vec<PlanValidationError>,

    /// List of validation warnings
    pub warnings: Vec<PlanValidationWarning>,

    /// Validation statistics
    pub stats: PlanValidationStats,
}

/// Statistics collected during plan validation
#[derive(Debug, Clone, Default)]
pub struct PlanValidationStats {
    /// Total courses in plan
    pub total_courses: usize,

    /// Total credits in plan
    pub total_credits: f32,

    /// Courses with prerequisites
    pub courses_with_prereqs: usize,

    /// Missing prerequisites found
    pub missing_prereqs: usize,

    /// Placeholder/elective courses
    pub placeholder_courses: usize,

    /// Courses by category
    pub courses_by_category: HashMap<String, usize>,

    /// Credits by category
    pub credits_by_category: HashMap<String, f32>,
}

/// Types of plan validation errors
#[derive(Debug, Clone, PartialEq)]
pub enum PlanValidationError {
    /// Required prerequisite is missing from the plan
    MissingPrerequisite {
        /// Course that requires the prerequisite
        course: String,
        /// The missing prerequisite course
        prerequisite: String,
    },

    /// Plan does not meet minimum credit requirement
    InsufficientCredits {
        /// Credits in the plan
        actual: f32,
        /// Required minimum credits
        required: f32,
    },

    /// Required category has insufficient courses
    InsufficientCategoryCourses {
        /// Category name
        category: String,
        /// Actual course count
        actual: usize,
        /// Required minimum count
        required: usize,
    },

    /// Course referenced but not defined in catalog
    UndefinedCourse {
        /// The undefined course key
        course: String,
    },
}

/// Types of plan validation warnings
#[derive(Debug, Clone, PartialEq)]
pub enum PlanValidationWarning {
    /// Course has "Unknown" or generic name (placeholder)
    UnnamedCourse {
        /// The course key
        course: String,
    },

    /// Course appears to be a placeholder (generated elective)
    PlaceholderCourse {
        /// The course key
        course: String,
    },

    /// Plan exceeds expected credit count significantly
    ExcessCredits {
        /// Credits in the plan
        actual: f32,
        /// Expected maximum credits
        expected: f32,
    },

    /// Optional prerequisite not included (one of multiple options)
    OptionalPrerequisiteNotIncluded {
        /// Course that has the optional prerequisite
        course: String,
        /// The options available (only one needed)
        options: Vec<String>,
    },
}

impl PlanValidationResult {
    /// Create a new validation result
    #[must_use]
    pub fn new() -> Self {
        Self {
            is_valid: true,
            errors: Vec::new(),
            warnings: Vec::new(),
            stats: PlanValidationStats::default(),
        }
    }

    /// Add an error to the result
    pub fn add_error(&mut self, error: PlanValidationError) {
        self.is_valid = false;
        self.errors.push(error);
    }

    /// Add a warning to the result
    pub fn add_warning(&mut self, warning: PlanValidationWarning) {
        self.warnings.push(warning);
    }

    /// Format the validation result as a human-readable string
    #[must_use]
    pub fn format_report(&self) -> String {
        let mut report = String::new();

        if self.is_valid {
            let _ = writeln!(report, "✓ Plan is valid");
        } else {
            let _ = writeln!(report, "✗ Plan has validation errors");
        }

        // Statistics
        let _ = writeln!(report, "\nPlan Statistics:");
        let _ = writeln!(report, "  Total courses: {}", self.stats.total_courses);
        let _ = writeln!(report, "  Total credits: {:.1}", self.stats.total_credits);
        let _ = writeln!(
            report,
            "  Courses with prerequisites: {}",
            self.stats.courses_with_prereqs
        );
        let _ = writeln!(
            report,
            "  Placeholder courses: {}",
            self.stats.placeholder_courses
        );

        if !self.stats.courses_by_category.is_empty() {
            let _ = writeln!(report, "  Courses by category:");
            let mut categories: Vec<_> = self.stats.courses_by_category.iter().collect();
            categories.sort_by_key(|(k, _)| *k);
            for (cat, count) in categories {
                let credits = self.stats.credits_by_category.get(cat).unwrap_or(&0.0);
                let _ = writeln!(report, "    {cat}: {count} courses ({credits:.1} credits)");
            }
        }

        // Errors
        if !self.errors.is_empty() {
            let _ = writeln!(report, "\nErrors ({}):", self.errors.len());
            for (i, error) in self.errors.iter().enumerate() {
                let _ = writeln!(report, "  {}. {}", i + 1, format_error(error));
            }
        }

        // Warnings
        if !self.warnings.is_empty() {
            let _ = writeln!(report, "\nWarnings ({}):", self.warnings.len());

            // Group warnings by type
            let mut unnamed = Vec::new();
            let mut placeholder = Vec::new();
            let mut excess = Vec::new();
            let mut optional_prereq = Vec::new();

            for warning in &self.warnings {
                match warning {
                    PlanValidationWarning::UnnamedCourse { .. } => unnamed.push(warning),
                    PlanValidationWarning::PlaceholderCourse { .. } => placeholder.push(warning),
                    PlanValidationWarning::ExcessCredits { .. } => excess.push(warning),
                    PlanValidationWarning::OptionalPrerequisiteNotIncluded { .. } => {
                        optional_prereq.push(warning);
                    }
                }
            }

            let mut print_group = |title: &str, warnings: &[&PlanValidationWarning]| {
                if !warnings.is_empty() {
                    let _ = writeln!(report, "\n  {title} ({}):", warnings.len());
                    for warning in warnings {
                        let _ = writeln!(report, "    - {}", format_warning(warning));
                    }
                }
            };

            print_group("Unnamed Courses", &unnamed);
            print_group("Placeholder Courses", &placeholder);
            print_group("Optional Prerequisites", &optional_prereq);
            print_group("Credit Warnings", &excess);
        }

        report
    }
}

// ============================================================================
// Plan Validator
// ============================================================================

/// Configuration for plan validation
#[derive(Debug, Clone)]
pub struct PlanValidatorConfig {
    /// Target total credits (if specified)
    pub target_credits: Option<f32>,

    /// Tolerance for credit matching (e.g., 3.0 means +/- 3 credits)
    pub credit_tolerance: f32,

    /// Whether to check prerequisites strictly
    pub strict_prerequisites: bool,

    /// Expected courses per category (category name -> min count)
    pub category_requirements: HashMap<String, usize>,
}

impl Default for PlanValidatorConfig {
    fn default() -> Self {
        Self {
            target_credits: None,
            credit_tolerance: 3.0,
            strict_prerequisites: true,
            category_requirements: HashMap::new(),
        }
    }
}

/// Validates generated degree plans
pub struct PlanValidator<'a> {
    /// Course catalog for lookups
    courses: &'a HashMap<String, Course>,

    /// Course graph for prerequisite checking
    graph: &'a CourseGraph,

    /// Validation configuration
    config: PlanValidatorConfig,
}

impl<'a> PlanValidator<'a> {
    /// Create a new plan validator
    #[must_use]
    pub const fn new(
        courses: &'a HashMap<String, Course>,
        graph: &'a CourseGraph,
        config: PlanValidatorConfig,
    ) -> Self {
        Self {
            courses,
            graph,
            config,
        }
    }

    /// Validate a plan variant
    ///
    /// Checks for:
    /// - Missing prerequisites
    /// - Credit requirements
    /// - Category course counts
    /// - Undefined/placeholder courses
    #[must_use]
    pub fn validate(&self, plan: &PlanVariant) -> PlanValidationResult {
        let mut result = PlanValidationResult::new();
        let plan_courses: HashSet<&str> = plan.courses.iter().map(String::as_str).collect();

        // Collect statistics
        self.collect_statistics(plan, &mut result.stats);

        // Check for undefined/placeholder courses
        self.check_course_definitions(plan, &mut result);

        // Check prerequisites
        self.check_prerequisites(plan, &plan_courses, &mut result);

        // Check credit requirements
        self.check_credits(plan, &mut result);

        // Check category requirements
        self.check_category_requirements(plan, &mut result);

        result
    }

    /// Collect validation statistics
    fn collect_statistics(&self, plan: &PlanVariant, stats: &mut PlanValidationStats) {
        stats.total_courses = plan.courses.len();
        stats.total_credits = plan.total_credits;

        for course_key in &plan.courses {
            // Check if course has prerequisites
            if let Some(node) = self.graph.get(course_key) {
                if !node.prerequisite_paths.is_empty() || !node.prerequisites.is_empty() {
                    stats.courses_with_prereqs += 1;
                }
            }

            // Check if placeholder
            if is_placeholder_course(course_key) {
                stats.placeholder_courses += 1;
            }

            // Get credits for categorization
            let credits = self.courses.get(course_key).map_or(3.0, |c| c.credit_hours);

            // Categorize by requirement choice
            for (category, courses) in &plan.requirement_choices {
                if courses.contains(course_key) {
                    *stats
                        .courses_by_category
                        .entry(category.clone())
                        .or_insert(0) += 1;
                    *stats
                        .credits_by_category
                        .entry(category.clone())
                        .or_insert(0.0) += credits;
                    break;
                }
            }
        }
    }

    /// Check that all courses are defined or properly named
    fn check_course_definitions(&self, plan: &PlanVariant, result: &mut PlanValidationResult) {
        for course_key in &plan.courses {
            // Check if course exists in catalog
            if let Some(course) = self.courses.get(course_key) {
                // Check for "Unknown" name
                if course.name == "Unknown" || course.name.is_empty() {
                    result.add_warning(PlanValidationWarning::UnnamedCourse {
                        course: course_key.clone(),
                    });
                }
            } else if is_placeholder_course(course_key) {
                // Expected placeholder - just warn
                result.add_warning(PlanValidationWarning::PlaceholderCourse {
                    course: course_key.clone(),
                });
            } else {
                // Unexpected undefined course
                result.add_error(PlanValidationError::UndefinedCourse {
                    course: course_key.clone(),
                });
            }
        }
    }

    /// Check that all prerequisites are included in the plan
    fn check_prerequisites(
        &self,
        plan: &PlanVariant,
        plan_courses: &HashSet<&str>,
        result: &mut PlanValidationResult,
    ) {
        for course_key in &plan.courses {
            // Skip placeholder courses - they don't have prerequisites
            if is_placeholder_course(course_key) {
                continue;
            }

            // Get prerequisite information from graph
            if let Some(node) = self.graph.get(course_key) {
                // Check required prerequisites (from edges marked as required)
                for prereq in node.required_prerequisites() {
                    if !plan_courses.contains(prereq) {
                        if self.config.strict_prerequisites {
                            result.add_error(PlanValidationError::MissingPrerequisite {
                                course: course_key.clone(),
                                prerequisite: prereq.to_string(),
                            });
                        }
                        result.stats.missing_prereqs += 1;
                    }
                }

                // Check if at least one option is satisfied for OR prerequisites
                if !node.prerequisite_paths.is_empty() {
                    let any_path_satisfied = node.prerequisite_paths.iter().any(|path| {
                        path.iter()
                            .all(|prereq| plan_courses.contains(prereq.as_str()))
                    });

                    if !any_path_satisfied && !node.prerequisite_paths[0].is_empty() {
                        // Check if this is an OR choice where none are included
                        let all_options: Vec<String> = node
                            .prerequisite_paths
                            .iter()
                            .flat_map(|path| path.iter().cloned())
                            .collect();

                        if all_options.len() > 1 {
                            result.add_warning(
                                PlanValidationWarning::OptionalPrerequisiteNotIncluded {
                                    course: course_key.clone(),
                                    options: all_options,
                                },
                            );
                        }
                    }
                }
            }
        }
    }

    /// Check that credit requirements are met
    fn check_credits(&self, plan: &PlanVariant, result: &mut PlanValidationResult) {
        if let Some(target) = self.config.target_credits {
            if plan.total_credits < target - self.config.credit_tolerance {
                result.add_error(PlanValidationError::InsufficientCredits {
                    actual: plan.total_credits,
                    required: target,
                });
            } else if plan.total_credits > self.config.credit_tolerance.mul_add(2.0, target) {
                result.add_warning(PlanValidationWarning::ExcessCredits {
                    actual: plan.total_credits,
                    expected: target,
                });
            }
        }
    }

    /// Check that category requirements are met
    fn check_category_requirements(&self, plan: &PlanVariant, result: &mut PlanValidationResult) {
        for (category, required_count) in &self.config.category_requirements {
            let actual_count = plan.requirement_choices.get(category).map_or(0, Vec::len);

            if actual_count < *required_count {
                result.add_error(PlanValidationError::InsufficientCategoryCourses {
                    category: category.clone(),
                    actual: actual_count,
                    required: *required_count,
                });
            }
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Check if a course key appears to be a placeholder/generated course
fn is_placeholder_course(course_key: &str) -> bool {
    // Common placeholder patterns:
    // - ELEC### (elective placeholders)
    // - XX## where XX is 2-4 uppercase letters and ## is digits (gen-ed placeholders)
    // - Ends with 'S' indicating small/partial credit course

    if course_key.starts_with("ELEC") {
        return true;
    }

    // Check for short prefix + digits pattern (e.g., "FE01", "AC02")
    if course_key.len() <= 6 {
        let prefix: String = course_key
            .chars()
            .take_while(|c| c.is_alphabetic())
            .collect();
        let digits: String = course_key
            .chars()
            .skip_while(|c| c.is_alphabetic())
            .take_while(char::is_ascii_digit)
            .collect();

        if prefix.len() >= 2 && prefix.len() <= 4 && !digits.is_empty() && digits.len() <= 3 {
            // Likely a placeholder
            return true;
        }
    }

    false
}

/// Format a validation error as a string
fn format_error(error: &PlanValidationError) -> String {
    match error {
        PlanValidationError::MissingPrerequisite {
            course,
            prerequisite,
        } => {
            format!("Course '{course}' missing prerequisite '{prerequisite}'")
        }
        PlanValidationError::InsufficientCredits { actual, required } => {
            format!("Plan has {actual:.1} credits, needs {required:.1}")
        }
        PlanValidationError::InsufficientCategoryCourses {
            category,
            actual,
            required,
        } => {
            format!("Category '{category}' has {actual} courses, needs {required}")
        }
        PlanValidationError::UndefinedCourse { course } => {
            format!("Course '{course}' is not defined in the catalog")
        }
    }
}

/// Format a validation warning as a string
fn format_warning(warning: &PlanValidationWarning) -> String {
    match warning {
        PlanValidationWarning::UnnamedCourse { course } => {
            format!("Course '{course}' has no name or is named 'Unknown'")
        }
        PlanValidationWarning::PlaceholderCourse { course } => {
            format!("'{course}' is a placeholder course")
        }
        PlanValidationWarning::ExcessCredits { actual, expected } => {
            format!("Plan has {actual:.1} credits, expected ~{expected:.1}")
        }
        PlanValidationWarning::OptionalPrerequisiteNotIncluded { course, options } => {
            format!(
                "'{course}' needs one of [{}] but none included",
                options.join(", ")
            )
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_placeholder_course() {
        assert!(is_placeholder_course("ELEC001"));
        assert!(is_placeholder_course("ELEC01S"));
        assert!(is_placeholder_course("FE01"));
        assert!(is_placeholder_course("AC02"));
        assert!(is_placeholder_course("WC01S"));

        assert!(!is_placeholder_course("CS3000"));
        assert!(!is_placeholder_course("MATH1341"));
        assert!(!is_placeholder_course("ENGW1111"));
    }

    #[test]
    fn test_validation_result_format() {
        let mut result = PlanValidationResult::new();
        result.stats.total_courses = 40;
        result.stats.total_credits = 120.0;
        result.add_warning(PlanValidationWarning::PlaceholderCourse {
            course: "ELEC001".to_string(),
        });

        let report = result.format_report();
        assert!(report.contains("Plan is valid"));
        assert!(report.contains("40"));
        assert!(report.contains("120.0"));
    }

    #[test]
    fn test_validation_with_errors() {
        let mut result = PlanValidationResult::new();
        result.add_error(PlanValidationError::MissingPrerequisite {
            course: "CS3500".to_string(),
            prerequisite: "CS2500".to_string(),
        });

        assert!(!result.is_valid);
        let report = result.format_report();
        assert!(report.contains("validation errors"));
        assert!(report.contains("CS3500"));
        assert!(report.contains("CS2500"));
    }
}
