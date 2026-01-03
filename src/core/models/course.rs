//! Course model

use serde::{Deserialize, Serialize};

/// Represents a course in a curriculum
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Course {
    /// Course name (e.g., "Calculus for Physical Scientists I")
    pub name: String,

    /// Course prefix (e.g., "MATH", "CS")
    pub prefix: String,

    /// Course number (e.g., "1342", "2510")
    pub number: String,

    /// Prerequisites - stored as "PREFIX NUMBER" keys (e.g., "MATH 1341")
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
