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
}

impl Degree {
    /// Create a new degree
    ///
    /// # Arguments
    /// * `name` - Degree name
    /// * `degree_type` - Degree type (BS, BA, etc.)
    /// * `cip_code` - CIP code
    #[must_use]
    pub const fn new(name: String, degree_type: String, cip_code: String) -> Self {
        Self {
            name,
            degree_type,
            cip_code,
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
        );

        assert_eq!(degree.name, "Computer Science");
        assert_eq!(degree.degree_type, "BS");
        assert_eq!(degree.cip_code, "11.0701");
    }

    #[test]
    fn test_degree_id() {
        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            "11.0701".to_string(),
        );

        assert_eq!(degree.id(), "BS Computer Science");
    }

    #[test]
    fn test_different_degree_types() {
        let bs = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            "11.0701".to_string(),
        );

        let ba = Degree::new(
            "Computer Science".to_string(),
            "BA".to_string(),
            "11.0701".to_string(),
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
        );

        assert_eq!(degree.degree_type, "Master of Science");
        assert_eq!(degree.id(), "Master of Science Data Science");
    }
}
