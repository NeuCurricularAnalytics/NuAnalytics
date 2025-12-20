//! Lightweight cross-platform logger crate with feature-gated levels.
//! - `log-info` enables `info!` output (enabled by default).
//! - `log-debug` enables `debug!` output and a runtime debug flag.
//! - `verbose` enables `verbose!` output, a simple printer with no tags.
//! - `file-logging` enables writing log messages to a file (verbose does NOT go to file).
//! - `warn!` and `error!` are always active.
//!
//! On `wasm32`, logs go to `web_sys::console`; on native they use stdout/stderr.

use std::fmt::Arguments;
#[cfg(feature = "log-debug")]
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::LazyLock;

#[cfg(feature = "file-logging")]
use std::{
    fs::{File, OpenOptions},
    io::Write,
    sync::Mutex,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::JsValue;
#[cfg(target_arch = "wasm32")]
use web_sys::console;

/// Logging levels.
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

/// Determine the default logging level based on enabled features.
///
/// - When the `log-debug` feature is enabled, defaults to `Level::Debug`.
/// - Else when `log-info` is enabled, defaults to `Level::Info`.
/// - Otherwise defaults to `Level::Warn`.
const fn default_level() -> u8 {
    if cfg!(feature = "log-debug") {
        Level::Debug as u8
    } else if cfg!(feature = "log-info") {
        Level::Info as u8
    } else {
        Level::Warn as u8
    }
}

/// Global storage for the current log level.
static LOG_LEVEL: LazyLock<AtomicU8> = LazyLock::new(|| AtomicU8::new(default_level()));
/// Runtime flag controlling whether `debug!` messages should emit.
#[cfg(feature = "log-debug")]
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(true);
/// Runtime flag controlling whether `verbose!` output should emit.
#[cfg(feature = "verbose")]
static VERBOSE_ENABLED: AtomicBool = AtomicBool::new(false);
/// Global storage for the log file path and handle.
#[cfg(feature = "file-logging")]
static LOG_FILE: LazyLock<Mutex<Option<File>>> = LazyLock::new(|| Mutex::new(None));

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

/// Enable verbose output at runtime (no-op when verbose is disabled).
#[cfg(feature = "verbose")]
pub fn enable_verbose() {
    VERBOSE_ENABLED.store(true, Ordering::SeqCst);
}
#[cfg(not(feature = "verbose"))]
/// Enable verbose output at runtime (no-op when verbose is disabled).
pub fn enable_verbose() {}

/// Disable verbose output at runtime (no-op when verbose is disabled).
#[cfg(feature = "verbose")]
pub fn disable_verbose() {
    VERBOSE_ENABLED.store(false, Ordering::SeqCst);
}
#[cfg(not(feature = "verbose"))]
/// Disable verbose output at runtime (no-op when verbose is disabled).
pub fn disable_verbose() {}

/// Returns whether verbose output is enabled (false if `verbose` is disabled).
#[cfg(feature = "verbose")]
pub fn is_verbose_enabled() -> bool {
    VERBOSE_ENABLED.load(Ordering::SeqCst)
}

/// Returns whether verbose output is enabled (false if `verbose` is disabled).
#[cfg(not(feature = "verbose"))]
pub fn is_verbose_enabled() -> bool {
    false
}

/// Initialize file logging to the specified path.
/// Returns true on success, false on failure.
///
/// # Panics
///
/// Panics if the `LOG_FILE` mutex is poisoned.
#[cfg(feature = "file-logging")]
#[must_use]
pub fn init_file_logging(path: &std::path::Path) -> bool {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .is_ok_and(|file| {
            let mut log_file = LOG_FILE.lock().unwrap();
            *log_file = Some(file);
            true
        })
}

/// Initialize file logging to the specified path.
/// Returns true on success, false on failure.
#[cfg(not(feature = "file-logging"))]
pub fn init_file_logging(_path: &std::path::Path) -> bool {
    false
}

/// Write a message to the log file (if file logging is enabled).
#[cfg(feature = "file-logging")]
fn write_to_file(message: &str) {
    if let Ok(mut log_file) = LOG_FILE.lock() {
        if let Some(ref mut file) = *log_file {
            let _ = writeln!(file, "{message}");
            let _ = file.flush();
        }
    }
}

/// Write a message to the log file (if file logging is enabled).
#[cfg(not(feature = "file-logging"))]
fn write_to_file(_message: &str) {}

/// Returns true if file logging has been initialized and is active.
#[cfg(feature = "file-logging")]
fn is_file_logging_active() -> bool {
    LOG_FILE.lock().map(|lf| lf.is_some()).unwrap_or(false)
}

/// Returns false when file logging feature is disabled.
#[cfg(not(feature = "file-logging"))]
fn is_file_logging_active() -> bool {
    false
}

/// Internal emission helper.
///
/// Routes messages to platform-appropriate sinks:
/// - On `wasm32`, uses `web_sys::console` with light styling for `[ERROR]`/`[WARN]`.
/// - On native, writes to stdout by default and stderr for warnings/errors.
/// - When file-logging is enabled, also writes to the log file.
///
/// `prefix` controls the level tag (e.g., `[ERROR]`), while `to_stderr`
/// indicates whether the native path should use stderr.
#[allow(dead_code)]
fn emit(prefix: &str, msg: &str, to_stderr: bool) {
    // If file logging is enabled, write to file and do not echo to console.
    #[cfg(feature = "file-logging")]
    {
        if is_file_logging_active() && !prefix.is_empty() {
            let file_message = format!("{prefix} {msg}");
            write_to_file(&file_message);
            return;
        }
    }
    // Then emit to console/stdout
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
/// Decide whether a message at `level` should be emitted.
///
/// Applies feature gates first (`log-info`, `log-debug`), then compares against
/// the global runtime level. For debug messages, also requires `is_debug_enabled()`
/// to be true.
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

/// Internal logging dispatch used by the public macros.
///
/// Converts `args` to a `String` and emits to the appropriate sink configured
/// by `level`. Messages are suppressed when `should_log(level)` is false.
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
/// Logs an error-level message (always enabled). Emits to stderr on native.
macro_rules! error {
    ($($arg:tt)*) => { $crate::log_impl($crate::Level::Error, format_args!($($arg)*)) };
}

#[macro_export]
/// Logs a warning-level message (always enabled). Emits to stderr on native.
macro_rules! warn {
    ($($arg:tt)*) => { $crate::log_impl($crate::Level::Warn, format_args!($($arg)*)) };
}

#[macro_export]
/// Logs an info-level message (requires `log-info` feature).
macro_rules! info {
    ($($arg:tt)*) => { $crate::log_impl($crate::Level::Info, format_args!($($arg)*)) };
}

#[macro_export]
/// Logs a debug-level message (requires `log-debug` feature and runtime enablement).
macro_rules! debug {
    ($($arg:tt)*) => { $crate::log_impl($crate::Level::Debug, format_args!($($arg)*)) };
}

#[macro_export]
/// Prints a verbose message (requires `verbose` feature and runtime enablement).
/// This is a simple printer with no tags, and does NOT go to log files.
macro_rules! verbose {
    ($($arg:tt)*) => {
        #[cfg(feature = "verbose")]
        {
            if $crate::is_verbose_enabled() {
                println!($($arg)*);
            }
        }
    };
}

#[cfg(test)]
mod tests {
    use super::{disable_debug, enable_debug, set_level, Level};

    #[test]
    fn info_no_panic() {
        crate::info!("info {}", 1);
    }

    #[test]
    fn warn_no_panic() {
        crate::warn!("warn {}", 2);
    }

    #[test]
    fn error_no_panic() {
        crate::error!("error {}", 3);
    }

    #[cfg(feature = "log-debug")]
    #[test]
    fn debug_respects_runtime_flag() {
        set_level(Level::Debug);
        disable_debug();
        crate::debug!("should be silent");
        enable_debug();
        crate::debug!("should emit");
    }
}
