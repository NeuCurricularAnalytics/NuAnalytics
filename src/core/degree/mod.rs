//! Degree module for loading and representing complete degree programs
//!
//! This module provides:
//! - YAML degree schema structures (`models`)
//! - YAML parsing functions (`yaml_parser`)
//! - Direct conversion to unified `Degree` and `Course` models for metrics/plans
//!
//! # Example
//! ```no_run
//! use nu_analytics::core::degree::{load_degree_from_yaml, DegreeProgram};
//!
//! // Load from file
//! let program = load_degree_from_yaml("path/to/degree.yaml").unwrap();
//!
//! // Access metadata and process courses
//! println!("Program: {}", program.degree.name);
//! for (key, course) in program.courses {
//!     println!("{}: {}", key, course.name);
//! }
//! ```

pub mod yaml_parser;

// Re-export degree structure
pub use crate::core::models::degree::Degree;

// Re-export requirement-related types from unified models
pub use crate::core::models::degree::{
    CourseGroup, FromClause, Requirement, RequirementConstraints, RequirementOption,
    RequirementType,
};

// Re-export course credit range
pub use crate::core::models::course::CreditRange;

// Re-export the DegreeProgram structure (top-level degree container)
pub use crate::core::models::DegreeProgram;

pub use yaml_parser::{load_degree_from_yaml, parse_degree_yaml, DegreeParseError};
