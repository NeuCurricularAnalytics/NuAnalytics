//! YAML degree data models and structures
//!
//! This module defines the data structures for representing complete degree programs.
//! It uses the unified `Degree` and `Course` types from `models`, extended with
//! requirement structures specific to degree definitions.
//!
//! Key design notes:
//! - Each degree has complete metadata (institution, program name, requirements, courses)
//! - Courses section contains full course definitions with prerequisites as raw strings
//! - Requirements section uses a nested tree structure supporting `all`/`select`/`one_of` types
//! - Prerequisites are stored as raw strings for deferred parsing (stage 5)
//! - Course keys are institution-scoped (subject + number, e.g., ICS111)
//! - `DegreeMeta.into_degree()` converts to unified `Degree` model
//! - `YamlCourse.into_course()` converts to unified `Course` model

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export CreditRange from Course model for convenience
pub use crate::core::models::course::CreditRange;

/// Represents a complete degree program loaded from YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlDegree {
    /// Degree metadata section
    pub degree: DegreeMeta,

    /// Requirements section mapping requirement ID to requirement definition
    pub requirements: HashMap<String, Requirement>,

    /// Courses section mapping course key (e.g., "ICS111") to course definition
    pub courses: HashMap<String, YamlCourse>,
}

/// Degree metadata extracted from YAML
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeMeta {
    /// Unique identifier for this degree (e.g., "uhm-ics-bscs-general")
    pub id: String,

    /// Institution name
    pub institution: String,

    /// Full degree program name (e.g., "Bachelor of Science in Computer Science - General Track")
    pub program: String,

    /// Catalog year (e.g., "2024-2025")
    pub catalog_year: String,

    /// Source URL for the official catalog
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_url: Option<String>,

    // Credit requirements
    /// Total credits required for graduation
    pub total_credits: u32,

    /// Minimum upper-division (300+) credits required
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upper_division_credits: Option<u32>,

    /// Minimum credits within major subjects
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_major_credits: Option<u32>,

    // GPA requirements
    /// Minimum overall GPA required
    pub gpa_minimum: f32,

    /// Minimum GPA for major courses (if different from overall)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gpa_major: Option<f32>,

    // Grade requirements
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
    pub allow_double_counting: bool,
}

impl DegreeMeta {
    /// Convert to the unified `Degree` model
    ///
    /// This allows degree metadata from YAML to be used with existing
    /// metrics and plan infrastructure.
    #[must_use]
    pub fn into_degree(self) -> crate::core::models::Degree {
        crate::core::models::Degree {
            name: self.program.clone(),
            degree_type: String::new(), // Could be parsed from program name
            cip_code: String::new(),
            system_type: "semester".to_string(), // Default, could be added to YAML
            id: Some(self.id),
            institution: Some(self.institution),
            catalog_year: Some(self.catalog_year),
            source_url: self.source_url,
            total_credits: Some(self.total_credits),
            upper_division_credits: self.upper_division_credits,
            in_major_credits: self.in_major_credits,
            gpa_minimum: Some(self.gpa_minimum),
            gpa_major: self.gpa_major,
            grade_minimum: self.grade_minimum,
            grade_minimum_note: self.grade_minimum_note,
            major_subjects: self.major_subjects,
            allow_double_counting: Some(self.allow_double_counting),
        }
    }

    /// Convert to the unified `Degree` model (borrowing version)
    #[must_use]
    pub fn to_degree(&self) -> crate::core::models::Degree {
        self.clone().into_degree()
    }
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

/// A course definition in the courses section (internal YAML deserialization target)
///
/// This struct is used internally to deserialize YAML course definitions.
/// It should not be used directly in public APIs. Instead, convert to the unified
/// `Course` model via `into_course()` for use with metrics and DAG building.
///
/// **Note**: This type is not re-exported in the public module API to encourage
/// using the unified `Course` model instead.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YamlCourse {
    /// Course subject/prefix (e.g., "ICS", "MATH", "CS")
    pub subject: String,

    /// Course number (e.g., "111", "241", "301")
    pub number: String,

    /// Course title (e.g., "Introduction to Computer Science I")
    pub title: String,

    /// Credit hours (can be whole number or decimal), optional if `credit_range` is set
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credits: Option<f32>,

    /// Prerequisites as a raw string (e.g., "ICS111" or "(ICS311 | ECE367) & ICS314")
    /// Will be parsed in stage 5
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prerequisites: Option<String>,

    /// Co-requisites as raw string or list
    #[serde(skip_serializing_if = "Option::is_none")]
    pub corequisites: Option<Vec<String>>,

    /// Terms typically offered (e.g., `["fall", "spring"]`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub typically_offered: Option<Vec<String>>,

    /// General education attributes (e.g., `["FW", "DP"]`)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gen_ed_attributes: Option<Vec<String>>,

    /// Cross-listed as other courses
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cross_listed_as: Option<Vec<String>>,

    /// Whether the course can be repeated
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeatable: Option<bool>,

    /// Maximum credits that can be earned if repeatable
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_repeat_credits: Option<f32>,

    /// Variable credit range
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credit_range: Option<CreditRange>,
}

impl YamlCourse {
    /// Get the full course key (subject + number, e.g., "ICS111")
    /// Internal helper for YAML deserialization.
    #[must_use]
    pub fn course_key(&self) -> String {
        format!("{}{}", self.subject, self.number)
    }

    /// Get actual credits; if `credit_range` is set, use min; otherwise use credits
    /// Internal helper for YAML deserialization.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn actual_credits(&self) -> f32 {
        self.credit_range
            .as_ref()
            .map_or_else(|| self.credits.unwrap_or(0.0), |range| range.min as f32)
    }

    /// Convert to the unified `Course` model
    ///
    /// This allows YAML course definitions to be used with existing
    /// metrics, DAG building, and plan infrastructure.
    ///
    /// Note: `prerequisites` will be empty since the raw string needs
    /// to be parsed separately (stage 5). Use `prerequisites_raw` in
    /// the resulting `Course` to access the original expression.
    #[must_use]
    pub fn into_course(self) -> crate::core::models::Course {
        crate::core::models::Course {
            csv_id: None,
            id: None,
            name: self.title,
            prefix: self.subject,
            number: self.number,
            prerequisites: Vec::new(), // Will be populated after parsing prerequisites_raw
            corequisites: self.corequisites.unwrap_or_default(),
            strict_corequisites: Vec::new(),
            credit_hours: self.credits.unwrap_or(0.0),
            canonical_name: None,
            prerequisites_raw: self.prerequisites,
            typically_offered: self.typically_offered,
            gen_ed_attributes: self.gen_ed_attributes,
            cross_listed_as: self.cross_listed_as,
            repeatable: self.repeatable,
            max_repeat_credits: self.max_repeat_credits,
            credit_range: self.credit_range,
        }
    }

    /// Convert to the unified `Course` model (borrowing version)
    #[must_use]
    pub fn to_course(&self) -> crate::core::models::Course {
        self.clone().into_course()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_yaml_course_key() {
        let course = YamlCourse {
            subject: "ICS".to_string(),
            number: "111".to_string(),
            title: "Introduction to Computer Science I".to_string(),
            credits: Some(4.0),
            prerequisites: None,
            corequisites: None,
            typically_offered: None,
            gen_ed_attributes: None,
            cross_listed_as: None,
            repeatable: None,
            max_repeat_credits: None,
            credit_range: None,
        };

        assert_eq!(course.course_key(), "ICS111");
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(course.actual_credits(), 4.0);
        }
    }

    #[test]
    fn test_yaml_course_with_credit_range() {
        let course = YamlCourse {
            subject: "ICS".to_string(),
            number: "499".to_string(),
            title: "Directed Reading".to_string(),
            credits: None, // ignored when credit_range is set
            prerequisites: None,
            corequisites: None,
            typically_offered: None,
            gen_ed_attributes: None,
            cross_listed_as: None,
            repeatable: Some(true),
            max_repeat_credits: Some(6.0),
            credit_range: Some(CreditRange { min: 1, max: 3 }),
        };

        assert_eq!(course.course_key(), "ICS499");
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(course.actual_credits(), 1.0);
        }
    }

    #[test]
    fn test_yaml_course_to_unified_course() {
        let yaml_course = YamlCourse {
            subject: "ICS".to_string(),
            number: "311".to_string(),
            title: "Algorithms".to_string(),
            credits: Some(4.0),
            prerequisites: Some("ICS211 & ICS241".to_string()),
            corequisites: None,
            typically_offered: Some(vec!["fall".to_string(), "spring".to_string()]),
            gen_ed_attributes: None,
            cross_listed_as: None,
            repeatable: None,
            max_repeat_credits: None,
            credit_range: None,
        };

        let course = yaml_course.into_course();
        assert_eq!(course.key(), "ICS311");
        assert_eq!(course.name, "Algorithms");
        #[allow(clippy::float_cmp)]
        {
            assert_eq!(course.credit_hours, 4.0);
        }
        assert_eq!(
            course.prerequisites_raw,
            Some("ICS211 & ICS241".to_string())
        );
        assert!(course.typically_offered.is_some());
        // Prerequisites vec is empty - needs separate parsing
        assert!(course.prerequisites.is_empty());
    }

    #[test]
    fn test_degree_meta_to_unified_degree() {
        let meta = DegreeMeta {
            id: "test-degree".to_string(),
            institution: "Test University".to_string(),
            program: "BS Computer Science".to_string(),
            catalog_year: "2024-2025".to_string(),
            source_url: Some("https://example.com".to_string()),
            total_credits: 120,
            upper_division_credits: Some(45),
            in_major_credits: None,
            gpa_minimum: 2.0,
            gpa_major: None,
            grade_minimum: Some("C".to_string()),
            grade_minimum_note: None,
            major_subjects: Some(vec!["CS".to_string()]),
            allow_double_counting: false,
        };

        let degree = meta.into_degree();
        assert_eq!(degree.degree_id(), "test-degree");
        assert_eq!(degree.institution, Some("Test University".to_string()));
        assert_eq!(degree.total_credits, Some(120));
        assert_eq!(degree.gpa_minimum, Some(2.0));
        assert_eq!(degree.allow_double_counting, Some(false));
    }
}
