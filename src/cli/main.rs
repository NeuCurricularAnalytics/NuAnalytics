//! Command-line interface entry point for `NuAnalytics`

mod args;
mod commands;

use args::{Cli, Command, ReportFormatArg};
use clap::Parser;
use logger::{enable_debug, enable_verbose, info, init_file_logging, set_level, warn, Level};
use nu_analytics::config::Config;
use std::path::{Path, PathBuf};

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
            report_dir,
            metrics_dir,
            term_credits,
            no_csv,
            no_report,
        } => {
            run_planner(
                &config,
                &input_files,
                &output,
                report_format,
                report_dir,
                metrics_dir,
                term_credits,
                no_csv,
                no_report,
                verbose,
            );
        }
    }
}

/// Run the planner command with the given arguments
#[allow(clippy::too_many_arguments)]
fn run_planner(
    config: &Config,
    input_files: &[PathBuf],
    output: &[PathBuf],
    report_format: Option<ReportFormatArg>,
    report_dir: Option<PathBuf>,
    metrics_dir: Option<PathBuf>,
    term_credits: Option<f32>,
    no_csv: bool,
    no_report: bool,
    verbose: bool,
) {
    // Apply command-level directory overrides (these take precedence over global flags)
    let effective_metrics_dir = metrics_dir.map_or_else(
        || config.paths.metrics_dir.clone(),
        |p| p.to_string_lossy().to_string(),
    );
    let effective_reports_dir = report_dir.map_or_else(
        || config.paths.reports_dir.clone(),
        |p| p.to_string_lossy().to_string(),
    );

    // Validate output count matches input count if provided
    if !output.is_empty() && output.len() != input_files.len() {
        eprintln!(
            "✗ Output file count ({}) must match input file count ({})",
            output.len(),
            input_files.len()
        );
        return;
    }

    // Process each input file
    for (idx, input_file) in input_files.iter().enumerate() {
        let explicit_output = output.get(idx);

        // Determine what to generate based on -o extension or flags
        let (generate_csv, generate_report, output_path, effective_format) =
            determine_output_type(explicit_output, report_format, no_csv, no_report);

        // Generate CSV metrics
        if generate_csv {
            let csv_output = output_path.clone().filter(|p| {
                p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("csv"))
            });
            commands::planner::run_single(
                input_file,
                csv_output.as_deref(),
                &effective_metrics_dir,
                verbose,
            );
        }

        // Generate report
        if generate_report {
            if let Some(fmt) = effective_format {
                let report_output = output_path.filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .is_some_and(|e| !e.eq_ignore_ascii_case("csv"))
                });
                match commands::report::generate_report_file(
                    input_file,
                    report_output.as_deref(),
                    fmt,
                    &effective_reports_dir,
                    term_credits,
                ) {
                    Ok(path) => {
                        println!("✓ Report generated: {}", path.display());
                    }
                    Err(e) => {
                        eprintln!("{e}");
                    }
                }
            }
        }
    }
}

/// Determine output type based on explicit path or flags
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

/// Handle potential conflict between output extension and --report-format flag
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

fn parse_level(val: &str) -> Option<Level> {
    match val.to_ascii_lowercase().as_str() {
        "error" => Some(Level::Error),
        "warn" => Some(Level::Warn),
        "info" => Some(Level::Info),
        "debug" => Some(Level::Debug),
        _ => None,
    }
}
