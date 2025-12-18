//! WASM library entry point for NuAnalytics
//! This module exports functionality to JavaScript/TypeScript

mod rs;

// Re-export WASM bindings
pub use rs::bindings::*;
