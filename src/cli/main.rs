//! Command-line interface entry point for `NuAnalytics`

mod args;
mod commands;

use args::{Cli, Command};
use clap::Parser;
use logger::{enable_debug, enable_verbose, info, init_file_logging, set_level, Level};
use nu_analytics::config::Config;

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
            report,
            term_credits,
            no_csv,
        } => {
            // Run normal planner (CSV export) unless --no-csv is set
            if !no_csv {
                commands::planner::run(&input_files, &output, &config, verbose);
            }

            // Generate report if requested
            if let Some(format) = report {
                let reports_dir = std::path::PathBuf::from(&config.paths.reports_dir);
                if std::fs::create_dir_all(&reports_dir).is_err() {
                    eprintln!(
                        "✗ Failed to create reports directory: {}",
                        reports_dir.display()
                    );
                    return;
                }

                for input_file in &input_files {
                    match commands::report::generate_from_planner(
                        input_file,
                        &reports_dir,
                        &format,
                        term_credits,
                    ) {
                        Ok(report_path) => {
                            println!("✓ Report generated: {}", report_path.display());
                        }
                        Err(e) => {
                            eprintln!("{e}");
                        }
                    }
                }
            }
        }
        Command::Report {
            input_file,
            output,
            format,
            term_credits,
        } => {
            commands::report::run(
                &input_file,
                output.as_deref(),
                &format,
                term_credits,
                &config,
            );
        }
    }
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
