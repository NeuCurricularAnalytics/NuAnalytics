//! CLI argument definitions for `NuAnalytics`

use clap::{builder::BoolishValueParser, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use logger::Level;
use nu_analytics::config::ConfigOverrides;

/// CLI log level argument
///
/// Represents log levels that can be passed via CLI arguments. Converts to lowercase
/// strings for config storage and to `logger::Level` for runtime use.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum LogLevelArg {
    /// Error-level logging
    Error,
    /// Warning-level logging
    Warn,
    /// Info-level logging
    Info,
    /// Debug-level logging
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
        /// Optional configuration key to display (e.g., `level`, `file`, `out_dir`)
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
    /// Plan and analyze curricula.
    ///
    /// Load one or more curriculum CSV files and analyze the plan.
    Planner {
        /// Paths to curriculum CSV files (supports multiple)
        #[arg(value_name = "FILES", num_args = 1..)]
        input_files: Vec<std::path::PathBuf>,

        /// Output file paths (optional; defaults to config `out_dir` when omitted)
        ///
        /// When provided, must match the number of input files 1:1.
        #[arg(short, long, value_name = "FILES", num_args = 1..)]
        output: Vec<std::path::PathBuf>,

        /// Generate a report in the specified format (markdown, html, pdf)
        #[arg(long, value_name = "FORMAT")]
        report: Option<String>,

        /// Target credits per term for scheduling (default: 15.0)
        #[arg(long, value_name = "CREDITS")]
        term_credits: Option<f32>,

        /// Skip CSV metrics export (only generate report when --report is used)
        #[arg(long)]
        no_csv: bool,
    },
    /// Generate a curriculum report from a CSV file.
    ///
    /// Creates a formatted report with metrics, term scheduling, and visualizations.
    Report {
        /// Path to curriculum CSV file
        #[arg(value_name = "FILE")]
        input_file: std::path::PathBuf,

        /// Output file path (optional; defaults to input name with format extension)
        #[arg(short, long, value_name = "FILE")]
        output: Option<std::path::PathBuf>,

        /// Report format: markdown (md), html, or pdf
        #[arg(short, long, value_name = "FORMAT", default_value = "html")]
        format: String,

        /// Target credits per term for scheduling (default: 15.0)
        #[arg(long, value_name = "CREDITS")]
        term_credits: Option<f32>,
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
    ///
    /// Transforms CLI arguments into a `ConfigOverrides` struct that can be applied to
    /// the loaded configuration. Short-form flags (e.g., `--db-token`) take precedence
    /// over long-form flags (e.g., `--config-db-token`) when both are provided.
    ///
    /// # Returns
    /// A `ConfigOverrides` struct with values from CLI flags, where `None` means no override.
    ///
    /// # Examples
    /// ```ignore
    /// let args = Cli::parse();
    /// let overrides = args.to_config_overrides();
    /// config.apply_overrides(&overrides);
    /// ```
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_log_level_display() {
        assert_eq!(LogLevelArg::Error.to_string(), "error");
        assert_eq!(LogLevelArg::Warn.to_string(), "warn");
        assert_eq!(LogLevelArg::Info.to_string(), "info");
        assert_eq!(LogLevelArg::Debug.to_string(), "debug");
    }

    #[test]
    fn test_log_level_to_logger_level() {
        assert_eq!(Level::from(LogLevelArg::Error), Level::Error);
        assert_eq!(Level::from(LogLevelArg::Warn), Level::Warn);
        assert_eq!(Level::from(LogLevelArg::Info), Level::Info);
        assert_eq!(Level::from(LogLevelArg::Debug), Level::Debug);
    }

    #[test]
    fn test_to_config_overrides_empty() {
        let cli = Cli {
            log_level: None,
            verbose: false,
            debug_flag: false,
            log_file: None,
            config_level: None,
            config_log_file: None,
            config_verbose: None,
            config_db_token: None,
            db_token: None,
            config_db_endpoint: None,
            db_endpoint: None,
            config_out_dir: None,
            out_dir: None,
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert!(overrides.level.is_none());
        assert!(overrides.file.is_none());
        assert!(overrides.verbose.is_none());
        assert!(overrides.db_token.is_none());
        assert!(overrides.db_endpoint.is_none());
        assert!(overrides.out_dir.is_none());
    }

    #[test]
    fn test_to_config_overrides_with_values() {
        let cli = Cli {
            log_level: None,
            verbose: false,
            debug_flag: false,
            log_file: None,
            config_level: Some(LogLevelArg::Debug),
            config_log_file: Some(PathBuf::from("/tmp/test.log")),
            config_verbose: Some(true),
            config_db_token: None,
            db_token: Some("test-token".to_string()),
            config_db_endpoint: None,
            db_endpoint: Some("https://test.com".to_string()),
            config_out_dir: None,
            out_dir: Some(PathBuf::from("/output")),
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.level, Some("debug".to_string()));
        assert_eq!(overrides.file, Some("/tmp/test.log".to_string()));
        assert_eq!(overrides.verbose, Some(true));
        assert_eq!(overrides.db_token, Some("test-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://test.com".to_string()));
        assert_eq!(overrides.out_dir, Some("/output".to_string()));
    }

    #[test]
    fn test_short_form_precedence_over_long_form() {
        // Short-form flags should take precedence over long-form
        let cli = Cli {
            log_level: None,
            verbose: false,
            debug_flag: false,
            log_file: None,
            config_level: None,
            config_log_file: None,
            config_verbose: None,
            config_db_token: Some("long-token".to_string()),
            db_token: Some("short-token".to_string()),
            config_db_endpoint: Some("https://long.com".to_string()),
            db_endpoint: Some("https://short.com".to_string()),
            config_out_dir: Some(PathBuf::from("/long/out")),
            out_dir: Some(PathBuf::from("/short/out")),
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.db_token, Some("short-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://short.com".to_string()));
        assert_eq!(overrides.out_dir, Some("/short/out".to_string()));
    }

    #[test]
    fn test_long_form_when_short_form_absent() {
        // Long-form flags should be used when short-form is absent
        let cli = Cli {
            log_level: None,
            verbose: false,
            debug_flag: false,
            log_file: None,
            config_level: None,
            config_log_file: None,
            config_verbose: None,
            config_db_token: Some("long-token".to_string()),
            db_token: None,
            config_db_endpoint: Some("https://long.com".to_string()),
            db_endpoint: None,
            config_out_dir: Some(PathBuf::from("/long/out")),
            out_dir: None,
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.db_token, Some("long-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://long.com".to_string()));
        assert_eq!(overrides.out_dir, Some("/long/out".to_string()));
    }
}
