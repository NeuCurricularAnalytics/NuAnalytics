//! Integration tests for the `logger` crate

use logger::{debug, error, info, warn};
use logger::{set_level, set_level_from_str, Level};

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

#[cfg(feature = "log-debug")]
#[test]
fn debug_respects_runtime_flag() {
    use logger::{disable_debug, enable_debug};
    set_level(Level::Debug);
    disable_debug();
    debug!("should be silent");
    enable_debug();
    debug!("should emit");
}
