//! School model

use super::{Course, Degree, Plan};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Represents an educational institution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct School {
    /// School name
    pub name: String,

    /// Courses offered by the school, indexed by course key (PREFIXNUMBER)
    courses: HashMap<String, Course>,

    /// Degrees offered by the school
    pub degrees: Vec<Degree>,

    /// Curriculum plans offered by the school
    pub plans: Vec<Plan>,
}

impl School {
    /// Create a new school
    ///
    /// # Arguments
    /// * `name` - School name
    #[must_use]
    pub fn new(name: String) -> Self {
        Self {
            name,
            courses: HashMap::new(),
            degrees: Vec::new(),
            plans: Vec::new(),
        }
    }

    /// Add a course to the school
    ///
    /// # Arguments
    /// * `course` - Course to add
    ///
    /// # Returns
    /// `true` if the course was added, `false` if a course with that key already exists
    /// Add a course to the school (fails if key already exists)
    ///
    /// # Arguments
    /// * `course` - The course to add
    ///
    /// # Returns
    /// true if the course was added, false if a course with the same key already exists
    pub fn add_course(&mut self, course: Course) -> bool {
        let key = course.key();
        self.courses.insert(key, course).is_none()
    }

    /// Add a course with a custom key (for handling deduplication)
    ///
    /// # Arguments
    /// * `key` - Custom key for the course
    /// * `course` - The course to add
    pub fn add_course_with_key(&mut self, key: String, course: Course) {
        self.courses.insert(key, course);
    }

    /// Get a course by its storage key (which may include deduplicated suffixes)
    ///
    /// # Arguments
    /// * `storage_key` - The key used to store the course in the `HashMap`
    ///
    /// # Returns
    /// A reference to the course, or `None` if not found
    #[must_use]
    pub fn get_course(&self, storage_key: &str) -> Option<&Course> {
        self.courses.get(storage_key)
    }

    /// Get a course by its natural key (PREFIX NUMBER) - returns the first match if there are duplicates
    ///
    /// # Arguments
    /// * `natural_key` - The course key (e.g., "CS2510")
    ///
    /// # Returns
    /// A reference to the course, or `None` if not found
    #[must_use]
    pub fn get_course_by_natural_key(&self, natural_key: &str) -> Option<&Course> {
        self.courses
            .iter()
            .find(|(_, course)| course.key() == natural_key)
            .map(|(_, course)| course)
    }

    /// Get the storage key for a course (which may differ from its natural key due to deduplication)
    ///
    /// # Arguments
    /// * `natural_key` - The course key (e.g., "CS2510")
    ///
    /// # Returns
    /// The storage key (e.g., "`CS2510_2`" for a deduplicated course), or None if not found
    #[must_use]
    pub fn get_storage_key(&self, natural_key: &str) -> Option<String> {
        self.courses
            .iter()
            .find(|(_, course)| course.key() == natural_key)
            .map(|(storage_key, _)| storage_key.clone())
    }

    /// Get a mutable reference to a course by its key
    ///
    /// # Arguments
    /// * `key` - Course key (e.g., "CS 2510")
    ///
    /// # Returns
    /// A mutable reference to the course, or `None` if not found
    pub fn get_course_mut(&mut self, key: &str) -> Option<&mut Course> {
        self.courses.get_mut(key)
    }

    /// Get all courses
    #[must_use]
    pub fn courses(&self) -> Vec<&Course> {
        self.courses.values().collect()
    }

    /// Get all courses with their storage keys
    ///
    /// # Returns
    /// An iterator over (`storage_key`, course) pairs
    pub fn courses_with_keys(&self) -> impl Iterator<Item = (&String, &Course)> {
        self.courses.iter()
    }

    /// Add a degree to the school
    pub fn add_degree(&mut self, degree: Degree) {
        self.degrees.push(degree);
    }

    /// Get a degree by its ID
    ///
    /// # Arguments
    /// * `degree_id` - Degree identifier (e.g., "BS Computer Science")
    ///
    /// # Returns
    /// A reference to the degree, or `None` if not found
    #[must_use]
    pub fn get_degree(&self, degree_id: &str) -> Option<&Degree> {
        self.degrees.iter().find(|d| d.id() == degree_id)
    }

    /// Add a plan to the school
    pub fn add_plan(&mut self, plan: Plan) {
        self.plans.push(plan);
    }

    /// Get plans associated with a specific degree
    ///
    /// # Arguments
    /// * `degree_id` - Degree identifier
    ///
    /// # Returns
    /// A vector of references to plans for that degree
    #[must_use]
    pub fn get_plans_for_degree(&self, degree_id: &str) -> Vec<&Plan> {
        self.plans
            .iter()
            .filter(|p| p.degree_id == degree_id)
            .collect()
    }

    /// Validate that all courses in all plans exist in the school
    ///
    /// # Returns
    /// `Ok(())` if all courses exist, `Err(Vec<String>)` with missing course keys
    ///
    /// # Errors
    /// Returns `Err` with a list of error messages for courses referenced in plans that don't exist
    pub fn validate_plans(&self) -> Result<(), Vec<String>> {
        let mut missing = Vec::new();

        for plan in &self.plans {
            for course_key in &plan.courses {
                if self.get_course(course_key).is_none() {
                    missing.push(format!(
                        "Plan '{}': missing course '{}'",
                        plan.name, course_key
                    ));
                }
            }
        }

        if missing.is_empty() {
            Ok(())
        } else {
            Err(missing)
        }
    }

    /// Validate that all prerequisites and corequisites exist
    ///
    /// # Returns
    /// `Ok(())` if all references are valid, `Err(Vec<String>)` with invalid references
    ///
    /// # Errors
    /// Returns `Err` with a list of error messages for invalid prerequisite or corequisite references
    pub fn validate_course_dependencies(&self) -> Result<(), Vec<String>> {
        let mut invalid = Vec::new();

        for course in self.courses.values() {
            for prereq in &course.prerequisites {
                if self.get_course(prereq).is_none() {
                    invalid.push(format!(
                        "Course '{}': prerequisite '{}' not found",
                        course.key(),
                        prereq
                    ));
                }
            }

            for coreq in &course.corequisites {
                if self.get_course(coreq).is_none() {
                    invalid.push(format!(
                        "Course '{}': corequisite '{}' not found",
                        course.key(),
                        coreq
                    ));
                }
            }
        }

        if invalid.is_empty() {
            Ok(())
        } else {
            Err(invalid)
        }
    }

    /// Build a directed acyclic graph (DAG) of course prerequisites
    ///
    /// # Returns
    /// A DAG with all courses and their prerequisite relationships
    #[must_use]
    pub fn build_dag(&self) -> super::DAG {
        let mut dag = super::DAG::new();

        // Add all courses to the DAG using the keys they're stored under
        for stored_key in self.courses.keys() {
            dag.add_course(stored_key.clone());
        }

        // Add prerequisite relationships
        // Note: prerequisite keys stored in course.prerequisites are already the stored keys
        // (including deduplication suffixes), so we can add them directly to the DAG
        for (stored_key, course) in &self.courses {
            for prereq_key in &course.prerequisites {
                // Check if this prerequisite key exists in our courses
                if self.courses.contains_key(prereq_key) {
                    dag.add_prerequisite(stored_key.clone(), prereq_key.as_str());
                }
            }

            for coreq_key in &course.corequisites {
                // Check if this corequisite key exists in our courses
                if self.courses.contains_key(coreq_key) {
                    dag.add_corequisite(stored_key.clone(), coreq_key.as_str());
                }
            }

            for coreq_key in &course.strict_corequisites {
                if self.courses.contains_key(coreq_key) {
                    dag.add_corequisite(stored_key.clone(), coreq_key.as_str());
                }
            }
        }

        dag
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_school_creation() {
        let school = School::new("Northeastern University".to_string());

        assert_eq!(school.name, "Northeastern University");
        assert!(school.courses().is_empty());
        assert!(school.degrees.is_empty());
        assert!(school.plans.is_empty());
    }

    #[test]
    fn test_add_and_get_course() {
        let mut school = School::new("Test University".to_string());

        let course = Course::new(
            "Discrete Structures".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        );

        assert!(school.add_course(course));

        let retrieved = school.get_course("CS1800");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().name, "Discrete Structures");
    }

    #[test]
    fn test_add_duplicate_course() {
        let mut school = School::new("Test University".to_string());

        let course1 = Course::new(
            "Discrete Structures".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        );

        let course2 = Course::new(
            "Different Name".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        );

        assert!(school.add_course(course1));
        assert!(!school.add_course(course2)); // Should fail - duplicate key

        assert_eq!(school.courses().len(), 1);
    }

    #[test]
    fn test_course_lookup() {
        let mut school = School::new("Test University".to_string());

        for i in 1..=5 {
            let course = Course::new(
                format!("Course {i}"),
                "CS".to_string(),
                format!("{i}000"),
                4.0,
            );
            school.add_course(course);
        }

        assert!(school.get_course("CS3000").is_some());
        assert!(school.get_course("CS9999").is_none());
    }

    #[test]
    fn test_add_degree() {
        let mut school = School::new("Test University".to_string());

        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            "11.0701".to_string(),
            "semester".to_string(),
        );

        school.add_degree(degree);

        assert_eq!(school.degrees.len(), 1);

        let retrieved = school.get_degree("BS Computer Science");
        assert!(retrieved.is_some());
        assert_eq!(retrieved.unwrap().cip_code, "11.0701");
    }

    #[test]
    fn test_add_plan() {
        let mut school = School::new("Test University".to_string());

        let plan = Plan::new(
            "Standard Track".to_string(),
            "BS Computer Science".to_string(),
        );

        school.add_plan(plan);

        assert_eq!(school.plans.len(), 1);
    }

    #[test]
    fn test_get_plans_for_degree() {
        let mut school = School::new("Test University".to_string());

        let plan1 = Plan::new("Track 1".to_string(), "BS Computer Science".to_string());
        let plan2 = Plan::new("Track 2".to_string(), "BS Computer Science".to_string());
        let plan3 = Plan::new("Track 3".to_string(), "BA Data Science".to_string());

        school.add_plan(plan1);
        school.add_plan(plan2);
        school.add_plan(plan3);

        let cs_plans = school.get_plans_for_degree("BS Computer Science");
        assert_eq!(cs_plans.len(), 2);

        let ds_plans = school.get_plans_for_degree("BA Data Science");
        assert_eq!(ds_plans.len(), 1);
    }

    #[test]
    fn test_validate_plans_success() {
        let mut school = School::new("Test University".to_string());

        // Add courses
        school.add_course(Course::new(
            "Course 1".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        ));
        school.add_course(Course::new(
            "Course 2".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        ));

        // Add plan with valid courses
        let mut plan = Plan::new("Track".to_string(), "BS CS".to_string());
        plan.add_course("CS1800".to_string());
        plan.add_course("CS2510".to_string());
        school.add_plan(plan);

        assert!(school.validate_plans().is_ok());
    }

    #[test]
    fn test_validate_plans_failure() {
        let mut school = School::new("Test University".to_string());

        // Add only one course
        school.add_course(Course::new(
            "Course 1".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        ));

        // Add plan with invalid course
        let mut plan = Plan::new("Track".to_string(), "BS CS".to_string());
        plan.add_course("CS1800".to_string());
        plan.add_course("CS9999".to_string()); // Doesn't exist
        school.add_plan(plan);

        let result = school.validate_plans();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("CS9999"));
    }

    #[test]
    fn test_validate_course_dependencies_success() {
        let mut school = School::new("Test University".to_string());

        // Add courses
        school.add_course(Course::new(
            "Discrete".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        ));

        let mut course2 = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        );
        course2.add_prerequisite("CS1800".to_string());
        school.add_course(course2);

        assert!(school.validate_course_dependencies().is_ok());
    }

    #[test]
    fn test_validate_course_dependencies_failure() {
        let mut school = School::new("Test University".to_string());

        let mut course = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        );
        course.add_prerequisite("CS9999".to_string()); // Doesn't exist
        school.add_course(course);

        let result = school.validate_course_dependencies();
        assert!(result.is_err());
        let errors = result.unwrap_err();
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("prerequisite"));
    }

    #[test]
    fn test_get_course_mut() {
        let mut school = School::new("Test University".to_string());

        let course = Course::new(
            "Test".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        );
        school.add_course(course);

        {
            let course_mut = school.get_course_mut("CS1800").unwrap();
            course_mut.set_canonical_name("Testing 101".to_string());
        }

        let course = school.get_course("CS1800").unwrap();
        assert_eq!(course.canonical_name, Some("Testing 101".to_string()));
    }

    #[test]
    fn test_courses_iteration() {
        let mut school = School::new("Test University".to_string());

        school.add_course(Course::new(
            "C1".to_string(),
            "CS".to_string(),
            "1800".to_string(),
            4.0,
        ));
        school.add_course(Course::new(
            "C2".to_string(),
            "CS".to_string(),
            "2510".to_string(),
            4.0,
        ));

        let courses = school.courses();
        assert_eq!(courses.len(), 2);

        let keys: Vec<String> = courses.iter().map(|c| c.key()).collect();
        assert!(keys.contains(&"CS1800".to_string()));
        assert!(keys.contains(&"CS2510".to_string()));
    }
}
