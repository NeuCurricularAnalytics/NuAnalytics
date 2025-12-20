//! Tests for verbose and file-logging features.

use logger::{enable_verbose, error, info, verbose, warn};
use std::path::PathBuf;

#[cfg(feature = "verbose")]
#[test]
fn verbose_respects_runtime_flag() {
    // verbose should not output when disabled (default)
    verbose!("This should not appear");

    // Enable verbose
    enable_verbose();
    verbose!("This should appear: verbose test {}", 42);
}

#[cfg(feature = "file-logging")]
#[test]
fn file_logging_initialization() {
    use logger::init_file_logging;
    use std::fs;

    let log_path = PathBuf::from("/tmp/test_logger.log");

    // Clean up any existing file
    let _ = fs::remove_file(&log_path);

    // Initialize file logging
    assert!(init_file_logging(&log_path));

    // Write some logs
    info!("Test info message");
    warn!("Test warning message");
    error!("Test error message");

    // Note: verbose should NOT go to the file
    #[cfg(feature = "verbose")]
    {
        enable_verbose();
        verbose!("This verbose message should NOT be in the file");
    }

    // Read the file and verify logs are present
    let contents = fs::read_to_string(&log_path).expect("Failed to read log file");
    assert!(contents.contains("[INFO] Test info message"));
    assert!(contents.contains("[WARN] Test warning message"));
    assert!(contents.contains("[ERROR] Test error message"));
    assert!(!contents.contains("verbose message")); // verbose should NOT be in file

    // Clean up
    let _ = fs::remove_file(&log_path);
}
