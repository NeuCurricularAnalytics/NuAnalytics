//! Core library for `NuAnalytics`
//! Core functionality used by the CLI and other components

pub mod core;

pub use core::*;
// No logger re-exports: use the standalone `logger` crate directly.
