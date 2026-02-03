//! Degree model

use serde::{Deserialize, Serialize};

// Re-export CreditRange from Course model for convenience
pub use crate::core::models::course::CreditRange;

/// Represents a degree program
///
/// This struct supports both CSV plan loading (basic fields) and
/// full YAML degree loading (extended fields). Optional fields
/// are populated when loading from YAML degree definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Degree {
    // === Core fields (required, used by CSV and YAML) ===
    /// Degree name / program name (e.g., "Computer Science" or full "Bachelor of Science in Computer Science")
    #[serde(alias = "program")]
    pub name: String,

    /// Degree type (e.g., "BS", "BA", "MS")
    #[serde(default = "default_degree_type")]
    pub degree_type: String,

    /// CIP code (Classification of Instructional Programs) - optional
    #[serde(default)]
    pub cip_code: Option<String>,

    /// System type ("semester" or "quarter") - defaults to "semester"
    #[serde(default = "default_system_type")]
    pub system_type: String,

    // === Extended fields (optional, populated from YAML) ===
    /// Unique identifier for this degree (e.g., "uhm-ics-bscs-general")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,

    /// Institution name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub institution: Option<String>,

    /// Catalog year (e.g., "2024-2025")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_year: Option<String>,

    /// Source URL for the official catalog
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,

    /// Total credits required for graduation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_credits: Option<u32>,

    /// Minimum upper-division (300+) credits required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper_division_credits: Option<u32>,

    /// Minimum credits within major subjects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_major_credits: Option<u32>,

    /// Minimum overall GPA required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpa_minimum: Option<f32>,

    /// Minimum GPA for major courses (if different from overall)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpa_major: Option<f32>,

    /// Default minimum grade for major courses (e.g., "C", "B")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_minimum: Option<String>,

    /// Note clarifying grade requirements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_minimum_note: Option<String>,

    /// Subject codes that count toward the major (e.g., `["ICS"]`, `["CS", "CY", "DS"]`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub major_subjects: Option<Vec<String>>,

    /// Whether courses can satisfy multiple requirements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_double_counting: Option<bool>,
}

/// A requirement in a degree program
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Requirement {
    /// Human-readable requirement name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Requirement type
    #[serde(rename = "type")]
    pub req_type: RequirementType,

    /// Category: major, supporting, `gen_ed`, elective
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,

    /// List of courses (for type: all)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courses: Option<Vec<String>>,

    /// Selection options (for type: select)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<FromClause>,

    /// Number of courses to select (for type: select)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,

    /// Total credits to reach (for type: select)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<u32>,

    /// Variable credit range (for type: select)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_range: Option<CreditRange>,

    /// Constraints on the requirement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub constraints: Option<RequirementConstraints>,

    /// Mutually exclusive paths (for type: `one_of`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<Vec<RequirementOption>>,
}

/// Types of requirements
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RequirementType {
    /// Complete all listed courses
    All,

    /// Select N courses/credits from options
    Select,

    /// Choose one path (mutually exclusive options)
    OneOf,
}

/// Source of courses for a select requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FromClause {
    /// Explicit list of courses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub courses: Option<Vec<String>>,

    /// Pattern-based course selection (e.g., "ICS:400+", "CS:*")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,

    /// Courses/patterns to exclude
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<Vec<String>>,

    /// Grouped selection (pick from N groups)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups: Option<Vec<CourseGroup>>,

    /// How many groups to select (null = all)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub groups_required: Option<u32>,

    /// Courses to take per selected group
    #[serde(skip_serializing_if = "Option::is_none")]
    pub per_group: Option<u32>,
}

/// A group of courses within a select requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CourseGroup {
    /// Unique identifier for this group
    pub id: String,

    /// Human-readable group name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// List of courses in this group
    pub courses: Vec<String>,
}

/// Constraints on requirement satisfaction
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementConstraints {
    /// Exclude courses already used in prior requirements
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude_used: Option<bool>,

    /// Selections must be from different subjects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub distinct_subjects: Option<bool>,

    /// Minimum upper-division courses/credits
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_upper_division: Option<u32>,

    /// Maximum credits from any single subject
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_from_subject: Option<u32>,

    /// Pattern to limit (e.g., "ICS:400+")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_from_pattern: Option<String>,

    /// Credit limit for pattern
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_from_pattern_credits: Option<u32>,

    /// Override default grade requirement
    #[serde(skip_serializing_if = "Option::is_none")]
    pub grade_minimum: Option<String>,
}

/// A mutually exclusive option within a `one_of` requirement
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequirementOption {
    /// Unique identifier for this option
    pub id: String,

    /// Human-readable option name
    pub name: String,

    /// Nested requirements for this path
    pub requirements: Vec<Requirement>,
}

/// Default system type for degrees
fn default_system_type() -> String {
    "semester".to_string()
}

fn default_degree_type() -> String {
    "BS".to_string()
}

impl Degree {
    /// Create a new degree with basic fields (for CSV loading)
    ///
    /// # Arguments
    /// * `name` - Degree name
    /// * `degree_type` - Degree type (BS, BA, etc.)
    /// * `cip_code` - CIP code (optional)
    /// * `system_type` - System type ("semester" or "quarter", defaults to "semester")
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(
        name: String,
        degree_type: String,
        cip_code: Option<String>,
        system_type: String,
    ) -> Self {
        Self {
            name,
            degree_type,
            cip_code,
            system_type,
            // Extended fields default to None
            id: None,
            institution: None,
            catalog_year: None,
            source_url: None,
            total_credits: None,
            upper_division_credits: None,
            in_major_credits: None,
            gpa_minimum: None,
            gpa_major: None,
            grade_minimum: None,
            grade_minimum_note: None,
            major_subjects: None,
            allow_double_counting: None,
        }
    }

    /// Create a degree with full metadata (for YAML loading)
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::missing_const_for_fn)]
    pub fn with_metadata(
        id: String,
        institution: String,
        program: String,
        catalog_year: String,
        system_type: String,
        total_credits: u32,
        gpa_minimum: f32,
        allow_double_counting: bool,
    ) -> Self {
        Self {
            name: program,
            degree_type: String::new(), // Will be parsed from program name if needed
            cip_code: None,
            system_type,
            id: Some(id),
            institution: Some(institution),
            catalog_year: Some(catalog_year),
            source_url: None,
            total_credits: Some(total_credits),
            upper_division_credits: None,
            in_major_credits: None,
            gpa_minimum: Some(gpa_minimum),
            gpa_major: None,
            grade_minimum: None,
            grade_minimum_note: None,
            major_subjects: None,
            allow_double_counting: Some(allow_double_counting),
        }
    }

    /// Check if this degree uses a quarter system
    #[must_use]
    pub fn is_quarter_system(&self) -> bool {
        self.system_type.to_lowercase().contains("quarter")
    }

    /// Get the complexity scaling factor based on system type
    ///
    /// Quarter systems scale complexity by 2/3 compared to semester systems
    #[must_use]
    pub fn complexity_scale_factor(&self) -> f64 {
        if self.is_quarter_system() {
            2.0 / 3.0
        } else {
            1.0
        }
    }

    /// Get a unique identifier for this degree
    ///
    /// Returns the explicit id if set, otherwise generates from type + name
    #[must_use]
    pub fn degree_id(&self) -> String {
        self.id.as_ref().map_or_else(
            || format!("{} {}", self.degree_type, self.name),
            String::clone,
        )
    }

    /// Get a unique identifier for this degree (legacy method name)
    ///
    /// # Returns
    /// A string combining name and type (e.g., "BS Computer Science")
    #[must_use]
    #[deprecated(note = "Use degree_id() instead for clearer naming")]
    pub fn id(&self) -> String {
        self.degree_id()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_degree_creation() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        assert_eq!(degree.name, "Computer Science");
        assert_eq!(degree.degree_type, "BS");
        assert_eq!(degree.cip_code, Some("11.0701".to_string()));
        assert_eq!(degree.system_type, "semester");
    }

    #[test]
    fn test_degree_id() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        assert_eq!(degree.degree_id(), "BS Computer Science");
    }

    #[test]
    fn test_different_degree_types() {
        let bs = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        let ba = Degree::new(
            "Computer Science".to_string(),
            "BA".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        assert_ne!(bs, ba);
        assert_ne!(bs.degree_id(), ba.degree_id());
    }

    #[test]
    fn test_custom_degree_type() {
        let degree = Degree::new(
            "Data Science".to_string(),
            "Master of Science".to_string(),
            Some("30.7001".to_string()),
            "semester".to_string(),
        );

        assert_eq!(degree.degree_type, "Master of Science");
        assert_eq!(degree.degree_id(), "Master of Science Data Science");
    }

    #[test]
    fn test_quarter_system() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "quarter".to_string(),
        );

        assert!(degree.is_quarter_system());
        assert!((degree.complexity_scale_factor() - 2.0 / 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_semester_system() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        assert!(!degree.is_quarter_system());
        assert!((degree.complexity_scale_factor() - 1.0).abs() < f64::EPSILON);
    }
}
