//! Lightweight cross-platform logger with feature-gated levels.
//!
//! - `log-info` feature enables `info!` output (enabled by default).
//! - `log-debug` feature enables `debug!` output and the runtime debug flag.
//! - `warn!` and `error!` are always active.
//!
//! On `wasm32`, logs go to `web_sys::console`; on native they use stdout/stderr.
use std::fmt::Arguments;
#[cfg(feature = "log-debug")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use web_sys::console;

/// Logging levels.
/// Supported logging levels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Level {
    /// Error-level messages (always enabled).
    Error = 1,
    /// Warning-level messages (always enabled).
    Warn = 2,
    /// Info-level messages (requires `log-info` feature).
    Info = 3,
    /// Debug-level messages (requires `log-debug` feature and runtime flag).
    Debug = 4,
}

const fn default_level() -> u8 {
    if cfg!(feature = "log-debug") {
        Level::Debug as u8
    } else if cfg!(feature = "log-info") {
        Level::Info as u8
    } else {
        Level::Warn as u8
    }
}

static LOG_LEVEL: LazyLock<AtomicU8> = LazyLock::new(|| AtomicU8::new(default_level()));
#[cfg(feature = "log-debug")]
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(true);

/// Set the global log level.
pub fn set_level(level: Level) {
    LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

/// Parse and set level from a string (case-insensitive). Returns true on success.
#[must_use]
pub fn set_level_from_str(level: &str) -> bool {
    match level.to_ascii_lowercase().as_str() {
        "error" | "err" => {
            set_level(Level::Error);
            true
        }
        "warn" | "warning" => {
            set_level(Level::Warn);
            true
        }
        "info" => {
            set_level(Level::Info);
            true
        }
        "debug" => {
            set_level(Level::Debug);
            true
        }
        _ => false,
    }
}

/// Enable debug logging at runtime (no-op when log-debug is disabled).
#[cfg(feature = "log-debug")]
pub fn enable_debug() {
    DEBUG_ENABLED.store(true, Ordering::SeqCst);
}
#[cfg(not(feature = "log-debug"))]
/// Enable debug logging at runtime (no-op when log-debug is disabled).
pub fn enable_debug() {}

/// Disable debug logging at runtime (no-op when log-debug is disabled).
#[cfg(feature = "log-debug")]
pub fn disable_debug() {
    DEBUG_ENABLED.store(false, Ordering::SeqCst);
}
#[cfg(not(feature = "log-debug"))]
/// Disable debug logging at runtime (no-op when log-debug is disabled).
pub fn disable_debug() {}

/// Returns whether debug logging is enabled (false if `log-debug` is disabled).
#[cfg(feature = "log-debug")]
pub fn is_debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::SeqCst)
}

/// Returns whether debug logging is enabled (false if `log-debug` is disabled).
#[cfg(not(feature = "log-debug"))]
pub fn is_debug_enabled() -> bool {
    false
}

/// Internal emission helper.
#[allow(dead_code)]
fn emit(prefix: &str, msg: &str, to_stderr: bool) {
    #[cfg(target_arch = "wasm32")]
    {
        let _ = to_stderr; // routing is based on prefix on wasm
        if prefix.is_empty() {
            console::log_1(&JsValue::from_str(msg));
        } else {
            // Use CSS styling via %c to colorize the prefix in the browser console
            let formatted = format!("%c{} {}", prefix, msg);

            fn style_for(prefix: &str) -> &'static str {
                match prefix {
                    // Error: keep red tag styling
                    "[ERROR]" => "color:#fff;background:#c0392b;font-weight:bold;padding:1px 4px;border-radius:3px",
                    // Warn: light yellow caution
                    "[WARN]" => "color:#000;background:#ffeb3b;font-weight:bold;padding:1px 4px;border-radius:3px",
                    // Info: no special colors
                    "[INFO]" => "",
                    // Debug: subtle grey tag
                    "[DEBUG]" => "color:#000;background:#bdc3c7;padding:1px 4px;border-radius:3px",
                    _ => "font-weight:bold",
                }
            }

            let style = style_for(prefix);
            let formatted_js = JsValue::from_str(&formatted);
            let style_js = JsValue::from_str(style);

            match prefix {
                "[ERROR]" => console::error_2(&formatted_js, &style_js),
                "[WARN]" => console::warn_2(&formatted_js, &style_js),
                _ => console::log_2(&formatted_js, &style_js),
            }
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    {
        if to_stderr {
            if prefix.is_empty() {
                eprintln!("{msg}");
            } else {
                eprintln!("{prefix} {msg}");
            }
        } else if prefix.is_empty() {
            println!("{msg}");
        } else {
            println!("{prefix} {msg}");
        }
    }
}

#[allow(dead_code)]
fn should_log(level: Level) -> bool {
    // Feature gates first
    match level {
        Level::Info => {
            if !cfg!(feature = "log-info") {
                return false;
            }
        }
        Level::Debug => {
            if !cfg!(feature = "log-debug") {
                return false;
            }
        }
        _ => {}
    }

    // Runtime level check
    let current = LOG_LEVEL.load(Ordering::SeqCst);
    (level as u8) <= current && (level != Level::Debug || is_debug_enabled())
}

#[allow(dead_code)]
/// Internal logging dispatch used by the public macros.
pub fn log_impl(level: Level, args: Arguments) {
    if !should_log(level) {
        return;
    }
    let msg = args.to_string();
    match level {
        Level::Error => emit("[ERROR]", &msg, true),
        Level::Warn => emit("[WARN]", &msg, true),
        Level::Info => emit("[INFO]", &msg, false),
        Level::Debug => emit("[DEBUG]", &msg, false),
    }
}

/// Public logging macros (always available; respect feature/runtime gating).
#[macro_export]
/// Log an error-level message.
macro_rules! error {
    ($($arg:tt)*) => { $crate::shared::logger::log_impl($crate::shared::logger::Level::Error, format_args!($($arg)*)) };
}

#[macro_export]
/// Log a warning-level message.
macro_rules! warn {
    ($($arg:tt)*) => { $crate::shared::logger::log_impl($crate::shared::logger::Level::Warn, format_args!($($arg)*)) };
}

#[macro_export]
/// Log an info-level message (requires `log-info`).
macro_rules! info {
    ($($arg:tt)*) => { $crate::shared::logger::log_impl($crate::shared::logger::Level::Info, format_args!($($arg)*)) };
}

#[macro_export]
/// Log a debug-level message (requires `log-debug` and runtime enablement).
macro_rules! debug {
    ($($arg:tt)*) => { $crate::shared::logger::log_impl($crate::shared::logger::Level::Debug, format_args!($($arg)*)) };
}

#[cfg(test)]
mod tests {
    use super::{disable_debug, enable_debug, set_level, Level};

    #[test]
    fn info_no_panic() {
        info!("info {}", 1);
    }

    #[test]
    fn warn_no_panic() {
        warn!("warn {}", 2);
    }

    #[test]
    fn error_no_panic() {
        error!("error {}", 3);
    }

    #[cfg(feature = "log-debug")]
    #[test]
    fn debug_respects_runtime_flag() {
        set_level(Level::Debug);
        disable_debug();
        debug!("should be silent");
        enable_debug();
        debug!("should emit");
    }
}
