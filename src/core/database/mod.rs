//! Database integration module
//!
//! Provides Supabase connectivity, data models, and IPEDS ingestion.
//!
//! ## Configuration
//!
//! Set the following in your `nuanalytics.toml` or `~/.config/nuanalytics/config.toml`:
//!
//! ```toml
//! [database]
//! endpoint = "https://your-project.supabase.co"
//! anon_key = "your-anon-key"
//! enabled  = true
//! ```
//!
//! ## IPEDS Import
//!
//! Download files from <https://nces.ed.gov/ipeds/use-the-data>, then run:
//!
//! ```sh
//! nuanalytics db ipeds-import --year 2023 --dir ./ipeds_data/
//! ```

pub mod auth;
pub mod client;
pub mod error;
pub mod ipeds;
pub mod models;
pub mod query;

/// Supabase table name constants — use these instead of raw string literals.
pub mod tables {
    /// IPEDS institution directory
    pub const INSTITUTIONS: &str = "institutions";
    /// IPEDS degree completions (filtered to CS CIP codes)
    pub const COMPLETIONS: &str = "completions";
    /// Stored degree program YAML definitions
    pub const DEGREES: &str = "degrees";
    /// Pre-aggregated completion totals per institution/award-level/year (denomination cache)
    pub const INSTITUTION_COMPLETION_TOTALS: &str = "institution_completion_totals";
    /// CIP code taxonomy lookup
    pub const CIP_CODES: &str = "cip_codes";
}

pub use crate::core::config::DatabaseConfig;
pub use auth::{auth_file_path, clear_auth_state, load_auth_state, save_auth_state, AuthState};
pub use client::DbClient;
pub use error::{DatabaseError, DatabaseResult};
pub use models::{
    CipCode, Completion, DemographicRepresentation, Institution, InstitutionCompletionTotal,
    StoredDegree,
};
pub use query::QueryFilters;
