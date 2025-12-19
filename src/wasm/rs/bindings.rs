//! WASM bindings exported to JavaScript/TypeScript

use crate::get_version;
use crate::{debug, error, info, warn};
use wasm_bindgen::prelude::*;

/// Returns the current NuAnalytics version for the WASM build.
#[wasm_bindgen]
pub fn get_wasm_version() -> String {
    format!("NuAnalytics WASM v{}", get_version())
}

/// Returns a friendly greeting that includes the provided name.
#[wasm_bindgen]
pub fn greet(name: &str) -> String {
    info!("Greeting user: {name}");
    warn!("Sample warn from WASM");
    error!("Sample error from WASM");
    debug!("Sample debug from WASM");
    format!("Hello, {}! Welcome to NuAnalytics", name)
}

/// Emits sample log messages from WASM and returns a summary string.
#[wasm_bindgen]
pub fn sample_logs() -> String {
    info!("WASM sample info");
    warn!("WASM sample warn");
    error!("WASM sample error");
    debug!("WASM sample debug");
    "WASM logs emitted".to_string()
}
