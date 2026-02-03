//! Re-exports of degree-related structures from the unified models
//!
//! This module provides convenient access to degree and requirement structures
//! that are defined in the unified models. All structures should be imported
//! from `core::models` or `core::degree::degree_program` rather than duplicated here.

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
pub use super::degree_program::DegreeProgram;
