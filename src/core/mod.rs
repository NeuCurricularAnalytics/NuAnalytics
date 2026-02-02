//! Core module for common functionality across all targets

pub mod config;
pub mod degree;
pub mod metrics;
pub mod metrics_export;
pub mod models;
pub mod planner;
pub mod report;

// Re-export degree module types for convenience
pub use degree::{load_degree_from_yaml, parse_degree_yaml, DegreeParseError, YamlDegree};

// Add core domain modules here as they're developed:
// pub mod school;
// pub mod database;
// pub mod utils;

/// Returns the current version of the `NuAnalytics` crate
#[must_use]
pub const fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// The `logger` module moved to standalone crate; use `logger` directly.
