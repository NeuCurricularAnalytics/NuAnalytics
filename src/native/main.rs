//! Command-line interface entry point for `NuAnalytics`

use clap::{Parser, ValueEnum};
use nu_analytics::{debug, error, get_version, info, shared::logger, warn};

#[derive(Copy, Clone, Debug, ValueEnum)]
enum LogLevelArg {
    Error,
    Warn,
    Info,
    Debug,
}

impl From<LogLevelArg> for logger::Level {
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

    /// Enable info-level logging (shorthand)
    #[arg(short = 'v', long = "verbose")]
    verbose: bool,

    /// Enable debug-level logging and runtime debug flag (shorthand)
    #[arg(long = "debug")]
    debug_flag: bool,
}

fn main() {
    let args = Cli::parse();

    // Determine effective level with shorthand flags taking precedence
    let mut level: logger::Level = args.log_level.into();
    if args.debug_flag {
        level = logger::Level::Debug;
        logger::enable_debug();
    } else if args.verbose {
        // Only raise to info if not explicitly set higher
        if (level as u8) < (logger::Level::Info as u8) {
            level = logger::Level::Info;
        }
    }
    logger::set_level(level);

    println!("NuAnalytics CLI v{}", get_version());
    println!("Hello from the command-line interface!");

    info!(
        "CLI started with level {:?} (verbose={}, debug={})",
        level, args.verbose, args.debug_flag
    );
    warn!("Sample warning from CLI");
    error!("Sample error from CLI");
    debug!("Sample debug from CLI");
}
