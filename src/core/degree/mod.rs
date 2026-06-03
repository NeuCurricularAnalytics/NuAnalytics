//! Degree module for loading and representing complete degree programs
//!
//! This module provides:
//! - YAML degree schema structures (`models`)
//! - YAML parsing functions (`yaml_parser`)
//! - Direct conversion to unified `Degree` and `Course` models for metrics/plans
//! - Plan generation for degree analysis
//! - Plan selection for identifying special plans (shortest, longest, etc.)
//! - Gen-ed tracking for cross-category course sharing
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

pub mod audit;
pub mod course_reference;
pub mod gen_ed_tracker;
pub mod json_parser;
pub mod landscape_convert;
pub mod plan_generator;
pub mod plan_selector;
pub mod plan_validation;
pub mod plan_variant;
pub mod requirement_resolver;
pub mod trim;
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

// JSON input/output (unified format) and ai-landscape conversion
pub use json_parser::{
    load_degree_from_json, parse_degree_auto, parse_degree_json, parse_degree_json_with_warnings,
    save_degree_to_json, serialize_degree_json, to_unified_value, unified_value_to_string,
};
pub use landscape_convert::{
    convert_landscape, convert_landscape_str, extract_cluster_programs, ConversionResult,
    LandscapeProgram,
};

// Re-export course reference types
pub use course_reference::CourseReference;

// Re-export validation types
pub use validation::{
    validate_degree_program, validate_degree_program_with_options, ValidationError,
    ValidationOptions, ValidationResult, ValidationWarning,
};

// Re-export plan generation types
pub use plan_generator::{
    PlanGenerationStats, PlanGenerator, PlanGeneratorConfig, SamplingStrategy,
};
pub use plan_variant::PlanVariant;
pub use requirement_resolver::{RequirementResolver, ResolvedRequirement};

// Re-export plan selection types
pub use plan_selector::{
    PlanCategory, PlanScore, PlanSelector, PlanSelectorConfig, ScoredPlan, SelectedPlans,
};

// Re-export trim types
pub use trim::{trim_program, TrimOptions, TrimReport};

// Re-export plan validation types
pub use plan_validation::{
    PlanValidationError, PlanValidationResult, PlanValidationStats, PlanValidationWarning,
    PlanValidator, PlanValidatorConfig,
};

// Re-export gen-ed tracking types
pub use gen_ed_tracker::{GenEdSummary, GenEdTracker};
