//! Degree module for loading and representing complete degree programs
//!
//! This module provides:
//! - YAML degree schema structures (`models`)
//! - YAML parsing functions (`yaml_parser`)
//! - Direct conversion to unified `Degree` and `Course` models for metrics/plans
//! - Plan generation for degree analysis
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

pub mod course_reference;
pub mod plan_generator;
pub mod plan_variant;
pub mod requirement_resolver;
pub mod validation;
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

pub use yaml_parser::{
    load_degree_from_yaml, parse_degree_yaml, save_degree_to_yaml, serialize_degree_yaml,
    DegreeParseError,
};

// Re-export course reference types
pub use course_reference::CourseReference;

// Re-export validation types
pub use validation::{
    validate_degree_program, ValidationError, ValidationResult, ValidationWarning,
};

// Re-export plan generation types
pub use plan_generator::{PlanGenerationStats, PlanGenerator, PlanGeneratorConfig};
pub use plan_variant::PlanVariant;
pub use requirement_resolver::{RequirementResolver, ResolvedRequirement};
