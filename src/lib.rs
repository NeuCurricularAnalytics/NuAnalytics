//! Core library for `NuAnalytics`
//! Core functionality used by the CLI and other components

pub mod core;
pub mod logger;

#[cfg(feature = "mcp")]
pub mod mcp;

#[cfg(feature = "database")]
pub use core::database;

pub use core::*;
