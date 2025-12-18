//! WASM bindings exported to JavaScript/TypeScript

use wasm_bindgen::prelude::*;
use crate::get_version;

/// Returns the current NuAnalytics version for the WASM build.
#[wasm_bindgen]
pub fn get_wasm_version() -> String {
    format!("NuAnalytics WASM v{}", get_version())
}

/// Returns a friendly greeting that includes the provided name.
#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    format!("Hello, {}! Welcome to NuAnalytics", name)
}
