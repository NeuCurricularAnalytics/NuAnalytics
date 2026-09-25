//! CLI argument definitions for `NuAnalytics`

use clap::{builder::BoolishValueParser, Parser, Subcommand, ValueEnum};
use std::path::PathBuf;

use nu_analytics::config::ConfigOverrides;
use nu_analytics::logger::Level;

/// Default number of concurrent worker processes for `degree analyze` on a
/// multi-file batch. A small, machine-independent default that keeps memory
/// bounded while still overlapping I/O-bound scrape conversions.
pub const DEFAULT_ANALYZE_JOBS: usize = 8;

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
    /// Stratified - not yet implemented; currently behaves as shuffled
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

/// Subcommands of `nuanalytics degree`.
///
/// `degree` used to be a flat command driven by action flags
/// (`--validate`, `--analyze`, …). It is now a subcommand dispatcher;
/// each action takes its own file list and per-action flags.
/// Serialisation format for a unified degree.
#[derive(Copy, Clone, PartialEq, Eq, clap::ValueEnum, Debug)]
pub enum DegreeFormat {
    /// Unified degree JSON — what the analysis pipeline reads and the database stores.
    Json,
    /// The same degree as YAML, for hand editing.
    Yaml,
}

impl DegreeFormat {
    /// Suffix this format writes, replacing the input file's stem extension.
    #[must_use]
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Json => "unified.json",
            Self::Yaml => "unified.yaml",
        }
    }
}

#[derive(Debug, Subcommand)]
pub enum DegreeSubcommand {
    /// Validate a degree program YAML file's structure, requirements, and
    /// cross-listings.
    Validate {
        /// Degree YAML file(s) to validate. Per-file failures are reported
        /// inline but do not abort the batch.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,
    },

    /// Print the course prerequisite graph for a degree program.
    PrintGraph {
        /// Degree YAML file(s) to print.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,
    },

    /// Run an audit report on a degree program.
    ///
    /// Includes validation, missing prerequisites analysis, and deep
    /// prerequisite chain detection.
    Audit {
        /// Degree YAML file(s) to audit.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,
    },

    /// Run full degree analysis: generate plans, compute metrics, produce
    /// HTML reports with statistics, and export CSV plan files.
    Analyze {
        /// Degree YAML file(s) to analyze. Optional when `--from-db` is given;
        /// otherwise at least one file is required.
        #[arg(value_name = "FILES", num_args = 0..)]
        files: Vec<PathBuf>,

        /// Analyze a stored program's canonical degree fetched from the
        /// database instead of a local file. The value matches a program by
        /// exact `program_key`/`degree_id`, then falls back to a name
        /// substring. Ambiguous matches are listed and the run stops.
        /// Mutually exclusive with positional FILES; single-program only
        /// (no worker pool / `--jobs`).
        #[cfg(feature = "database")]
        #[arg(long, value_name = "NAME")]
        from_db: Option<String>,

        /// Calculation strategy for aggregate metrics (median or mean)
        #[arg(long, value_enum, value_name = "STRATEGY")]
        calc_strategy: Option<CalcStrategyArg>,

        /// Sampling strategy for plan enumeration (sequential, shuffled, stratified).
        /// `stratified` is accepted but not yet implemented and behaves as `shuffled`.
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

        /// Courses to always include in all generated plans (comma-separated).
        ///
        /// These courses are pinned into every plan including the shortest path.
        /// If an included course satisfies a picklist requirement, other options
        /// for that requirement are not considered.
        ///
        /// Example: --include "CS3500,MATH2331,PHIL1145"
        #[arg(long, value_name = "COURSES", value_delimiter = ',')]
        include: Option<Vec<String>>,

        /// Number of files to analyze concurrently, each in its own process so
        /// a pathological degree (e.g. a full-catalog scrape) can't take down
        /// the whole batch. Applies only when multiple files are given; use
        /// `-j 1` to run sequentially in-process with full per-degree output.
        #[arg(short = 'j', long, value_name = "N", default_value_t = DEFAULT_ANALYZE_JOBS)]
        jobs: usize,

        /// Treat all input files as programs of one school and also emit a
        /// combined `<school>_school_report.json` rolling up degree-level
        /// metrics across the programs. The value is the school name.
        #[arg(long, value_name = "NAME")]
        school: Option<String>,

        /// Compute earliest-semester stats for a specific target course and
        /// print them as JSON to stdout. When set alongside `--no-report
        /// --no-csv`, this is the fastest way to query a single course's
        /// first-semester number without generating full reports.
        ///
        /// Example: --target-course CSE475
        #[arg(long, value_name = "COURSE_ID")]
        target_course: Option<String>,

        /// When `--target-course` is set, write the full analysis JSON
        /// (course complexity, plan stats, `target_course_stats`, etc.) to
        /// this path in addition to printing `target_course_stats` to stdout.
        ///
        /// Example: `--metrics-out metrics/tulane-cmps2200.json`
        #[arg(long, value_name = "PATH", requires = "target_course")]
        metrics_out: Option<PathBuf>,
    },

    /// Trim a degree program to a single entry path per course.
    ///
    /// Prerequisite alternatives and `Select` option lists are collapsed to
    /// the shortest shared entry path, except where every alternative belongs
    /// to a protected subject (the degree's `major_subjects`, plus any extra
    /// subjects passed via `--keep-all`).
    ///
    /// Note: comments in the source YAML are not preserved (serializer
    /// limitation).
    ///
    /// # Examples
    /// ```sh
    /// # Default: protect major subjects only, write next to the input
    /// nuanalytics degree trim samples/degrees/neu-khoury-bscs-boston.yaml
    ///
    /// # Also protect MATH alternatives, write to a chosen file
    /// nuanalytics degree trim degree.yaml --keep-all MATH -o degree.trim.yaml
    ///
    /// # Batch with shell wildcards, all outputs into one directory
    /// nuanalytics degree trim samples/degrees/*.yaml -o trimmed/
    ///
    /// # Pin specific picks (overrides shortest-path metric)
    /// nuanalytics degree trim degree.yaml --include "MATH2331,PHIL1145"
    /// ```
    Trim {
        /// Source degree YAML file(s) to trim. Shell wildcards are expanded
        /// by the shell, so `samples/degrees/*.yaml` works.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,

        /// Output destination. Without `-o`, each trimmed file is written
        /// next to its input as `<input-stem>_trimmed.<ext>`. With `-o`,
        /// the value can be either:
        ///
        /// * a **file** path — only valid with a single input; written verbatim.
        /// * a **directory** (existing, or ending with a path separator) —
        ///   each input becomes `<dir>/<input-stem>_trimmed.<ext>`;
        ///   created on demand. Required when multiple inputs are passed.
        ///
        /// The command refuses to overwrite any input file.
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,

        /// Subject prefixes to protect in addition to the degree's
        /// `major_subjects`. Repeatable; also accepts comma-separated values
        /// (e.g. `--keep-all MATH,PHIL`).
        #[arg(long, value_name = "SUBJECT", value_delimiter = ',')]
        keep_all: Vec<String>,

        /// Course keys to pin as winners at any choice point that lists
        /// them. Comma-separated. Same semantics as the `analyze --include`
        /// flag, repurposed for trim.
        #[arg(long, value_name = "COURSES", value_delimiter = ',')]
        include: Option<Vec<String>>,
    },

    /// Convert program file(s) to the unified degree JSON.
    ///
    /// Accepts raw ai-landscape program JSON (auto-detected and converted),
    /// existing unified JSON, or YAML, and emits unified JSON with structured
    /// prerequisites. Data-quality issues (e.g. assumed credits) are reported
    /// and embedded as `conversion_warnings` in the output.
    ///
    /// This converter is transitional — once upstream emits unified JSON
    /// directly it is no longer needed.
    Convert {
        /// Source program file(s). Shell wildcards are expanded by the shell.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,

        /// Output destination. Without `-o`, each file is written next to its
        /// input as `<input-stem>.unified.json`. With `-o`, the value is a
        /// file (single input) or a directory (one output per input).
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,

        /// Pretty-print the JSON output (default is compact, one line).
        #[arg(long)]
        pretty: bool,

        /// Output format. JSON is what the analysis pipeline reads and the database
        /// stores; YAML is the same degree in a form that is easier to hand-edit.
        ///
        /// Input is auto-detected either way, so a degree can be authored in YAML,
        /// converted to JSON for storage, and converted back for editing.
        #[arg(long, value_name = "FORMAT", default_value = "json")]
        format: DegreeFormat,
    },

    /// Print the JSON Schema for the unified degree format.
    ///
    /// Useful for validating unified JSON files in other tools/pipelines.
    /// Writes to `-o <path>` if given, otherwise prints to stdout.
    Schema {
        /// Write the schema to this file instead of stdout.
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,
    },

    /// Normalize degree file(s) to a flat, format-agnostic course set.
    ///
    /// Accepts unified JSON, YAML, or raw ai-landscape cluster pipeline files
    /// (the same three formats as `degree convert`). Each input produces one
    /// `<stem>.normalized.json` file per program with courses keyed by
    /// normalized code and prerequisites in AND-of-OR list form.
    ///
    /// Intended as the common representation for test-suite comparisons between
    /// automated fetchers (e.g. `degree-author` vs. the ai-landscape pipeline).
    ///
    /// # Examples
    /// ```sh
    /// # Normalize a cluster pipeline file (one output per program)
    /// nuanalytics degree normalize Northeastern_University.json -o out/
    ///
    /// # Normalize a unified JSON (single output)
    /// nuanalytics degree normalize neu__bscs.unified.json -o out/
    ///
    /// # Normalize a degree-author YAML
    /// nuanalytics degree normalize neu-khoury-bscs-boston.yaml -o out/
    /// ```
    Normalize {
        /// Source file(s). Shell wildcards are expanded by the shell.
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<PathBuf>,

        /// Output destination. Without `-o`, each normalized file is written
        /// next to its input as `<stem>.normalized.json`. With `-o`, the value
        /// is a file (single non-cluster input only) or a directory.
        #[arg(short, long, value_name = "PATH")]
        out: Option<PathBuf>,

        /// Pretty-print the JSON output (default is compact, one line).
        #[arg(long)]
        pretty: bool,
    },
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
    /// Validate, analyze, audit, trim, or convert a degree program (YAML or JSON).
    ///
    /// Dispatches to one of several actions via subcommand. Inputs may be YAML
    /// or unified-JSON degree files (raw ai-landscape JSON is auto-converted).
    /// Circular prerequisites are automatically broken by removing optional
    /// edges to create a valid DAG for analysis.
    ///
    /// # Examples
    /// ```sh
    /// # Validate one or more files
    /// nuanalytics degree validate samples/degrees/neu-khoury-bscs-boston.yaml
    ///
    /// # Run full plan-enumeration analysis
    /// nuanalytics degree analyze samples/degrees/csu-cs-bscs-general.yaml
    ///
    /// # Trim alternatives down to a single shared shortest path
    /// nuanalytics degree trim samples/degrees/neu-khoury-bscs-boston.yaml
    ///
    /// # Convert ai-landscape program JSON to unified JSON (glob into a dir)
    /// nuanalytics degree convert ai-landscape-tools/validation_jsons/*.json -o converted/
    ///
    /// # Emit the unified-degree JSON Schema
    /// nuanalytics degree schema -o degree.schema.json
    /// ```
    Degree {
        #[command(subcommand)]
        subcommand: DegreeSubcommand,
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

        /// Overwrite existing files in `<DIR>`. Without this, init aborts if
        /// any target file already exists.
        #[arg(long)]
        force: bool,
    },
}

/// Database management subcommands
#[cfg(feature = "database")]
#[derive(Debug, Subcommand)]
pub enum DbSubcommand {
    /// Sign in to Supabase and save the session for database operations.
    ///
    /// Two ways in. OAuth (the default) opens your browser to authenticate with the
    /// chosen provider, then redirects back to a temporary local server; the provider
    /// must be enabled in the project under Authentication → Providers, which on a
    /// self-hosted stack means registering an OAuth application first. A password sign-in
    /// (`--email`) needs no provider at all, so it is the way in to a stack that has not
    /// had one set up — the password is prompted for, never passed as an argument.
    ///
    /// Neither path creates an account: the user must already exist on the backend, so
    /// `--email` does not require signup to be enabled.
    ///
    /// Requires `database.endpoint` and `database.anon_key` to be set in config.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db login                          # OAuth via GitHub
    /// nuanalytics db login --provider google
    /// nuanalytics db login --email you@example.edu   # prompts for a password
    /// ```
    Login {
        /// OAuth provider to use (github, google, gitlab, discord, azure, ...). Default: github.
        #[arg(long, value_name = "PROVIDER", conflicts_with = "email")]
        provider: Option<String>,
        /// Sign in with this email address and a prompted password instead of OAuth.
        #[arg(long, value_name = "ADDRESS")]
        email: Option<String>,
    },
    /// Remove the locally saved session token. Revokes nothing server-side.
    ///
    /// This deletes the auth file and nothing else. The access token stays valid at the
    /// backend until it expires (up to an hour), and the refresh token is not revoked, so
    /// this is not an offboarding step. To remove someone's access, delete their
    /// `auth.users` row on the backend.
    Logout,
    /// Show the currently signed-in user (if any).
    Whoami,
    /// Execute an SQL file via the Supabase Management API. Cloud projects only.
    ///
    /// This is a Supabase-cloud path. A self-hosted stack has no Management API and no
    /// project ref — apply SQL to it directly instead (`psql -f <file>`).
    ///
    /// Requires two settings, both explicit:
    ///
    /// ```sh
    /// nuanalytics config set database.project_ref <ref>
    /// nuanalytics config set database.management_key <pat>
    /// ```
    ///
    /// `project_ref` is not derived from `database.endpoint`: a custom-domain cloud
    /// project has no `.supabase.co` in its URL, and a self-hosted host would otherwise
    /// yield a meaningless ref. Get a PAT at
    /// <https://app.supabase.com/account/tokens>; it gives DDL access (CREATE TABLE,
    /// INSERT, ...) which the project anon key cannot do.
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
    /// Report the configured backend, which config file supplied it, session validity,
    /// and whether an authenticated read succeeds. Exits 1 when the read fails.
    Status,
    /// Apply the schema and seed files a deployment needs, in the order they require.
    ///
    /// The ordering is the part that goes wrong: the seed files insert into tables the
    /// schema files create, so running them out of sequence fails on a missing relation.
    ///
    /// How the SQL reaches the database depends on the deployment. A Supabase cloud
    /// project has a Management API that accepts SQL, so `db bootstrap` can apply the
    /// files itself once `database.project_ref` and `database.management_key` are set. A
    /// self-hosted stack has neither, and this tool speaks only `PostgREST` over HTTP — so
    /// for those, `--print` emits every file in order for you to pipe wherever you like.
    ///
    /// Every object uses `IF NOT EXISTS` and policies are dropped before being recreated,
    /// so both paths are safe to re-run against a live database.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db bootstrap --print | psql "$DATABASE_URL"   # any deployment
    /// nuanalytics db bootstrap --print > schema.sql             # review it first
    /// nuanalytics db bootstrap                                  # Supabase cloud only
    /// ```
    Bootstrap {
        /// Write the combined SQL to stdout instead of applying it. Works for any
        /// deployment and makes no network calls.
        #[arg(long)]
        print: bool,
    },
    /// Check the stored data against the IPEDS file it was imported from.
    ///
    /// `db doctor` asks whether the deployment is set up correctly; this asks whether
    /// what is in it is *right*. Two checks run:
    ///
    /// - **Fidelity** — every column compared against the survey file, with mismatch
    ///   counts and examples. Catches a stale year, a partial import, or rows that never
    ///   landed. It parses the file with the importer's own code, so it cannot detect a
    ///   parsing defect — both sides would share it.
    /// - **Provenance** — for measures IPEDS ships under several column names in the
    ///   same file, which one the stored data actually agrees with. Pick the wrong
    ///   vintage and every row count still ties; only the meaning is wrong. This is the
    ///   check a count comparison cannot make.
    ///
    /// The year is taken from `--year` and used only to build comparable rows; it does
    /// not filter the backend.
    ///
    /// Exits 1 if any column disagrees or the data came from an unintended column.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db validate ~/Downloads/HD2025.zip --year 2025
    /// nuanalytics db validate ./hd2025.csv --year 2025
    /// ```
    Validate {
        /// IPEDS survey file to compare against — `.csv` or `.zip`.
        #[arg(value_name = "FILE")]
        file: std::path::PathBuf,
        /// Survey year the file is from.
        #[arg(long, value_name = "YEAR")]
        year: u16,
    },
    /// Drop old analysis runs, keeping a bounded history per program.
    ///
    /// Runs accumulate — every re-analysis appends rather than replacing, which is what
    /// lets you compare a metric across analyzer versions. This bounds that.
    ///
    /// History is kept per **program and variant**: `full` and `trimmed` are different
    /// analyses of one degree, not competing versions of it, so a burst of `full`
    /// re-runs never evicts a degree's only `trimmed` run.
    ///
    /// Always reports what it would remove before removing it; pass `--dry-run` to stop
    /// there.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db prune --keep 3 --dry-run
    /// nuanalytics db prune --keep 3
    /// nuanalytics db prune --analyzer-version 0.5.3
    /// ```
    Prune {
        /// Keep this many newest runs per program and variant; delete older ones.
        #[arg(long, value_name = "N", conflicts_with = "analyzer_version")]
        keep: Option<usize>,

        /// Delete every run produced by this analyzer version, regardless of age.
        ///
        /// A correctness decision rather than a retention policy — "that release
        /// computed the metric wrongly" — so it is not softened by a keep floor and may
        /// empty a program's history.
        #[arg(long, value_name = "VERSION")]
        analyzer_version: Option<String>,

        /// Report what would be deleted and stop.
        #[arg(long)]
        dry_run: bool,
    },
    /// Diagnose a whole deployment: config source, reachability, RLS behaviour, session,
    /// schema completeness and seed data.
    ///
    /// Written for somebody who did not set the backend up. Cloud and self-hosted should
    /// behave identically; the one real difference is whether the schema and seed files
    /// were applied, which this reports table by table. Exits 1 on any hard failure —
    /// missing seed data is a warning, not a failure.
    Doctor,
    /// Read the database: schools, degrees, analysis metrics, IPEDS demographics, CIP codes, or the lookup tables.
    ///
    /// Read-only. Results print as JSON by default because the usual caller is a script
    /// or an LLM; `--format table` is for reading in a terminal.
    ///
    /// The filters are deliberately a small set, for quick questions — "which schools in
    /// Hawaii", "what degrees does this one have".
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db query schools --state HI
    /// nuanalytics db query schools --name hawaii --format table
    /// nuanalytics db query degrees --school 141574
    /// nuanalytics db query metrics --degree <PROGRAM_KEY> --variant trimmed
    /// nuanalytics db query demographics --school 141574 --cip 11.
    /// nuanalytics db query cip --search computer
    /// nuanalytics db query lookup --table carnegie_class
    /// ```
    Query {
        #[command(subcommand)]
        subcommand: QuerySubcommand,
        /// Output format: `json` (default, machine-readable) or `table`.
        #[arg(long, value_enum, default_value = "json", global = true)]
        format: crate::output::OutputFormat,
    },
    /// Import IPEDS data from locally downloaded CSV or ZIP files into Supabase.
    ///
    /// Only two files are needed — the completions file is used in a single pass to
    /// populate both the `completions` table (every row of the file: all CIP codes and
    /// both major numbers, roughly 313,000 rows per year) and the
    /// `institution_completions` table (all-major totals used for representation ratios).
    ///
    /// That per-year magnitude is worth knowing before writing a query against it: it is
    /// what makes a `PGRST_DB_MAX_ROWS` cap bite, and `db doctor`'s row-limit check is
    /// what detects it.
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
        /// Import an HD file older than the data already stored.
        ///
        /// Institutions are keyed on `unitid` with no year dimension, so the last HD
        /// import wins outright. Importing an older year over a newer one silently
        /// replaces current attributes with stale ones; this is refused unless you say
        /// you mean it. Completions are unaffected — `year` is part of their key.
        #[arg(long)]
        force: bool,
        /// Academic year for the data (e.g. 2023 for 2023-2024 data)
        #[arg(long, default_value = "2023")]
        year: u16,
    },
    /// Import degree analysis report(s) into the normalized program tables.
    ///
    /// Each input is a degree-first analysis report (`*_report.json`) or a plain
    /// unified degree (JSON/YAML). The shared import core parses the document,
    /// resolves its institution against IPEDS, and upserts the program projection
    /// (`programs`, `courses`, `program_courses`, `program_requirements`) plus, when
    /// the report carries an `analysis` block, one analysis run with its course
    /// metrics and selected plans.
    ///
    /// Each positional argument may be a file or a directory; a directory is
    /// expanded to its `*_report.json` files (falling back to `*.json` when none
    /// match). Overwriting an existing program requires `--replace` (unverified) or
    /// `--force` (verified) — that explicit flag is the double-check, so there is no
    /// interactive prompt. `--dry-run` reports the row counts without writing.
    ///
    /// Examples:
    /// ```sh
    /// nuanalytics db import metrics/neu-khoury-bscs-boston_report.json
    /// nuanalytics db import metrics/ --dry-run
    /// nuanalytics db import report.json --unitid 167358 --replace
    /// ```
    Import {
        /// Degree report / unified degree file(s) and/or directories. A directory
        /// is expanded to its `*_report.json` files (fallback `*.json`).
        #[arg(value_name = "FILES", num_args = 1..)]
        files: Vec<std::path::PathBuf>,

        /// Analysis-run variant label. `full` (default) writes the program
        /// projection; a non-`full` variant only attaches an analysis run.
        #[arg(long, value_name = "NAME")]
        variant: Option<String>,

        /// Override the resolved institution IPEDS unit id.
        #[arg(long, value_name = "N")]
        unitid: Option<i32>,

        /// Override the institution name used for resolution.
        #[arg(long, value_name = "NAME")]
        institution: Option<String>,

        /// Override the CIP code (part of the natural program key).
        #[arg(long, value_name = "CIP")]
        cip: Option<String>,

        /// Override the catalog year (part of the program identity).
        #[arg(long, value_name = "YEAR")]
        catalog: Option<String>,

        /// Override the degree id.
        #[arg(long = "degree-id", value_name = "ID")]
        degree_id: Option<String>,

        /// Overwrite a verified program / skip confirmation.
        #[arg(long)]
        force: bool,

        /// Replace an existing (unverified) program.
        #[arg(long)]
        replace: bool,

        /// Skip the program entirely if it already exists.
        #[arg(long = "skip-existing")]
        skip_existing: bool,

        /// Build the plan and report counts, but write nothing.
        #[arg(long = "dry-run")]
        dry_run: bool,

        /// Number of files to process concurrently. v1 always runs sequentially;
        /// reserved for a future worker pool.
        #[arg(short = 'j', long, value_name = "N", default_value_t = 1)]
        jobs: usize,
    },
}

/// Filters for `db query demographics`.
///
/// Its own struct rather than inline variant fields so the dispatch stays a single call —
/// the command fans out to three different engines depending on `group_by`.
#[derive(clap::Args, Clone, Debug)]
pub struct DemographicsArgs {
    /// IPEDS unitid. Required for `--group-by cip`; narrows the others to one school.
    #[arg(long, value_name = "UNITID")]
    pub school: Option<i32>,
    /// CIP code prefix, e.g. `11.` for all computing, `11.07` for computer science.
    #[arg(long, value_name = "PREFIX")]
    pub cip: Option<String>,
    /// Exact CIP codes, comma-separated. Takes priority over `--cip`.
    #[arg(long = "cip-codes", value_name = "LIST")]
    pub cip_codes: Option<String>,
    /// Academic year, e.g. 2024. Defaults to the most recent year stored.
    #[arg(long, value_name = "YEAR")]
    pub year: Option<i32>,
    /// Award level: 3 associate, 5 bachelors, 7 masters, 9 doctoral. Omit for all.
    #[arg(long = "award-level", value_name = "N")]
    pub award_level: Option<i32>,
    /// Two-letter state code. Refused with `--group-by cip`, which is one school.
    #[arg(long, value_name = "CODE")]
    pub state: Option<String>,
    /// Control code: 1 public, 2 private non-profit, 3 private for-profit.
    #[arg(long, value_name = "N")]
    pub control: Option<i32>,
    /// Carnegie classification code, e.g. 15 for R1.
    #[arg(long = "carnegie-class", value_name = "N")]
    pub carnegie_class: Option<i32>,
    /// Only historically Black colleges and universities. Needs `--group-by school`.
    #[arg(long)]
    pub hbcu: bool,
    /// Only tribal colleges. Needs `--group-by school`.
    #[arg(long)]
    pub tribal: bool,
    /// Shape of the result: grouped by demographic only, by school, or by CIP code.
    #[arg(long = "group-by", value_enum, default_value = "total")]
    pub group_by: DemographicsGrouping,
    /// Counts only — skips the baseline query, so the ratio fields come back null
    /// (rendered blank by `--format table`).
    #[arg(long)]
    pub raw: bool,
    /// Maximum schools returned by `--group-by school` (default 50, max 200).
    #[arg(long, value_name = "N")]
    pub limit: Option<usize>,
}

/// How `db query demographics` should group its rows.
#[derive(clap::ValueEnum, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DemographicsGrouping {
    /// One row per race/gender group, aggregated across every matched institution.
    Total,
    /// One row per institution.
    School,
    /// One row per CIP code at a single school. Requires `--school`.
    Cip,
}

// Only the `db query demographics` dispatch uses this, and that is `database`-gated.
#[cfg(feature = "database")]
impl DemographicsGrouping {
    /// The `--group-by` value that selects this variant, for use in error messages.
    ///
    /// Written out rather than read from `to_possible_value()`, which borrows from a
    /// temporary and so cannot yield `&'static str` without leaking. The duplication is
    /// held honest by `test_grouping_as_str_matches_the_accepted_flag_values`: an error
    /// must never name a value clap would reject.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Total => "total",
            Self::School => "school",
            Self::Cip => "cip",
        }
    }
}

/// What to read. Each variant maps onto one query engine in `nu_analytics::core::query`.
/// Every engine except `metrics` is also an MCP tool, so the two front ends return the
/// same shapes.
#[derive(Subcommand, Debug, Clone)]
pub enum QuerySubcommand {
    /// List institutions, optionally filtered.
    ///
    /// `--name` is always a case-insensitive substring match: `hawaii` finds
    /// "University of Hawaii at Manoa". There is no exact or anchored form — the
    /// engine wraps the value in wildcards either way.
    Schools {
        /// Institution name — case-insensitive substring, e.g. `hawaii`.
        #[arg(long, value_name = "TEXT")]
        name: Option<String>,
        /// Two-letter state code (e.g. `HI`).
        #[arg(long, value_name = "CODE")]
        state: Option<String>,
        /// Carnegie classification code. `db query lookup --table carnegie_class` lists them.
        #[arg(long, value_name = "N")]
        carnegie_class: Option<i32>,
        /// Control code: 1 public, 2 private non-profit, 3 private for-profit.
        #[arg(long, value_name = "N")]
        control: Option<i32>,
        /// Only historically Black colleges and universities.
        #[arg(long)]
        hbcu: bool,
        /// Only tribal colleges.
        #[arg(long)]
        tribal: bool,
        /// Maximum rows (engine default 25, capped at 100).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// List stored degree programs, optionally filtered.
    Degrees {
        /// IPEDS unitid of the institution.
        #[arg(long, value_name = "UNITID")]
        school: Option<i32>,
        /// CIP code prefix, e.g. `11.` for computing or `11.07` for computer science.
        #[arg(long, value_name = "PREFIX")]
        cip: Option<String>,
        /// Catalog year, e.g. `2024-2025`.
        #[arg(long, value_name = "YEAR")]
        catalog_year: Option<String>,
        /// Normalized degree type, e.g. `BS`, `BA`, `MINOR`.
        #[arg(long, value_name = "CODE")]
        degree_type: Option<String>,
        /// Program kind, e.g. `major`, `minor`, `concentration`, `certificate`.
        #[arg(long, value_name = "KIND")]
        kind: Option<String>,
        /// Maximum rows (engine default 20, capped at 50).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// IPEDS completion demographics: who earns degrees, by race and gender.
    ///
    /// Reports raw counts and a representation ratio, where 1.0 is parity. `--raw` drops
    /// the ratio and leaves the counts.
    ///
    /// The ratio's baseline always comes from one table, `institution_completion_totals`:
    /// the group's share of all-major completions. This database holds no enrolment data
    /// at all, so a ratio below 1.0 means "under-represented among these graduates
    /// relative to all graduates there", never anything about who enrolled. Note the
    /// output columns are called `enrolled`, `total_enrolled` and `enrollment_pct` for
    /// historical reasons; they hold completions. `total` pools that denominator across
    /// every matched institution, while `school` and `cip` use each school's own.
    ///
    /// `--group-by` picks what a row is: `total` gives one row per race/gender group
    /// aggregated over everything matched, `school` one row per institution, and `cip`
    /// one row per CIP code at a single school. The three read different engines and
    /// accept different filters, so a filter the chosen grouping cannot apply is refused
    /// by name rather than silently ignored.
    Demographics(DemographicsArgs),
    /// Show stored analysis metrics for one degree program.
    ///
    /// Runs append — re-importing a program adds a row rather than replacing one, so a
    /// program accumulates runs across analyzer versions. Only the newest run per variant
    /// is shown unless `--all` is given.
    Metrics {
        /// Program key (exact) or degree id. `db query degrees` lists both.
        #[arg(long, value_name = "KEY")]
        degree: String,
        /// Restrict to one variant, e.g. `full` or `trimmed`. Omit for every variant.
        #[arg(long, value_name = "NAME")]
        variant: Option<String>,
        /// Show every stored run, not just the newest per variant.
        #[arg(long)]
        all: bool,
        /// Maximum runs to read back (engine default 50, capped at 200).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Search the CIP code catalogue.
    Cip {
        /// Match against the CIP title, wildcard.
        #[arg(long, value_name = "TEXT")]
        search: Option<String>,
        /// CIP code prefix, e.g. `11.`.
        #[arg(long, value_name = "PREFIX")]
        prefix: Option<String>,
        /// Maximum rows (engine default 25, capped at 100).
        #[arg(long, value_name = "N")]
        limit: Option<usize>,
    },
    /// Dump one of the IPEDS lookup tables (what the numeric codes mean).
    Lookup {
        /// One of: `award_levels`, `carnegie_class`, `institution_control`,
        /// `institution_level`, `institution_sector`, `institution_locale`,
        /// `institution_size`.
        #[arg(long, value_name = "TABLE")]
        table: String,
    },
}

#[derive(Parser, Debug)]
#[command(
    name = "nuanalytics",
    about = "NuAnalytics command-line interface",
    version = env!("CARGO_PKG_VERSION")
)]
/// Top-level `nuanalytics` invocation: the global flags plus the chosen subcommand.
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
    #[cfg(feature = "database")]
    use super::DemographicsGrouping;

    #[test]
    #[cfg(feature = "database")]
    fn test_grouping_as_str_matches_the_accepted_flag_values() {
        // `as_str` is duplicated from the ValueEnum by necessity. If a variant is
        // renamed and only one side is updated, an error message would tell the user to
        // pass a value clap rejects.
        use clap::ValueEnum as _;
        for g in DemographicsGrouping::value_variants() {
            let accepted = g
                .to_possible_value()
                .expect("variant is a clap value")
                .get_name()
                .to_string();
            assert_eq!(g.as_str(), accepted, "{g:?} names a value clap rejects");
        }
    }

    #[test]
    fn test_cli_command_tree_is_well_formed() {
        // Clap validates the tree when the command is built and *panics* rather than
        // returning an error, so a duplicate long name or a misplaced `global` ships as
        // a panic on every invocation instead of a test failure.
        use clap::CommandFactory as _;
        Cli::command().debug_assert();
    }

    #[test]
    #[cfg(feature = "database")]
    fn test_every_query_subcommand_is_reachable_from_the_cli() {
        // Guards against adding a QuerySubcommand variant and forgetting to expose it.
        use clap::CommandFactory as _;
        let cmd = Cli::command();
        let query = cmd
            .get_subcommands()
            .find(|c| c.get_name() == "db")
            .expect("db")
            .get_subcommands()
            .find(|c| c.get_name() == "query")
            .expect("query");
        let names: Vec<&str> = query
            .get_subcommands()
            .map(clap::Command::get_name)
            .collect();
        for expected in [
            "schools",
            "degrees",
            "metrics",
            "demographics",
            "cip",
            "lookup",
        ] {
            assert!(
                names.contains(&expected),
                "{expected} missing from {names:?}"
            );
        }
    }

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
