//! CLI argument definitions for `NuAnalytics`

use clap::{builder::BoolishValueParser, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use nu_analytics::config::ConfigOverrides;
use nu_analytics::logger::Level;

/// CLI log level argument
///
/// Represents log levels that can be passed via CLI arguments. Converts to lowercase
/// strings for config storage and to `nu_analytics::logger::Level` for runtime use.
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

/// Report format argument for CLI
///
/// Specifies the output format for curriculum reports.
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq)]
pub enum ReportFormatArg {
    /// HTML format with interactive visualizations
    Html,
    /// Markdown format for documentation
    Md,
    /// PDF format (not yet implemented)
    Pdf,
}

/// Calculation strategy for aggregate metrics
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq, Default)]
pub enum CalcStrategyArg {
    /// Median (default) - robust to outliers
    #[default]
    Median,
    /// Mean - arithmetic average
    Mean,
}

impl std::fmt::Display for CalcStrategyArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Median => write!(f, "median"),
            Self::Mean => write!(f, "mean"),
        }
    }
}

/// Sampling strategy for plan enumeration
#[derive(Copy, Clone, Debug, ValueEnum, PartialEq, Eq, Default)]
pub enum SamplingStrategyArg {
    /// Sequential - enumerate in order (may bias statistics)
    Sequential,
    /// Shuffled (default) - randomize order for unbiased sampling
    #[default]
    Shuffled,
    /// Stratified - ensure coverage across option space
    Stratified,
}

impl std::fmt::Display for SamplingStrategyArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequential => write!(f, "sequential"),
            Self::Shuffled => write!(f, "shuffled"),
            Self::Stratified => write!(f, "stratified"),
        }
    }
}

impl ReportFormatArg {
    /// Get the file extension for this format
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Md => "md",
            Self::Pdf => "pdf",
        }
    }

    /// Try to infer format from a file extension
    #[must_use]
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_lowercase().as_str() {
            "html" | "htm" => Some(Self::Html),
            "md" | "markdown" => Some(Self::Md),
            "pdf" => Some(Self::Pdf),
            _ => None,
        }
    }
}

impl std::fmt::Display for ReportFormatArg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.extension())
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
    /// Load one or more curriculum CSV files, compute metrics, and generate reports.
    /// By default, generates both CSV metrics files and HTML reports.
    ///
    /// # Examples
    /// ```sh
    /// # Generate both CSV and HTML for multiple files
    /// nuanalytics planner course1.csv course2.csv
    ///
    /// # Generate only HTML report with explicit output
    /// nuanalytics planner course.csv -o report.html
    ///
    /// # Generate only CSV metrics
    /// nuanalytics planner course.csv --no-report
    ///
    /// # Generate Markdown report to custom directory
    /// nuanalytics planner course.csv --report-format md --report-dir ./docs
    /// ```
    Planner {
        /// Paths to curriculum CSV files (supports multiple)
        #[arg(value_name = "FILES", num_args = 1..)]
        input_files: Vec<std::path::PathBuf>,

        /// Explicit output file paths (1:1 mapping with input files, space-separated)
        ///
        /// When provided, the extension determines output type:
        /// - `.csv` → generates only CSV metrics (implies --no-report)
        /// - `.html`, `.md`, `.pdf` → generates only report (implies --no-csv)
        ///
        /// Must match the number of input files when provided.
        #[arg(short, long, value_name = "FILES", num_args = 1..)]
        output: Vec<std::path::PathBuf>,

        /// Report format when generating reports (html, md, pdf)
        ///
        /// Used when -o is not provided or when -o extension conflicts (with warning).
        /// Defaults to html if not specified.
        #[arg(long, value_enum, value_name = "FORMAT")]
        report_format: Option<ReportFormatArg>,

        /// Custom PDF converter command (e.g., chrome, chromium, wkhtmltopdf)
        ///
        /// When generating PDF reports, specifies which tool to use for HTML-to-PDF conversion.
        /// If not provided, will auto-detect Chrome/Chromium.
        #[arg(long, value_name = "COMMAND")]
        pdf_converter: Option<String>,

        /// Override reports output directory (from config)
        #[arg(long, value_name = "DIR")]
        report_dir: Option<std::path::PathBuf>,

        /// Override metrics output directory (from config)
        #[arg(long, value_name = "DIR")]
        metrics_dir: Option<std::path::PathBuf>,

        /// Target credits per term for scheduling (default: 15.0)
        #[arg(long, value_name = "CREDITS")]
        term_credits: Option<f32>,

        /// Skip CSV metrics generation
        #[arg(long)]
        no_csv: bool,

        /// Skip report generation
        #[arg(long)]
        no_report: bool,
    },
    /// Validate and analyze degree program YAML files.
    ///
    /// Load a degree program YAML file and validate its structure, requirements,
    /// prerequisites, and cross-listing relationships. By default (no flags),
    /// runs full degree analysis (--analyze). Use specific flags to run only
    /// validation, graph printing, or audit.
    ///
    /// Circular prerequisites are automatically broken by removing optional edges
    /// to create a valid DAG for analysis.
    ///
    /// # Examples
    /// ```sh
    /// # Run full degree analysis (default action)
    /// nuanalytics degree samples/degrees/csu-cs-bscs-general.yaml
    ///
    /// # Batch mode — process every YAML in a directory
    /// nuanalytics degree samples/degrees/*.yaml
    ///
    /// # Validate a degree program only
    /// nuanalytics degree --validate samples/degrees/csu-cs-bscs-general.yaml
    ///
    /// # Print prerequisite graph
    /// nuanalytics degree --print-graph samples/degrees/csu-cs-bscs-general.yaml
    ///
    /// # Analyze with custom settings
    /// nuanalytics degree --analyze --calc-strategy mean --sample-plans 10 degree.yaml
    /// ```
    Degree {
        /// Paths to one or more degree program YAML files. Each file is processed
        /// independently in order; per-file failures are reported but do not
        /// abort the batch.
        #[arg(value_name = "FILES", num_args = 0..)]
        files: Vec<PathBuf>,

        /// Validate the degree program YAML file
        #[arg(long)]
        validate: bool,

        /// Print the course prerequisite graph
        #[arg(long)]
        print_graph: bool,

        /// Run an audit report on the degree program
        /// Includes validation, missing prerequisites analysis, and deep chain detection
        #[arg(long)]
        audit: bool,

        /// Run full degree analysis: generate all plans and produce HTML report with statistics.
        /// This is the default action when no other flags are specified.
        #[arg(long)]
        analyze: bool,

        /// Calculation strategy for aggregate metrics (median or mean)
        #[arg(long, value_enum, value_name = "STRATEGY")]
        calc_strategy: Option<CalcStrategyArg>,

        /// Sampling strategy for plan enumeration (sequential, shuffled, stratified)
        #[arg(long, value_enum, value_name = "STRATEGY")]
        sampling_strategy: Option<SamplingStrategyArg>,

        /// Number of random plans to sample and export (default: 5)
        #[arg(long, value_name = "COUNT")]
        sample_plans: Option<usize>,

        /// Maximum number of plans to generate (safety cap)
        #[arg(long, value_name = "COUNT")]
        max_plans: Option<usize>,

        /// Generate all plan combinations without deduplication (overrides default)
        #[arg(long)]
        full_run: bool,

        /// Override reports output directory (from config)
        #[arg(long, value_name = "DIR")]
        report_dir: Option<PathBuf>,

        /// Override metrics output directory (from config)
        #[arg(long, value_name = "DIR")]
        metrics_dir: Option<PathBuf>,

        /// Skip CSV plan export
        #[arg(long)]
        no_csv: bool,

        /// Skip HTML report generation
        #[arg(long)]
        no_report: bool,

        /// Courses to always include in all plans (comma-separated course codes)
        ///
        /// These courses will be included in every generated plan, including the shortest path.
        /// If an included course satisfies a requirement (e.g., a picklist), other options
        /// for that requirement will not be considered.
        ///
        /// Example: --include "CS3500,MATH2331,PHIL1145"
        #[arg(long, value_name = "COURSES", value_delimiter = ',')]
        include: Option<Vec<String>>,
    },
    /// Manage the `NuAnalytics` database (IPEDS data, status, import).
    #[cfg(feature = "database")]
    Db {
        /// Database subcommand to run
        #[command(subcommand)]
        subcommand: DbSubcommand,
    },
    /// Run the MCP (Model Context Protocol) server.
    ///
    /// Starts a server that exposes `NuAnalytics` tools for AI model integration
    /// via stdio transport. Compatible with Claude Desktop, Claude Code, and
    /// any MCP-compatible client.
    #[cfg(feature = "mcp")]
    #[command(long_about = "Run the MCP (Model Context Protocol) server.\n\n\
            Starts a server that exposes NuAnalytics tools for AI model integration\n\
            via stdio transport. Compatible with Claude Desktop, Claude Code, and\n\
            any MCP-compatible client.\n\n\
            Available tools:\n\
            \x20 get_degree_schema  Get degree YAML schema documentation\n\
            \x20 validate_degree    Validate a degree YAML and return errors/warnings\n\
            \x20 audit_degree       Comprehensive audit (validation + prereq analysis)\n\
            \x20 analyze_degree     Full plan analysis with aggregate metrics and schedules\n\n\
            Examples:\n\
            \x20 nuanalytics mcp\n\
            \x20 nuanalytics --log-level debug mcp\n\
            \x20 npx @modelcontextprotocol/inspector nuanalytics mcp")]
    Mcp,
    /// Initialize a new `NuAnalytics` research project directory.
    ///
    /// Scaffolds a directory with a `.claude/` folder pre-wired to the
    /// `NuAnalytics` MCP server and SKILL.md skills for degree authoring,
    /// review, and curriculum-plan analysis.
    ///
    /// # Examples
    /// ```sh
    /// nuanalytics init my-cs-study
    /// nuanalytics init ./projects/curriculum-2026 --force
    /// ```
    #[command(
        long_about = "Initialize a new NuAnalytics research project directory.\n\n\
            Creates <DIR> if it does not exist and scaffolds:\n\
            \x20 .claude/settings.json       MCP wiring (auto-detected binary path)\n\
            \x20 .claude/skills/             SKILL.md skills for Claude Code\n\
            \x20 degrees/                    workspace for degree YAML files\n\
            \x20 plans/                      workspace for curriculum CSV plans\n\
            \x20 nuanalytics.toml            local config (overrides global)\n\
            \x20 README.md                   one-page orientation\n\n\
            If any target file already exists, init aborts unless --force is set.\n\n\
            Examples:\n\
            \x20 nuanalytics init my-cs-study\n\
            \x20 nuanalytics init ./projects/curriculum-2026 --force"
    )]
    Init {
        /// Target directory to scaffold (created if it does not exist).
        #[arg(value_name = "DIR")]
        dir: std::path::PathBuf,

        /// Overwrite existing files in <DIR>. Without this, init aborts if any
        /// target file already exists.
        #[arg(long)]
        force: bool,
    },
}

/// Database management subcommands
#[cfg(feature = "database")]
#[derive(Debug, Subcommand)]
pub enum DbSubcommand {
    /// Sign in to Supabase via OAuth and save the session for database operations.
    ///
    /// Opens your browser to authenticate with the chosen OAuth provider (default: GitHub).
    /// After authorising, the browser redirects back to a temporary local server and
    /// the session token is saved automatically.
    ///
    /// Requires `database.endpoint` and `database.anon_key` to be set in config.
    /// The provider must be enabled in your Supabase project under Authentication → Providers.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db login                    # uses GitHub
    /// nuanalytics db login --provider google
    /// nuanalytics db login --provider gitlab
    /// ```
    Login {
        /// OAuth provider to use (github, google, gitlab, discord, azure, ...)
        #[arg(long, value_name = "PROVIDER", default_value = "github")]
        provider: String,
    },
    /// Sign out and remove the saved session token.
    Logout,
    /// Show the currently signed-in user (if any).
    Whoami,
    /// Execute an SQL file against the database via the Supabase Management API.
    ///
    /// Requires `database.management_key` (a Supabase Personal Access Token) to be set:
    ///
    /// ```sh
    /// nuanalytics config set database.management_key <pat>
    /// ```
    ///
    /// Get a PAT at <https://app.supabase.com/account/tokens>. The PAT gives DDL
    /// access (CREATE TABLE, INSERT, etc.) which the project anon key cannot do.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db exec-sql docs/database/schema.sql
    /// nuanalytics db exec-sql docs/database/cip-seed.sql
    /// ```
    ExecSql {
        /// Path to the SQL file to execute
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
    },
    /// Check database connectivity and display row counts
    Status,
    /// Import IPEDS data from locally downloaded CSV or ZIP files into Supabase.
    ///
    /// Only two files are needed — the completions file is used in a single pass to
    /// populate both the `completions` table (CS CIP codes only) and the
    /// `institution_completions` table (all-major totals used for representation ratios).
    ///
    /// Download from <https://nces.ed.gov/ipeds/use-the-data>:
    /// - HD{year}.csv or HD{year}.zip  (institution directory)
    /// - C{year}_A.csv or C{year}_A.zip  (completions by award level)
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db ipeds-import --year 2024 --dir ./ipeds_data/
    /// nuanalytics db ipeds-import --year 2024 --institutions HD2024.zip --completions C2024_A.zip
    /// ```
    IpedsImport {
        /// Directory containing IPEDS CSV/ZIP files (auto-detected by filename pattern)
        #[arg(long, value_name = "DIR")]
        dir: Option<std::path::PathBuf>,
        /// Path to the HD (institutions) CSV or ZIP file
        #[arg(long, value_name = "FILE")]
        institutions: Option<std::path::PathBuf>,
        /// Path to the `C_A` (completions) CSV or ZIP file
        #[arg(long, value_name = "FILE")]
        completions: Option<std::path::PathBuf>,
        /// Academic year for the data (e.g. 2023 for 2023-2024 data)
        #[arg(long, default_value = "2023")]
        year: u16,
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

    /// Override config database anon key (Supabase anonymous key for the project)
    #[arg(long = "config-db-anon-key", value_name = "KEY")]
    pub config_db_anon_key: Option<String>,

    /// Override config database anon key (short form)
    #[arg(long = "db-anon-key", value_name = "KEY")]
    pub db_anon_key: Option<String>,

    /// Override config database endpoint
    #[arg(long = "config-db-endpoint", value_name = "URL")]
    pub config_db_endpoint: Option<String>,

    /// Override config database endpoint (short form)
    #[arg(long = "db-endpoint", value_name = "URL")]
    pub db_endpoint: Option<String>,

    /// Override config metrics output directory
    #[arg(long = "metrics-dir", value_name = "DIR")]
    pub metrics_dir: Option<PathBuf>,

    /// Override config reports output directory
    #[arg(long = "reports-dir", value_name = "DIR")]
    pub reports_dir: Option<PathBuf>,

    /// Subcommand to execute.
    /// A subcommand is required to run the CLI.
    #[command(subcommand)]
    pub command: Command,
}

impl Cli {
    /// Convert CLI flags into config overrides
    ///
    /// Transforms CLI arguments into a `ConfigOverrides` struct that can be applied to
    /// the loaded configuration. Short-form flags (e.g., `--db-anon-key`) take precedence
    /// over long-form flags (e.g., `--config-db-anon-key`) when both are provided.
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
            db_anon_key: self
                .db_anon_key
                .clone()
                .or_else(|| self.config_db_anon_key.clone()),
            db_endpoint: self
                .db_endpoint
                .clone()
                .or_else(|| self.config_db_endpoint.clone()),
            metrics_dir: self
                .metrics_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            reports_dir: self
                .reports_dir
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
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
    fn test_report_format_arg_extension_roundtrip() {
        for fmt in [
            ReportFormatArg::Html,
            ReportFormatArg::Md,
            ReportFormatArg::Pdf,
        ] {
            assert_eq!(ReportFormatArg::from_extension(fmt.extension()), Some(fmt));
        }
    }

    #[test]
    fn test_report_format_arg_from_extension_aliases() {
        assert_eq!(
            ReportFormatArg::from_extension("HTML"),
            Some(ReportFormatArg::Html)
        );
        assert_eq!(
            ReportFormatArg::from_extension("htm"),
            Some(ReportFormatArg::Html)
        );
        assert_eq!(
            ReportFormatArg::from_extension("markdown"),
            Some(ReportFormatArg::Md)
        );
        assert_eq!(
            ReportFormatArg::from_extension("PDF"),
            Some(ReportFormatArg::Pdf)
        );
    }

    #[test]
    fn test_report_format_arg_from_extension_rejects_unknown() {
        assert_eq!(ReportFormatArg::from_extension("xlsx"), None);
        assert_eq!(ReportFormatArg::from_extension(""), None);
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
            config_db_anon_key: None,
            db_anon_key: None,
            config_db_endpoint: None,
            db_endpoint: None,
            metrics_dir: None,
            reports_dir: None,
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert!(overrides.level.is_none());
        assert!(overrides.file.is_none());
        assert!(overrides.verbose.is_none());
        assert!(overrides.db_anon_key.is_none());
        assert!(overrides.db_endpoint.is_none());
        assert!(overrides.metrics_dir.is_none());
        assert!(overrides.reports_dir.is_none());
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
            config_db_anon_key: None,
            db_anon_key: Some("test-token".to_string()),
            config_db_endpoint: None,
            db_endpoint: Some("https://test.com".to_string()),
            metrics_dir: Some(PathBuf::from("/metrics")),
            reports_dir: Some(PathBuf::from("/reports")),
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.level, Some("debug".to_string()));
        assert_eq!(overrides.file, Some("/tmp/test.log".to_string()));
        assert_eq!(overrides.verbose, Some(true));
        assert_eq!(overrides.db_anon_key, Some("test-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://test.com".to_string()));
        assert_eq!(overrides.metrics_dir, Some("/metrics".to_string()));
        assert_eq!(overrides.reports_dir, Some("/reports".to_string()));
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
            config_db_anon_key: Some("long-token".to_string()),
            db_anon_key: Some("short-token".to_string()),
            config_db_endpoint: Some("https://long.com".to_string()),
            db_endpoint: Some("https://short.com".to_string()),
            metrics_dir: Some(PathBuf::from("/metrics")),
            reports_dir: Some(PathBuf::from("/reports")),
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.db_anon_key, Some("short-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://short.com".to_string()));
        assert_eq!(overrides.metrics_dir, Some("/metrics".to_string()));
        assert_eq!(overrides.reports_dir, Some("/reports".to_string()));
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
            config_db_anon_key: Some("long-token".to_string()),
            db_anon_key: None,
            config_db_endpoint: Some("https://long.com".to_string()),
            db_endpoint: None,
            metrics_dir: None,
            reports_dir: None,
            command: Command::Config { subcommand: None },
        };

        let overrides = cli.to_config_overrides();
        assert_eq!(overrides.db_anon_key, Some("long-token".to_string()));
        assert_eq!(overrides.db_endpoint, Some("https://long.com".to_string()));
        assert!(overrides.metrics_dir.is_none());
        assert!(overrides.reports_dir.is_none());
    }
}
