//! Internal logger module (migrated from crates/logger).
//! Feature flags: `log-info`, `log-debug`, `verbose`, `file-logging`.

// This logger was originally a seperate filesystem crate used for mutiple projects
// but copied into this project for easier deploy - needs updating - ACL

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
    /// Debug-level messages (requires `log-debug` feature and runtime enablement).
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

/// Global storage for the current log level.
static LOG_LEVEL: LazyLock<AtomicU8> = LazyLock::new(|| AtomicU8::new(default_level()));
#[cfg(feature = "log-debug")]
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(true);
#[cfg(feature = "verbose")]
static VERBOSE_ENABLED: AtomicBool = AtomicBool::new(false);
#[cfg(feature = "file-logging")]
static LOG_FILE: LazyLock<Mutex<Option<File>>> = LazyLock::new(|| Mutex::new(None));

/// Set the global log level.
pub fn set_level(level: Level) {
    LOG_LEVEL.store(level as u8, Ordering::SeqCst);
}

#[must_use]
/// Parse level from string (case-insensitive) and set it. Returns `true` on success.
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

#[cfg(feature = "log-debug")]
/// Enable debug logging at runtime.
pub fn enable_debug() {
    DEBUG_ENABLED.store(true, Ordering::SeqCst);
}
#[cfg(not(feature = "log-debug"))]
/// Enable debug logging at runtime (no-op when `log-debug` feature is disabled).
pub fn enable_debug() {}

#[cfg(feature = "log-debug")]
/// Disable debug logging at runtime.
pub fn disable_debug() {
    DEBUG_ENABLED.store(false, Ordering::SeqCst);
}
#[cfg(not(feature = "log-debug"))]
/// Disable debug logging at runtime (no-op when `log-debug` feature is disabled).
pub fn disable_debug() {}

#[cfg(feature = "log-debug")]
/// Returns whether debug logging is enabled.
pub fn is_debug_enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::SeqCst)
}
#[cfg(not(feature = "log-debug"))]
/// Returns whether debug logging is enabled (always false when feature is disabled).
pub fn is_debug_enabled() -> bool {
    false
}

#[cfg(feature = "verbose")]
/// Enable verbose output at runtime.
pub fn enable_verbose() {
    VERBOSE_ENABLED.store(true, Ordering::SeqCst);
}
#[cfg(not(feature = "verbose"))]
/// Enable verbose output at runtime (no-op when `verbose` feature is disabled).
pub fn enable_verbose() {}

#[cfg(feature = "verbose")]
/// Disable verbose output at runtime.
pub fn disable_verbose() {
    VERBOSE_ENABLED.store(false, Ordering::SeqCst);
}
#[cfg(not(feature = "verbose"))]
/// Disable verbose output at runtime (no-op when `verbose` feature is disabled).
pub fn disable_verbose() {}

#[cfg(feature = "verbose")]
/// Returns whether verbose output is enabled.
pub fn is_verbose_enabled() -> bool {
    VERBOSE_ENABLED.load(Ordering::SeqCst)
}
#[cfg(not(feature = "verbose"))]
/// Returns whether verbose output is enabled (always false when feature is disabled).
pub fn is_verbose_enabled() -> bool {
    false
}

#[cfg(feature = "file-logging")]
#[must_use]
/// Initialize file logging to a specific path. Returns `true` on success.
pub fn init_file_logging(path: &std::path::Path) -> bool {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .is_ok_and(|file| {
            LOG_FILE.lock().is_ok_and(|mut log_file| {
                *log_file = Some(file);
                true
            })
        })
}

#[cfg(not(feature = "file-logging"))]
/// Initialize file logging (no-op when `file-logging` feature is disabled).
pub fn init_file_logging(_path: &std::path::Path) -> bool {
    false
}

#[cfg(feature = "file-logging")]
fn write_to_file(message: &str) {
    if let Ok(mut log_file) = LOG_FILE.lock() {
        if let Some(ref mut file) = *log_file {
            let _ = writeln!(file, "{message}");
            let _ = file.flush();
        }
    }
}

#[cfg(not(feature = "file-logging"))]
fn write_to_file(_message: &str) {}

#[cfg(feature = "file-logging")]
fn is_file_logging_active() -> bool {
    LOG_FILE.lock().map(|lf| lf.is_some()).unwrap_or(false)
}
#[cfg(not(feature = "file-logging"))]
fn is_file_logging_active() -> bool {
    false
}

fn emit(prefix: &str, msg: &str, to_stderr: bool) {
    #[cfg(feature = "file-logging")]
    {
        if is_file_logging_active() && !prefix.is_empty() {
            let file_message = format!("{prefix} {msg}");
            write_to_file(&file_message);
            return;
        }
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = to_stderr;
        if prefix.is_empty() {
            console::log_1(&JsValue::from_str(msg));
        } else {
            let formatted = format!("%c{} {}", prefix, msg);
            fn style_for(prefix: &str) -> &'static str {
                match prefix {
                    "[ERROR]" => "color:#fff;background:#c0392b;font-weight:bold;padding:1px 4px;border-radius:3px",
                    "[WARN]" => "color:#000;background:#ffeb3b;font-weight:bold;padding:1px 4px;border-radius:3px",
                    "[INFO]" => "",
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

fn should_log(level: Level) -> bool {
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
    let current = LOG_LEVEL.load(Ordering::SeqCst);
    (level as u8) <= current && (level != Level::Debug || is_debug_enabled())
}

/// Internal logging dispatcher used by public macros.
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

#[macro_export]
/// Logs an error-level message (always enabled).
macro_rules! error { ($($arg:tt)*) => { $crate::logger::log_impl($crate::logger::Level::Error, format_args!($($arg)*)) }; }
#[macro_export]
/// Logs a warning-level message (always enabled).
macro_rules! warn  { ($($arg:tt)*) => { $crate::logger::log_impl($crate::logger::Level::Warn,  format_args!($($arg)*)) }; }
#[macro_export]
/// Logs an info-level message (requires `log-info` feature).
macro_rules! info  { ($($arg:tt)*) => { $crate::logger::log_impl($crate::logger::Level::Info,  format_args!($($arg)*)) }; }
#[macro_export]
/// Logs a debug-level message (requires `log-debug` feature and runtime enablement).
macro_rules! debug { ($($arg:tt)*) => { $crate::logger::log_impl($crate::logger::Level::Debug, format_args!($($arg)*)) }; }
#[macro_export]
/// Prints a verbose message (requires `verbose` feature and runtime enablement). This does not write to log files.
macro_rules! verbose {
    ($($arg:tt)*) => {
        #[cfg(feature = "verbose")]
        {
            if $crate::logger::is_verbose_enabled() { println!($($arg)*); }
        }
    }
}
