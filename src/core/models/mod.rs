//! Data models for `NuAnalytics`

pub mod course;
pub mod course_graph;
pub mod dag;
pub mod degree;
pub mod degree_program;
pub mod plan;
pub mod school;

pub use course::Course;
pub use course_graph::{
    CourseGraph, CourseGraphResult, CourseNode, PrerequisiteEdge, PrerequisiteType,
};
pub use dag::DAG;
pub use degree::Degree;
pub use degree_program::DegreeProgram;
pub use plan::Plan;
pub use school::School;

// Re-export validation types from degree module to maintain backward compatibility
pub use crate::core::degree::validation::{ValidationError, ValidationResult, ValidationWarning};
