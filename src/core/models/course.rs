//! Course model

use serde::{Deserialize, Deserializer, Serialize};

/// Variable credit range for courses with flexible credits
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CreditRange {
    /// Minimum credits
    pub min: u32,
    /// Maximum credits
    pub max: u32,
}

/// Represents a course in a curriculum
///
/// This struct supports both CSV plan loading (basic fields) and
/// full YAML degree loading (extended fields). Optional fields
/// are populated when loading from YAML degree definitions.
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
///
/// Deserializer for `Vec<String>` that handles null values and strings
fn deserialize_vec_string<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum VecStringOrNull {
        Vec(Vec<String>),
        String(String),
        Null,
    }

    match VecStringOrNull::deserialize(deserializer)? {
        VecStringOrNull::Vec(v) => Ok(v),
        VecStringOrNull::String(_s) => Ok(vec![]), // Ignore string prerequisites for now
        VecStringOrNull::Null => Ok(vec![]),
    }
}

/// Deserializer for `prerequisites_raw` that captures string prerequisites
fn deserialize_prerequisites_raw<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum PrereqRaw {
        String(String),
        Vec(Vec<String>),
        Null,
    }

    match PrereqRaw::deserialize(deserializer)? {
        PrereqRaw::String(s) => Ok(Some(s)),
        PrereqRaw::Vec(_v) => Ok(None), // Vec is resolved prerequisites, not raw
        PrereqRaw::Null => Ok(None),
    }
}

/// Represents a course in a curriculum
///
/// This struct supports both CSV plan loading (basic fields) and
/// full YAML degree loading (extended fields). Optional fields
/// are populated when loading from YAML degree definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Course {
    // === Core fields (used by CSV and YAML) ===
    /// Original course ID from the curriculum file
    pub csv_id: Option<String>,

    /// Unique course identifier (optional, used for deduplication when multiple courses have same key)
    pub id: Option<String>,

    /// Course name/title (e.g., "Calculus for Physical Scientists I")
    #[serde(alias = "title")]
    pub name: String,

    /// Course prefix/subject (e.g., "MATH", "CS", "ICS")
    #[serde(alias = "subject")]
    pub prefix: String,

    /// Course number (e.g., "1342", "2510", "111")
    pub number: String,

    /// Prerequisites - stored as "PREFIX NUMBER" keys (e.g., "MATH 1341")
    /// Currently assumes ALL prerequisites must be satisfied (AND semantics)
    /// For YAML sources, use `prerequisites_raw` for boolean expressions
    #[serde(default, deserialize_with = "deserialize_vec_string", skip)]
    pub prerequisites: Vec<String>,

    /// Co-requisites - stored as "PREFIX NUMBER" keys
    #[serde(default, deserialize_with = "deserialize_vec_string")]
    pub corequisites: Vec<String>,

    /// Strict co-requisites - stored as "PREFIX NUMBER" keys (must be taken together)
    #[serde(default, deserialize_with = "deserialize_vec_string")]
    pub strict_corequisites: Vec<String>,

    /// Credit hours (can be fractional)
    #[serde(alias = "credits", default)]
    pub credit_hours: f32,

    /// Canonical name for cross-institution lookup (e.g., "Calculus I")
    pub canonical_name: Option<String>,

    // === Extended fields (optional, populated from YAML) ===
    /// Raw prerequisite expression string (e.g., "(ICS311 | ECE367) & ICS314")
    /// Stored as-is for future parsing; the `prerequisites` Vec is the resolved form
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        alias = "prerequisites",
        deserialize_with = "deserialize_prerequisites_raw"
    )]
    pub prerequisites_raw: Option<String>,

    /// Terms typically offered (e.g., `["fall", "spring"]`)
    #[serde(skip_serializing_if = "Option::is_none", alias = "typically_offered")]
    pub typically_offered: Option<Vec<String>>,

    /// General education attributes (e.g., `["FW", "DP"]`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen_ed_attributes: Option<Vec<String>>,

    /// Cross-listed as other courses (e.g., `["DATA434", "CINE484"]`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_listed_as: Option<Vec<String>>,

    /// Whether the course can be repeated for credit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeatable: Option<bool>,

    /// Maximum credits that can be earned if repeatable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_repeat_credits: Option<f32>,

    /// Variable credit range (alternative to fixed `credit_hours`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_range: Option<CreditRange>,
}

impl Course {
    /// Create a new course with basic fields (for CSV loading)
    ///
    /// # Arguments
    /// * `name` - Full course name
    /// * `prefix` - Course prefix
    /// * `number` - Course number
    /// * `credit_hours` - Credit hours (can be fractional)
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(name: String, prefix: String, number: String, credit_hours: f32) -> Self {
        Self {
            csv_id: None,
            id: None,
            name,
            prefix,
            number,
            prerequisites: Vec::new(),
            corequisites: Vec::new(),
            strict_corequisites: Vec::new(),
            credit_hours,
            canonical_name: None,
            // Extended fields default to None
            prerequisites_raw: None,
            typically_offered: None,
            gen_ed_attributes: None,
            cross_listed_as: None,
            repeatable: None,
            max_repeat_credits: None,
            credit_range: None,
        }
    }

    /// Create a course from YAML fields
    ///
    /// # Arguments
    /// * `prefix` - Course subject/prefix (e.g., "ICS")
    /// * `number` - Course number (e.g., "111")
    /// * `title` - Course title
    /// * `credits` - Credit hours (optional if `credit_range` is used)
    /// * `prerequisites_raw` - Raw prerequisite expression string
    #[must_use]
    pub fn from_yaml(
        prefix: String,
        number: String,
        title: String,
        credits: Option<f32>,
        prerequisites_raw: Option<String>,
    ) -> Self {
        Self {
            csv_id: None,
            id: None,
            name: title,
            prefix,
            number,
            prerequisites: Vec::new(), // Will be populated after parsing prerequisites_raw
            corequisites: Vec::new(),
            strict_corequisites: Vec::new(),
            credit_hours: credits.unwrap_or(0.0),
            canonical_name: None,
            prerequisites_raw,
            typically_offered: None,
            gen_ed_attributes: None,
            cross_listed_as: None,
            repeatable: None,
            max_repeat_credits: None,
            credit_range: None,
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

    /// Get actual credits, accounting for variable credit ranges
    ///
    /// If `credit_range` is set, returns the minimum; otherwise returns `credit_hours`
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn actual_credits(&self) -> f32 {
        self.credit_range
            .as_ref()
            .map_or(self.credit_hours, |range| range.min as f32)
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

    /// Add a strict co-requisite by course key
    pub fn add_strict_corequisite(&mut self, coreq_key: String) {
        if !self.strict_corequisites.contains(&coreq_key) {
            self.strict_corequisites.push(coreq_key);
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
