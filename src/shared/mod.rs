//! Shared module for common functionality across all targets

// Add shared modules here
// pub mod models;
// pub mod services;
// pub mod utils;

/// Returns the current version of the `NuAnalytics` crate
#[must_use]
pub const fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}
