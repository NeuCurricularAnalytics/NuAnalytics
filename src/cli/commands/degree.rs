//! Degree command handler for validating degree program YAML files

use crate::args::DegreeFormat;

use nu_analytics::config::Config;
use nu_analytics::core::degree::audit::{
    detect_lowest_course_level, find_deep_chains, find_upper_level_without_prereqs,
};
use nu_analytics::core::degree::ValidationOptions;
use nu_analytics::core::degree::{
    load_degree_from_json, load_degree_from_yaml, DegreeParseError, PlanGeneratorConfig,
    PlanValidator, PlanValidatorConfig, SamplingStrategy,
};
use nu_analytics::core::models::{CourseGraph, School};
use nu_analytics::core::report::degree_report::{DegreeReportContext, DegreeReportGenerator};
use nu_analytics::core::report::plan_export::{
    export_degree_summary_jsonl, export_index_csv, export_selected_plans, PlanExportConfig,
};
use nu_analytics::core::statistics::aggregator::MetricsAggregator;
use nu_analytics::core::{validate_degree_program, validate_degree_program_with_options};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process;

/// Validate a degree program YAML file
///
/// Loads the degree program from the specified YAML file and runs comprehensive
/// validation checks including:
/// - Course definitions and references
/// - Prerequisite chains and circular dependencies
/// - Requirement structures and course lists
/// - Cross-listing bidirectionality
/// - Bundle and equivalent course syntax
///
/// # Arguments
/// * `degree_path` - Path to the degree program YAML file
/// * `opts` - Validation options, such as allowing unmatched patterns
/// * `verbose` - Whether to print verbose output
///
/// # Returns
/// Returns `Ok(())` if validation succeeds, `Err(String)` with error message if it fails
pub fn validate_degree(
    degree_path: &Path,
    opts: ValidationOptions,
    verbose: bool,
) -> Result<(), String> {
    if verbose {
        eprintln!("Loading degree program from: {}", degree_path.display());
    }

    // Load the degree program
    let program = load_degree_auto(degree_path).map_err(|e| {
        format!(
            "Failed to load degree program from {}: {}",
            degree_path.display(),
            e
        )
    })?;

    if verbose {
        eprintln!("✓ Successfully loaded degree program");
        eprintln!(
            "  Degree: {} {}",
            program.degree.degree_type, program.degree.name
        );
        eprintln!("  System: {}", program.degree.system_type);
        if let Some(credits) = program.degree.total_credits {
            eprintln!("  Total Credits: {credits}");
        }
        eprintln!("  Courses: {}", program.courses.len());
        eprintln!("  Requirements: {}", program.requirements.len());
        eprintln!();
        eprintln!("Running validation checks...");
    }

    let result = validate_degree_program_with_options(&program, opts);

    // Print the validation report
    println!("{}", result.format_report());

    // Return error if there are validation errors
    if !result.errors.is_empty() {
        Err("Validation failed with errors".to_string())
    } else if verbose && !result.warnings.is_empty() {
        eprintln!("\n⚠ Validation passed with warnings");
        Ok(())
    } else if verbose {
        eprintln!("\n✓ Validation passed successfully");
        Ok(())
    } else {
        Ok(())
    }
}

/// Print the course prerequisite graph for a degree program
///
/// Builds and displays the course graph showing all prerequisite relationships.
/// Uses an association list format for easy readability.
///
/// # Arguments
/// * `degree_path` - Path to the degree program YAML file
/// * `verbose` - Whether to print verbose output
///
/// # Returns
/// Returns `Ok(())` on success, `Err(String)` with error message on failure
pub fn print_graph(degree_path: &Path, verbose: bool) -> Result<(), String> {
    // Load and build graph
    let (program, result) = load_and_build_graph(degree_path, verbose)?;

    // Print all sections
    print_graph_header(&program, &result);
    print_graph_issues(&result);
    print_graph_statistics(&result);
    print_prerequisite_map(&result);

    if verbose {
        eprintln!("\n✓ Graph printed successfully");
    }

    Ok(())
}
/// Load degree program and build course graph
fn load_and_build_graph(
    degree_path: &Path,
    verbose: bool,
) -> Result<
    (
        nu_analytics::core::DegreeProgram,
        nu_analytics::core::models::CourseGraphResult,
    ),
    String,
> {
    if verbose {
        eprintln!("Loading degree program from: {}", degree_path.display());
    }

    let program = load_degree_auto(degree_path).map_err(|e| {
        format!(
            "Failed to load degree program from {}: {}",
            degree_path.display(),
            e
        )
    })?;

    if verbose {
        eprintln!("✓ Successfully loaded degree program");
        eprintln!(
            "  Degree: {} {}",
            program.degree.degree_type, program.degree.name
        );
        eprintln!("  Courses: {}", program.courses.len());
        eprintln!();
        eprintln!("Building course graph...");
    }

    let result = CourseGraph::from_degree_program(&program);
    Ok((program, result))
}

/// Print graph header with basic information
fn print_graph_header(
    program: &nu_analytics::core::DegreeProgram,
    result: &nu_analytics::core::models::CourseGraphResult,
) {
    println!("Course Prerequisite Graph");
    println!("=========================");
    println!(
        "Degree: {} {}",
        program.degree.degree_type, program.degree.name
    );
    if let Some(institution) = &program.degree.institution {
        println!("Institution: {institution}");
    }
    println!("Total Courses: {}", result.graph.len());
    println!();
}

/// Print graph issues (cycles and missing courses)
fn print_graph_issues(result: &nu_analytics::core::models::CourseGraphResult) {
    if !result.cycles.is_empty() {
        println!("⚠ Circular Prerequisites Detected:");
        // `detect_cycles` closes each path, repeating its start at the end.
        for cycle in &result.cycles {
            println!("  {}", cycle.join(" → "));
        }
        println!();
    }

    if !result.missing_courses.is_empty() {
        let mut missing = result.missing_courses.clone();
        missing.sort();
        println!("⚠ Missing Courses (referenced but not defined):");
        for course in &missing {
            println!("  {course}");
        }
        println!();
    }
}

/// Print graph statistics
fn print_graph_statistics(result: &nu_analytics::core::models::CourseGraphResult) {
    let leaves = result.graph.leaf_courses();
    let terminals = result.graph.terminal_courses();
    println!("Graph Statistics:");
    println!("  Entry Points (no prerequisites): {}", leaves.len());
    println!("  Terminal Courses (no dependents): {}", terminals.len());
    if !result.graph.has_cycles() {
        if let Some(order) = result.graph.topological_order() {
            println!("  Topological Levels: {} courses in order", order.len());
        }
    }
    println!();
}

/// Print the prerequisite map as an association list
fn print_prerequisite_map(result: &nu_analytics::core::models::CourseGraphResult) {
    println!("Prerequisite Map (course → prerequisites):");
    println!("------------------------------------------");

    let mut keys: Vec<&str> = result.graph.course_keys();
    keys.sort_unstable();

    for key in keys {
        if let Some(node) = result.graph.get(key) {
            print_course_prerequisites(key, node);
        }
    }
}

/// Print prerequisites for a single course
fn print_course_prerequisites(key: &str, node: &nu_analytics::core::models::CourseNode) {
    let mut parts = Vec::new();

    let prereq_str = node.format_prerequisite_paths();
    if !prereq_str.is_empty() {
        parts.push(prereq_str);
    }

    let coreqs: Vec<&str> = node.corequisites();
    if !coreqs.is_empty() {
        parts.push(format!("co: {}", coreqs.join(", ")));
    }

    if parts.is_empty() {
        println!("  {key} → (none)");
    } else {
        println!("  {key} → {}", parts.join(" + "));
    }
}

/// Run an audit report on a degree program
///
/// The audit includes:
/// 1. Validation report (errors and warnings)
/// 2. Upper-level courses missing prerequisites (courses above lowest level without prereqs)
/// 3. Courses with deep prerequisite chains (above configurable threshold)
///
/// # Arguments
/// * `degree_path` - Path to the degree program YAML file
/// * `config` - Configuration containing audit thresholds
/// * `verbose` - Whether to print verbose output
///
/// # Returns
/// Returns `Ok(())` on success, `Err(String)` with error message on failure
pub fn audit_degree(degree_path: &Path, config: &Config, verbose: bool) -> Result<(), String> {
    if verbose {
        eprintln!("Loading degree program from: {}", degree_path.display());
    }

    // Load the degree program
    let program = load_degree_auto(degree_path).map_err(|e| {
        format!(
            "Failed to load degree program from {}: {}",
            degree_path.display(),
            e
        )
    })?;

    if verbose {
        eprintln!("✓ Successfully loaded degree program");
        eprintln!(
            "  Degree: {} {}",
            program.degree.degree_type, program.degree.name
        );
        eprintln!("  Courses: {}", program.courses.len());
        eprintln!();
    }

    // Print header
    print_audit_header(&program);

    // Section 1: Validation Report
    let validation_result = print_validation_section(&program);

    // Build the course graph for analysis
    let graph_result = CourseGraph::from_degree_program(&program);

    // Section 2: Upper-level courses missing prerequisites
    let missing_prereqs = print_missing_prereqs_section(&program, &graph_result);

    // Section 3: Deep prerequisite chains
    let threshold = config.audit.prerequisite_chain_threshold;
    let deep_chains = print_deep_chains_section(&program, &graph_result, threshold, verbose);

    // Summary
    print_audit_summary(
        &validation_result,
        &missing_prereqs,
        &deep_chains,
        threshold,
    );

    if verbose {
        eprintln!("\n✓ Audit completed successfully");
    }

    // Return error if there are validation errors
    if validation_result.errors.is_empty() {
        Ok(())
    } else {
        Err("Audit found validation errors".to_string())
    }
}

/// Print the audit report header
fn print_audit_header(program: &nu_analytics::core::DegreeProgram) {
    println!("Degree Audit Report");
    println!("===================");
    println!(
        "Degree: {} {}",
        program.degree.degree_type, program.degree.name
    );
    if let Some(institution) = &program.degree.institution {
        println!("Institution: {institution}");
    }
    println!("Total Courses: {}", program.courses.len());
    println!();
}

/// Print the validation section and return the result
fn print_validation_section(
    program: &nu_analytics::core::DegreeProgram,
) -> nu_analytics::core::ValidationResult {
    println!("1. Validation Report");
    println!("--------------------");
    let validation_result = validate_degree_program(program);
    println!("{}", validation_result.format_report());
    println!();
    validation_result
}

/// Print the missing prerequisites section and return the list
fn print_missing_prereqs_section(
    program: &nu_analytics::core::DegreeProgram,
    graph_result: &nu_analytics::core::models::CourseGraphResult,
) -> Vec<(String, u32)> {
    println!("2. Upper-Level Courses Missing Prerequisites");
    println!("--------------------------------------------");

    let lowest_level = detect_lowest_course_level(program);
    let missing_prereqs = find_upper_level_without_prereqs(graph_result, lowest_level);

    if missing_prereqs.is_empty() {
        println!("✓ All upper-level courses have prerequisites defined.");
    } else {
        println!(
            "⚠ Found {} upper-level course(s) without prerequisites:",
            missing_prereqs.len()
        );
        println!("  (Lowest course level detected: {lowest_level})");
        println!();
        for (course, level) in &missing_prereqs {
            println!("  • {course} (level {level})");
        }
    }
    println!();
    missing_prereqs
}

/// Print the deep chains section and return the list
fn print_deep_chains_section(
    program: &nu_analytics::core::DegreeProgram,
    graph_result: &nu_analytics::core::models::CourseGraphResult,
    threshold: usize,
    verbose: bool,
) -> Vec<(String, usize, String)> {
    println!("3. Deep Prerequisite Chains");
    println!("---------------------------");

    let deep_chains = find_deep_chains(program, graph_result, threshold);

    if deep_chains.is_empty() {
        println!("✓ No major courses have prerequisite chains >= {threshold} courses.");
    } else {
        println!(
            "⚠ Found {} major course(s) with prerequisite chains >= {threshold}:",
            deep_chains.len()
        );
        println!();
        for entry in &deep_chains {
            println!("  • {} (chains: {})", entry.course, entry.branch_lengths);
            if verbose {
                println!("    Chain: {}", entry.chain);
            }
        }
    }
    println!();
    deep_chains
        .into_iter()
        .map(|entry| {
            let max_len = entry
                .branch_lengths
                .split(", ")
                .filter_map(|n| n.parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            (entry.course, max_len, entry.chain)
        })
        .collect()
}

/// Print the audit summary
fn print_audit_summary(
    validation_result: &nu_analytics::core::ValidationResult,
    missing_prereqs: &[(String, u32)],
    deep_chains: &[(String, usize, String)],
    threshold: usize,
) {
    println!("Audit Summary");
    println!("-------------");
    let error_count = validation_result.errors.len();
    let warning_count = validation_result.warnings.len();
    let missing_prereq_count = missing_prereqs.len();
    let deep_chain_count = deep_chains.len();

    if error_count == 0 && missing_prereq_count == 0 && deep_chain_count == 0 {
        println!("✓ Audit passed with no critical issues.");
    } else {
        if error_count > 0 {
            println!("  ✗ Validation errors: {error_count}");
        }
        if warning_count > 0 {
            println!("  ⚠ Validation warnings: {warning_count}");
        }
        if missing_prereq_count > 0 {
            println!("  ⚠ Upper-level courses without prerequisites: {missing_prereq_count}");
        }
        if deep_chain_count > 0 {
            println!("  ⚠ Courses with deep chains (≥{threshold}): {deep_chain_count}");
        }
    }
}

/// Options for `degree analyze`.
#[derive(Debug, Default, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct AnalyzeOptions {
    /// Calculation strategy override ("median" or "mean"), recorded in the run's parameters
    pub calc_strategy: Option<String>,
    /// Sampling strategy override ("sequential", "shuffled", "stratified")
    pub sampling_strategy: Option<String>,
    /// Number of random plans to sample
    pub sample_plans: Option<usize>,
    /// Maximum plans to generate
    pub max_plans: Option<usize>,
    /// Generate all plans without deduplication (disables `ignore_duplicates`)
    pub full_run: bool,
    /// Override reports directory
    pub report_dir: Option<std::path::PathBuf>,
    /// Override metrics directory
    pub metrics_dir: Option<std::path::PathBuf>,
    /// Skip the CSV files (plan CSVs, the `index.csv` row); the report JSON is still written
    pub no_csv: bool,
    /// Skip every metrics-directory output, the report JSON and a `--school` roll-up
    /// included; a worker pool still records failures in `failures.log`
    pub no_metrics: bool,
    /// Skip HTML report
    pub no_report: bool,
    /// Whether to print verbose output
    pub verbose: bool,
    /// Courses to always include in all plans
    pub include_courses: Option<Vec<String>>,
    /// Concurrent worker processes for a multi-file batch (1 = in-process).
    pub jobs: usize,
    /// When set, treat the inputs as programs of one school and also emit a
    /// combined `<school>_school_report.json` rolling up degree-level metrics.
    pub school: Option<String>,
    /// When set, print where this course lands across the analyzed plans as JSON on
    /// stdout, instead of writing reports.
    pub target_course: Option<String>,
    /// With `target_course`, also write the degree's report JSON here, with the
    /// course's statistics under `analysis.target_course_stats`.
    pub metrics_out: Option<PathBuf>,
}

/// Run `degree validate` over one or more files.
///
/// `allow_unmatched_patterns` reports a pattern that matches no listed course as a
/// warning rather than an error.
pub fn run_validate(files: &[PathBuf], allow_unmatched_patterns: bool, verbose: bool) {
    let opts = ValidationOptions {
        allow_unmatched_patterns,
    };
    run_batch(files, |path| validate_degree(path, opts, verbose));
}

/// Run `degree print-graph` over one or more files.
pub fn run_print_graph(files: &[PathBuf], verbose: bool) {
    run_batch(files, |path| print_graph(path, verbose));
}

/// Run `degree audit` over one or more files.
pub fn run_audit(files: &[PathBuf], config: &Config, verbose: bool) {
    run_batch(files, |path| audit_degree(path, config, verbose));
}

/// Environment marker set on spawned worker processes so they run a single
/// file in-process instead of recursively spawning their own pool.
const WORKER_ENV: &str = "NU_ANALYZE_WORKER";

/// Run `degree analyze`. A multi-file batch is processed as a pool of isolated
/// worker processes (`--jobs`, default 8) so one pathological degree can't take
/// down the whole run; single-file, `--school`, `--target-course`, `-j 1`, and
/// worker-mode invocations run in-process.
pub fn run_analyze(files: &[PathBuf], options: &AnalyzeOptions, config: &Config) {
    let in_worker = std::env::var_os(WORKER_ENV).is_some();
    // A `--target-course` query answers on stdout, which a worker's is not connected to.
    if in_worker || options.jobs <= 1 || options.school.is_some() || options.target_course.is_some()
    {
        run_analyze_inprocess(files, options, config);
        return;
    }

    let inputs = filter_degree_inputs(files);
    match inputs.len() {
        0 => {
            eprintln!("Error: No degree files to process after filtering.");
            process::exit(1);
        }
        // A single file gains nothing from a worker process; run it in-process
        // so the user gets the full per-degree output.
        1 => run_analyze_inprocess(files, options, config),
        _ => run_analyze_parallel(&inputs, options),
    }
}

/// One stored `programs` row, projected to the columns needed to resolve and
/// analyze a program by name (the lossless `document` plus identity fields used
/// for the candidate list and the "loaded" banner).
#[cfg(feature = "database")]
#[derive(Debug, Clone, serde::Deserialize)]
struct StoredProgramRow {
    program_key: String,
    name: String,
    unitid: Option<i32>,
    institution_raw: Option<String>,
    catalog_year: Option<String>,
    document: serde_json::Value,
}

/// Columns selected when resolving a `--from-db` program (lossless `document`
/// last, identity fields for the banner / candidate list first).
#[cfg(feature = "database")]
const FROM_DB_COLS: &str =
    "program_key,degree_id,name,unitid,institution_raw,catalog_year,document";

/// Cap on candidate rows fetched while resolving a `--from-db` name.
#[cfg(feature = "database")]
const FROM_DB_MAX_ROWS: usize = 25;

/// Run `degree analyze --from-db <NAME>`: fetch one stored program's canonical
/// degree from the database and analyze it with the same options as the
/// file-based path. This is single-program only (no worker pool / `--jobs`).
///
/// Resolution ladder: exact `program_key`, then exact `degree_id`, then a
/// `name` substring (`ILIKE`). Zero matches exits non-zero; multiple matches
/// print the candidates and exit non-zero; exactly one is parsed from its
/// `document` and analyzed.
#[cfg(feature = "database")]
pub fn run_analyze_from_db(name: &str, options: &AnalyzeOptions, config: &Config) {
    use nu_analytics::database::DbClient;

    let Some(rt) = super::db::make_runtime() else {
        process::exit(1);
    };

    let client = match rt.block_on(DbClient::from_config(&config.database)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ Database not available: {e}");
            super::db::report_db_error(&e, &config.database.endpoint);
            process::exit(1);
        }
    };

    let rows = match rt.block_on(resolve_program_rows(&client, name)) {
        Ok(rows) => rows,
        Err(e) => {
            eprintln!("✗ Database query failed: {e}");
            process::exit(1);
        }
    };

    match rows.len() {
        0 => {
            eprintln!("✗ no stored program matches '{name}'");
            process::exit(1);
        }
        1 => {
            let row = &rows[0];
            let program = match nu_analytics::core::degree::from_unified_value(&row.document) {
                Ok(p) => p,
                Err(e) => {
                    eprintln!(
                        "✗ failed to parse stored document for '{}': {e}",
                        row.program_key
                    );
                    process::exit(1);
                }
            };
            print_loaded_program(row);
            if let Err(e) = analyze_program(&program, options, config) {
                eprintln!("Error: {e}");
                process::exit(1);
            }
        }
        n => {
            eprintln!(
                "⚠ '{name}' matched {n} stored programs — re-run with a more specific name or the exact program_key:"
            );
            for line in format_program_candidates(&rows) {
                eprintln!("  • {line}");
            }
            process::exit(1);
        }
    }
}

/// Resolve a `--from-db` name to candidate `programs` rows: an exact `program_key`, then
/// an exact `degree_id`, then a `name` substring (`find_programs`), each capped at
/// [`FROM_DB_MAX_ROWS`] so an over-broad name can't pull the whole table.
#[cfg(feature = "database")]
async fn resolve_program_rows(
    client: &nu_analytics::database::DbClient,
    name: &str,
) -> Result<Vec<StoredProgramRow>, nu_analytics::database::DatabaseError> {
    nu_analytics::core::query::degrees::find_programs(
        client,
        FROM_DB_COLS,
        nu_analytics::database::QueryFilters::new,
        name,
        FROM_DB_MAX_ROWS,
    )
    .await
}

/// Print the "loaded program" banner for a resolved `--from-db` row
/// (name + `program_key` + unitid).
#[cfg(feature = "database")]
fn print_loaded_program(row: &StoredProgramRow) {
    let unitid = row
        .unitid
        .map_or_else(|| "unresolved".to_string(), |u| u.to_string());
    // Status, not output: on stderr like the file path's "Loaded degree", so a
    // `--target-course` answer on stdout stays parseable.
    eprintln!(
        "✓ Loaded stored program: {} (program_key {} · unitid {unitid})",
        row.name, row.program_key
    );
}

/// Format candidate program rows into one display line each for the ambiguous
/// `--from-db` message: `program_key  ·  name  ·  institution_raw  ·
/// catalog_year`. Pure (no DB / no I/O) so it can be unit-tested offline.
#[cfg(feature = "database")]
fn format_program_candidates(rows: &[StoredProgramRow]) -> Vec<String> {
    rows.iter()
        .map(|r| {
            let institution = r.institution_raw.as_deref().unwrap_or("(unknown)");
            let catalog = r.catalog_year.as_deref().unwrap_or("(no catalog year)");
            format!(
                "{}  ·  {}  ·  {institution}  ·  {catalog}",
                r.program_key, r.name
            )
        })
        .collect()
}

/// Poll interval for reaping finished worker processes.
const WORKER_POLL: std::time::Duration = std::time::Duration::from_millis(50);

/// The metrics output directory from `options`, defaulting to `metrics/`.
fn metrics_dir_or_default(options: &AnalyzeOptions) -> PathBuf {
    options
        .metrics_dir
        .clone()
        .unwrap_or_else(|| PathBuf::from("metrics"))
}

/// Analyze a multi-file batch as a rolling pool of up to `options.jobs` worker
/// processes (each re-invokes this binary on one file). Worker output is
/// suppressed; a progress line and final summary are printed, and any failures
/// (path + exit status) are written to `<metrics-dir>/failures.log`.
///
/// Isolation here is reactive: a worker that exhausts memory is killed by the
/// OS and recorded as a failure. Unlike `scripts/analyze-batch.sh`, the pool
/// imposes no per-process memory cap or timeout — use that script when a hard
/// `ulimit -v` / `timeout` guard is required.
fn run_analyze_parallel(inputs: &[&Path], options: &AnalyzeOptions) {
    use std::io::Write;

    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Error: cannot locate the nuanalytics executable: {e}");
            process::exit(1);
        }
    };
    let metrics_dir = metrics_dir_or_default(options);

    // Write the index.csv header once so concurrent workers only append rows.
    if !options.no_csv && !options.no_metrics {
        if let Err(e) =
            nu_analytics::core::report::plan_export::write_index_csv_header(&metrics_dir)
        {
            eprintln!("Warning: could not initialize index.csv: {e}");
        }
    }

    let jobs = options.jobs.max(1);
    let child_flags = analyze_child_flags(options);
    let total = inputs.len();
    println!("Analyzing {total} programs with {jobs} worker process(es)…");

    let mut next = 0usize;
    let mut running: Vec<(PathBuf, std::process::Child)> = Vec::new();
    let mut done = 0usize;
    let mut failed: Vec<(PathBuf, String)> = Vec::new();

    loop {
        while running.len() < jobs && next < total {
            let f = inputs[next];
            next += 1;
            match spawn_analyze_worker(&exe, f, &child_flags) {
                Ok(child) => running.push((f.to_path_buf(), child)),
                Err(e) => {
                    failed.push((f.to_path_buf(), format!("spawn error: {e}")));
                    done += 1;
                }
            }
        }
        if running.is_empty() {
            break;
        }

        let reaped = reap_finished(&mut running, &mut failed);
        if reaped > 0 {
            done += reaped;
            print!("\r  {done}/{total} done ({} failed)   ", failed.len());
            let _ = std::io::stdout().flush();
        } else {
            std::thread::sleep(WORKER_POLL);
        }
    }
    println!();

    report_pool_outcome(total, &failed, &metrics_dir);
}

/// Reap every worker that has finished, removing it from `running` and
/// recording non-success exits in `failed` (path + status string). Returns the
/// number reaped this pass (0 ⇒ nothing finished yet).
fn reap_finished(
    running: &mut Vec<(PathBuf, std::process::Child)>,
    failed: &mut Vec<(PathBuf, String)>,
) -> usize {
    let mut reaped = 0;
    let mut i = 0;
    while i < running.len() {
        match running[i].1.try_wait() {
            Ok(Some(status)) => {
                let (f, _) = running.remove(i);
                if !status.success() {
                    // ExitStatus Display includes the signal on Unix, so an
                    // OOM-killed worker reads e.g. "signal: 9 (SIGKILL)".
                    failed.push((f, status.to_string()));
                }
                reaped += 1;
            }
            Ok(None) => i += 1,
            Err(e) => {
                let (f, _) = running.remove(i);
                failed.push((f, format!("wait error: {e}")));
                reaped += 1;
            }
        }
    }
    reaped
}

/// Spawn one isolated worker process to analyze `file` (output suppressed).
fn spawn_analyze_worker(
    exe: &Path,
    file: &Path,
    flags: &[String],
) -> std::io::Result<std::process::Child> {
    std::process::Command::new(exe)
        .arg("degree")
        .arg("analyze")
        .arg(file)
        .args(flags)
        .env(WORKER_ENV, "1")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
}

/// Print the batch summary; on any failure, write `failures.log` (one
/// `path<TAB>status` line each) and exit 1.
fn report_pool_outcome(total: usize, failed: &[(PathBuf, String)], metrics_dir: &Path) {
    use std::fmt::Write as _;

    let succeeded = total - failed.len();
    println!(
        "✓ analyzed {succeeded}/{total} programs ({} failed)",
        failed.len()
    );
    if failed.is_empty() {
        return;
    }
    let faillog = metrics_dir.join("failures.log");
    let mut body = String::new();
    for (path, reason) in failed {
        let _ = writeln!(body, "{}\t{reason}", path.display());
    }
    if std::fs::write(&faillog, body).is_ok() {
        println!("  failures listed in {}", faillog.display());
    }
    if let Some((first, _)) = failed.first() {
        println!(
            "  re-run one for details: nuanalytics degree analyze {} -j 1",
            first.display()
        );
    }
    process::exit(1);
}

/// Reconstruct the `degree analyze` flags for a worker child from `options`.
/// Excludes `--jobs` (the worker marker prevents re-pooling) and `--school`
/// (the pool only runs when school mode is off).
///
/// This must mirror every *result-affecting* flag on the `Analyze` subcommand
/// in `src/cli/args.rs`: a flag added there but omitted here is silently dropped
/// for pooled runs, so workers would analyze with different settings than the
/// user asked for. Covered by `test_analyze_child_flags_*`.
fn analyze_child_flags(o: &AnalyzeOptions) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    if let Some(d) = &o.metrics_dir {
        a.push("--metrics-dir".into());
        a.push(d.display().to_string());
    }
    if let Some(d) = &o.report_dir {
        a.push("--report-dir".into());
        a.push(d.display().to_string());
    }
    if o.no_report {
        a.push("--no-report".into());
    }
    if o.no_csv {
        a.push("--no-csv".into());
    }
    if o.no_metrics {
        a.push("--no-metrics".into());
    }
    if let Some(n) = o.max_plans {
        a.push("--max-plans".into());
        a.push(n.to_string());
    }
    if let Some(n) = o.sample_plans {
        a.push("--sample-plans".into());
        a.push(n.to_string());
    }
    if let Some(s) = &o.sampling_strategy {
        a.push("--sampling-strategy".into());
        a.push(s.clone());
    }
    if let Some(s) = &o.calc_strategy {
        a.push("--calc-strategy".into());
        a.push(s.clone());
    }
    if o.full_run {
        a.push("--full-run".into());
    }
    if let Some(courses) = &o.include_courses {
        if !courses.is_empty() {
            a.push("--include".into());
            a.push(courses.join(","));
        }
    }
    a
}

/// Run `degree analyze` in-process (single file, school mode, `--target-course`,
/// `-j 1`, or as a spawned worker). Without `--school`, each file is analyzed independently;
/// with `--school`, a combined `<school>_school_report.json` is also written.
fn run_analyze_inprocess(files: &[PathBuf], options: &AnalyzeOptions, config: &Config) {
    let Some(school_name) = options.school.clone() else {
        run_batch(files, |path| {
            analyze_degree(path, options, config).map(|_| ())
        });
        return;
    };

    // School mode: collect per-program rollups across the batch.
    let inputs = filter_degree_inputs(files);
    if inputs.is_empty() {
        eprintln!("Error: No degree files to process after filtering.");
        process::exit(1);
    }

    let total = inputs.len();
    let mut rollups = Vec::new();
    let mut had_failure = false;
    for (idx, path) in inputs.iter().enumerate() {
        if total > 1 {
            if idx > 0 {
                print_separator();
            }
            println!("=== [{}/{}] {} ===", idx + 1, total, path.display());
        }
        match analyze_degree(path, options, config) {
            Ok(rollup) => rollups.extend(rollup),
            Err(e) => {
                eprintln!("Error: {e}");
                had_failure = true;
            }
        }
    }

    if !rollups.is_empty() && !options.no_metrics {
        let metrics_dir = metrics_dir_or_default(options);
        match nu_analytics::core::report::unified_report::export_school_report_json(
            &school_name,
            &rollups,
            &metrics_dir,
        ) {
            Ok(path) => println!("✓ School report: {}", path.display()),
            Err(e) => eprintln!("Error: Failed to write school report: {e}"),
        }
    }

    if had_failure {
        process::exit(1);
    }
}

/// Run `degree trim` over one or more input files.
///
/// `out` resolution rules:
///
/// - `None` → each trimmed file is written next to its input as
///   `<input-stem>_trimmed.<ext>`.
/// - `Some(dir)` (existing directory, or a path ending in a separator) →
///   each input becomes `<dir>/<input-stem>_trimmed.<ext>`. Directory is
///   created on demand.
/// - `Some(file)` → only valid with a single input; written verbatim.
///   Multiple inputs with a file-mode `-o` is rejected.
pub fn run_trim(
    inputs: &[PathBuf],
    out: Option<&Path>,
    keep_all: &[String],
    include: Option<&[String]>,
    verbose: bool,
) {
    if inputs.is_empty() {
        eprintln!("Error: No degree file specified.");
        process::exit(1);
    }

    let degree_inputs = filter_degree_inputs(inputs);
    if degree_inputs.is_empty() {
        eprintln!("Error: No degree files to process after filtering.");
        process::exit(1);
    }

    let dir_mode = out.is_some_and(looks_like_directory);

    if let Some(file_out) = out.filter(|_| degree_inputs.len() > 1 && !dir_mode) {
        eprintln!(
            "Error: -o {} is a file path, but {} input files were given; pass a directory (or end the path with '/') instead",
            file_out.display(),
            degree_inputs.len()
        );
        process::exit(1);
    }

    if let Some(dir) = out.filter(|_| dir_mode) {
        if let Err(e) = std::fs::create_dir_all(dir) {
            eprintln!(
                "Error: failed to create output directory {}: {e}",
                dir.display()
            );
            process::exit(1);
        }
    }

    let total = degree_inputs.len();
    let mut had_failure = false;
    for (idx, input) in degree_inputs.iter().enumerate() {
        if total > 1 {
            if idx > 0 {
                print_separator();
            }
            println!("=== [{}/{}] {} ===", idx + 1, total, input.display());
        }
        let out_path = resolve_trim_output(input, out, dir_mode);
        if let Err(e) = trim_one(input, &out_path, keep_all, include, verbose) {
            eprintln!("Error: {e}");
            had_failure = true;
        }
    }

    if had_failure {
        process::exit(1);
    }
}

/// Shared batch driver for the multi-file `degree` subcommands.
///
/// Filters out non-YAML paths with a warning, processes each file in order,
/// prints a per-file header when there's more than one input, and exits
/// non-zero if any file failed.
fn run_batch<F>(files: &[PathBuf], mut action: F)
where
    F: FnMut(&Path) -> Result<(), String>,
{
    if files.is_empty() {
        eprintln!("Error: No degree file specified.");
        process::exit(1);
    }

    let yaml_files = filter_degree_inputs(files);
    if yaml_files.is_empty() {
        eprintln!("Error: No degree files to process after filtering.");
        process::exit(1);
    }

    let total = yaml_files.len();
    let mut had_failure = false;

    for (idx, path) in yaml_files.iter().enumerate() {
        if total > 1 {
            if idx > 0 {
                print_separator();
            }
            println!("=== [{}/{}] {} ===", idx + 1, total, path.display());
        }
        if let Err(e) = action(path) {
            eprintln!("Error: {e}");
            had_failure = true;
        }
    }

    if had_failure {
        process::exit(1);
    }
}

/// Filter `files` to those `accept`ed, warning to stderr (with `expected` naming
/// the wanted kind) about each path skipped. Shared by the degree subcommands.
fn filter_inputs<'a>(
    files: &'a [PathBuf],
    accept: fn(&Path) -> bool,
    expected: &str,
) -> Vec<&'a Path> {
    files
        .iter()
        .filter_map(|p| {
            if accept(p) {
                Some(p.as_path())
            } else {
                eprintln!("Skipping non-{expected} file: {}", p.display());
                None
            }
        })
        .collect()
}

/// Returns `true` if the path has a `.yaml` or `.yml` extension (case-insensitive).
fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

/// Returns `true` if the path has a `.json` extension (case-insensitive).
fn is_json_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// A degree input is a YAML or JSON file (JSON may be unified or raw
/// ai-landscape — both handled by [`load_degree_auto`]).
fn is_degree_input_path(path: &Path) -> bool {
    is_yaml_path(path) || is_json_path(path)
}

/// Filter inputs to degree files (YAML or JSON), warning about the rest.
/// Used by batch commands that accept both formats.
fn filter_degree_inputs(files: &[PathBuf]) -> Vec<&Path> {
    filter_inputs(files, is_degree_input_path, "degree (.yaml/.yml/.json)")
}

/// Filename suffix appended to the input stem when `degree trim` runs
/// without an explicit `-o`/`--out` path.
const TRIM_OUTPUT_SUFFIX: &str = "_trimmed";

/// Extract `(file_stem, extension)` from a path with degree-YAML-friendly
/// fallbacks when either piece is missing or non-UTF-8. Centralised so
/// [`default_trim_output`] and [`resolve_trim_output`] stay in sync.
fn trim_output_stem_ext(input: &Path) -> (&str, &str) {
    let stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("degree");
    let ext = input.extension().and_then(|e| e.to_str()).unwrap_or("yaml");
    (stem, ext)
}

/// Default output path for `degree trim` when `-o` is not given:
/// `<input-stem>_trimmed.<ext>` next to the input file.
fn default_trim_output(input: &Path) -> PathBuf {
    let (stem, ext) = trim_output_stem_ext(input);
    input.with_file_name(format!("{stem}{TRIM_OUTPUT_SUFFIX}.{ext}"))
}

/// True if `p` should be treated as a directory destination for
/// `degree trim -o`. An existing directory always wins; otherwise we
/// honour the user's intent if they typed a trailing path separator.
fn looks_like_directory(p: &Path) -> bool {
    if p.is_dir() {
        return true;
    }
    let s = p.to_string_lossy();
    s.ends_with('/') || s.ends_with(std::path::MAIN_SEPARATOR)
}

/// Resolve an output path from a `-o` argument: `None` writes `filename` next to
/// the input, a directory destination joins `filename` under it, and a file
/// destination is used verbatim. Shared by `trim` and `convert`.
fn resolve_output_path(
    input: &Path,
    out: Option<&Path>,
    dir_mode: bool,
    filename: &str,
) -> PathBuf {
    match out {
        None => input.with_file_name(filename),
        Some(dir) if dir_mode => dir.join(filename),
        Some(file) => file.to_path_buf(),
    }
}

/// Resolve the on-disk output path for `degree trim` given the user's `-o`
/// argument and whether we determined it to be a directory destination.
fn resolve_trim_output(input: &Path, out: Option<&Path>, dir_mode: bool) -> PathBuf {
    if out.is_none() {
        return default_trim_output(input);
    }
    let (stem, ext) = trim_output_stem_ext(input);
    resolve_output_path(
        input,
        out,
        dir_mode,
        &format!("{stem}{TRIM_OUTPUT_SUFFIX}.{ext}"),
    )
}

/// Load `input`, apply
/// [`trim_program`](nu_analytics::core::degree::trim_program) with the
/// given options, and write the result to `out_path`. Prints a success
/// banner; emits the protected-subject set and orphan-course list when
/// `verbose` is set.
fn trim_one(
    input: &Path,
    out_path: &Path,
    keep_all: &[String],
    include: Option<&[String]>,
    verbose: bool,
) -> Result<(), String> {
    use nu_analytics::core::degree::{trim_program, TrimOptions};

    let program = load_degree_auto(input)
        .map_err(|e| format!("Failed to load {}: {}", input.display(), e))?;

    let opts = TrimOptions {
        keep_all_subjects: keep_all.iter().map(|s| s.to_uppercase()).collect(),
        include_courses: include
            .map(|v| v.iter().cloned().collect())
            .unwrap_or_default(),
    };

    let (trimmed, report) = trim_program(&program, &opts);

    if out_path == input {
        return Err(format!(
            "refusing to overwrite input file {}; pass an explicit -o path or rely on the default _trimmed suffix",
            input.display()
        ));
    }

    // Round-trip the input format: a JSON input yields a trimmed JSON file
    // (resolve_trim_output preserves the extension), YAML stays YAML.
    save_degree_auto(&trimmed, out_path)?;

    println!("✓ Trimmed degree written to: {}", out_path.display());
    if verbose {
        let scope = if report.protected_subjects_derived {
            "derived"
        } else {
            "from major_subjects"
        };
        println!(
            "  Protected subjects ({scope}): {}",
            report.protected_subjects.join(", ")
        );
        if !report.orphan_courses_removed.is_empty() {
            println!(
                "  Removed {} orphan course(s): {}",
                report.orphan_courses_removed.len(),
                report.orphan_courses_removed.join(", ")
            );
        }
    } else if !report.orphan_courses_removed.is_empty() {
        println!(
            "  Removed {} orphan course(s)",
            report.orphan_courses_removed.len()
        );
    }
    Ok(())
}

/// What the CLI's output layer reads from one analysis: the run's results, plus the
/// CLI's own settings.
struct AnalysisContext<'a> {
    program: &'a nu_analytics::core::DegreeProgram,
    /// The degree's course graph, cycles broken.
    graph: &'a CourseGraph,
    school: &'a School,
    gen_config: &'a PlanGeneratorConfig,
    /// `mean` | `median` — recorded on the analysis run so it can be reproduced.
    calc_strategy: String,
    verbose: bool,
    /// Map from course key to all equivalent courses (including itself)
    equivalences: &'a HashMap<String, HashSet<String>>,
    /// The run's statistics reduced to what the report and exports read.
    report_stats: &'a nu_analytics::core::report::ReportStats,
}

impl AnalysisContext<'_> {
    /// What the report records about this run, so it can be reproduced.
    fn run_parameters(&self) -> nu_analytics::core::report::unified_report::RunParameters<'_> {
        nu_analytics::core::report::unified_report::RunParameters {
            max_plans: self.gen_config.max_plans,
            sample_count: self.gen_config.sample_count,
            sampling_strategy: self.gen_config.sampling_strategy.as_str(),
            calc_strategy: &self.calc_strategy,
            ignore_duplicates: self.gen_config.ignore_duplicates,
            included_courses: &self.gen_config.include_courses,
            random_seed: self.gen_config.random_seed,
        }
    }
}

/// Run full degree analysis: generate plans, compute metrics, produce report
///
/// This is the main entry point for the `--analyze` flag. It:
/// 1. Loads the degree program and builds the course graph
/// 2. Generates all possible plans from requirements
/// 3. Streams metrics computation and aggregation
/// 4. Selects special plans (shortest, longest, calc-ready)
/// 5. Generates HTML report with box plots and statistics
/// 6. Exports CSV files for selected plans
fn analyze_degree(
    degree_path: &Path,
    options: &AnalyzeOptions,
    config: &Config,
) -> Result<Option<nu_analytics::core::report::unified_report::ProgramRollup>, String> {
    // Load and validate the degree program, then run the shared single-degree
    // analysis on it. This is the in-process, non-worker-pool seam shared with
    // the `--from-db` path (which supplies an already-loaded program).
    let program = load_degree_program(degree_path, options.verbose)?;
    analyze_program(&program, options, config)
}

/// Run the full single-degree analysis on an already-loaded `DegreeProgram`.
///
/// This is the shared analysis seam: [`analyze_degree`] calls it after loading a
/// file, and the `--from-db` path calls it with a program parsed from a stored
/// `document`. It runs the analysis pipeline, validates the selected plans, writes the
/// report + CSV outputs, prints the summary, and returns the program rollup.
///
/// A `--target-course` query instead prints where that course lands, writes only the
/// `--metrics-out` file if one was asked for, and returns `None`: it produces no rollup.
fn analyze_program(
    program: &nu_analytics::core::DegreeProgram,
    options: &AnalyzeOptions,
    config: &Config,
) -> Result<Option<nu_analytics::core::report::unified_report::ProgramRollup>, String> {
    use nu_analytics::core::degree::analysis::analyze;
    let verbose = options.verbose;
    let analysis_config = analysis_config(options, config);
    let progress_interval = (analysis_config.max_plans / 20).max(100);
    let analysis = analyze(program.clone(), &analysis_config, &mut |event| {
        report_analysis_event(&event, verbose, progress_interval);
    });
    if verbose {
        print_selection_summary(&analysis.selected, analysis.plans_processed);
    }

    let ctx = AnalysisContext {
        program: &analysis.program,
        graph: &analysis.graph,
        school: &analysis.school,
        gen_config: &analysis.gen_config,
        calc_strategy: options
            .calc_strategy
            .clone()
            .unwrap_or_else(|| config.degree_analysis.calc_strategy.clone()),
        verbose,
        equivalences: &analysis.equivalences,
        report_stats: &analysis.report_stats,
    };

    if let Some(stats) = &analysis.target_course_stats {
        emit_target_course_stats(&ctx, &analysis, stats, options.metrics_out.as_deref())?;
        return Ok(None);
    }

    // Validate the selected plans and report any issues
    validate_selected_plans(&ctx, &analysis.selected);

    // Generate outputs
    generate_analysis_outputs(&ctx, options, &analysis.aggregator, &analysis.selected)?;

    // Print summary
    print_analysis_summary(&ctx, &analysis.aggregator, analysis.plans_processed);

    Ok(Some(
        nu_analytics::core::report::unified_report::ProgramRollup::from_analysis(
            ctx.program,
            &analysis.aggregator,
        ),
    ))
}

/// What `degree analyze` asks the pipeline for: each option given, otherwise the
/// configuration's value.
fn analysis_config<'a>(
    options: &'a AnalyzeOptions,
    config: &Config,
) -> nu_analytics::core::degree::analysis::AnalysisConfig<'a> {
    let base =
        nu_analytics::core::degree::analysis::AnalysisConfig::from_config(&config.degree_analysis);
    nu_analytics::core::degree::analysis::AnalysisConfig {
        max_plans: options.max_plans.unwrap_or(base.max_plans),
        // --full-run counts duplicate plans too
        ignore_duplicates: !options.full_run && base.ignore_duplicates,
        sample_count: options.sample_plans.unwrap_or(base.sample_count),
        // An unparseable --sampling-strategy falls back to the configuration's
        sampling_strategy: options
            .sampling_strategy
            .as_ref()
            .and_then(|s| s.parse::<SamplingStrategy>().ok())
            .unwrap_or(base.sampling_strategy),
        include_courses: options.include_courses.clone().unwrap_or_default(),
        target_course: options.target_course.as_deref(),
        ..base
    }
}

/// Answer a `--target-course` query: print the course's term statistics as JSON, and with
/// `--metrics-out` write the degree's report JSON — the file `degree analyze` writes as
/// `<degree>_report.json` — with the statistics added to its `analysis` block.
fn emit_target_course_stats(
    ctx: &AnalysisContext<'_>,
    analysis: &nu_analytics::core::degree::analysis::DegreeAnalysis,
    stats: &nu_analytics::core::degree::analysis::TargetCourseStats,
    metrics_out: Option<&Path>,
) -> Result<(), String> {
    use nu_analytics::core::report::unified_report::{build_degree_report, write_degree_report};
    let stats_json = serde_json::to_value(stats)
        .map_err(|e| format!("--target-course: cannot serialize the statistics: {e}"))?;
    println!(
        "{}",
        serde_json::to_string_pretty(&stats_json).unwrap_or_default()
    );
    let Some(out_path) = metrics_out else {
        return Ok(());
    };
    // Surfaced, not swallowed: --metrics-out is an explicit request for a file.
    let params = ctx.run_parameters();
    let mut report = build_degree_report(
        ctx.program,
        &analysis.aggregator,
        &analysis.selected,
        params.sampling_strategy,
        &params,
    )
    .map_err(|e| format!("--metrics-out: cannot build the report: {e}"))?;
    if let Some(block) = report
        .get_mut("analysis")
        .and_then(serde_json::Value::as_object_mut)
    {
        block.insert("target_course_stats".to_string(), stats_json);
    }
    write_degree_report(&report, out_path).map_err(|e| format!("--metrics-out: {e}"))
}

/// Print an analysis run's progress to stderr, when verbose.
fn report_analysis_event(
    event: &nu_analytics::core::degree::analysis::AnalysisEvent<'_>,
    verbose: bool,
    progress_interval: usize,
) {
    use nu_analytics::core::degree::analysis::AnalysisEvent;
    if !verbose {
        return;
    }
    match event {
        AnalysisEvent::CyclesBroken { cycles, removed } => {
            eprintln!("⚠ Detected {cycles} circular prerequisite(s), breaking cycles...");
            for (course, prereq) in *removed {
                eprintln!("  Removed edge: {course} → {prereq}");
            }
        }
        AnalysisEvent::Planning { stats, config } => {
            eprintln!();
            eprintln!("Plan Generation:");
            if !config.include_courses.is_empty() {
                eprintln!("  Included courses: {}", config.include_courses.join(", "));
            }
            eprintln!("  Estimated total plans: {}", stats.total_possible);
            eprintln!("  Variable requirements: {}", stats.variable_requirements);
            if stats.total_possible > config.max_plans {
                eprintln!(
                    "  ⚠ Will cap at {} plans (use --max-plans to adjust)",
                    config.max_plans
                );
            }
            eprintln!();
            eprintln!("Processing plans...");
        }
        AnalysisEvent::Processed(n) => {
            if n % progress_interval == 0 {
                eprintln!("  Processed {n} plans...");
            }
        }
        AnalysisEvent::PlanSkipped(e) => {
            eprintln!("  Warning: Failed to compute metrics for plan: {e}");
        }
    }
}

/// The JSON Schema for the unified degree format, embedded at build time.
const UNIFIED_DEGREE_SCHEMA: &str = include_str!("../../assets/degree.schema.json");

/// Emit the unified-degree JSON Schema to a file (`out`) or stdout.
pub fn run_schema(out: Option<&Path>) {
    if let Some(path) = out {
        match std::fs::write(path, UNIFIED_DEGREE_SCHEMA) {
            Ok(()) => println!("✓ Schema written to: {}", path.display()),
            Err(e) => {
                eprintln!("Error: Failed to write {}: {e}", path.display());
                process::exit(1);
            }
        }
    } else {
        print!("{UNIFIED_DEGREE_SCHEMA}");
    }
}

/// Run `degree normalize` over one or more inputs, emitting a flat normalized
/// course set per program as `<stem>.normalized.json`.
///
/// Accepts the same three input shapes as `degree convert`:
/// - Cluster pipeline JSON (`course_verifier` / `course_scraper`) → one file per program
/// - Unified degree JSON → one file
/// - Degree YAML → one file
pub fn run_normalize(files: &[PathBuf], out: Option<&Path>, pretty: bool, verbose: bool) {
    let dir_mode = out
        .is_some_and(|p| p.is_dir() || p.to_string_lossy().ends_with(std::path::MAIN_SEPARATOR))
        || files.len() > 1;

    let mut total_programs: usize = 0;
    let mut total_warnings: usize = 0;

    for file in files {
        match normalize_file(file, out, dir_mode, pretty, verbose) {
            Ok((programs, warnings)) => {
                total_programs += programs;
                total_warnings += warnings;
            }
            Err(e) => eprintln!("✗ {}: {e}", file.display()),
        }
    }

    if files.len() > 1 {
        println!("Normalized {total_programs} program(s) with {total_warnings} warning(s)");
    }
}

/// Normalize one input file. Returns `(programs_written, warnings)`.
fn normalize_file(
    input: &Path,
    out: Option<&Path>,
    dir_mode: bool,
    pretty: bool,
    verbose: bool,
) -> Result<(usize, usize), String> {
    use nu_analytics::core::degree::{
        convert_landscape, extract_cluster_programs, normalize_program,
    };

    let contents = std::fs::read_to_string(input)
        .map_err(|e| format!("Failed to read {}: {e}", input.display()))?;

    if is_json_path(input) {
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
            if let Some(cluster_programs) = extract_cluster_programs(&value) {
                let out_dir = cluster_out_dir(input, out);
                let stem = file_stem_or(input, "degree");
                let mut warnings = 0usize;
                for (prog_name, landscape_prog) in &cluster_programs {
                    let result = convert_landscape(landscape_prog);
                    warnings += result.warnings.len();
                    let normalized = normalize_program(&result.program, None, Some(prog_name));
                    let out_path = out_dir.join(format!(
                        "{stem}{}{}.normalized.json",
                        CLUSTER_NAME_SEP,
                        slug_filename(prog_name),
                    ));
                    write_normalized(&normalized, &out_path, pretty, verbose)?;
                }
                println!(
                    "✓ {}: {} program(s), {} warning(s)",
                    input.display(),
                    cluster_programs.len(),
                    warnings,
                );
                return Ok((cluster_programs.len(), warnings));
            }
        }
    }

    // Single-program path: unified JSON or YAML.
    let program =
        load_degree_auto(input).map_err(|e| format!("Failed to load {}: {e}", input.display()))?;
    let normalized = normalize_program(&program, None, None);
    let out_path = resolve_normalize_output(input, out, dir_mode);
    write_normalized(&normalized, &out_path, pretty, verbose)?;
    println!("✓ {} -> {}", input.display(), out_path.display());
    Ok((1, 0))
}

/// Output path for a single-program normalize: `<stem>.normalized.json`.
fn resolve_normalize_output(input: &Path, out: Option<&Path>, dir_mode: bool) -> PathBuf {
    let stem = file_stem_or(input, "degree");
    resolve_output_path(input, out, dir_mode, &format!("{stem}.normalized.json"))
}

/// Serialize a [`NormalizedProgram`](nu_analytics::core::degree::NormalizedProgram) and write it to `path`.
fn write_normalized(
    normalized: &nu_analytics::core::degree::NormalizedProgram,
    path: &Path,
    pretty: bool,
    verbose: bool,
) -> Result<(), String> {
    let json = if pretty {
        serde_json::to_string_pretty(normalized)
    } else {
        serde_json::to_string(normalized)
    }
    .map_err(|e| format!("Failed to serialize normalized output: {e}"))?;

    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }
    }
    std::fs::write(path, &json).map_err(|e| format!("Failed to write {}: {e}", path.display()))?;

    if verbose {
        eprintln!("  wrote {}", path.display());
    }
    Ok(())
}

/// Build a filesystem-safe slug from an arbitrary program name for use in
/// cluster output filenames (mirrors the slug logic in `convert_cluster`).
fn slug_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

/// Run `degree convert` over one or more inputs, emitting unified JSON.
pub fn run_convert(
    files: &[PathBuf],
    out: Option<&Path>,
    pretty: bool,
    out_format: DegreeFormat,
    verbose: bool,
) {
    // Directory mode when -o is a directory, or when multiple inputs share one -o.
    let dir_mode = out.is_some_and(looks_like_directory) || (out.is_some() && files.len() > 1);
    run_batch(files, |path| {
        convert_file(path, out, dir_mode, pretty, out_format, verbose)
    });
}

/// Convert one input. A cluster pipeline file (the full multi-stage ai-landscape
/// state) expands to one unified JSON per program; everything else is a single
/// unified file.
fn convert_file(
    input: &Path,
    out: Option<&Path>,
    dir_mode: bool,
    pretty: bool,
    out_format: DegreeFormat,
    verbose: bool,
) -> Result<(), String> {
    let contents = std::fs::read_to_string(input)
        .map_err(|e| format!("Failed to read {}: {e}", input.display()))?;

    if is_json_path(input) {
        // Only parse the Value to route by shape; malformed JSON falls through
        // to `convert_single`, which surfaces a proper parse error.
        if let Ok(value) = serde_json::from_str::<serde_json::Value>(&contents) {
            if let Some(programs) = nu_analytics::core::degree::extract_cluster_programs(&value) {
                let out_dir = cluster_out_dir(input, out);
                return convert_cluster(out_format, input, &programs, &out_dir, pretty, verbose);
            }
            // Valid JSON that is neither a cluster file, an ai-landscape program
            // (`courses` category map), nor a unified degree (top-level `degree`)
            // is skipped rather than failing the batch (e.g. pipeline sidecar
            // files like checkpoint/metrics in a cluster dump).
            if !is_landscape_value(&value) && value.get("degree").is_none() {
                println!(
                    "• {}: skipped (not a degree/program/cluster file)",
                    input.display()
                );
                return Ok(());
            }
        }
    }

    let out_path = resolve_convert_output(input, out, dir_mode, out_format);
    convert_single(input, &out_path, &contents, pretty, out_format, verbose)
}

/// Filename suffix for `degree convert` output (unified JSON).
/// Suffix for converted output, per format. `.unified.json` is what the pipeline and
/// the importer look for; `.unified.yaml` is the hand-editable twin.
fn convert_output_suffix(out_format: DegreeFormat) -> String {
    format!(".{}", out_format.extension())
}

/// Separator between school and program in a cluster output filename.
const CLUSTER_NAME_SEP: &str = "__";

/// File stem of `input` as a `&str`, or `default` when missing/non-UTF-8.
fn file_stem_or<'a>(input: &'a Path, default: &'a str) -> &'a str {
    input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(default)
}

/// Output path for `degree convert`: `<stem>.unified.{json,yaml}` (next to input, or
/// inside an `-o` directory), or the verbatim `-o` file for a single input.
fn resolve_convert_output(
    input: &Path,
    out: Option<&Path>,
    dir_mode: bool,
    out_format: DegreeFormat,
) -> PathBuf {
    let stem = file_stem_or(input, "degree");
    resolve_output_path(
        input,
        out,
        dir_mode,
        &format!("{stem}{}", convert_output_suffix(out_format)),
    )
}

/// Convert a single-program input (YAML, unified JSON, or a flat ai-landscape
/// program file) to one unified JSON at `out_path`.
fn convert_single(
    input: &Path,
    out_path: &Path,
    contents: &str,
    pretty: bool,
    out_format: DegreeFormat,
    verbose: bool,
) -> Result<(), String> {
    use nu_analytics::core::degree::json_parser::{
        parse_degree_json_with_warnings, to_unified_value,
    };

    if out_path == input {
        return Err(format!(
            "refusing to overwrite input file {}; pass an explicit -o path",
            input.display()
        ));
    }

    let (mut program, warnings) = if is_json_path(input) {
        parse_degree_json_with_warnings(contents)
            .map_err(|e| format!("Failed to parse {}: {e}", input.display()))?
    } else {
        let program = load_degree_from_yaml(input)
            .map_err(|e| format!("Failed to load {}: {e}", input.display()))?;
        (program, Vec::new())
    };
    let flagged = nu_analytics::core::degree::fill_electives::mark_fill_to_total(&mut program);

    let mut value = to_unified_value(&program)
        .map_err(|e| format!("Failed to build unified JSON for {}: {e}", input.display()))?;
    write_converted(
        &program, &mut value, &warnings, out_path, pretty, out_format,
    )?;

    println!("✓ Converted {} -> {}", input.display(), out_path.display());
    report_fill_flags(&flagged);
    report_warnings(&warnings, verbose);
    Ok(())
}

/// Say which requirements conversion marked as fill-to-total.
///
/// The flag changes how a block is sized in every plan, so it is announced rather than
/// set silently — the converted file is where a wrong call gets corrected.
fn report_fill_flags(flagged: &[String]) {
    if !flagged.is_empty() {
        println!("  • fills_to_total set on: {}", flagged.join(", "));
    }
}

/// Expand a cluster pipeline file into one unified JSON per program, written as
/// `<school-stem>__<program>.unified.json` under `out_dir`.
fn convert_cluster(
    out_format: DegreeFormat,
    input: &Path,
    programs: &[(String, nu_analytics::core::degree::LandscapeProgram)],
    out_dir: &Path,
    pretty: bool,
    verbose: bool,
) -> Result<(), String> {
    if programs.is_empty() {
        println!("• {}: no convertible programs", input.display());
        return Ok(());
    }

    let school = file_stem_or(input, "school");
    let mut total_warnings = 0usize;
    // Distinct program names can sanitize to the same stem; disambiguate so no
    // program silently overwrites another.
    let mut used_stems: HashSet<String> = HashSet::new();
    for (name, prog) in programs {
        total_warnings += write_cluster_program(
            input,
            school,
            name,
            prog,
            out_dir,
            &mut used_stems,
            pretty,
            out_format,
            verbose,
        )?;
    }

    let warn_note = if total_warnings > 0 {
        format!(", {total_warnings} warning(s)")
    } else {
        String::new()
    };
    println!(
        "✓ {}: {} program(s){warn_note}",
        input.display(),
        programs.len()
    );
    Ok(())
}

/// Convert one cluster program to `<school>__<program>.unified.json` under
/// `out_dir` (disambiguating colliding stems via `used_stems`), returning its
/// conversion-warning count.
#[allow(clippy::too_many_arguments)]
fn write_cluster_program(
    input: &Path,
    school: &str,
    name: &str,
    prog: &nu_analytics::core::degree::LandscapeProgram,
    out_dir: &Path,
    used_stems: &mut HashSet<String>,
    pretty: bool,
    out_format: DegreeFormat,
    verbose: bool,
) -> Result<usize, String> {
    use nu_analytics::core::degree::{convert_landscape, json_parser::to_unified_value};

    let mut result = convert_landscape(prog);
    let flagged =
        nu_analytics::core::degree::fill_electives::mark_fill_to_total(&mut result.program);
    let mut value = to_unified_value(&result.program).map_err(|e| {
        format!(
            "Failed to build unified JSON for {} / {name}: {e}",
            input.display()
        )
    })?;
    let base = format!(
        "{}{CLUSTER_NAME_SEP}{}",
        safe_filename(school),
        safe_filename(name)
    );
    let stem = unique_stem(base, used_stems);
    let out_path = out_dir.join(format!("{stem}{}", convert_output_suffix(out_format)));
    write_converted(
        &result.program,
        &mut value,
        &result.warnings,
        &out_path,
        pretty,
        out_format,
    )?;
    if verbose {
        println!("  ✓ {}", out_path.display());
        report_fill_flags(&flagged);
    }
    Ok(result.warnings.len())
}

/// Directory destination for a cluster file's per-program outputs: the `-o`
/// path (always a directory, since a cluster expands to many files; created on
/// demand), or next to the input when `-o` is omitted.
fn cluster_out_dir(input: &Path, out: Option<&Path>) -> PathBuf {
    if let Some(dir) = out {
        return dir.to_path_buf();
    }
    // `-o` omitted: write next to the input. `Path::parent` is `Some("")` for a
    // bare filename, so treat an empty parent as the current directory.
    input
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
}

/// Embed `conversion_warnings` (when any), create parent dirs, then write
/// `value` to `out_path` as JSON (pretty or compact).
/// Write a converted degree in the requested format.
///
/// The two formats are **not** the same document serialised two ways. JSON is the
/// unified shape — `to_unified_value`, with prerequisites structured. YAML is the
/// `DegreeProgram` shape, which is what `parse_degree_yaml` reads and what people
/// hand-author.
///
/// Writing unified-shaped YAML looks fine and round-trips lossily: serde drops the
/// fields the reader's struct does not name, so `external_credits`, `external_note` and
/// `external_requirement` came back `null`. Each format must be written in the shape its
/// own reader parses.
fn write_converted(
    program: &nu_analytics::core::DegreeProgram,
    value: &mut serde_json::Value,
    warnings: &[String],
    out_path: &Path,
    pretty: bool,
    out_format: DegreeFormat,
) -> Result<(), String> {
    match out_format {
        DegreeFormat::Json => write_unified_value(value, warnings, out_path, pretty),
        DegreeFormat::Yaml => {
            if let Some(parent) = out_path.parent() {
                if !parent.as_os_str().is_empty() {
                    std::fs::create_dir_all(parent)
                        .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
                }
            }
            let text = nu_analytics::core::degree::serialize_degree_yaml(program)
                .map_err(|e| format!("Failed to serialize YAML for {}: {e}", out_path.display()))?;
            std::fs::write(out_path, text)
                .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))
        }
    }
}

fn write_unified_value(
    value: &mut serde_json::Value,
    warnings: &[String],
    out_path: &Path,
    pretty: bool,
) -> Result<(), String> {
    if !warnings.is_empty() {
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "conversion_warnings".to_string(),
                serde_json::Value::Array(
                    warnings
                        .iter()
                        .map(|w| serde_json::Value::String(w.clone()))
                        .collect(),
                ),
            );
        }
    }
    if let Some(parent) = out_path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Failed to create {}: {e}", parent.display()))?;
        }
    }
    // Emit with `degree` first for readability (see unified_value_to_string).
    let text = nu_analytics::core::degree::unified_value_to_string(value, pretty)
        .map_err(|e| format!("Failed to serialize JSON for {}: {e}", out_path.display()))?;
    std::fs::write(out_path, text)
        .map_err(|e| format!("Failed to write {}: {e}", out_path.display()))
}

/// Print a conversion-warning tally, expanding each warning when `verbose`.
fn report_warnings(warnings: &[String], verbose: bool) {
    if warnings.is_empty() {
        return;
    }
    println!("  {} conversion warning(s)", warnings.len());
    if verbose {
        for w in warnings {
            println!("    - {w}");
        }
    }
}

/// Return a filename stem unique within `used`: `base`, else `base-2`, `base-3`,
/// … Records the chosen stem in `used`.
fn unique_stem(base: String, used: &mut HashSet<String>) -> String {
    if used.insert(base.clone()) {
        return base;
    }
    let mut n = 2;
    loop {
        let candidate = format!("{base}-{n}");
        if used.insert(candidate.clone()) {
            return candidate;
        }
        n += 1;
    }
}

/// True if `value` is an ai-landscape flat program: a `courses` object whose
/// values are arrays (category -> list).
fn is_landscape_value(value: &serde_json::Value) -> bool {
    value
        .get("courses")
        .and_then(serde_json::Value::as_object)
        .is_some_and(|m| m.values().next().is_some_and(serde_json::Value::is_array))
}

/// Replace filesystem-hostile characters with `_`. Local to this binary crate;
/// the library's equivalent (`plan_export::sanitize_filename`) is crate-private.
fn safe_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '_',
            _ => c,
        })
        .collect()
}

/// Load a degree program, dispatching on file extension: `.json` uses the
/// unified-JSON loader (which also auto-converts raw ai-landscape files), and
/// everything else is treated as YAML.
fn load_degree_auto<P: AsRef<Path>>(
    path: P,
) -> Result<nu_analytics::core::DegreeProgram, DegreeParseError> {
    let path = path.as_ref();
    if is_json_path(path) {
        load_degree_from_json(path)
    } else {
        load_degree_from_yaml(path)
    }
}

/// Save a degree program, dispatching on file extension to mirror
/// [`load_degree_auto`]: `.json` writes unified JSON, everything else YAML. The
/// error carries the destination path.
fn save_degree_auto(
    program: &nu_analytics::core::DegreeProgram,
    path: &Path,
) -> Result<(), String> {
    use nu_analytics::core::degree::{save_degree_to_json, save_degree_to_yaml};
    let result = if is_json_path(path) {
        save_degree_to_json(program, path)
    } else {
        save_degree_to_yaml(program, path)
    };
    result.map_err(|e| format!("Failed to write {}: {e}", path.display()))
}

fn load_degree_program(
    degree_path: &Path,
    verbose: bool,
) -> Result<nu_analytics::core::DegreeProgram, String> {
    if verbose {
        eprintln!("Starting degree analysis...");
        eprintln!("Loading degree program from: {}", degree_path.display());
    }

    let program = load_degree_auto(degree_path).map_err(|e| {
        format!(
            "Failed to load degree program from {}: {}",
            degree_path.display(),
            e
        )
    })?;

    if verbose {
        let degree = &program.degree;
        eprintln!("✓ Loaded degree: {} {}", degree.degree_type, degree.name);
        eprintln!("  Courses: {}", program.courses.len());
        eprintln!("  Requirements: {}", program.requirements.len());
    }

    Ok(program)
}

/// Print summary of selected plans
fn print_selection_summary(
    selected: &nu_analytics::core::degree::SelectedPlans,
    plans_processed: usize,
) {
    eprintln!("✓ Processed {plans_processed} plans");
    eprintln!();
    eprintln!("Selected Plans:");
    eprintln!(
        "  Shortest: {} terms",
        selected
            .shortest
            .as_ref()
            .map_or_else(|| "N/A".to_string(), |p| p.score.terms_required.to_string())
    );
    eprintln!(
        "  Longest: {} terms",
        selected
            .longest
            .as_ref()
            .map_or_else(|| "N/A".to_string(), |p| p.score.terms_required.to_string())
    );
    eprintln!(
        "  Calc-Ready: {}",
        if selected.calc_ready_shortest.is_some() {
            "found"
        } else {
            "N/A"
        }
    );
    eprintln!("  Random Samples: {}", selected.random_samples.len());
}

/// Write the HTML report and the metrics-directory files, as the `--no-*` flags allow.
fn generate_analysis_outputs(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    aggregator: &MetricsAggregator,
    selected: &nu_analytics::core::degree::SelectedPlans,
) -> Result<Vec<String>, String> {
    let mut outputs_generated = Vec::new();

    // Generate HTML report
    if !options.no_report {
        let report_path = generate_html_report(ctx, options, selected)?;
        outputs_generated.push(format!("Report: {}", report_path.display()));
    }

    if !options.no_metrics {
        outputs_generated.extend(export_metrics_files(ctx, options, aggregator, selected)?);
    }

    if ctx.verbose && !outputs_generated.is_empty() {
        eprintln!();
        eprintln!("Generated Files:");
        for output in &outputs_generated {
            eprintln!("  ✓ {output}");
        }
    }

    Ok(outputs_generated)
}

/// Write the metrics-directory files: the plan CSVs and the `index.csv` row unless
/// `--no-csv`, then the summary JSONL and the report JSON, which `--no-csv` leaves alone —
/// the report JSON is what `db import` reads. Returns a line per file for the verbose list.
fn export_metrics_files(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    aggregator: &MetricsAggregator,
    selected: &nu_analytics::core::degree::SelectedPlans,
) -> Result<Vec<String>, String> {
    let mut outputs = Vec::new();
    let metrics_dir = metrics_dir_or_default(options);
    let warn = |what: &str, e: &dyn std::fmt::Display| {
        if ctx.verbose {
            eprintln!("Warning: Failed to export {what}: {e}");
        }
    };

    if !options.no_csv {
        for path in export_csv_files(ctx, options, selected)? {
            outputs.push(format!("CSV: {path}"));
        }
        match export_index_csv(
            ctx.school,
            &ctx.program.degree,
            ctx.report_stats,
            selected,
            &metrics_dir,
        ) {
            Ok(path) => outputs.push(format!("Index: {}", path.display())),
            Err(e) => warn("index CSV", &e),
        }
    }

    match export_degree_summary_jsonl(
        ctx.school,
        &ctx.program.degree,
        ctx.report_stats,
        selected,
        &metrics_dir,
    ) {
        Ok(path) => outputs.push(format!("JSONL: {}", path.display())),
        Err(e) => warn("JSONL summary", &e),
    }

    // The whole degree structure plus degree- and course-level metrics, for downstream
    // visualization and `db import`.
    let run_params = ctx.run_parameters();
    match nu_analytics::core::report::unified_report::export_degree_report_json(
        ctx.program,
        aggregator,
        selected,
        run_params.sampling_strategy,
        &run_params,
        &metrics_dir,
    ) {
        Ok(path) => outputs.push(format!("Report JSON: {}", path.display())),
        Err(e) => warn("unified report JSON", &e),
    }
    Ok(outputs)
}

/// Generate HTML report
fn generate_html_report(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    selected: &nu_analytics::core::degree::SelectedPlans,
) -> Result<std::path::PathBuf, String> {
    let report_dir = options
        .report_dir
        .clone()
        .unwrap_or_else(|| std::path::PathBuf::from("."));

    // Create report directory if needed
    if !report_dir.exists() {
        std::fs::create_dir_all(&report_dir).map_err(|e| {
            format!(
                "Failed to create report directory {}: {e}",
                report_dir.display()
            )
        })?;
    }

    let report_path = report_dir.join(nu_analytics::core::report::report_file_name(
        &ctx.program.degree.degree_id(),
    ));

    if ctx.verbose {
        eprintln!();
        eprintln!("Generating HTML report: {}", report_path.display());
    }

    let report_ctx = DegreeReportContext::new(
        ctx.school,
        &ctx.program.degree,
        ctx.report_stats,
        selected,
        ctx.equivalences,
    );
    let generator = DegreeReportGenerator::new();
    generator
        .generate(&report_ctx, &report_path)
        .map_err(|e| format!("Failed to generate report: {e}"))?;

    Ok(report_path)
}

/// Export CSV files for selected plans
fn export_csv_files(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    selected: &nu_analytics::core::degree::SelectedPlans,
) -> Result<Vec<String>, String> {
    let metrics_dir = metrics_dir_or_default(options);
    let metrics_dir = metrics_dir.to_string_lossy();

    let export_config = PlanExportConfig {
        base_dir: format!("{metrics_dir}/plans"),
        create_dirs: true,
    };

    if ctx.verbose {
        eprintln!("Exporting CSV files to: {}", export_config.base_dir);
    }

    export_selected_plans(ctx.school, &ctx.program.degree, selected, &export_config)
        .map_err(|e| format!("Failed to export CSV files: {e}"))
}

/// Print final analysis summary
fn print_analysis_summary(
    ctx: &AnalysisContext<'_>,
    aggregator: &MetricsAggregator,
    plans_processed: usize,
) {
    println!();
    println!("Degree Analysis Complete");
    println!("========================");
    println!(
        "Degree: {} {}",
        ctx.program.degree.degree_type, ctx.program.degree.name
    );
    println!("Plans analyzed: {plans_processed}");

    let degree_stats = aggregator.degree_stats();
    println!();
    println!("Degree Statistics (across all plans):");
    println!(
        "  Complexity: median {:.1}, range {:.1}-{:.1}",
        degree_stats.total_complexity.median,
        degree_stats.total_complexity.min,
        degree_stats.total_complexity.max
    );
    println!(
        "  Longest Delay: median {:.1}, range {:.1}-{:.1}",
        degree_stats.longest_delay.median,
        degree_stats.longest_delay.min,
        degree_stats.longest_delay.max
    );
}

/// Validate selected plans and report any issues
#[allow(clippy::cast_precision_loss)] // Safe: credit values are small
fn validate_selected_plans(
    ctx: &AnalysisContext<'_>,
    selected: &nu_analytics::core::degree::SelectedPlans,
) {
    // Configure validation
    let validator_config = PlanValidatorConfig {
        target_credits: ctx.gen_config.target_credits.map(|c| c as f32),
        strict_prerequisites: false, // Non-strict for now, just report warnings
        ..Default::default()
    };

    let validator = PlanValidator::new(&ctx.program.courses, ctx.graph, validator_config);

    // Validate the shortest plan (most commonly used)
    if let Some(shortest) = &selected.shortest {
        let result = validator.validate(&shortest.variant);

        if ctx.verbose {
            eprintln!();
            eprintln!("Plan Validation (Shortest Path):");
            eprintln!("  Courses: {}", result.stats.total_courses);
            eprintln!("  Credits: {:.1}", result.stats.total_credits);
            eprintln!("  Placeholders: {}", result.stats.placeholder_courses);

            if !result.errors.is_empty() {
                eprintln!("  ⚠ Errors: {}", result.errors.len());
            }
            if !result.warnings.is_empty() {
                eprintln!("  ⚠ Warnings: {}", result.warnings.len());
            }
        }

        // Print detailed issues if there are any and verbose mode
        if ctx.verbose && (!result.errors.is_empty() || !result.warnings.is_empty()) {
            eprintln!();
            eprintln!("{}", result.format_report());
        }
    }
}

/// Print a separator between sections
fn print_separator() {
    println!();
    println!("================================================================================");
    println!();
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each option overrides the configuration; an unparseable `--sampling-strategy` falls
    /// back to the configuration's, not to the default.
    #[test]
    fn test_analysis_config_takes_each_option_over_the_configuration() {
        let mut config = Config::from_defaults();
        config.degree_analysis.sampling_strategy = "sequential".into();
        let none = AnalyzeOptions::default();
        let defaults = analysis_config(&none, &config);
        assert_eq!(
            (
                defaults.max_plans,
                defaults.sample_count,
                defaults.ignore_duplicates
            ),
            (
                config.degree_analysis.max_plans,
                config.degree_analysis.sample_plan_count,
                config.degree_analysis.ignore_duplicates
            )
        );
        assert_eq!(defaults.sampling_strategy, SamplingStrategy::Sequential);
        assert!(defaults.random_seed.is_none() && defaults.time_limit.is_none());

        let options = AnalyzeOptions {
            max_plans: Some(7),
            sample_plans: Some(2),
            full_run: true,
            sampling_strategy: Some("stratified".into()),
            include_courses: Some(vec!["CS1".into()]),
            target_course: Some("CS2".into()),
            ..Default::default()
        };
        let given = analysis_config(&options, &config);
        assert_eq!(
            (given.max_plans, given.sample_count, given.ignore_duplicates),
            (7, 2, false)
        );
        assert_eq!(given.sampling_strategy, SamplingStrategy::Stratified);
        assert_eq!(given.include_courses, ["CS1"]);
        assert_eq!(given.target_course, Some("CS2"));

        let bogus = AnalyzeOptions {
            sampling_strategy: Some("bogus".into()),
            ..Default::default()
        };
        assert_eq!(
            analysis_config(&bogus, &config).sampling_strategy,
            SamplingStrategy::Sequential
        );
    }

    /// A `--target-course` query writes the `--metrics-out` report, creating its directory,
    /// and no other output; it produces no rollup. An unwritable path is an error naming
    /// `--metrics-out`.
    #[test]
    fn test_a_target_course_run_writes_only_the_metrics_out_report() {
        let (program, _) = nu_analytics::core::degree::parse_degree_auto(include_str!(
            "../../../samples/degrees/csu-cs-bscs-general.yaml"
        ))
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("nested/deeper/cs165.json");
        let options = AnalyzeOptions {
            max_plans: Some(20),
            target_course: Some("CS165".into()),
            metrics_out: Some(out.clone()),
            report_dir: Some(dir.path().join("reports")),
            metrics_dir: Some(dir.path().join("metrics")),
            ..Default::default()
        };
        let config = Config::from_defaults();
        assert!(analyze_program(&program, &options, &config)
            .unwrap()
            .is_none());
        assert!(!dir.path().join("reports").exists() && !dir.path().join("metrics").exists());

        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&out).unwrap()).unwrap();
        assert_eq!(
            report["analysis"]["target_course_stats"]["course_id"],
            "CS165"
        );
        assert_eq!(report["analysis"]["parameters"]["max_plans"], 20);
        assert!(report["degree"].is_object() && report["courses"].is_object());

        let blocked = AnalyzeOptions {
            metrics_out: Some(out.join("under-a-file.json")),
            ..options
        };
        let err = analyze_program(&program, &blocked, &config).unwrap_err();
        assert!(err.starts_with("--metrics-out"), "{err}");
    }

    #[test]
    fn test_validate_degree_allows_an_unmatched_pattern_only_when_asked() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("pool.yaml");
        std::fs::write(
            &path,
            r#"
degree: {id: pool, institution: T, program: T, total_credits: 7, gpa_minimum: 2.0}
requirements:
  core: {name: Core, type: all, category: major, courses: [CS101]}
  humanities: {name: Hum, type: select, category: gen_ed, credits: 3, from: {pattern: "HUM:100+"}}
courses:
  CS101: {title: Intro, prefix: CS, number: "101", credits: 4}
"#,
        )
        .expect("write");
        assert!(
            validate_degree(&path, ValidationOptions::default(), false).is_err(),
            "an unmatched pattern is an error by default"
        );
        let allowed = ValidationOptions {
            allow_unmatched_patterns: true,
        };
        assert_eq!(validate_degree(&path, allowed, false), Ok(()));
    }

    #[test]
    fn test_unique_stem_disambiguates_collisions() {
        let mut used = HashSet::new();
        assert_eq!(
            unique_stem("School__CS".to_string(), &mut used),
            "School__CS"
        );
        // Same base again -> suffixed.
        assert_eq!(
            unique_stem("School__CS".to_string(), &mut used),
            "School__CS-2"
        );
        assert_eq!(
            unique_stem("School__CS".to_string(), &mut used),
            "School__CS-3"
        );
        // A distinct base is untouched.
        assert_eq!(
            unique_stem("School__AI".to_string(), &mut used),
            "School__AI"
        );
    }

    #[test]
    fn test_safe_filename_replaces_hostile_chars() {
        assert_eq!(
            safe_filename("Computer Science (BS): A/B"),
            "Computer_Science_(BS)__A_B"
        );
        // Clean names pass through untouched.
        assert_eq!(safe_filename("CS-BS_2024"), "CS-BS_2024");
    }

    #[test]
    fn test_cluster_out_dir_branches() {
        // None -> next to the input (parent dir); bare filename -> ".".
        assert_eq!(
            cluster_out_dir(Path::new("clusters/uni.json"), None),
            PathBuf::from("clusters")
        );
        assert_eq!(
            cluster_out_dir(Path::new("uni.json"), None),
            PathBuf::from(".")
        );
        // Any `-o` is the output directory (a cluster expands to many files,
        // so it can't target a single file).
        assert_eq!(
            cluster_out_dir(Path::new("clusters/uni.json"), Some(Path::new("out"))),
            PathBuf::from("out")
        );
    }

    #[test]
    fn test_is_landscape_value() {
        // `courses` object whose first value is an array -> landscape.
        let yes: serde_json::Value =
            serde_json::from_str(r#"{"courses":{"cs_course_core":[{"course_code":"CS1"}]}}"#)
                .unwrap();
        assert!(is_landscape_value(&yes));
        // Unified: course keys map to objects, not arrays.
        let unified: serde_json::Value =
            serde_json::from_str(r#"{"courses":{"CS1":{"name":"Intro"}}}"#).unwrap();
        assert!(!is_landscape_value(&unified));
        // No `courses`, or `courses` not an object.
        assert!(!is_landscape_value(&serde_json::json!({"degree": "BS"})));
        assert!(!is_landscape_value(&serde_json::json!({"courses": []})));
    }

    #[test]
    fn yaml_and_json_conversion_round_trips_without_losing_fields() {
        // The two formats are different shapes, not one document serialised twice:
        // JSON is the unified shape, YAML is the `DegreeProgram` shape its own parser
        // reads. Writing unified-shaped YAML round-tripped lossily — serde silently
        // drops fields the reader's struct does not name — so this pins that a degree
        // survives JSON -> YAML -> JSON.
        use nu_analytics::core::degree::{
            json_parser::to_unified_value, parse_degree_yaml, serialize_degree_yaml,
        };
        let json = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("tests/assets/degrees/bowdoin-college-computer-science.unified.json"),
        )
        .expect("fixture");
        let original = nu_analytics::core::degree::json_parser::parse_degree_json(&json)
            .expect("parse fixture");

        let yaml = serialize_degree_yaml(&original).expect("to yaml");
        let back = parse_degree_yaml(&yaml).expect("from yaml");

        let a = to_unified_value(&original).expect("unified a");
        let b = to_unified_value(&back).expect("unified b");
        assert_eq!(
            serde_json::to_string(&a).expect("a"),
            serde_json::to_string(&b).expect("b"),
            "a degree must survive a YAML round trip unchanged"
        );
    }

    #[test]
    fn test_resolve_convert_output_branches() {
        let input = Path::new("degrees/neu.json");
        assert_eq!(
            resolve_convert_output(input, None, false, DegreeFormat::Json),
            PathBuf::from("degrees/neu.unified.json")
        );
        assert_eq!(
            resolve_convert_output(input, Some(Path::new("out")), true, DegreeFormat::Json),
            PathBuf::from("out/neu.unified.json")
        );
        assert_eq!(
            resolve_convert_output(
                input,
                Some(Path::new("out/explicit.json")),
                false,
                DegreeFormat::Json
            ),
            PathBuf::from("out/explicit.json")
        );
    }

    #[test]
    fn test_is_yaml_path_yaml_extension() {
        assert!(is_yaml_path(Path::new("foo.yaml")));
        assert!(is_yaml_path(Path::new("samples/degrees/foo.yaml")));
    }

    #[test]
    fn test_is_yaml_path_yml_extension() {
        assert!(is_yaml_path(Path::new("foo.yml")));
    }

    #[test]
    fn test_is_yaml_path_case_insensitive() {
        assert!(is_yaml_path(Path::new("foo.YAML")));
        assert!(is_yaml_path(Path::new("foo.YML")));
        assert!(is_yaml_path(Path::new("foo.Yaml")));
    }

    #[test]
    fn test_is_yaml_path_other_extensions_rejected() {
        assert!(!is_yaml_path(Path::new("foo.md")));
        assert!(!is_yaml_path(Path::new("foo.json")));
        assert!(!is_yaml_path(Path::new("foo.txt")));
        assert!(!is_yaml_path(Path::new("foo")));
    }

    #[test]
    fn test_is_yaml_path_no_extension() {
        assert!(!is_yaml_path(Path::new("README")));
        assert!(!is_yaml_path(Path::new("/path/to/dir/")));
    }

    #[test]
    fn default_trim_output_appends_suffix_preserving_extension() {
        assert_eq!(
            default_trim_output(Path::new("degree.yaml")),
            PathBuf::from("degree_trimmed.yaml")
        );
        assert_eq!(
            default_trim_output(Path::new("my-degree.yml")),
            PathBuf::from("my-degree_trimmed.yml")
        );
        assert_eq!(
            default_trim_output(Path::new("samples/degrees/neu.yaml")),
            PathBuf::from("samples/degrees/neu_trimmed.yaml")
        );
    }

    #[test]
    fn default_trim_output_falls_back_for_missing_extension() {
        assert_eq!(
            default_trim_output(Path::new("degree")),
            PathBuf::from("degree_trimmed.yaml")
        );
    }

    #[test]
    fn looks_like_directory_honours_trailing_separator() {
        assert!(looks_like_directory(Path::new("out/")));
        // A path without a trailing separator and without an existing
        // directory on disk is treated as a file. (We use a name that
        // definitely doesn't exist.)
        assert!(!looks_like_directory(Path::new(
            "/tmp/__nuanalytics_test_definitely_missing_xyz"
        )));
    }

    #[test]
    fn looks_like_directory_detects_existing_dir() {
        let tmp = tempfile::TempDir::new().expect("tempdir");
        assert!(looks_like_directory(tmp.path()));
    }

    #[test]
    fn looks_like_directory_rejects_existing_file() {
        // An actual file on disk must not be misclassified as a directory,
        // even if some future caller bypasses the trailing-separator hint.
        let f = tempfile::NamedTempFile::new().expect("tempfile");
        assert!(!looks_like_directory(f.path()));
    }

    #[test]
    fn resolve_trim_output_none_falls_back_to_default() {
        let input = Path::new("samples/degrees/neu.yaml");
        assert_eq!(
            resolve_trim_output(input, None, false),
            PathBuf::from("samples/degrees/neu_trimmed.yaml")
        );
    }

    #[test]
    fn resolve_trim_output_dir_mode_joins_under_out() {
        let input = Path::new("samples/degrees/neu.yaml");
        let out = Path::new("trimmed");
        assert_eq!(
            resolve_trim_output(input, Some(out), true),
            PathBuf::from("trimmed/neu_trimmed.yaml")
        );
    }

    #[test]
    fn resolve_trim_output_file_mode_returns_out_verbatim() {
        let input = Path::new("samples/degrees/neu.yaml");
        let out = Path::new("out/explicit.yaml");
        assert_eq!(
            resolve_trim_output(input, Some(out), false),
            PathBuf::from("out/explicit.yaml")
        );
    }

    #[test]
    fn test_analyze_child_flags_default_is_empty() {
        // A worker started from default options should carry no extra flags.
        let flags = analyze_child_flags(&AnalyzeOptions::default());
        assert!(flags.is_empty(), "expected no flags, got {flags:?}");
    }

    #[test]
    fn test_analyze_child_flags_excludes_jobs_and_school() {
        // --jobs and --school must never be forwarded: the worker marker stops
        // re-pooling and the pool only runs when school mode is off.
        let opts = AnalyzeOptions {
            jobs: 16,
            school: Some("Northeastern".to_string()),
            ..Default::default()
        };
        let flags = analyze_child_flags(&opts);
        assert!(!flags.iter().any(|f| f == "--jobs" || f == "-j"));
        assert!(!flags.iter().any(|f| f == "--school"));
        assert!(flags.is_empty(), "expected no flags, got {flags:?}");
    }

    #[test]
    fn test_analyze_child_flags_propagates_result_affecting_options() {
        let opts = AnalyzeOptions {
            metrics_dir: Some(PathBuf::from("m")),
            report_dir: Some(PathBuf::from("r")),
            no_report: true,
            no_csv: true,
            no_metrics: true,
            max_plans: Some(500),
            sample_plans: Some(50),
            sampling_strategy: Some("shuffled".to_string()),
            calc_strategy: Some("median".to_string()),
            full_run: true,
            include_courses: Some(vec!["CS3500".to_string(), "MATH2331".to_string()]),
            ..Default::default()
        };
        let flags = analyze_child_flags(&opts);
        let joined = flags.join(" ");
        for expected in [
            "--metrics-dir m",
            "--report-dir r",
            "--no-report",
            "--no-csv",
            "--no-metrics",
            "--max-plans 500",
            "--sample-plans 50",
            "--sampling-strategy shuffled",
            "--calc-strategy median",
            "--full-run",
            "--include CS3500,MATH2331",
        ] {
            assert!(
                joined.contains(expected),
                "missing {expected:?} in {joined:?}"
            );
        }
    }

    #[test]
    fn test_analyze_child_flags_empty_include_is_omitted() {
        // An empty include list must not produce a dangling `--include`.
        let opts = AnalyzeOptions {
            include_courses: Some(Vec::new()),
            ..Default::default()
        };
        let flags = analyze_child_flags(&opts);
        assert!(!flags.iter().any(|f| f == "--include"));
    }

    #[test]
    fn test_analyze_child_flags_roundtrips_through_clap() {
        // The reconstructed flags must parse back on the `analyze` subcommand,
        // guarding against a flag name drifting away from args.rs.
        use crate::args::{Cli, Command, DegreeSubcommand, SamplingStrategyArg};
        use clap::Parser;

        let opts = AnalyzeOptions {
            no_report: true,
            max_plans: Some(1234),
            sampling_strategy: Some("stratified".to_string()),
            ..Default::default()
        };
        let mut argv = vec![
            "nuanalytics".to_string(),
            "degree".to_string(),
            "analyze".to_string(),
            "some.json".to_string(),
        ];
        argv.extend(analyze_child_flags(&opts));
        let cli = Cli::try_parse_from(argv).expect("reconstructed flags should parse");
        match cli.command {
            Command::Degree { subcommand } => match subcommand {
                DegreeSubcommand::Analyze {
                    no_report,
                    max_plans,
                    sampling_strategy,
                    ..
                } => {
                    assert!(no_report);
                    assert_eq!(max_plans, Some(1234));
                    assert_eq!(sampling_strategy, Some(SamplingStrategyArg::Stratified));
                }
                other => panic!("expected Analyze, got {other:?}"),
            },
            other => panic!("expected Degree command, got {other:?}"),
        }
    }

    #[test]
    fn test_metrics_dir_or_default_falls_back_to_metrics() {
        assert_eq!(
            metrics_dir_or_default(&AnalyzeOptions::default()),
            PathBuf::from("metrics")
        );
        let opts = AnalyzeOptions {
            metrics_dir: Some(PathBuf::from("custom")),
            ..Default::default()
        };
        assert_eq!(metrics_dir_or_default(&opts), PathBuf::from("custom"));
    }

    #[cfg(unix)]
    #[test]
    fn test_reap_finished_records_failed_exit_and_keeps_running() {
        use std::process::Command;
        // One child exits non-zero immediately; the other stays alive.
        let dead = Command::new("/bin/sh")
            .arg("-c")
            .arg("exit 7")
            .spawn()
            .expect("spawn dead");
        let alive = Command::new("/bin/sh")
            .arg("-c")
            .arg("sleep 30")
            .spawn()
            .expect("spawn alive");
        let mut running = vec![
            (PathBuf::from("dead.json"), dead),
            (PathBuf::from("alive.json"), alive),
        ];
        let mut failed: Vec<(PathBuf, String)> = Vec::new();

        // Poll until the short-lived child is reaped (it exits near-instantly).
        let mut total = 0;
        for _ in 0..200 {
            total += reap_finished(&mut running, &mut failed);
            if total >= 1 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        assert_eq!(total, 1, "the exited child should be reaped exactly once");
        assert_eq!(failed.len(), 1, "non-zero exit must be recorded");
        assert_eq!(failed[0].0, PathBuf::from("dead.json"));
        assert!(
            !failed[0].1.is_empty(),
            "a status string should be recorded"
        );
        assert_eq!(running.len(), 1, "the still-running child must remain");
        assert_eq!(running[0].0, PathBuf::from("alive.json"));

        let _ = running[0].1.kill();
        let _ = running[0].1.wait();
    }

    // --- --from-db helpers (offline; no DB) --------------------------------

    #[cfg(feature = "database")]
    #[test]
    fn test_stored_program_row_parses_the_select_shape() {
        // Pins `StoredProgramRow`'s contract against what the `programs` select
        // returns: `name` required, `unitid` optional, malformed rows dropped.
        // The dropping itself is `core::json::parse_json_array`'s, tested there;
        // what is local is which columns this row type demands.
        let value = serde_json::json!([
            {
                "program_key": "prog:1|11.0701|2024-2025|BS",
                "degree_id": "cs-2024",
                "name": "Computer Science",
                "unitid": 1,
                "institution_raw": "Test U",
                "catalog_year": "2024-2025",
                "document": {}
            },
            { "program_key": "prog:bad", "document": {} },
            "not-an-object"
        ]);
        let rows = nu_analytics::core::json::parse_json_array::<StoredProgramRow>(&value);
        assert_eq!(rows.len(), 1, "only the well-formed row parses");
        assert_eq!(rows[0].program_key, "prog:1|11.0701|2024-2025|BS");
        assert_eq!(rows[0].name, "Computer Science");
        assert_eq!(rows[0].unitid, Some(1));
    }

    #[cfg(feature = "database")]
    #[test]
    fn test_format_program_candidates_layout_and_fallbacks() {
        let rows = vec![
            StoredProgramRow {
                program_key: "prog:1|11.0701|2024-2025|BS".to_string(),
                name: "Computer Science".to_string(),
                unitid: Some(1),
                institution_raw: Some("Test University".to_string()),
                catalog_year: Some("2024-2025".to_string()),
                document: serde_json::Value::Null,
            },
            StoredProgramRow {
                program_key: "fp:abcdef".to_string(),
                name: "Data Science".to_string(),
                unitid: None,
                institution_raw: None,
                catalog_year: None,
                document: serde_json::Value::Null,
            },
        ];
        let lines = format_program_candidates(&rows);
        assert_eq!(
            lines[0],
            "prog:1|11.0701|2024-2025|BS  ·  Computer Science  ·  Test University  ·  2024-2025"
        );
        // Missing institution_raw / catalog_year fall back to placeholders.
        assert_eq!(
            lines[1],
            "fp:abcdef  ·  Data Science  ·  (unknown)  ·  (no catalog year)"
        );
    }
}
