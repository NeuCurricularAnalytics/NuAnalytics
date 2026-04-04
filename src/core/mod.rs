//! Core module for common functionality across all targets

pub mod config;
pub mod degree;
pub mod metrics;
pub mod metrics_export;
pub mod models;
pub mod planner;
pub mod prerequisite_parser;
pub mod report;
pub mod statistics;

// Re-export degree module types for convenience
pub use degree::{
    load_degree_from_yaml, parse_degree_yaml, save_degree_to_yaml, serialize_degree_yaml,
    validate_degree_program, DegreeParseError, DegreeProgram, ValidationError, ValidationResult,
    ValidationWarning,
};

// Re-export statistics types for convenience
pub use statistics::{CalculationStrategy, DescriptiveStats, MeanStrategy, MedianStrategy};

// Database integration (feature-gated)
#[cfg(feature = "database")]
pub mod database;

/// Returns the current version of the `NuAnalytics` crate
#[must_use]
pub const fn get_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// The `logger` module moved to standalone crate; use `logger` directly.
