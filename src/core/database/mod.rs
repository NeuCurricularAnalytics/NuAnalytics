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
pub mod bootstrap;
pub mod client;
pub mod doctor;
pub mod error;
#[cfg(feature = "database")]
pub mod filters;
pub mod import;
pub mod ipeds;
pub mod models;
pub mod prune;
pub mod validate;

/// Backend function names, defined in `docs/database/programs-schema.sql`.
pub mod functions {
    /// `query_readonly(q, max_rows)` — one ad-hoc read-only statement.
    pub const QUERY_READONLY: &str = "query_readonly";
    /// `query_readonly_params(q, params, max_rows)` — one read-only statement with `$1`
    /// bound to a JSON object.
    pub const QUERY_READONLY_PARAMS: &str = "query_readonly_params";

    /// How to install or update both — named wherever one turns out to be missing.
    ///
    /// Two routes because a user may have no checkout: `db bootstrap --print` carries the
    /// same file compiled in.
    pub const INSTALL_STEP: &str = "apply docs/database/programs-schema.sql (or pipe \
        `nuanalytics db bootstrap --print` into psql) and reload the schema cache \
        (docs/database/setup.md, step 4c)";
}

/// Error codes the backend reports that this client acts on.
///
/// Postgres SQLSTATEs, plus `PostgREST`'s own `PGRST*` codes.
pub mod codes {
    /// `PostgREST`: no function with this name and these arguments in its schema cache.
    pub const FUNCTION_NOT_FOUND: &str = "PGRST202";
    /// A write inside a read-only transaction.
    pub const READ_ONLY_TRANSACTION: &str = "25006";
    /// Cancelled — `statement_timeout` expired.
    pub const QUERY_CANCELED: &str = "57014";
    /// `undefined_table`.
    pub const UNDEFINED_TABLE: &str = "42P01";
    /// `undefined_column`.
    pub const UNDEFINED_COLUMN: &str = "42703";
    /// `undefined_function` — also a missing operator for these argument types.
    pub const UNDEFINED_FUNCTION: &str = "42883";
    /// `invalid_text_representation` — a cast of a value that is not of that type.
    pub const INVALID_TEXT_REPRESENTATION: &str = "22P02";
    /// `syntax_error`.
    pub const SYNTAX_ERROR: &str = "42601";
}

/// `analysis_runs.variant` labels the tools write and default to.
pub mod variants {
    /// A run of the degree as written; the default everywhere a variant is optional.
    pub const FULL: &str = "full";
    /// A run of the degree with its alternatives trimmed to one entry path.
    pub const TRIMMED: &str = "trimmed";
}

/// Supabase table name constants — use these instead of raw string literals.
pub mod tables {
    /// IPEDS institution directory
    pub const INSTITUTIONS: &str = "institutions";
    /// IPEDS degree completions — every row of the `C_A` file: all CIP codes, both major
    /// numbers, ~313,000 rows per survey year. Not filtered.
    pub const COMPLETIONS: &str = "completions";
    /// Stored degree program YAML definitions
    pub const DEGREES: &str = "degrees";
    /// Pre-aggregated completion totals per institution/award-level/year (denomination cache)
    pub const INSTITUTION_COMPLETION_TOTALS: &str = "institution_completion_totals";
    /// CIP code taxonomy lookup
    pub const CIP_CODES: &str = "cip_codes";
    /// Imported degree programs (normalized; one row per program + lossless `document`)
    pub const PROGRAMS: &str = "programs";
    /// Shared per-institution course catalog
    pub const COURSES: &str = "courses";
    /// M:N junction linking programs to courses (with per-program overrides)
    pub const PROGRAM_COURSES: &str = "program_courses";
    /// Flattened requirement tree per program (addressed by `req_path`)
    pub const PROGRAM_REQUIREMENTS: &str = "program_requirements";
    /// Lookup for normalized `degree_type` codes
    pub const DEGREE_TYPES: &str = "degree_types";
    /// One row per `degree analyze` run of a program (params + variant + degree-level metrics)
    pub const ANALYSIS_RUNS: &str = "analysis_runs";
    /// Per run x course graph metrics (complexity/centrality/delay/blocking)
    pub const ANALYSIS_COURSE_METRICS: &str = "analysis_course_metrics";
    /// Per run x selected exemplar plan (shortest/longest/samples)
    pub const ANALYSIS_PLANS: &str = "analysis_plans";

    /// IPEDS award-level lookup (seeded by `lookup-seed.sql`)
    pub const AWARD_LEVELS: &str = "award_levels";
    /// Carnegie classification lookup
    pub const CARNEGIE_CLASS: &str = "carnegie_class";
    /// Institution control lookup (public / private)
    pub const INSTITUTION_CONTROL: &str = "institution_control";
    /// Institution level lookup (4-year / 2-year / less-than-2-year)
    pub const INSTITUTION_LEVEL: &str = "institution_level";
    /// Institution locale lookup (city / suburb / town / rural)
    pub const INSTITUTION_LOCALE: &str = "institution_locale";
    /// Institution sector lookup
    pub const INSTITUTION_SECTOR: &str = "institution_sector";
    /// Institution size-category lookup
    pub const INSTITUTION_SIZE: &str = "institution_size";

    /// Every table the schema defines, for whole-deployment checks.
    ///
    /// A missing entry here is what distinguishes "the schema was applied" from "the
    /// schema and the seeds were applied" — the seven lookup tables come from
    /// `lookup-seed.sql`, so their absence means a half-finished bootstrap.
    pub const ALL: &[&str] = &[
        ANALYSIS_COURSE_METRICS,
        ANALYSIS_PLANS,
        ANALYSIS_RUNS,
        AWARD_LEVELS,
        CARNEGIE_CLASS,
        CIP_CODES,
        COMPLETIONS,
        COURSES,
        DEGREES,
        DEGREE_TYPES,
        INSTITUTIONS,
        INSTITUTION_COMPLETION_TOTALS,
        INSTITUTION_CONTROL,
        INSTITUTION_LEVEL,
        INSTITUTION_LOCALE,
        INSTITUTION_SECTOR,
        INSTITUTION_SIZE,
        PROGRAMS,
        PROGRAM_COURSES,
        PROGRAM_REQUIREMENTS,
    ];
}

pub use crate::core::config::DatabaseConfig;
pub use auth::{
    auth_file_path, clear_auth_state, load_auth_state, save_auth_state, sign_in_with_password,
    AuthState, SignInError,
};
pub use client::DbClient;
pub use error::{BackendError, DatabaseError, DatabaseResult};
pub use filters::QueryFilters;
pub use models::{
    CipCode, Completion, DemographicRepresentation, Institution, InstitutionCompletionTotal,
    StoredDegree,
};
