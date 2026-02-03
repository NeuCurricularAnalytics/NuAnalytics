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
