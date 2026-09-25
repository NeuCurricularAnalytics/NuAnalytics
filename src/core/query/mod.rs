//! Read-only query engines over the database tables.
//!
//! Protocol-agnostic: each engine exposes
//! `pub async fn execute_*_json(&Arc<DbClient>, XRequest) -> String` returning
//! pretty-printed JSON, so the MCP tool router and `nuanalytics db query` call the same
//! function rather than keeping two implementations that drift.
//!
//! The request structs derive `schemars::JsonSchema` so `rmcp`'s `#[tool]` router can use
//! them directly as `Parameters<T>`. Nothing here depends on `rmcp` or on the `mcp`
//! feature — `src/core/` must stay free of both, or `--features database` alone stops
//! building. The CI matrix covers exactly that.

pub mod cip_codes;
pub mod completions;
pub mod degrees;
pub mod institutions;
pub mod lookup;

// Request types only. The `execute_*` functions are deliberately not re-exported:
// `execute_json` collides across modules, and `institutions::execute_search_json` reads
// better at the call site than a flattened name.
pub use cip_codes::SearchCipCodesRequest;
pub use completions::{
    CompletionDemographicsRequest, GetInstitutionCompletionsRequest,
    GetSchoolsCompletionDemographicsRequest,
};
pub use degrees::{
    CompareDegreesRequest, CompareMetricsFn, GetDegreeRequest, SearchDegreesRequest,
    StoreDegreeRequest,
};
pub use institutions::{GetInstitutionRequest, SearchInstitutionsRequest};
pub use lookup::GetLookupCodesRequest;
