//! Command-line interface entry point for `NuAnalytics`
//!
//! This module provides the main entry point for the CLI application.
//! It handles argument parsing, configuration loading, and command dispatch.

mod args;
mod commands;

use args::{Cli, Command, ReportFormatArg};
use clap::Parser;
use logger::{enable_debug, enable_verbose, info, init_file_logging, set_level, warn, Level};
use nu_analytics::config::Config;
use std::path::{Path, PathBuf};

/// Main entry point for the `NuAnalytics` CLI
///
/// Parses command-line arguments, loads configuration, sets up logging,
/// and dispatches to the appropriate subcommand handler.
fn main() {
    let args = Cli::parse();

    // Load configuration once at startup and apply CLI overrides to it
    let mut config = Config::load();
    let defaults = Config::from_defaults();
    config.apply_overrides(&args.to_config_overrides());

    // Determine effective runtime log level: CLI flag overrides config; otherwise use config logging.level; fallback warn
    let effective_level = args
        .log_level
        .map(std::convert::Into::into)
        .or_else(|| parse_level(&config.logging.level))
        .unwrap_or(Level::Warn);

    let mut level = effective_level;
    if args.debug_flag || level == Level::Debug {
        level = Level::Debug;
        enable_debug();
    }

    // Verbose: enable if CLI flag OR config has verbose=true
    let verbose = args.verbose || config.logging.verbose;
    if verbose {
        enable_verbose();
    }
    set_level(level);

    // Initialize file logging: CLI flag wins, otherwise use config logging.file if set
    let config_log_path: Option<std::path::PathBuf> = if config.logging.file.is_empty() {
        None
    } else {
        Some(std::path::PathBuf::from(&config.logging.file))
    };

    if let Some(log_path) = args.log_file.as_ref().or(config_log_path.as_ref()) {
        let display_path = log_path.to_string_lossy();
        if init_file_logging(log_path) {
            if verbose {
                eprintln!("✓ File logging initialized at: {display_path}");
            } else {
                info!("File logging initialized at: {display_path}");
            }
        } else {
            eprintln!("✗ Failed to initialize file logging at: {display_path}");
        }
    }

    // Handle subcommands
    match args.command {
        Command::Config { subcommand } => {
            commands::config::run(subcommand, &mut config, &defaults);
        }
        Command::Planner {
            input_files,
            output,
            report_format,
            pdf_converter,
            report_dir,
            metrics_dir,
            term_credits,
            no_csv,
            no_report,
        } => {
            let opts = PlannerOptions {
                input_files: &input_files,
                output: &output,
                report_format,
                pdf_converter: pdf_converter.as_deref(),
                report_dir,
                metrics_dir,
                term_credits,
                no_csv,
                no_report,
                verbose,
            };
            run_planner(&config, &opts);
        }
    }
}

/// Options for the planner command
///
/// Collects all planner-related options into a single struct to avoid
/// passing many individual arguments to functions.
struct PlannerOptions<'a> {
    /// Input CSV files to process
    input_files: &'a [PathBuf],
    /// Optional explicit output paths (must match input count)
    output: &'a [PathBuf],
    /// Report format override
    report_format: Option<ReportFormatArg>,
    /// Custom PDF converter command
    pdf_converter: Option<&'a str>,
    /// Override reports output directory
    report_dir: Option<PathBuf>,
    /// Override metrics output directory
    metrics_dir: Option<PathBuf>,
    /// Target credits per term for scheduling
    term_credits: Option<f32>,
    /// Skip CSV metrics export
    no_csv: bool,
    /// Skip report generation
    no_report: bool,
    /// Enable verbose output
    verbose: bool,
}

/// Runs the planner command with the given options
///
/// Processes each input file, generating CSV metrics and/or reports
/// based on the provided options. Output paths are determined by either:
/// - Explicit `-o` arguments (must match input count)
/// - Configured directories with auto-generated filenames
fn run_planner(config: &Config, opts: &PlannerOptions<'_>) {
    // Apply command-level directory overrides (these take precedence over global flags)
    let effective_metrics_dir = opts.metrics_dir.as_ref().map_or_else(
        || config.paths.metrics_dir.clone(),
        |p| p.to_string_lossy().to_string(),
    );
    let effective_reports_dir = opts.report_dir.as_ref().map_or_else(
        || config.paths.reports_dir.clone(),
        |p| p.to_string_lossy().to_string(),
    );

    // Validate output count matches input count if provided
    if !opts.output.is_empty() && opts.output.len() != opts.input_files.len() {
        eprintln!(
            "✗ Output file count ({}) must match input file count ({})",
            opts.output.len(),
            opts.input_files.len()
        );
        return;
    }

    // Process each input file
    for (idx, input_file) in opts.input_files.iter().enumerate() {
        process_single_input(
            input_file,
            opts.output.get(idx),
            opts,
            &effective_metrics_dir,
            &effective_reports_dir,
        );
    }
}

/// Processes a single input file, generating CSV and/or report output
fn process_single_input(
    input_file: &Path,
    explicit_output: Option<&PathBuf>,
    opts: &PlannerOptions<'_>,
    metrics_dir: &str,
    reports_dir: &str,
) {
    // Determine what to generate based on -o extension or flags
    let (generate_csv, generate_report, output_path, effective_format) = determine_output_type(
        explicit_output,
        opts.report_format,
        opts.no_csv,
        opts.no_report,
    );

    // Generate CSV metrics
    if generate_csv {
        let csv_output = output_path.clone().filter(|p| {
            p.extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
        });
        commands::planner::run_single(input_file, csv_output.as_deref(), metrics_dir, opts.verbose);
    }

    // Generate report
    if generate_report {
        if let Some(fmt) = effective_format {
            generate_report_output(input_file, output_path, fmt, reports_dir, opts);
        }
    }
}

/// Generates a report file for the given input
fn generate_report_output(
    input_file: &Path,
    output_path: Option<PathBuf>,
    format: ReportFormatArg,
    reports_dir: &str,
    opts: &PlannerOptions<'_>,
) {
    let report_output = output_path.filter(|p| {
        p.extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| !e.eq_ignore_ascii_case("csv"))
    });

    match commands::report::generate_report_file(
        input_file,
        report_output.as_deref(),
        format,
        reports_dir,
        opts.term_credits,
        opts.pdf_converter,
    ) {
        Ok(path) => {
            println!("✓ Report generated: {}", path.display());
        }
        Err(e) => {
            eprintln!("{e}");
        }
    }
}

/// Determines output type and format based on explicit path or flags
///
/// Logic:
/// - If no explicit output: use configured directories, respect `--no-csv`/`--no-report` flags
/// - If explicit output with `.csv` extension: CSV only
/// - If explicit output with report extension (`.html`, `.md`, `.pdf`): report only
/// - Unknown extension: treat as report with default HTML format
///
/// # Returns
/// Tuple containing: generate CSV flag, generate report flag, output path, format
fn determine_output_type(
    explicit_output: Option<&PathBuf>,
    report_format: Option<ReportFormatArg>,
    no_csv: bool,
    no_report: bool,
) -> (bool, bool, Option<PathBuf>, Option<ReportFormatArg>) {
    explicit_output.map_or_else(
        || {
            // No explicit output - use directories and flags
            let do_csv = !no_csv;
            let do_report = !no_report;
            let fmt = if do_report {
                Some(report_format.unwrap_or(ReportFormatArg::Html))
            } else {
                None
            };
            (do_csv, do_report, None, fmt)
        },
        |out_path| {
            // Explicit output path provided - infer type from extension
            let ext = out_path.extension().and_then(|e| e.to_str()).unwrap_or("");

            if ext.eq_ignore_ascii_case("csv") {
                // CSV output only
                (true, false, Some(out_path.clone()), None)
            } else if let Some(fmt) = ReportFormatArg::from_extension(ext) {
                // Report output only - check for conflict with --report-format
                handle_report_format_conflict(out_path, ext, fmt, report_format)
            } else {
                // Unknown extension - treat as report with default format
                let fmt = report_format.unwrap_or(ReportFormatArg::Html);
                (false, true, Some(out_path.clone()), Some(fmt))
            }
        },
    )
}

/// Handles conflict between output file extension and `--report-format` flag
///
/// When the output path extension (e.g., `.html`) doesn't match the
/// `--report-format` flag (e.g., `pdf`), the flag takes precedence and
/// a warning is printed.
///
/// # Returns
/// Tuple containing: generate CSV flag, generate report flag, output path, format
fn handle_report_format_conflict(
    out_path: &Path,
    ext: &str,
    inferred_fmt: ReportFormatArg,
    cli_format: Option<ReportFormatArg>,
) -> (bool, bool, Option<PathBuf>, Option<ReportFormatArg>) {
    cli_format.map_or_else(
        || (false, true, Some(out_path.to_path_buf()), Some(inferred_fmt)),
        |cli_fmt| {
            if cli_fmt != inferred_fmt {
                warn!(
                    "Output extension .{ext} conflicts with --report-format {cli_fmt}; using --report-format"
                );
                eprintln!(
                    "⚠ Warning: Output extension .{ext} conflicts with --report-format {cli_fmt}; using --report-format"
                );
            }
            (
                false,
                true,
                Some(out_path.with_extension(cli_fmt.extension())),
                Some(cli_fmt),
            )
        },
    )
}

/// Parses a log level string into a `Level` enum
///
/// Supported values (case-insensitive): "error", "warn", "info", "debug"
///
/// # Returns
/// `Some(Level)` if the string is valid, `None` otherwise
fn parse_level(val: &str) -> Option<Level> {
    match val.to_ascii_lowercase().as_str() {
        "error" => Some(Level::Error),
        "warn" => Some(Level::Warn),
        "info" => Some(Level::Info),
        "debug" => Some(Level::Debug),
        _ => None,
    }
}
