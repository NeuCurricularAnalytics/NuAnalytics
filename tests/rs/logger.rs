//! Integration tests for logger behavior.
//!
//! The logger was briefly a standalone crate (`8991591`) and is now the internal
//! `nu_analytics::logger` module; the macros are `#[macro_export]`ed at the crate root.

use nu_analytics::logger::{set_level, set_level_from_str, Level};
use nu_analytics::{debug, error, info, warn};

#[test]
fn level_parse_accepts_valid() {
    assert!(set_level_from_str("error"));
    assert!(set_level_from_str("warn"));
    assert!(set_level_from_str("info"));
    assert!(set_level_from_str("debug"));
}

#[test]
fn level_parse_rejects_invalid() {
    assert!(!set_level_from_str("invalid"));
    assert!(!set_level_from_str(""));
}

#[test]
fn logs_do_not_panic() {
    set_level(Level::Debug);
    info!("info integration");
    warn!("warn integration");
    error!("error integration");
    debug!("debug integration");
}
