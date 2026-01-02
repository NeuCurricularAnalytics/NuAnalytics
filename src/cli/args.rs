//! CLI argument definitions for `NuAnalytics`

use clap::{builder::BoolishValueParser, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use logger::Level;
use nu_analytics::config::ConfigOverrides;

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum LogLevelArg {
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

impl std::fmt::Display for LogLevelArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let as_str = match self {
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
        };
        write!(f, "{as_str}")
    }
}

#[derive(Debug, Subcommand)]
pub enum ConfigSubcommand {
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
pub enum Command {
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
pub struct Cli {
    /// Set the runtime log level (error|warn|info|debug). Falls back to config if omitted.
    #[arg(long, value_enum)]
    pub log_level: Option<LogLevelArg>,

    /// Enable verbose output (runtime only)
    #[arg(short = 'v', long = "verbose")]
    pub verbose: bool,

    /// Enable debug-level logging and runtime debug flag (shorthand)
    #[arg(long = "debug")]
    pub debug_flag: bool,

    /// Write runtime logs to a file
    #[arg(long, value_name = "PATH")]
    pub log_file: Option<PathBuf>,

    // --- Config overrides ---
    /// Override config logging level (stored in config file)
    #[arg(long = "config-level", value_enum)]
    pub config_level: Option<LogLevelArg>,

    /// Override config log file path
    #[arg(long = "config-log-file", value_name = "PATH")]
    pub config_log_file: Option<PathBuf>,

    /// Override config verbose flag (true/false)
    #[arg(long = "config-verbose", value_parser = BoolishValueParser::new())]
    pub config_verbose: Option<bool>,

    /// Override config database token
    #[arg(long = "config-db-token", value_name = "TOKEN")]
    pub config_db_token: Option<String>,

    /// Override config database token (short form)
    #[arg(long = "db-token", value_name = "TOKEN")]
    pub db_token: Option<String>,

    /// Override config database endpoint
    #[arg(long = "config-db-endpoint", value_name = "URL")]
    pub config_db_endpoint: Option<String>,

    /// Override config database endpoint (short form)
    #[arg(long = "db-endpoint", value_name = "URL")]
    pub db_endpoint: Option<String>,

    /// Override config plans directory
    #[arg(long = "config-plans-dir", value_name = "DIR")]
    pub config_plans_dir: Option<PathBuf>,

    /// Override config plans directory (short form)
    #[arg(long = "plans-dir", value_name = "DIR")]
    pub plans_dir: Option<PathBuf>,

    /// Override config output directory
    #[arg(long = "config-out-dir", value_name = "DIR")]
    pub config_out_dir: Option<PathBuf>,

    /// Override config output directory (short form)
    #[arg(long = "out-dir", value_name = "DIR")]
    pub out_dir: Option<PathBuf>,

    /// Subcommand to execute.
    /// A subcommand is required to run the CLI.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Convert CLI flags into config overrides
    pub fn to_config_overrides(&self) -> ConfigOverrides {
        ConfigOverrides {
            level: self.config_level.map(|lvl| lvl.to_string().to_lowercase()),
            file: self
                .config_log_file
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            verbose: self.config_verbose,
            db_token: self
                .db_token
                .clone()
                .or_else(|| self.config_db_token.clone()),
            db_endpoint: self
                .db_endpoint
                .clone()
                .or_else(|| self.config_db_endpoint.clone()),
            plans_dir: self
                .plans_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .or_else(|| {
                    self.config_plans_dir
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                }),
            out_dir: self
                .out_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
                .or_else(|| {
                    self.config_out_dir
                        .as_ref()
                        .map(|p| p.to_string_lossy().to_string())
                }),
        }
    }
}
