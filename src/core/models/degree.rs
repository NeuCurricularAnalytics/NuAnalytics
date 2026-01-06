//! Degree model

use serde::{Deserialize, Serialize};

/// Represents a degree program
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Degree {
    /// Degree name (e.g., "Computer Science")
    pub name: String,

    /// Degree type (e.g., "BS", "BA", "MS")
    pub degree_type: String,

    /// CIP code (Classification of Instructional Programs)
    pub cip_code: String,

    /// System type ("semester" or "quarter")
    pub system_type: String,
}

impl Degree {
    /// Create a new degree
    ///
    /// # Arguments
    /// * `name` - Degree name
    /// * `degree_type` - Degree type (BS, BA, etc.)
    /// * `cip_code` - CIP code
    /// * `system_type` - System type ("semester" or "quarter")
    #[must_use]
    pub const fn new(
        name: String,
        degree_type: String,
        cip_code: String,
        system_type: String,
    ) -> Self {
        Self {
            name,
            degree_type,
            cip_code,
            system_type,
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
    /// # Returns
    /// A string combining name and type (e.g., "BS Computer Science")
    #[must_use]
    pub fn id(&self) -> String {
        format!("{} {}", self.degree_type, self.name)
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

        assert_eq!(degree.id(), "BS Computer Science");
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
        assert_ne!(bs.id(), ba.id());
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
        assert_eq!(degree.id(), "Master of Science Data Science");
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
