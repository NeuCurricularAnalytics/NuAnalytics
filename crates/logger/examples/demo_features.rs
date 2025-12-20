//! Example demonstrating the verbose and file-logging features

use logger::{
    debug, enable_debug, enable_verbose, error, info, init_file_logging, set_level, verbose, warn,
    Level,
};
use std::path::PathBuf;

fn main() {
    println!("=== Logger Feature Demo ===\n");

    // Set log level to Debug
    set_level(Level::Debug);
    enable_debug();

    // Initialize file logging
    let log_file = PathBuf::from("/tmp/logger_demo.log");
    if init_file_logging(&log_file) {
        println!("✓ File logging enabled at: {}\n", log_file.display());
    } else {
        println!("✗ Failed to initialize file logging\n");
    }

    // Enable verbose output
    enable_verbose();
    println!("✓ Verbose output enabled\n");

    println!("--- Standard Log Messages (these go to file ONLY) ---");
    error!("This is an error message");
    warn!("This is a warning message");
    info!("This is an info message");
    debug!("This is a debug message");

    println!("\n--- Verbose Output (console only, NOT in file) ---");
    verbose!("Processing item 1 of 10");
    verbose!("Processing item 2 of 10");
    verbose!("Processing item 3 of 10");
    verbose!("Progress: {}%", 30);
    verbose!("Almost done...");
    verbose!("Complete!");

    println!("\n--- Check the log file ---");
    println!("Run: cat /tmp/logger_demo.log");
    println!("The log file will contain error/warn/info/debug messages but NOT verbose output.");
}
