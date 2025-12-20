//! Command-line interface entry point for `NuAnalytics`

use clap::{Parser, ValueEnum};
use logger::{
    debug, enable_debug, enable_verbose, error, info, init_file_logging, is_debug_enabled,
    set_level, verbose, warn, Level,
};
use nu_analytics::get_version;
use std::path::PathBuf;

#[derive(Copy, Clone, Debug, ValueEnum)]
enum LogLevelArg {
    Error,
    Warn,
    Info,
    Debug,
}

impl From<LogLevelArg> for Level {
    fn from(arg: LogLevelArg) -> Self {
        match arg {
            LogLevelArg::Error => Self::Error,
            LogLevelArg::Warn => Self::Warn,
            LogLevelArg::Info => Self::Info,
            LogLevelArg::Debug => Self::Debug,
        }
    }
}

#[derive(Parser, Debug)]
#[command(name = "nuanalytics-cli", about = "NuAnalytics command-line interface")]
struct Cli {
    /// Set the log level (error|warn|info|debug)
    #[arg(long, value_enum, default_value = "warn")]
    log_level: LogLevelArg,

    /// Enable verbose output
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Enable debug-level logging and runtime debug flag (shorthand)
    #[arg(long = "debug")]
    debug_flag: bool,

    /// Write logs to a file
    #[arg(long, value_name = "PATH")]
    log_file: Option<PathBuf>,
}

fn main() {
    let args = Cli::parse();

    // Determine effective level with shorthand flags taking precedence
    let mut level: Level = args.log_level.into();
    if args.debug_flag || level == Level::Debug {
        level = Level::Debug;
        enable_debug();
    }
    if args.verbose {
        // Verbose is separate from log level; enable it regardless
        enable_verbose();
    }
    set_level(level);

    // Initialize file logging if requested
    if let Some(log_path) = &args.log_file {
        if init_file_logging(log_path) {
            eprintln!("✓ File logging initialized at: {}", log_path.display());
        } else {
            eprintln!(
                "✗ Failed to initialize file logging at: {}",
                log_path.display()
            );
        }
    }

    println!("NuAnalytics CLI v{}", get_version());
    println!("Hello from the command-line interface!");

    // Use verbose! for verbose output when enabled
    if args.verbose {
        verbose!("CLI started with level {:?}, verbose enabled", level);
        verbose!("Debug enabled: {}", is_debug_enabled());
    }

    warn!("Sample warning from CLI");
    error!("Sample error from CLI");
    info!("Sample info from CLI");
    debug!("Sample debug from CLI");
}
