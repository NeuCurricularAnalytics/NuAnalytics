//! MCP tool implementations
//!
//! This module contains the implementations of MCP tools exposed by the server.

pub mod analyze;
pub mod audit;
pub mod schema;
pub mod shared;
pub mod validate;

// Database-backed tools (require feature = "database")
#[cfg(feature = "database")]
pub mod cip_codes;
#[cfg(feature = "database")]
pub mod completions;
#[cfg(feature = "database")]
pub mod degrees;
#[cfg(feature = "database")]
pub mod institutions;
#[cfg(feature = "database")]
pub mod lookup;

// Re-export tool types for convenience
pub use analyze::AnalyzeDegreeRequest;
pub use audit::AuditDegreeRequest;
pub use schema::GetSchemaRequest;
pub use validate::{
    DegreeContext, ValidateDegreeRequest, ValidationErrorInfo, ValidationResponse,
    ValidationWarningInfo,
};

// Re-export database tool types
#[cfg(feature = "database")]
pub use cip_codes::SearchCipCodesRequest;
#[cfg(feature = "database")]
pub use completions::{
    CompletionDemographicsRequest, GetInstitutionCompletionsRequest,
    GetSchoolsCompletionDemographicsRequest,
};
#[cfg(feature = "database")]
pub use degrees::{
    CompareDegreesRequest, GetDegreeRequest, SearchDegreesRequest, StoreDegreeRequest,
};
#[cfg(feature = "database")]
pub use institutions::{GetInstitutionRequest, SearchInstitutionsRequest};
#[cfg(feature = "database")]
pub use lookup::GetLookupCodesRequest;
