//! Plan model

use serde::{Deserialize, Serialize};

/// Represents a curriculum plan (graduation plan)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Plan {
    /// Plan name (e.g., "Standard CS Track", "Honors Track")
    pub name: String,

    /// Required courses - stored as course keys (e.g., "CS 2510")
    pub courses: Vec<String>,

    /// Associated degree identifier (e.g., "BS Computer Science")
    pub degree_id: String,

    /// Institution name (optional, defaults to parent school)
    pub institution: Option<String>,
}

impl Plan {
    /// Create a new plan
    ///
    /// # Arguments
    /// * `name` - Plan name
    /// * `degree_id` - Identifier for the associated degree
    #[must_use]
    pub const fn new(name: String, degree_id: String) -> Self {
        Self {
            name,
            courses: Vec::new(),
            degree_id,
            institution: None,
        }
    }

    /// Add a course to the plan
    ///
    /// # Arguments
    /// * `course_key` - Course key in format "PREFIX NUMBER"
    pub fn add_course(&mut self, course_key: String) {
        if !self.courses.contains(&course_key) {
            self.courses.push(course_key);
        }
    }

    /// Remove a course from the plan
    ///
    /// # Arguments
    /// * `course_key` - Course key to remove
    ///
    /// # Returns
    /// `true` if the course was removed, `false` if it wasn't in the plan
    pub fn remove_course(&mut self, course_key: &str) -> bool {
        if let Some(pos) = self.courses.iter().position(|c| c == course_key) {
            self.courses.remove(pos);
            true
        } else {
            false
        }
    }

    /// Set the institution name (for transfer plans)
    pub fn set_institution(&mut self, institution: String) {
        self.institution = Some(institution);
    }

    /// Get total number of courses in the plan
    #[must_use]
    pub const fn course_count(&self) -> usize {
        self.courses.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_plan_creation() {
        let plan = Plan::new(
            "Standard Track".to_string(),
            "BS Computer Science".to_string(),
        );

        assert_eq!(plan.name, "Standard Track");
        assert_eq!(plan.degree_id, "BS Computer Science");
        assert!(plan.courses.is_empty());
        assert!(plan.institution.is_none());
    }

    #[test]
    fn test_add_course() {
        let mut plan = Plan::new(
            "Standard Track".to_string(),
            "BS Computer Science".to_string(),
        );

        plan.add_course("CS1800".to_string());
        plan.add_course("CS2510".to_string());

        assert_eq!(plan.course_count(), 2);
        assert!(plan.courses.contains(&"CS1800".to_string()));
        assert!(plan.courses.contains(&"CS2510".to_string()));
    }

    #[test]
    fn test_add_duplicate_course() {
        let mut plan = Plan::new(
            "Standard Track".to_string(),
            "BS Computer Science".to_string(),
        );

        plan.add_course("CS1800".to_string());
        plan.add_course("CS1800".to_string());

        assert_eq!(plan.course_count(), 1);
    }

    #[test]
    fn test_remove_course() {
        let mut plan = Plan::new(
            "Standard Track".to_string(),
            "BS Computer Science".to_string(),
        );

        plan.add_course("CS1800".to_string());
        plan.add_course("CS2510".to_string());

        assert!(plan.remove_course("CS1800"));
        assert_eq!(plan.course_count(), 1);
        assert!(!plan.courses.contains(&"CS1800".to_string()));

        // Try removing again - should return false
        assert!(!plan.remove_course("CS1800"));
    }

    #[test]
    fn test_set_institution() {
        let mut plan = Plan::new(
            "Transfer Plan".to_string(),
            "BS Computer Science".to_string(),
        );

        plan.set_institution("Community College".to_string());
        assert_eq!(plan.institution, Some("Community College".to_string()));
    }

    #[test]
    fn test_plan_with_multiple_courses() {
        let mut plan = Plan::new(
            "Honors Track".to_string(),
            "BS Computer Science".to_string(),
        );

        let courses = vec!["CS1800", "CS2510", "CS3500", "MATH1342"];
        for course in courses {
            plan.add_course(course.to_string());
        }

        assert_eq!(plan.course_count(), 4);
    }
}
