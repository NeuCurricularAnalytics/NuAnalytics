//! Plan variant representation for degree analysis
//!
//! Represents a single possible plan through a degree program,
//! capturing the specific courses chosen for each requirement.

use std::collections::{HashMap, HashSet};

/// A specific plan variant through a degree program
///
/// Represents one possible way to complete all degree requirements,
/// with specific course selections for each variable requirement.
#[derive(Debug, Clone, PartialEq)]
pub struct PlanVariant {
    /// All courses in this plan (unique, sorted for consistency)
    pub courses: Vec<String>,

    /// Mapping from requirement ID to chosen courses for that requirement
    pub requirement_choices: HashMap<String, Vec<String>>,

    /// Total credits in this plan
    pub total_credits: f32,

    /// Fingerprint for duplicate detection (hash of sorted courses)
    fingerprint: u64,
}

impl PlanVariant {
    /// Create a new plan variant from requirement choices
    ///
    /// # Arguments
    /// * `requirement_choices` - Map of requirement ID to selected courses
    /// * `course_credits` - Map of course key to credits for total calculation
    #[must_use]
    pub fn new(
        requirement_choices: HashMap<String, Vec<String>>,
        course_credits: &HashMap<String, f32>,
    ) -> Self {
        // Collect all unique courses
        let mut course_set: HashSet<String> = HashSet::new();
        for courses in requirement_choices.values() {
            course_set.extend(courses.iter().cloned());
        }

        // Sort for consistent ordering
        let mut courses: Vec<String> = course_set.into_iter().collect();
        courses.sort();

        // Calculate total credits
        // For placeholder courses (not in course_credits), use pattern-based credits:
        // - Courses ending in 'S' are "small" courses (2 credits)
        // - Other placeholders default to 3 credits
        let total_credits = courses
            .iter()
            .map(|c| {
                course_credits.get(c).copied().unwrap_or_else(|| {
                    if c.ends_with('S') {
                        2.0
                    } else {
                        3.0
                    }
                })
            })
            .sum();

        // Compute fingerprint for duplicate detection
        let fingerprint = Self::compute_fingerprint(&courses);

        Self {
            courses,
            requirement_choices,
            total_credits,
            fingerprint,
        }
    }

    /// Create a plan variant with pre-computed values (for efficiency)
    #[must_use]
    pub fn from_parts(
        courses: Vec<String>,
        requirement_choices: HashMap<String, Vec<String>>,
        total_credits: f32,
    ) -> Self {
        let fingerprint = Self::compute_fingerprint(&courses);
        Self {
            courses,
            requirement_choices,
            total_credits,
            fingerprint,
        }
    }

    /// Get the fingerprint for duplicate detection
    #[must_use]
    pub const fn fingerprint(&self) -> u64 {
        self.fingerprint
    }

    /// Check if this plan has the same courses as another (for deduplication)
    #[must_use]
    pub fn is_equivalent_to(&self, other: &Self) -> bool {
        self.fingerprint == other.fingerprint && self.courses == other.courses
    }

    /// Get the number of courses in this plan
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::len() is not const
    pub fn course_count(&self) -> usize {
        self.courses.len()
    }

    /// Check if a specific course is in this plan
    #[must_use]
    pub fn contains_course(&self, course: &str) -> bool {
        self.courses.binary_search(&course.to_string()).is_ok()
    }

    /// Compute a fingerprint hash from sorted courses
    fn compute_fingerprint(courses: &[String]) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        courses.hash(&mut hasher);
        hasher.finish()
    }
}

impl std::fmt::Display for PlanVariant {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Plan({} courses, {:.1} credits)",
            self.courses.len(),
            self.total_credits
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_credits() -> HashMap<String, f32> {
        let mut credits = HashMap::new();
        credits.insert("CS1000".to_string(), 4.0);
        credits.insert("CS2000".to_string(), 4.0);
        credits.insert("CS3000".to_string(), 4.0);
        credits.insert("MATH1000".to_string(), 3.0);
        credits.insert("MATH2000".to_string(), 3.0);
        credits
    }

    #[test]
    fn test_plan_variant_creation() {
        let credits = sample_credits();
        let mut choices = HashMap::new();
        choices.insert(
            "core".to_string(),
            vec!["CS1000".to_string(), "CS2000".to_string()],
        );
        choices.insert("math".to_string(), vec!["MATH1000".to_string()]);

        let plan = PlanVariant::new(choices, &credits);

        assert_eq!(plan.course_count(), 3);
        assert!((plan.total_credits - 11.0).abs() < f32::EPSILON);
        assert!(plan.contains_course("CS1000"));
        assert!(plan.contains_course("MATH1000"));
        assert!(!plan.contains_course("CS9999"));
    }

    #[test]
    fn test_plan_variant_deduplication() {
        let credits = sample_credits();

        // Same courses, different order in choices
        let mut choices1 = HashMap::new();
        choices1.insert("req1".to_string(), vec!["CS1000".to_string()]);
        choices1.insert("req2".to_string(), vec!["CS2000".to_string()]);

        let mut choices2 = HashMap::new();
        choices2.insert("req2".to_string(), vec!["CS2000".to_string()]);
        choices2.insert("req1".to_string(), vec!["CS1000".to_string()]);

        let plan1 = PlanVariant::new(choices1, &credits);
        let plan2 = PlanVariant::new(choices2, &credits);

        assert!(plan1.is_equivalent_to(&plan2));
        assert_eq!(plan1.fingerprint(), plan2.fingerprint());
    }

    #[test]
    fn test_plan_variant_different_courses() {
        let credits = sample_credits();

        let mut choices1 = HashMap::new();
        choices1.insert("req".to_string(), vec!["CS1000".to_string()]);

        let mut choices2 = HashMap::new();
        choices2.insert("req".to_string(), vec!["CS2000".to_string()]);

        let plan1 = PlanVariant::new(choices1, &credits);
        let plan2 = PlanVariant::new(choices2, &credits);

        assert!(!plan1.is_equivalent_to(&plan2));
    }

    #[test]
    fn test_plan_variant_display() {
        let credits = sample_credits();
        let mut choices = HashMap::new();
        choices.insert(
            "core".to_string(),
            vec!["CS1000".to_string(), "CS2000".to_string()],
        );

        let plan = PlanVariant::new(choices, &credits);
        let display = format!("{plan}");

        assert!(display.contains("2 courses"));
        assert!(display.contains("8.0 credits"));
    }

    #[test]
    fn test_plan_variant_default_credits() {
        // Course not in credits map should default to 3.0
        let credits = HashMap::new();
        let mut choices = HashMap::new();
        choices.insert("req".to_string(), vec!["UNKNOWN100".to_string()]);

        let plan = PlanVariant::new(choices, &credits);
        assert!((plan.total_credits - 3.0).abs() < f32::EPSILON);
    }
}
