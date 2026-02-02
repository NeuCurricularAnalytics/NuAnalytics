//! Degree model

use serde::{Deserialize, Serialize};

/// Represents a degree program
///
/// This struct supports both CSV plan loading (basic fields) and
/// full YAML degree loading (extended fields). Optional fields
/// are populated when loading from YAML degree definitions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Degree {
    // === Core fields (required, used by CSV and YAML) ===
    /// Degree name / program name (e.g., "Computer Science" or full "Bachelor of Science in Computer Science")
    pub name: String,

    /// Degree type (e.g., "BS", "BA", "MS")
    pub degree_type: String,

    /// CIP code (Classification of Instructional Programs)
    #[serde(default)]
    pub cip_code: String,

    /// System type ("semester" or "quarter")
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

impl Degree {
    /// Create a new degree with basic fields (for CSV loading)
    ///
    /// # Arguments
    /// * `name` - Degree name
    /// * `degree_type` - Degree type (BS, BA, etc.)
    /// * `cip_code` - CIP code
    /// * `system_type` - System type ("semester" or "quarter")
    #[must_use]
    #[allow(clippy::missing_const_for_fn)]
    pub fn new(name: String, degree_type: String, cip_code: String, system_type: String) -> Self {
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
            cip_code: String::new(),
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
            "11.0701".to_string(),
            "semester".to_string(),
        );

        assert_eq!(degree.name, "Computer Science");
        assert_eq!(degree.degree_type, "BS");
        assert_eq!(degree.cip_code, "11.0701");
        assert_eq!(degree.system_type, "semester");
    }

    #[test]
    fn test_degree_id() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            "11.0701".to_string(),
            "semester".to_string(),
        );

        assert_eq!(degree.degree_id(), "BS Computer Science");
    }

    #[test]
    fn test_different_degree_types() {
        let bs = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            "11.0701".to_string(),
            "semester".to_string(),
        );

        let ba = Degree::new(
            "Computer Science".to_string(),
            "BA".to_string(),
            "11.0701".to_string(),
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
            "30.7001".to_string(),
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
            "11.0701".to_string(),
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
            "11.0701".to_string(),
            "semester".to_string(),
        );

        assert!(!degree.is_quarter_system());
        assert!((degree.complexity_scale_factor() - 1.0).abs() < f64::EPSILON);
    }
}
