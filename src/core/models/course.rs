//! Course model

use serde::{Deserialize, Serialize};

/// Represents a course in a curriculum
///
/// # Note on Complex Prerequisites
/// Currently, prerequisites are stored as a flat list with implicit AND semantics.
/// However, real curricula often have complex boolean expressions like:
/// - "CS101 OR CS102"
/// - "(CS101 AND MATH156) OR CS200"
/// - "CS101 OR (CS102 AND MATH156)"
///
/// Several approaches to handle complex prerequisites:
///
/// 1. **Disjunctive Normal Form (DNF)**: Store as `Vec<Vec<String>>` where
///    outer Vec is OR, inner Vec is AND. Example: `[[\"CS101\"], [\"CS102\", \"MATH156\"]]`
///    means CS101 OR (CS102 AND MATH156). This is a standard form in logic.
///
/// 2. **Prerequisite Expression Trees**: Use a recursive enum:
///    ```ignore
///    enum PrereqExpr {
///        Course(String),
///        And(Vec<PrereqExpr>),
///        Or(Vec<PrereqExpr>),
///    }
///    ```
///    This can represent any boolean expression and is most flexible.
///
/// 3. **Virtual Courses**: Create synthetic course keys like `\"CS101_OR_CS102\"` or
///    `\"CS101_AND_MATH156\"` in the DAG. Each virtual course represents a requirement
///    that can be satisfied by its components.
///
/// 4. **Hypergraph Representation**: Extend DAG to support hyperedges where a single
///    edge can connect to multiple prerequisite sets with boolean operators.
///
/// 5. **Choice Resolution at Plan Build Time** (Recommended for Plan Analysis): The Course
///    struct stores the full prerequisite expression (using one of the above approaches),
///    but when building a DAG from a Plan, the plan specifies which alternative was chosen.
///    This keeps plan DAGs simple while preserving the full requirement information in courses.
///    Plans represent actual student selections where boolean logic has been resolved.
///
/// **Note**: Regardless of approach, the Course struct must be able to represent the full
/// prerequisite expression. The choice resolution approach means that when analyzing a
/// specific plan, we only include the paths the student actually took, not all possibilities.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Course {
    /// Course name (e.g., "Calculus for Physical Scientists I")
    pub name: String,

    /// Course prefix (e.g., "MATH", "CS")
    pub prefix: String,

    /// Course number (e.g., "1342", "2510")
    pub number: String,

    /// Prerequisites - stored as "PREFIX NUMBER" keys (e.g., "MATH 1341")
    /// Currently assumes ALL prerequisites must be satisfied (AND semantics)
    pub prerequisites: Vec<String>,

    /// Co-requisites - stored as "PREFIX NUMBER" keys
    pub corequisites: Vec<String>,

    /// Credit hours (can be fractional)
    pub credit_hours: f32,

    /// Canonical name for cross-institution lookup (e.g., "Calculus I")
    pub canonical_name: Option<String>,
}

impl Course {
    /// Create a new course
    ///
    /// # Arguments
    /// * `name` - Full course name
    /// * `prefix` - Course prefix
    /// * `number` - Course number
    /// * `credit_hours` - Credit hours (can be fractional)
    #[must_use]
    pub const fn new(name: String, prefix: String, number: String, credit_hours: f32) -> Self {
        Self {
            name,
            prefix,
            number,
            prerequisites: Vec::new(),
            corequisites: Vec::new(),
            credit_hours,
            canonical_name: None,
        }
    }

    /// Get the course key for lookups (prefix + number)
    ///
    /// # Returns
    /// A string in the format "PREFIXNUMBER" (e.g., "CS2510")
    #[must_use]
    pub fn key(&self) -> String {
        format!("{}{}", self.prefix, self.number)
    }

    /// Add a prerequisite by course key
    pub fn add_prerequisite(&mut self, prereq_key: String) {
        if !self.prerequisites.contains(&prereq_key) {
            self.prerequisites.push(prereq_key);
        }
    }

    /// Add a co-requisite by course key
    pub fn add_corequisite(&mut self, coreq_key: String) {
        if !self.corequisites.contains(&coreq_key) {
            self.corequisites.push(coreq_key);
        }
    }

    /// Set the canonical name
    pub fn set_canonical_name(&mut self, name: String) {
        self.canonical_name = Some(name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_course_creation() {
        let course = Course::new(
            "Discrete Structures".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        );

        assert_eq!(course.name, "Discrete Structures");
        assert_eq!(course.prefix, "CS");
        assert_eq!(course.number, "1800");
        assert!((course.credit_hours - 4.0).abs() < f32::EPSILON);
        assert!(course.prerequisites.is_empty());
        assert!(course.corequisites.is_empty());
        assert!(course.canonical_name.is_none());
    }

    #[test]
    fn test_course_key() {
        let course = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        );

        assert_eq!(course.key(), "CS2510");
    }

    #[test]
    fn test_fractional_credits() {
        let course = Course::new(
            "Lab".to_string(),
            "PHYS".to_string(),
            "1151".to_string(),
            1.5,
        );

        assert!((course.credit_hours - 1.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_add_prerequisite() {
        let mut course = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        );

        course.add_prerequisite("CS1800".to_string());
        assert_eq!(course.prerequisites.len(), 1);
        assert_eq!(course.prerequisites[0], "CS1800");

        // Adding duplicate should not duplicate
        course.add_prerequisite("CS1800".to_string());
        assert_eq!(course.prerequisites.len(), 1);
    }

    #[test]
    fn test_add_corequisite() {
        let mut course = Course::new(
            "Physics I".to_string(),
            "PHYS".to_string(),
            "1151".to_string(),
            4.0,
        );

        course.add_corequisite("PHYS1152".to_string());
        assert_eq!(course.corequisites.len(), 1);
        assert_eq!(course.corequisites[0], "PHYS1152");
    }

    #[test]
    fn test_canonical_name() {
        let mut course = Course::new(
            "Calculus for Physical Scientists I".to_string(),
            "MATH".to_string(),
            "1342".to_string(),
            4.0,
        );

        assert!(course.canonical_name.is_none());

        course.set_canonical_name("Calculus I".to_string());
        assert_eq!(course.canonical_name, Some("Calculus I".to_string()));
    }
}
