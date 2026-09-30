//! MCP tool implementations
//!
//! This module contains the implementations of MCP tools exposed by the server.

pub mod analyze;
pub mod audit;
pub mod convert;
pub mod course_detail;
pub mod json_schema;
pub mod match_courses;
pub mod plan_graph;
pub mod reference;
pub mod report;
pub mod samples;
pub mod schema;
pub mod shared;
pub mod trim;
pub mod validate;

// The database tools' engines live in `crate::core::query`, shared with the CLI. These two
// stay here: `compare_degrees` computes fresh metrics with the analysis in this module, and
// `import_degree` parses MCP-shaped input.
pub mod degrees;
pub mod import;

// Re-export tool types for convenience
pub use analyze::AnalyzeDegreeRequest;
pub use audit::AuditDegreeRequest;
pub use convert::ConvertDegreeRequest;
pub use course_detail::GetCourseDetailRequest;
pub use degrees::CompareDegreesRequest;
pub use import::ImportDegreeRequest;
pub use match_courses::FindCoursesMatchingRequest;
pub use plan_graph::RenderPlanGraphRequest;
pub use reference::GetReferenceRequest;
pub use report::RenderDegreeReportRequest;
pub use samples::ListSampleDegreesRequest;
pub use trim::{TrimDegreeRequest, TrimReportInfo, TrimResponse};
pub use validate::{
    DegreeContext, ValidateDegreeRequest, ValidationErrorInfo, ValidationResponse,
    ValidationWarningInfo,
};
