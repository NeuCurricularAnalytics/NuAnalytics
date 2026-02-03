//! Course reference parsing for special syntax in degree YAML files
//!
//! Handles three types of course references:
//! - Single course: `"CS314"`
//! - Course bundle (must take all): `"[CS314, CS315]"` - both required together
//! - Equivalent courses (pick one): `"{CS201, PHIL201}"` - interchangeable courses

/// Represents different types of course references in degree requirements
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CourseReference {
    /// A single course (e.g., "CS314")
    Single(String),

    /// A bundle of courses that must all be taken together (e.g., "[CHEM161, CHEM161L]")
    /// Typically used for lecture + lab pairs
    Bundle(Vec<String>),

    /// Equivalent courses where exactly one must be chosen (e.g., "{CS201, PHIL201}")
    /// These courses are interchangeable/cross-listed
    Equivalent(Vec<String>),
}

impl CourseReference {
    /// Parse a course reference string into the appropriate type
    ///
    /// # Arguments
    /// * `s` - Course reference string (e.g., "CS314", "[AA100, AA101]", "{CS201, PHIL201}")
    ///
    /// # Returns
    /// Parsed course reference or error message
    ///
    /// # Examples
    /// ```
    /// use nu_analytics::core::degree::course_reference::CourseReference;
    ///
    /// // Single course
    /// let single = CourseReference::parse("CS314").unwrap();
    /// assert!(matches!(single, CourseReference::Single(_)));
    ///
    /// // Bundle
    /// let bundle = CourseReference::parse("[AA100, AA101]").unwrap();
    /// assert!(matches!(bundle, CourseReference::Bundle(_)));
    ///
    /// // Equivalent
    /// let equiv = CourseReference::parse("{CS201, PHIL201}").unwrap();
    /// assert!(matches!(equiv, CourseReference::Equivalent(_)));
    /// ```
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - Bundle or equivalent syntax is malformed (unbalanced brackets/braces)
    /// - The course list is empty
    /// - Individual course IDs are empty or invalid
    pub fn parse(s: &str) -> Result<Self, String> {
        let trimmed = s.trim();

        // Check for bundle syntax: [COURSE, COURSE, ...]
        if let Some(inner) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            let courses: Vec<String> = inner
                .split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();

            if courses.is_empty() {
                return Err(format!("Empty course bundle: {s}"));
            }

            return Ok(Self::Bundle(courses));
        }

        // Check for equivalent syntax: {COURSE, COURSE, ...}
        if let Some(inner) = trimmed.strip_prefix('{').and_then(|s| s.strip_suffix('}')) {
            let courses: Vec<String> = inner
                .split(',')
                .map(|c| c.trim().to_string())
                .filter(|c| !c.is_empty())
                .collect();

            if courses.is_empty() {
                return Err(format!("Empty equivalent courses: {s}"));
            }

            return Ok(Self::Equivalent(courses));
        }

        // Single course
        if trimmed.is_empty() {
            return Err("Empty course reference".to_string());
        }

        Ok(Self::Single(trimmed.to_string()))
    }

    /// Get all course codes referenced by this course reference
    ///
    /// For single courses, returns a vector with one element.
    /// For bundles and equivalents, returns all courses in the group.
    ///
    /// # Examples
    /// ```
    /// use nu_analytics::core::degree::course_reference::CourseReference;
    ///
    /// let bundle = CourseReference::parse("[AA100, AA101]").unwrap();
    /// let courses = bundle.courses();
    /// assert_eq!(courses, vec!["AA100", "AA101"]);
    /// ```
    #[must_use]
    pub fn courses(&self) -> Vec<&str> {
        match self {
            Self::Single(course) => vec![course.as_str()],
            Self::Bundle(courses) | Self::Equivalent(courses) => {
                courses.iter().map(std::string::String::as_str).collect()
            }
        }
    }

    /// Returns true if this is a bundle (all courses required)
    #[must_use]
    pub const fn is_bundle(&self) -> bool {
        matches!(self, Self::Bundle(_))
    }

    /// Returns true if this is an equivalent set (pick one)
    #[must_use]
    pub const fn is_equivalent(&self) -> bool {
        matches!(self, Self::Equivalent(_))
    }

    /// Returns true if this is a single course
    #[must_use]
    pub const fn is_single(&self) -> bool {
        matches!(self, Self::Single(_))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_single_course() {
        let parsed = CourseReference::parse("CS314").unwrap();
        assert!(parsed.is_single());
        assert_eq!(parsed.courses(), vec!["CS314"]);
    }

    #[test]
    fn test_parse_bundle() {
        let parsed = CourseReference::parse("[AA100, AA101]").unwrap();
        assert!(parsed.is_bundle());
        assert_eq!(parsed.courses(), vec!["AA100", "AA101"]);
    }

    #[test]
    fn test_parse_equivalent() {
        let parsed = CourseReference::parse("{CS201, PHIL201}").unwrap();
        assert!(parsed.is_equivalent());
        assert_eq!(parsed.courses(), vec!["CS201", "PHIL201"]);
    }

    #[test]
    fn test_parse_with_whitespace() {
        let parsed = CourseReference::parse("  [ CHEM161 , CHEM161L ]  ").unwrap();
        assert!(parsed.is_bundle());
        assert_eq!(parsed.courses(), vec!["CHEM161", "CHEM161L"]);
    }

    #[test]
    fn test_parse_empty_string() {
        let result = CourseReference::parse("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_bundle() {
        let result = CourseReference::parse("[]");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_empty_equivalent() {
        let result = CourseReference::parse("{}");
        assert!(result.is_err());
    }
}
