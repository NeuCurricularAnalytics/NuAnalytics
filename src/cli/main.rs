//! Command-line interface entry point for `NuAnalytics`

mod commands;

use clap::{Parser, Subcommand, ValueEnum};
use logger::{
    debug, enable_debug, enable_verbose, error, info, init_file_logging, is_debug_enabled,
    set_level, verbose, warn, Level,
};
use nu_analytics::config::Config;
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

#[derive(Debug, Subcommand)]
enum ConfigSubcommand {
    /// Display configuration values.
    ///
    /// If a KEY is provided, displays only that configuration value.
    /// If no KEY is provided, displays all configuration values.
    Get {
        /// Optional configuration key to display (e.g., `level`, `file`, `plans_dir`)
        #[arg(value_name = "KEY")]
        key: Option<String>,
    },
    /// Set a configuration value.
    Set {
        /// Configuration key to set
        #[arg(value_name = "KEY")]
        key: String,
        /// Value to set
        #[arg(value_name = "VALUE")]
        value: String,
    },
    /// Unset a configuration value.
    Unset {
        /// Configuration key to unset
        #[arg(value_name = "KEY")]
        key: String,
    },
    /// Reset configuration to defaults (requires confirmation).
    Reset,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Manage configuration.
    ///
    /// If no subcommand is provided, displays all configuration values.
    Config {
        #[command(subcommand)]
        subcommand: Option<ConfigSubcommand>,
    },
}

#[derive(Parser, Debug)]
#[command(
    name = "nuanalytics",
    about = "NuAnalytics command-line interface",
    version = env!("CARGO_PKG_VERSION")
)]
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

    /// Subcommand to execute.
    /// A subcommand is required to run the CLI.
    #[command(subcommand)]
    command: Command,
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

    // Load configuration once at startup
    let mut config = Config::load();
    let defaults = Config::from_defaults();

    // Handle subcommands
    match args.command {
        Command::Config { subcommand } => {
            match subcommand {
                None => {
                    // No subcommand provided, display all config values
                    commands::config::handle_config_get(&config, None);
                }
                Some(ConfigSubcommand::Get { key }) => {
                    commands::config::handle_config_get(&config, key);
                }
                Some(ConfigSubcommand::Set { key, value }) => {
                    commands::config::handle_config_set(&mut config, &key, &value);
                }
                Some(ConfigSubcommand::Unset { key }) => {
                    commands::config::handle_config_unset(&mut config, &defaults, &key);
                }
                Some(ConfigSubcommand::Reset) => {
                    commands::config::handle_config_reset();
                }
            }
            return;
        }
    }

    // This line is unreachable since all commands return
    #[allow(unreachable_code)]
    {
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
}
