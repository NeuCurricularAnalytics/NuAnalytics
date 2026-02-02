//! Degree module for loading and representing complete degree programs
//!
//! This module provides:
//! - YAML degree schema structures (`models`)
//! - YAML parsing functions (`yaml_parser`)
//! - Conversion to unified `Degree` and `Course` models via `into_degree()` and `into_course()`
//!
//! # Example
//! ```no_run
//! use nu_analytics::core::degree::{load_degree_from_yaml, parse_degree_yaml};
//!
//! // Load from file
//! let degree_program = load_degree_from_yaml("path/to/degree.yaml").unwrap();
//!
//! // Convert to unified Degree model for metrics/plans
//! let degree = degree_program.degree.into_degree();
//!
//! // Convert courses to unified Course model
//! for (key, yaml_course) in degree_program.courses {
//!     let course = yaml_course.into_course();
//!     println!("{}: {}", course.key(), course.name);
//! }
//!
//! // Or parse from string (e.g., from network/database)
//! let yaml_content = std::fs::read_to_string("path/to/degree.yaml").unwrap();
//! let degree = parse_degree_yaml(&yaml_content).unwrap();
//! ```

pub mod models;
pub mod yaml_parser;

// Re-export commonly used types
pub use models::{
    CourseGroup, CreditRange, DegreeMeta, FromClause, Requirement, RequirementConstraints,
    RequirementOption, RequirementType, YamlDegree,
};

// Type alias for clearer naming
pub use models::YamlDegree as DegreeProgram;

pub use yaml_parser::{load_degree_from_yaml, parse_degree_yaml, DegreeParseError};
