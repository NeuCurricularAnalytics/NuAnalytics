//! Core module for common functionality across all targets

pub mod config;
pub mod metrics;
pub mod metrics_export;
pub mod models;
pub mod planner;

// Add core domain modules here as they're developed:
// pub mod degree;
// pub mod school;
// pub mod database;
// pub mod utils;

/// Returns the current version of the `NuAnalytics` crate
#[must_use]
pub const fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// The `logger` module moved to standalone crate; use `logger` directly.
