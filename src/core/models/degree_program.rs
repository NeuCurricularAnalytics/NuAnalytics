//! Degree program structure for YAML deserialization
//!
//! A degree program consists of:
//! - Degree metadata (name, institution, requirements, etc.)
//! - Requirements mapping (ID -> Requirement)
//! - Courses mapping (subject+number -> Course)

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::degree::Requirement;
use super::{Course, Degree};

/// A complete degree program with metadata, requirements, and courses
///
/// This is the top-level structure deserialized from YAML degree files.
/// It combines:
/// - A `Degree` for program metadata
/// - A requirements map for degree requirements
/// - A courses map for course definitions
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeProgram {
    /// Degree metadata (id, institution, credits, GPA, etc.)
    pub degree: Degree,

    /// Requirements mapping (requirement ID -> Requirement definition)
    pub requirements: HashMap<String, Requirement>,

    /// Courses mapping (course key like "ICS111" -> Course)
    pub courses: HashMap<String, Course>,
}

impl DegreeProgram {
    /// Get all courses that are cross-listed with the given course
    ///
    /// This follows the cross-listing relationship to find all equivalent courses.
    /// If the course is not cross-listed, returns an empty vector.
    ///
    /// # Arguments
    /// * `course_key` - The course key to look up (e.g., "CS201")
    ///
    /// # Returns
    /// A vector of course keys that are cross-listed with the given course
    ///
    /// # Example
    /// ```no_run
    /// use nu_analytics::core::degree::load_degree_from_yaml;
    ///
    /// let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml").unwrap();
    /// let equivalents = program.get_cross_listed_courses("CS201");
    /// // Returns ["PHIL201"] since CS201 is cross-listed with PHIL201
    /// ```
    #[must_use]
    pub fn get_cross_listed_courses(&self, course_key: &str) -> Vec<String> {
        self.courses
            .get(course_key)
            .and_then(|course| course.cross_listed_as.as_ref())
            .cloned()
            .unwrap_or_default()
    }

    /// Check if two courses are cross-listed (equivalent)
    ///
    /// Returns true if either course lists the other in its `cross_listed_as` field.
    ///
    /// # Arguments
    /// * `course_a` - First course key
    /// * `course_b` - Second course key
    ///
    /// # Returns
    /// True if the courses are cross-listed with each other
    #[must_use]
    pub fn are_cross_listed(&self, course_a: &str, course_b: &str) -> bool {
        self.courses
            .get(course_a)
            .is_some_and(|c| c.is_cross_listed_with(course_b))
            || self
                .courses
                .get(course_b)
                .is_some_and(|c| c.is_cross_listed_with(course_a))
    }

    /// Get all courses that are equivalent to the given course
    ///
    /// This returns a set of all courses that should be treated as equivalent,
    /// including the original course itself. Useful for plan validation to ensure
    /// students don't take multiple equivalent courses.
    ///
    /// # Arguments
    /// * `course_key` - The course key to look up
    ///
    /// # Returns
    /// A set of all equivalent course keys including the original
    #[must_use]
    pub fn get_equivalent_course_set(&self, course_key: &str) -> Vec<String> {
        let mut equivalents = vec![course_key.to_string()];
        equivalents.extend(self.get_cross_listed_courses(course_key));
        equivalents
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_program() -> DegreeProgram {
        let mut courses = HashMap::new();

        let cs101 = Course::new(
            "Intro to CS".to_string(),
            "CS".to_string(),
            "101".to_string(),
            4.0,
        );
        // No cross-listing

        let mut cs201 = Course::new(
            "Ethics in Computing".to_string(),
            "CS".to_string(),
            "201".to_string(),
            4.0,
        );
        cs201.cross_listed_as = Some(vec!["PHIL201".to_string()]);

        let mut phil201 = Course::new(
            "Ethics in Computing".to_string(),
            "PHIL".to_string(),
            "201".to_string(),
            4.0,
        );
        phil201.cross_listed_as = Some(vec!["CS201".to_string()]);

        courses.insert("CS101".to_string(), cs101);
        courses.insert("CS201".to_string(), cs201);
        courses.insert("PHIL201".to_string(), phil201);

        DegreeProgram {
            degree: Degree::new(
                "Computer Science".to_string(),
                "BS".to_string(),
                None,
                "semester".to_string(),
            ),
            requirements: HashMap::new(),
            courses,
        }
    }

    #[test]
    fn test_get_cross_listed_courses() {
        let program = make_program();
        let cross = program.get_cross_listed_courses("CS201");
        assert_eq!(cross, vec!["PHIL201".to_string()]);
    }

    #[test]
    fn test_get_cross_listed_courses_none() {
        let program = make_program();
        let cross = program.get_cross_listed_courses("CS101");
        assert!(cross.is_empty());
    }

    #[test]
    fn test_get_cross_listed_courses_missing() {
        let program = make_program();
        let cross = program.get_cross_listed_courses("NONEXISTENT");
        assert!(cross.is_empty());
    }

    #[test]
    fn test_are_cross_listed() {
        let program = make_program();
        assert!(program.are_cross_listed("CS201", "PHIL201"));
        assert!(program.are_cross_listed("PHIL201", "CS201")); // symmetric
        assert!(!program.are_cross_listed("CS101", "CS201"));
    }

    #[test]
    fn test_get_equivalent_course_set() {
        let program = make_program();
        let equiv = program.get_equivalent_course_set("CS201");
        assert!(equiv.contains(&"CS201".to_string()));
        assert!(equiv.contains(&"PHIL201".to_string()));
        assert_eq!(equiv.len(), 2);
    }

    #[test]
    fn test_get_equivalent_course_set_no_cross_listing() {
        let program = make_program();
        let equiv = program.get_equivalent_course_set("CS101");
        assert_eq!(equiv, vec!["CS101".to_string()]);
    }
}
