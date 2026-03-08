//! Data models for `NuAnalytics`

pub mod course;
pub mod course_graph;
pub mod dag;
pub mod degree;
pub mod degree_program;
pub mod plan;
pub mod school;
pub mod validation;

pub use course::Course;
pub use course_graph::{
    CourseGraph, CourseGraphResult, CourseNode, PrerequisiteEdge, PrerequisiteType,
};
pub use dag::DAG;
pub use degree::Degree;
pub use degree_program::DegreeProgram;
pub use plan::Plan;
pub use school::School;
pub use validation::{ValidationError, ValidationResult, ValidationWarning};
