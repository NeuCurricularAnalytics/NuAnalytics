//! Shared library for `NuAnalytics`
//! Contains core functionality used across CLI, native, and WASM targets

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub mod shared;

pub use shared::*;
