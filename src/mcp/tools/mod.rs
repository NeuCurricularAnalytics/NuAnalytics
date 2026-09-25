//! MCP tool implementations
//!
//! This module contains the implementations of MCP tools exposed by the server.

pub mod analyze;
pub mod audit;
pub mod cache;
pub mod convert;
pub mod course_detail;
pub mod json_schema;
pub mod match_courses;
pub mod pipeline;
pub mod plan_graph;
pub mod report;
pub mod samples;
pub mod schema;
pub mod shared;
pub mod trim;
pub mod validate;
pub mod visualize;

// Database-backed tools (require feature = "database")
//
// `cip_codes`, `institutions` and `lookup` moved to `crate::core::query` so the CLI can
// call the same engines without the `mcp` feature. Re-exported here under their old
// names: the `#[tool]` handlers in `server.rs` keep one import site and do not change.
#[cfg(feature = "database")]
pub use crate::core::query::{cip_codes, completions, institutions, lookup};

#[cfg(feature = "database")]
pub mod degrees;
#[cfg(feature = "database")]
pub mod import;
#[cfg(feature = "database")]
pub mod scaffold;

// Re-export tool types for convenience
pub use analyze::AnalyzeDegreeRequest;
pub use audit::AuditDegreeRequest;
pub use cache::CacheYamlRequest;
pub use convert::ConvertDegreeRequest;
pub use course_detail::GetCourseDetailRequest;
pub use json_schema::GetDegreeJsonSchemaRequest;
pub use match_courses::FindCoursesMatchingRequest;
pub use pipeline::DegreePipelineRequest;
pub use plan_graph::RenderPlanGraphRequest;
pub use report::GenerateDegreeReportRequest;
pub use samples::ListSampleDegreesRequest;
pub use schema::GetSchemaRequest;
pub use trim::{TrimDegreeRequest, TrimReportInfo, TrimResponse};
pub use validate::{
    DegreeContext, ValidateDegreeRequest, ValidationErrorInfo, ValidationResponse,
    ValidationWarningInfo,
};
pub use visualize::GetCurriculumVisualizationRequest;

// Re-export database tool types
#[cfg(feature = "database")]
pub use crate::core::query::cip_codes::SearchCipCodesRequest;
#[cfg(feature = "database")]
pub use crate::core::query::completions::{
    CompletionDemographicsRequest, GetInstitutionCompletionsRequest,
    GetSchoolsCompletionDemographicsRequest,
};
#[cfg(feature = "database")]
pub use crate::core::query::degrees::{
    CompareDegreesRequest, GetDegreeRequest, SearchDegreesRequest, StoreDegreeRequest,
};
#[cfg(feature = "database")]
pub use crate::core::query::institutions::{GetInstitutionRequest, SearchInstitutionsRequest};
#[cfg(feature = "database")]
pub use crate::core::query::lookup::GetLookupCodesRequest;
#[cfg(feature = "database")]
pub use import::ImportDegreeRequest;
#[cfg(feature = "database")]
pub use scaffold::ScaffoldDegreeYamlRequest;
