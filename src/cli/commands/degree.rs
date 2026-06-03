//! Degree command handler for validating degree program YAML files

use std::collections::HashMap as StdHashMap;

use nu_analytics::config::Config;
use nu_analytics::core::degree::audit::{
    detect_lowest_course_level, find_deep_chains, find_upper_level_without_prereqs,
};
use nu_analytics::core::degree::{
    load_degree_from_json, load_degree_from_yaml, DegreeParseError, PlanGenerator,
    PlanGeneratorConfig, PlanSelector, PlanSelectorConfig, PlanValidator, PlanValidatorConfig,
    PlanVariant, SamplingStrategy,
};
use nu_analytics::core::metrics::compute_all_metrics;
use nu_analytics::core::models::course_graph::{CourseNode, PrerequisiteEdge, PrerequisiteType};
use nu_analytics::core::models::degree::Requirement;
use nu_analytics::core::models::{CourseGraph, School, DAG};
use nu_analytics::core::report::degree_report::{DegreeReportContext, DegreeReportGenerator};
use nu_analytics::core::report::plan_export::{
    export_degree_summary_jsonl, export_index_csv, export_selected_plans, PlanExportConfig,
};
use nu_analytics::core::report::term_scheduler::SchedulerConfig;
use nu_analytics::core::statistics::aggregator::{AggregatorConfig, MetricsAggregator};
use nu_analytics::core::validate_degree_program;
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
/// * `verbose` - Whether to print verbose output
///
/// # Returns
/// Returns `Ok(())` if validation succeeds, `Err(String)` with error message if it fails
pub fn validate_degree(degree_path: &Path, verbose: bool) -> Result<(), String> {
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

    // Run validation
    let result = validate_degree_program(&program);

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
        for cycle in &result.cycles {
            println!("  {} → {}", cycle[0], cycle.join(" → "));
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
    /// Calculation strategy override ("median" or "mean") - reserved for future use
    #[allow(dead_code)]
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
    /// Skip CSV export
    pub no_csv: bool,
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
}

/// Run `degree validate` over one or more files.
pub fn run_validate(files: &[PathBuf], verbose: bool) {
    run_batch(files, |path| validate_degree(path, verbose));
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
/// down the whole run; single-file, `--school`, `-j 1`, and worker-mode
/// invocations run in-process.
pub fn run_analyze(files: &[PathBuf], options: &AnalyzeOptions, config: &Config) {
    let in_worker = std::env::var_os(WORKER_ENV).is_some();
    if in_worker || options.jobs <= 1 || options.school.is_some() {
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
    if !options.no_csv {
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

/// Run `degree analyze` in-process (single file, school mode, `-j 1`, or as a
/// spawned worker). Without `--school`, each file is analyzed independently;
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
            Ok(rollup) => rollups.push(rollup),
            Err(e) => {
                eprintln!("Error: {e}");
                had_failure = true;
            }
        }
    }

    if !rollups.is_empty() {
        let metrics_dir = options
            .metrics_dir
            .as_ref()
            .map_or_else(|| std::path::PathBuf::from("metrics"), Clone::clone);
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

    let yaml_inputs = filter_yaml_inputs(inputs);
    if yaml_inputs.is_empty() {
        eprintln!("Error: No YAML files to process after filtering.");
        process::exit(1);
    }

    let dir_mode = out.is_some_and(looks_like_directory);

    if let Some(file_out) = out.filter(|_| yaml_inputs.len() > 1 && !dir_mode) {
        eprintln!(
            "Error: -o {} is a file path, but {} input files were given; pass a directory (or end the path with '/') instead",
            file_out.display(),
            yaml_inputs.len()
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

    let total = yaml_inputs.len();
    let mut had_failure = false;
    for (idx, input) in yaml_inputs.iter().enumerate() {
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

/// Pick out the YAML inputs from a mixed list of paths, warning to stderr
/// about anything skipped. Shared between [`run_trim`] and [`run_batch`].
/// Filter `files` to those `accept`ed, warning (with `expected`) about the rest.
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

fn filter_yaml_inputs(files: &[PathBuf]) -> Vec<&Path> {
    filter_inputs(files, is_yaml_path, "YAML")
}

/// Returns `true` if the path has a `.yaml` or `.yml` extension (case-insensitive).
fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

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
    use nu_analytics::core::degree::{save_degree_to_yaml, trim_program, TrimOptions};

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

    save_degree_to_yaml(&trimmed, out_path)
        .map_err(|e| format!("Failed to write {}: {}", out_path.display(), e))?;

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

/// Analysis context holding all data needed for degree analysis
struct AnalysisContext<'a> {
    program: &'a nu_analytics::core::DegreeProgram,
    school: School,
    dag: DAG,
    graph: &'a CourseGraph,
    gen_config: PlanGeneratorConfig,
    verbose: bool,
    /// Map from course key to all equivalent courses (including itself)
    /// Built from requirement definitions using `{A, B, C}` syntax
    equivalences: HashMap<String, HashSet<String>>,
    /// Courses to avoid adding as prerequisites
    /// Built from alternatives to included courses in `select count:1` requirements
    exclude_from_prereqs: HashSet<String>,
}

/// Build an equivalence map from requirement definitions
///
/// Scans requirements for equivalent course syntax like `{MATH215, MATH241, MATH251A}`
/// and builds a bidirectional map where each course maps to all its equivalents.
fn build_equivalence_map(
    requirements: &HashMap<String, Requirement>,
) -> HashMap<String, HashSet<String>> {
    let mut equivalences: HashMap<String, HashSet<String>> = HashMap::new();

    for req in requirements.values() {
        // Check courses list for equivalent syntax
        if let Some(courses) = &req.courses {
            for course_ref in courses {
                if let Some(equiv_set) = parse_equivalent_courses(course_ref) {
                    // Add bidirectional mappings
                    for course in &equiv_set {
                        equivalences
                            .entry(course.clone())
                            .or_default()
                            .extend(equiv_set.iter().cloned());
                    }
                }
            }
        }

        // Check from.courses for equivalent syntax
        if let Some(from) = &req.from {
            if let Some(courses) = &from.courses {
                for course_ref in courses {
                    if let Some(equiv_set) = parse_equivalent_courses(course_ref) {
                        for course in &equiv_set {
                            equivalences
                                .entry(course.clone())
                                .or_default()
                                .extend(equiv_set.iter().cloned());
                        }
                    }
                }
            }
        }

        // Check nested options
        if let Some(options) = &req.options {
            for option in options {
                for nested in &option.requirements {
                    if let Some(courses) = &nested.courses {
                        for course_ref in courses {
                            if let Some(equiv_set) = parse_equivalent_courses(course_ref) {
                                for course in &equiv_set {
                                    equivalences
                                        .entry(course.clone())
                                        .or_default()
                                        .extend(equiv_set.iter().cloned());
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    equivalences
}

/// Build a set of courses to exclude from prerequisite expansion
///
/// Identifies courses that should be excluded when expanding prerequisites for included courses.
/// This prevents adding conflicting prerequisite paths.
///
/// Excludes:
/// 1. Alternative prerequisite paths: If MATH156 is included with prereqs `(MATH124 & MATH126) | MATH127`,
///    we exclude the longer path (MATH124, MATH125, MATH126, etc.) and prefer the shorter (MATH127).
/// 2. Pathway courses: Courses whose prerequisites REQUIRE excluded courses (all alternatives excluded).
fn build_exclude_set(include_courses: &[String], graph: &CourseGraph) -> HashSet<String> {
    let mut exclude_set = HashSet::new();

    if include_courses.is_empty() {
        return exclude_set;
    }

    let include_set: HashSet<&str> = include_courses.iter().map(String::as_str).collect();

    // Phase 1: For included courses with OR-group prerequisites, exclude the non-preferred paths
    // For example, if MATH156 is included with prereqs `(MATH124 & MATH126) | MATH127`,
    // we exclude MATH124, MATH125, MATH126, MATH117, MATH118 (the longer path).
    for include_course in include_courses {
        if let Some(node) = graph.get(include_course) {
            exclude_set.extend(find_excluded_prereq_paths(node, graph));
        }
    }

    // Phase 2: Expand exclusions to include "pathway" courses
    // These are courses that REQUIRE excluded courses as prerequisites
    // We iterate until no new exclusions are found
    let mut changed = true;
    while changed {
        changed = false;
        let current_excludes: Vec<String> = exclude_set.iter().cloned().collect();

        for (course_key, node) in graph.iter() {
            // Skip if already excluded or included
            if exclude_set.contains(course_key) || include_set.contains(course_key.as_str()) {
                continue;
            }

            // Check if this course REQUIRES any excluded course
            // (i.e., all OR-alternatives for a prereq group are excluded)
            if course_requires_excluded(node, &current_excludes, &include_set, graph) {
                exclude_set.insert(course_key.clone());
                changed = true;
            }
        }
    }

    exclude_set
}

/// Find courses that should be excluded based on included course's prereq OR-groups
///
/// For an included course like MATH156 with prereqs `(MATH124 & MATH126) | MATH127`,
/// we identify the shorter path (MATH127) and exclude courses that are ONLY needed
/// for the longer path (MATH124, MATH125, MATH126, MATH117, MATH118).
fn find_excluded_prereq_paths(node: &CourseNode, graph: &CourseGraph) -> HashSet<String> {
    let mut excluded = HashSet::new();

    // Group prerequisites by their or_group (only care about actual OR-groups, not None)
    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // For each OR-group, identify the shorter path and exclude courses from longer paths
    for (group_id, edges) in or_groups {
        // Skip non-OR-groups (required prereqs)
        if group_id.is_none() || edges.len() <= 1 {
            continue;
        }

        // Calculate the total prerequisite chain length for each option
        let mut option_chains: Vec<(String, HashSet<String>)> = Vec::new();

        for edge in &edges {
            let chain = collect_all_prereqs(&edge.prerequisite, graph, &mut HashSet::new());
            option_chains.push((edge.prerequisite.clone(), chain));
        }

        // Find the option with the shortest total chain
        if let Some((shortest_prereq, shortest_chain)) =
            option_chains.iter().min_by_key(|(_, chain)| chain.len())
        {
            // Exclude courses from other chains that aren't in the shortest chain
            for (prereq, chain) in &option_chains {
                if prereq != shortest_prereq {
                    for course in chain {
                        if !shortest_chain.contains(course) {
                            excluded.insert(course.clone());
                        }
                    }
                    // Also exclude the top-level alternative prereq itself
                    if !shortest_chain.contains(prereq) {
                        excluded.insert(prereq.clone());
                    }
                }
            }
        }
    }

    excluded
}

/// Collect all prerequisites transitively for a course
fn collect_all_prereqs(
    course: &str,
    graph: &CourseGraph,
    visited: &mut HashSet<String>,
) -> HashSet<String> {
    let mut prereqs = HashSet::new();

    if visited.contains(course) {
        return prereqs;
    }
    visited.insert(course.to_string());

    let Some(node) = graph.get(course) else {
        return prereqs;
    };

    for edge in &node.prerequisites {
        prereqs.insert(edge.prerequisite.clone());
        prereqs.extend(collect_all_prereqs(&edge.prerequisite, graph, visited));
    }

    prereqs
}

/// Check if a course requires an excluded course (no valid alternative)
///
/// Returns true if the course has a prerequisite OR-group where ALL options
/// are either excluded or their prerequisites require excluded courses.
fn course_requires_excluded(
    node: &CourseNode,
    exclude_set: &[String],
    include_set: &HashSet<&str>,
    graph: &CourseGraph,
) -> bool {
    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // Check each OR-group
    for (group_id, edges) in or_groups {
        // Handle required prerequisites (not part of any OR-group)
        if group_id.is_none() {
            for edge in &edges {
                if edge.prereq_type == PrerequisiteType::Required
                    && exclude_set.contains(&edge.prerequisite)
                    && !include_set.contains(edge.prerequisite.as_str())
                {
                    return true;
                }
            }
            continue;
        }

        // For OR-groups, check if ALL options are problematic
        // (excluded directly, or their prereqs are exclusively excluded)
        let all_problematic = edges.iter().all(|edge| {
            let prereq = &edge.prerequisite;

            // Directly excluded
            if exclude_set.contains(prereq) && !include_set.contains(prereq.as_str()) {
                return true;
            }

            // Check if this prereq's prerequisites eventually require excluded courses
            prereq_chain_requires_excluded(
                prereq,
                exclude_set,
                include_set,
                graph,
                &mut HashSet::new(),
            )
        });

        if all_problematic && !edges.is_empty() {
            return true;
        }
    }

    false
}

/// Recursively check if a course's prerequisite chain requires excluded courses
///
/// Returns true if ALL prerequisite options for ANY OR-group lead to excluded courses.
/// This handles transitive exclusions where a course's prerequisites eventually
/// require an excluded course with no valid alternatives.
fn prereq_chain_requires_excluded(
    course: &str,
    exclude_set: &[String],
    include_set: &HashSet<&str>,
    graph: &CourseGraph,
    visited: &mut HashSet<String>,
) -> bool {
    // Avoid infinite loops
    if visited.contains(course) {
        return false;
    }
    visited.insert(course.to_string());

    // If included, it's fine
    if include_set.contains(course) {
        return false;
    }

    // If excluded, this path requires excluded courses
    if exclude_set.contains(&course.to_string()) {
        return true;
    }

    // Check the course's prerequisites
    let Some(node) = graph.get(course) else {
        return false;
    };

    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // Check if any OR-group has all options leading to excluded courses
    for (_group_id, edges) in or_groups {
        if edges.is_empty() {
            continue;
        }

        let all_require_excluded = edges.iter().all(|edge| {
            prereq_chain_requires_excluded(
                &edge.prerequisite,
                exclude_set,
                include_set,
                graph,
                visited,
            )
        });

        if all_require_excluded {
            return true;
        }
    }

    false
}

/// Group prerequisite edges by their OR-group
///
/// Returns a map where:
/// - `None` key contains required prerequisites (not part of any OR-group)
/// - Numeric keys contain optional prerequisites grouped by their `or_group` ID
fn group_prereqs_by_or_group(
    prerequisites: &[PrerequisiteEdge],
) -> StdHashMap<Option<usize>, Vec<&PrerequisiteEdge>> {
    let mut or_groups: StdHashMap<Option<usize>, Vec<&PrerequisiteEdge>> = StdHashMap::new();
    for edge in prerequisites {
        or_groups.entry(edge.or_group).or_default().push(edge);
    }
    or_groups
}

/// Parse equivalent courses from `{A, B, C}` syntax
///
/// Returns `Some(set)` if the input is an equivalent group, `None` otherwise.
fn parse_equivalent_courses(course_ref: &str) -> Option<HashSet<String>> {
    if course_ref.starts_with('{') && course_ref.ends_with('}') {
        let inner = &course_ref[1..course_ref.len() - 1];
        let courses: HashSet<String> = inner
            .split(',')
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if courses.len() > 1 {
            return Some(courses);
        }
    }
    None
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
) -> Result<nu_analytics::core::report::unified_report::ProgramRollup, String> {
    let verbose = options.verbose;

    // Load and validate the degree program
    let program = load_degree_program(degree_path, verbose)?;

    // Build course graph and handle cycles by breaking them
    let mut graph_result = CourseGraph::from_degree_program(&program);
    if !graph_result.cycles.is_empty() {
        if verbose {
            eprintln!(
                "⚠ Detected {} circular prerequisite(s), breaking cycles...",
                graph_result.cycles.len()
            );
        }
        let removed = graph_result.graph.break_cycles(&graph_result.cycles);
        if verbose {
            for (course, prereq) in &removed {
                eprintln!("  Removed edge: {course} → {prereq}");
            }
        }
        // Clear cycles since we broke them
        graph_result.cycles.clear();
    }

    // Parse sampling strategy: CLI option takes precedence over config
    let sampling_strategy = options
        .sampling_strategy
        .as_ref()
        .and_then(|s| s.parse::<SamplingStrategy>().ok())
        .unwrap_or_else(|| {
            config
                .degree_analysis
                .sampling_strategy
                .parse::<SamplingStrategy>()
                .unwrap_or_default()
        });

    // Build equivalence map from requirements
    let equivalences = build_equivalence_map(&program.requirements);

    // Build exclude set from alternatives to included courses
    let include_courses = options.include_courses.clone().unwrap_or_default();
    let exclude_from_prereqs = build_exclude_set(&include_courses, &graph_result.graph);

    // Build analysis context
    let ctx = AnalysisContext {
        program: &program,
        school: build_school_from_program(&program),
        dag: build_dag_from_graph(&graph_result.graph),
        graph: &graph_result.graph,
        gen_config: PlanGeneratorConfig {
            max_plans: options
                .max_plans
                .unwrap_or(config.degree_analysis.max_plans),
            // Default is ignore_duplicates=true; --full-run disables it
            ignore_duplicates: !options.full_run && config.degree_analysis.ignore_duplicates,
            sample_count: options
                .sample_plans
                .unwrap_or(config.degree_analysis.sample_plan_count),
            target_credits: program.degree.total_credits,
            sampling_strategy,
            include_courses,
            exclude_courses: exclude_from_prereqs.iter().cloned().collect(),
            ..Default::default()
        },
        verbose,
        equivalences,
        exclude_from_prereqs,
    };

    // Run plan enumeration and metrics aggregation
    let (aggregator, selected, plans_processed) = enumerate_and_analyze_plans(&ctx);

    // Validate the selected plans and report any issues
    validate_selected_plans(&ctx, &selected);

    // Generate outputs
    generate_analysis_outputs(&ctx, options, &aggregator, &selected)?;

    // Print summary
    print_analysis_summary(&ctx, &aggregator, plans_processed);

    Ok(
        nu_analytics::core::report::unified_report::ProgramRollup::from_analysis(
            ctx.program,
            &aggregator,
        ),
    )
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

/// Run `degree convert` over one or more inputs, emitting unified JSON.
pub fn run_convert(files: &[PathBuf], out: Option<&Path>, pretty: bool, verbose: bool) {
    // Directory mode when -o is a directory, or when multiple inputs share one -o.
    let dir_mode = out.is_some_and(looks_like_directory) || (out.is_some() && files.len() > 1);
    run_batch(files, |path| {
        convert_file(path, out, dir_mode, pretty, verbose)
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
                return convert_cluster(input, &programs, &out_dir, pretty, verbose);
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

    let out_path = resolve_convert_output(input, out, dir_mode);
    convert_single(input, &out_path, &contents, pretty, verbose)
}

/// Filename suffix for `degree convert` output (unified JSON).
const CONVERT_OUTPUT_SUFFIX: &str = ".unified.json";

/// Separator between school and program in a cluster output filename.
const CLUSTER_NAME_SEP: &str = "__";

/// File stem of `input` as a `&str`, or `default` when missing/non-UTF-8.
fn file_stem_or<'a>(input: &'a Path, default: &'a str) -> &'a str {
    input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or(default)
}

/// Output path for `degree convert`: `<stem>.unified.json` (next to input, or
/// inside an `-o` directory), or the verbatim `-o` file for a single input.
fn resolve_convert_output(input: &Path, out: Option<&Path>, dir_mode: bool) -> PathBuf {
    let stem = file_stem_or(input, "degree");
    resolve_output_path(
        input,
        out,
        dir_mode,
        &format!("{stem}{CONVERT_OUTPUT_SUFFIX}"),
    )
}

/// Convert a single-program input (YAML, unified JSON, or a flat ai-landscape
/// program file) to one unified JSON at `out_path`.
fn convert_single(
    input: &Path,
    out_path: &Path,
    contents: &str,
    pretty: bool,
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

    let (program, warnings) = if is_json_path(input) {
        parse_degree_json_with_warnings(contents)
            .map_err(|e| format!("Failed to parse {}: {e}", input.display()))?
    } else {
        let program = load_degree_from_yaml(input)
            .map_err(|e| format!("Failed to load {}: {e}", input.display()))?;
        (program, Vec::new())
    };

    let mut value = to_unified_value(&program)
        .map_err(|e| format!("Failed to build unified JSON for {}: {e}", input.display()))?;
    write_unified_value(&mut value, &warnings, out_path, pretty)?;

    println!("✓ Converted {} -> {}", input.display(), out_path.display());
    report_warnings(&warnings, verbose);
    Ok(())
}

/// Expand a cluster pipeline file into one unified JSON per program, written as
/// `<school-stem>__<program>.unified.json` under `out_dir`.
fn convert_cluster(
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
    verbose: bool,
) -> Result<usize, String> {
    use nu_analytics::core::degree::{convert_landscape, json_parser::to_unified_value};

    let result = convert_landscape(prog);
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
    let out_path = out_dir.join(format!("{stem}{CONVERT_OUTPUT_SUFFIX}"));
    write_unified_value(&mut value, &result.warnings, &out_path, pretty)?;
    if verbose {
        println!("  ✓ {}", out_path.display());
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
    let text = if pretty {
        serde_json::to_string_pretty(value)
    } else {
        serde_json::to_string(value)
    }
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

/// Enumerate all plans and compute aggregated metrics
fn enumerate_and_analyze_plans(
    ctx: &AnalysisContext<'_>,
) -> (
    MetricsAggregator,
    nu_analytics::core::degree::SelectedPlans,
    usize,
) {
    // Create plan generator
    let generator = PlanGenerator::new(
        &ctx.program.requirements,
        &ctx.program.courses,
        ctx.gen_config.clone(),
    );
    let stats = generator.get_stats();

    if ctx.verbose {
        eprintln!();
        eprintln!("Plan Generation:");
        if !ctx.gen_config.include_courses.is_empty() {
            eprintln!(
                "  Included courses: {}",
                ctx.gen_config.include_courses.join(", ")
            );
        }
        eprintln!("  Estimated total plans: {}", stats.total_possible);
        eprintln!("  Variable requirements: {}", stats.variable_requirements);
        if stats.total_possible > ctx.gen_config.max_plans {
            eprintln!(
                "  ⚠ Will cap at {} plans (use --max-plans to adjust)",
                ctx.gen_config.max_plans
            );
        }
    }

    // Configure metrics aggregation
    let agg_config = AggregatorConfig {
        reservoir_size: 1000,
        track_per_course: true,
        exact_mode: stats.total_possible <= 10000,
    };

    // Configure plan selection
    let selector_config = PlanSelectorConfig {
        sample_count: ctx.gen_config.sample_count,
        scheduler_config: SchedulerConfig::default(),
        ..Default::default()
    };

    // Initialize aggregator and selector
    let mut aggregator = MetricsAggregator::new(agg_config);
    let mut selector = PlanSelector::new(&ctx.school, &ctx.dag, selector_config);

    if ctx.verbose {
        eprintln!();
        eprintln!("Processing plans...");
    }

    // Process plans
    let plans_processed = process_plan_variants(ctx, &generator, &mut aggregator, &mut selector);

    let selected = selector.into_selected_plans();

    if ctx.verbose {
        print_selection_summary(&selected, plans_processed);
    }

    (aggregator, selected, plans_processed)
}

/// Process all plan variants, computing metrics and updating aggregator/selector
fn process_plan_variants(
    ctx: &AnalysisContext<'_>,
    generator: &PlanGenerator<'_>,
    aggregator: &mut MetricsAggregator,
    selector: &mut PlanSelector<'_>,
) -> usize {
    let mut plans_processed = 0;
    let mut seen_fingerprints = HashSet::new();
    let progress_interval = (ctx.gen_config.max_plans / 20).max(100);

    for variant in generator.generate() {
        if plans_processed >= ctx.gen_config.max_plans {
            break;
        }

        // Skip duplicates if configured
        if ctx.gen_config.ignore_duplicates {
            let fp = variant.fingerprint();
            if seen_fingerprints.contains(&fp) {
                continue;
            }
            seen_fingerprints.insert(fp);
        }

        // Expand courses to include all prerequisites
        let include_set: HashSet<String> = ctx.gen_config.include_courses.iter().cloned().collect();
        let expanded_courses = expand_courses_with_prerequisites(
            &variant.courses,
            ctx.graph,
            &ctx.equivalences,
            &ctx.exclude_from_prereqs,
            &include_set,
        );

        // Build plan-specific DAG and compute metrics
        let plan_dag = build_dag_for_plan(
            &expanded_courses,
            ctx.graph,
            &include_set,
            &ctx.equivalences,
        );
        let course_metrics = match compute_all_metrics(&plan_dag) {
            Ok(metrics) => metrics,
            Err(e) => {
                if ctx.verbose {
                    eprintln!("  Warning: Failed to compute metrics for plan: {e}");
                }
                continue;
            }
        };

        // Create the expanded variant (adds elective placeholders to reach target credits)
        let expanded_variant = create_expanded_variant(
            &variant,
            &expanded_courses,
            &ctx.school,
            ctx.gen_config.target_credits,
        );

        // Use the variant's total_credits which includes elective placeholders
        aggregator.add_plan(&course_metrics, f64::from(expanded_variant.total_credits));

        // Update plan selection
        selector.process_plan(&expanded_variant, &course_metrics, &plan_dag);

        plans_processed += 1;

        // Progress reporting
        if ctx.verbose && plans_processed % progress_interval == 0 {
            eprintln!("  Processed {plans_processed} plans...");
        }
    }

    plans_processed
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

/// Generate HTML report and CSV exports
fn generate_analysis_outputs(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    aggregator: &MetricsAggregator,
    selected: &nu_analytics::core::degree::SelectedPlans,
) -> Result<Vec<String>, String> {
    let mut outputs_generated = Vec::new();

    // Generate HTML report
    if !options.no_report {
        let report_path = generate_html_report(ctx, options, aggregator, selected)?;
        outputs_generated.push(format!("Report: {}", report_path.display()));
    }

    // Export CSV files (including JSONL and index for aggregation)
    if !options.no_csv {
        let exported = export_csv_files(ctx, options, selected)?;
        for path in exported {
            outputs_generated.push(format!("CSV: {path}"));
        }

        // Export JSONL and index CSV for aggregation (only when CSV export is enabled)
        let metrics_dir = options
            .metrics_dir
            .as_ref()
            .map_or_else(|| std::path::PathBuf::from("metrics"), Clone::clone);

        // Export degree summary JSONL
        match export_degree_summary_jsonl(
            &ctx.school,
            &ctx.program.degree,
            aggregator,
            selected,
            &metrics_dir,
        ) {
            Ok(path) => outputs_generated.push(format!("JSONL: {}", path.display())),
            Err(e) => {
                if ctx.verbose {
                    eprintln!("Warning: Failed to export JSONL summary: {e}");
                }
            }
        }

        // Export to index CSV for multi-degree analysis
        match export_index_csv(
            &ctx.school,
            &ctx.program.degree,
            aggregator,
            selected,
            &metrics_dir,
        ) {
            Ok(path) => outputs_generated.push(format!("Index: {}", path.display())),
            Err(e) => {
                if ctx.verbose {
                    eprintln!("Warning: Failed to export index CSV: {e}");
                }
            }
        }

        // Unified metrics-rich report JSON (the whole degree structure plus
        // degree- and course-level metrics) for downstream viz/DB. Grouped with
        // the other metrics-dir exports, so `--no-csv` suppresses it too.
        let sample_type = sampling_strategy_label(&ctx.gen_config.sampling_strategy);
        match nu_analytics::core::report::unified_report::export_degree_report_json(
            ctx.program,
            aggregator,
            selected,
            sample_type,
            &metrics_dir,
        ) {
            Ok(path) => outputs_generated.push(format!("Report JSON: {}", path.display())),
            Err(e) => {
                if ctx.verbose {
                    eprintln!("Warning: Failed to export unified report JSON: {e}");
                }
            }
        }
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

/// Human-readable label for a sampling strategy (used as `sample_type` in the
/// unified report JSON).
const fn sampling_strategy_label(strategy: &SamplingStrategy) -> &'static str {
    match strategy {
        SamplingStrategy::Sequential => "sequential",
        SamplingStrategy::Shuffled => "shuffled",
        SamplingStrategy::Stratified => "stratified",
    }
}

/// Generate HTML report
fn generate_html_report(
    ctx: &AnalysisContext<'_>,
    options: &AnalyzeOptions,
    aggregator: &MetricsAggregator,
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

    let report_path = report_dir.join(format!("{}-analysis.html", ctx.program.degree.degree_id()));

    if ctx.verbose {
        eprintln!();
        eprintln!("Generating HTML report: {}", report_path.display());
    }

    let report_ctx = DegreeReportContext::new(
        &ctx.school,
        &ctx.program.degree,
        aggregator,
        selected,
        &ctx.dag,
        &ctx.equivalences,
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
    let metrics_dir = options.metrics_dir.as_ref().map_or_else(
        || "metrics".to_string(),
        |p| p.to_string_lossy().to_string(),
    );

    let export_config = PlanExportConfig {
        base_dir: format!("{metrics_dir}/plans"),
        create_dirs: true,
    };

    if ctx.verbose {
        eprintln!("Exporting CSV files to: {}", export_config.base_dir);
    }

    export_selected_plans(&ctx.school, &ctx.program.degree, selected, &export_config)
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

/// Build a School model from the degree program
fn build_school_from_program(program: &nu_analytics::core::DegreeProgram) -> School {
    let mut school = School::new(
        program
            .degree
            .institution
            .clone()
            .unwrap_or_else(|| "Unknown".to_string()),
    );

    for (key, course) in &program.courses {
        let mut school_course = nu_analytics::core::models::Course::new(
            course.name.clone(),
            course.prefix.clone(),
            course.number.clone(),
            course.credit_hours,
        );
        school_course.canonical_name = Some(key.clone());

        // Copy prerequisites from raw string if available
        school_course
            .prerequisites_raw
            .clone_from(&course.prerequisites_raw);

        // Also populate the prerequisites vector from prerequisites_raw
        if let Some(raw) = &course.prerequisites_raw {
            school_course.prerequisites = parse_prerequisites_from_raw(raw);
        }

        // Copy corequisites (Vec<String>, not Option)
        school_course.corequisites.clone_from(&course.corequisites);

        // Copy typically offered and gen_ed attributes
        school_course
            .typically_offered
            .clone_from(&course.typically_offered);
        school_course
            .gen_ed_attributes
            .clone_from(&course.gen_ed_attributes);

        school.add_course(school_course);
    }

    school
}

/// Parse prerequisite course codes from a raw prerequisite string
///
/// Extracts course codes from expressions like:
/// - `"CS165[C]"` → `["CS165"]`
/// - `"(CS220[C] & CS165[C])"` → `["CS220", "CS165"]`
/// - `"CS162[C] | CS163[C] | CS164[C]"` → `["CS162", "CS163", "CS164"]`
fn parse_prerequisites_from_raw(raw: &str) -> Vec<String> {
    let mut prereqs = Vec::new();

    // Replace operators and brackets with spaces
    let cleaned = raw.replace(['(', ')', '&', '|', '[', ']'], " ");

    for part in cleaned.split_whitespace() {
        // Skip grade requirements like "B", "C", etc.
        if part.len() <= 2
            && part
                .chars()
                .all(|c| c.is_alphabetic() || c == '-' || c == '+')
        {
            continue;
        }

        // Must start with a letter (course code)
        if part.chars().next().is_some_and(char::is_alphabetic) {
            // Remove any trailing grade requirement
            let key = part
                .find(|c: char| !c.is_alphanumeric())
                .map_or(part, |idx| &part[..idx]);
            if !key.is_empty() && !prereqs.contains(&key.to_string()) {
                prereqs.push(key.to_string());
            }
        }
    }

    prereqs
}

/// Build a DAG from the course graph
fn build_dag_from_graph(graph: &CourseGraph) -> DAG {
    let mut dag = DAG::new();

    for key in graph.course_keys() {
        if let Some(node) = graph.get(key) {
            // Add node to DAG
            dag.add_course(key.to_string());

            // Add edges for prerequisites (using prerequisite_paths for flattened list)
            // Use the first path (simplest/shortest) from DNF form
            if !node.prerequisite_paths.is_empty() {
                for prereq in &node.prerequisite_paths[0] {
                    dag.add_prerequisite(key.to_string(), prereq);
                }
            }

            // Also add required prerequisites from edges
            for prereq in node.required_prerequisites() {
                dag.add_prerequisite(key.to_string(), prereq);
            }
        }
    }

    dag
}

/// Expand a plan's courses to include all required prerequisites
///
/// For each course in the plan, finds the minimum prerequisite chain and adds
/// any missing prerequisites to the course list. This ensures the plan is
/// complete and can be properly scheduled.
///
/// Uses a two-phase approach:
/// 1. First pass: Sort courses by prerequisite depth (deepest first) so courses
///    that need prerequisites are processed after their potential prereqs are known
/// 2. Second pass: Remove redundant prerequisites where an alternative already exists
///
/// This prevents adding MATH117 for STAT301 when MATH127 (needed by MATH156)
/// would also satisfy STAT301's prerequisite.
///
/// Uses the equivalence map to check if a prerequisite is satisfied by an
/// equivalent course already in the plan.
///
/// Uses the exclude set to avoid adding courses that are alternatives to included
/// courses (e.g., don't add MATH160 if user included MATH156 and they're alternatives).
fn expand_courses_with_prerequisites(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
    exclude_from_prereqs: &HashSet<String>,
    protected_courses: &HashSet<String>,
) -> Vec<String> {
    // Phase 1: Sort courses by prerequisite depth (deepest chains first)
    // This ensures courses like MATH156 (which needs MATH127) are processed
    // before courses like STAT301 (which can use MATH127 as an alternative)
    let mut sorted_courses: Vec<(String, usize)> = courses
        .iter()
        .map(|c| {
            let depth = graph.min_prerequisite_depth(c).unwrap_or(0);
            (c.clone(), depth)
        })
        .collect();
    sorted_courses.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));

    let mut expanded: HashSet<String> = courses.iter().cloned().collect();
    let mut to_process: Vec<String> = sorted_courses.into_iter().map(|(c, _)| c).collect();

    while let Some(course_key) = to_process.pop() {
        // Get the minimum prerequisite chain, preferring courses already in the plan
        // and avoiding excluded courses
        if let Some(prereq_chain) = graph.min_prerequisite_chain_with_exclusions(
            &course_key,
            &expanded,
            exclude_from_prereqs,
        ) {
            for prereq in prereq_chain {
                // Skip excluded courses (alternatives to included courses)
                if exclude_from_prereqs.contains(&prereq) {
                    continue;
                }

                // Check if this prerequisite is satisfied by an equivalent course
                let has_equivalent = equivalences
                    .get(&prereq)
                    .is_some_and(|equivs| equivs.iter().any(|e| expanded.contains(e)));

                if !has_equivalent && !expanded.contains(&prereq) {
                    expanded.insert(prereq.clone());
                    to_process.push(prereq); // Process this prereq's chain too
                }
            }
        }
    }

    // Phase 2: Remove redundant prerequisites
    // A prerequisite is redundant if:
    // - It was added as an OR-alternative for some course
    // - Another course in the plan would also satisfy that OR requirement
    // - OR an equivalent course is already in the plan
    // BUT never remove protected courses (explicitly included by user)
    let expanded_clone = expanded.clone();
    let redundant = find_redundant_prerequisites(&expanded_clone, graph, equivalences);
    for course in redundant {
        if !protected_courses.contains(&course) {
            expanded.remove(&course);
        }
    }

    let mut result: Vec<String> = expanded.into_iter().collect();
    result.sort();
    result
}

/// Find prerequisites that are redundant because an alternative already exists
///
/// For each course in the plan, checks if any of its OR-prerequisites could be
/// satisfied by a different course already in the plan. If so, and the current
/// prerequisite is ONLY used for this OR-group (not required elsewhere), it's redundant.
///
/// Also considers equivalent courses: if MATH241 is a prereq but MATH215 (equivalent)
/// is in the plan, MATH241 is redundant.
fn find_redundant_prerequisites(
    courses: &HashSet<String>,
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
) -> Vec<String> {
    let mut redundant = Vec::new();

    // Build a map of which courses ACTUALLY depend on which prerequisites
    // Only count a prerequisite as "used" if no other option in its OR-group is in the plan
    let mut prereq_usage: HashMap<String, Vec<String>> = HashMap::new();

    for course_key in courses {
        if let Some(node) = graph.get(course_key) {
            // Group prerequisites by OR-group
            let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
            let mut required_prereqs: Vec<&str> = Vec::new();

            for edge in &node.prerequisites {
                if edge.prereq_type
                    == nu_analytics::core::models::course_graph::PrerequisiteType::Required
                {
                    if courses.contains(&edge.prerequisite) {
                        required_prereqs.push(&edge.prerequisite);
                    }
                } else if let Some(group) = edge.or_group {
                    or_groups.entry(group).or_default().push(&edge.prerequisite);
                }
            }

            // Required prereqs are always used
            for prereq in required_prereqs {
                prereq_usage
                    .entry(prereq.to_string())
                    .or_default()
                    .push(course_key.clone());
            }

            // For OR-groups, only mark as "used" if this is the ONLY option in the plan
            for (_group, options) in or_groups {
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| courses.contains(opt))
                    .copied()
                    .collect();

                if in_plan.len() == 1 {
                    // Only one option satisfies this - it's truly needed
                    prereq_usage
                        .entry(in_plan[0].to_string())
                        .or_default()
                        .push(course_key.clone());
                }
                // If multiple options are in plan, we'll handle redundancy below
            }
        }
    }

    // Check for courses that are redundant because an equivalent is in the plan
    for course in courses {
        if let Some(equivs) = equivalences.get(course) {
            for equiv in equivs {
                if equiv != course && courses.contains(equiv) {
                    let usages = prereq_usage.get(course);
                    if usages.is_none_or(std::vec::Vec::is_empty) {
                        let equiv_satisfies_same = prereq_usage
                            .get(equiv)
                            .is_some_and(|equiv_usages| !equiv_usages.is_empty());

                        if equiv_satisfies_same {
                            redundant.push(course.clone());
                        }
                    }
                }
            }
        }
    }

    // For each course, check its OR-groups for redundant prerequisites
    for course_key in courses {
        if let Some(node) = graph.get(course_key) {
            // Group prerequisites by OR-group
            let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
            for edge in &node.prerequisites {
                if let Some(group) = edge.or_group {
                    if edge.prereq_type
                        == nu_analytics::core::models::course_graph::PrerequisiteType::Optional
                    {
                        or_groups.entry(group).or_default().push(&edge.prerequisite);
                    }
                }
            }

            // For each OR-group, check if we have multiple options in the plan
            for (_group, options) in or_groups {
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| courses.contains(opt))
                    .copied()
                    .collect();

                if in_plan.len() > 1 {
                    // Multiple options satisfy this OR-group - find redundant ones
                    // A course is redundant if another course in this OR-group
                    // is actually NEEDED by other courses (has real dependents)
                    for &option in &in_plan {
                        let option_usage = prereq_usage.get(option).map_or(0, Vec::len);

                        // Check if another option has MORE dependents (is more useful)
                        let better_exists = in_plan.iter().any(|&other| {
                            if other == option {
                                return false;
                            }
                            let other_usage = prereq_usage.get(other).map_or(0, Vec::len);
                            other_usage > option_usage
                        });

                        // If this option has no unique dependents and a better option exists
                        if option_usage == 0 && better_exists {
                            redundant.push(option.to_string());
                        }
                    }
                }
            }
        }
    }

    redundant
}

/// Create an expanded plan variant with additional prerequisite courses
///
/// Takes the original variant and creates a new one with the expanded course list,
/// preserving requirement choice metadata. Adjusts elective placeholders to ensure
/// the plan exactly reaches the target credits (not more).
///
/// # Arguments
/// * `original` - The original plan variant before prerequisite expansion
/// * `expanded_courses` - All courses including added prerequisites
/// * `school` - School data for credit lookup
/// * `target_credits` - Target total credits for the degree
fn create_expanded_variant(
    original: &PlanVariant,
    expanded_courses: &[String],
    school: &School,
    target_credits: Option<u32>,
) -> PlanVariant {
    let mut new_choices = original.requirement_choices.clone();

    // Find courses that were added (prerequisites not in original plan)
    let original_set: HashSet<&str> = original.courses.iter().map(String::as_str).collect();
    let added_prereqs: Vec<String> = expanded_courses
        .iter()
        .filter(|c| !original_set.contains(c.as_str()))
        .cloned()
        .collect();

    // Add prerequisites as a special requirement
    if !added_prereqs.is_empty() {
        new_choices.insert("_prerequisites".to_string(), added_prereqs);
    }

    // Calculate actual credits from non-elective courses
    let non_elective_credits: f32 = expanded_courses
        .iter()
        .filter(|c| !c.starts_with("ELEC"))
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |course| course.credit_hours)
        })
        .sum();

    // Adjust electives if we have a target
    #[allow(clippy::option_if_let_else)] // More readable with if-let here
    #[allow(clippy::cast_precision_loss)] // Safe: target credits < 1000
    let final_courses = if let Some(target) = target_credits {
        let target_f32 = target as f32;
        if non_elective_credits >= target_f32 {
            // Already at or over target - remove all electives
            new_choices.remove("_elective_placeholders");
            expanded_courses
                .iter()
                .filter(|c| !c.starts_with("ELEC"))
                .cloned()
                .collect()
        } else {
            // Need some electives - calculate exactly how many
            let elective_credits_needed = target_f32 - non_elective_credits;
            let new_electives = generate_elective_placeholders(elective_credits_needed);

            // Replace elective placeholders with exact amount needed
            if new_electives.is_empty() {
                new_choices.remove("_elective_placeholders");
            } else {
                new_choices.insert("_elective_placeholders".to_string(), new_electives.clone());
            }

            // Build final course list with new electives
            let mut courses: Vec<String> = expanded_courses
                .iter()
                .filter(|c| !c.starts_with("ELEC"))
                .cloned()
                .collect();
            courses.extend(new_electives);
            courses.sort();
            courses
        }
    } else {
        expanded_courses.to_vec()
    };

    // Calculate final total credits
    let total_credits: f32 = final_courses
        .iter()
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |course| course.credit_hours)
        })
        .sum();

    PlanVariant::from_parts(final_courses, new_choices, total_credits)
}

/// Generate placeholder elective courses for a given credit amount
///
/// Creates 3-credit electives with a possible 2-credit "small" elective
/// for the remainder.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn generate_elective_placeholders(credits_needed: f32) -> Vec<String> {
    if credits_needed <= 0.0 {
        return Vec::new();
    }

    let full_electives = (credits_needed / 3.0).floor() as usize;
    let remainder = credits_needed % 3.0;

    let mut electives = Vec::new();

    for i in 0..full_electives {
        electives.push(format!("ELEC{:03}", i + 1));
    }

    // Add partial elective if remainder is significant (> 0.5 credits)
    if remainder > 0.5 {
        electives.push(format!("ELEC{:03}S", full_electives + 1));
    }

    electives
}

/// Get credits for a placeholder course based on naming convention
fn placeholder_credits(course_key: &str) -> f32 {
    if course_key.ends_with('S') {
        2.0
    } else {
        3.0
    }
}

/// Build a DAG containing only the courses in a specific plan
///
/// Filters the full course graph to include only courses in the plan and
/// their prerequisite relationships within the plan. For OR-prerequisites,
/// only adds edges to prerequisites that are actually in the plan.
///
/// When multiple OR-group options are in the plan, prefers:
/// 1. Included courses (explicitly specified by user)
/// 2. Courses needed by more other courses in the plan
///
/// For required prerequisites, if the exact prereq isn't in the plan but an
/// equivalent course is (per the equivalences map), uses the equivalent.
fn build_dag_for_plan(
    courses: &[String],
    graph: &CourseGraph,
    include_courses: &HashSet<String>,
    equivalences: &HashMap<String, HashSet<String>>,
) -> DAG {
    let plan_courses: HashSet<&str> = courses.iter().map(String::as_str).collect();
    let mut dag = DAG::new();

    for course_key in courses {
        dag.add_course(course_key.clone());

        // Add prerequisites that are also in the plan
        if let Some(node) = graph.get(course_key) {
            // Group prerequisites by OR-group
            let mut or_groups: std::collections::HashMap<usize, Vec<&str>> =
                std::collections::HashMap::new();
            let mut required_prereqs: Vec<&str> = Vec::new();

            for edge in &node.prerequisites {
                // Skip corequisites
                if edge.prereq_type
                    == nu_analytics::core::models::course_graph::PrerequisiteType::Corequisite
                {
                    continue;
                }

                if edge.prereq_type
                    == nu_analytics::core::models::course_graph::PrerequisiteType::Required
                {
                    required_prereqs.push(&edge.prerequisite);
                } else if let Some(group) = edge.or_group {
                    // Optional (OR-group) prerequisite
                    or_groups.entry(group).or_default().push(&edge.prerequisite);
                }
            }

            // Add required prerequisites that are in the plan (directly or via equivalence)
            for prereq in required_prereqs {
                if plan_courses.contains(prereq) {
                    dag.add_prerequisite(course_key.clone(), prereq);
                } else if let Some(equiv) =
                    find_equivalent_in_plan_set(prereq, equivalences, &plan_courses)
                {
                    dag.add_prerequisite(course_key.clone(), equiv);
                }
            }

            // For each OR-group, add only ONE prerequisite to avoid spurious edges
            // Prefer included courses, then courses needed by more other courses
            for (_group, options) in or_groups {
                // Find options that are in the plan
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| plan_courses.contains(opt))
                    .copied()
                    .collect();

                if in_plan.is_empty() {
                    continue;
                }

                // If only one option, use it
                if in_plan.len() == 1 {
                    dag.add_prerequisite(course_key.clone(), in_plan[0]);
                    continue;
                }

                // Multiple options - first check if any is an included course
                let included_option = in_plan
                    .iter()
                    .find(|&&opt| include_courses.contains(opt))
                    .copied();

                if let Some(prereq) = included_option {
                    dag.add_prerequisite(course_key.clone(), prereq);
                    continue;
                }

                // No included course - prefer the one needed by other courses
                // Count how many OTHER courses in the plan need each option
                let best_prereq = in_plan
                    .iter()
                    .max_by_key(|&&opt| {
                        // Count courses that have this as a prerequisite
                        courses
                            .iter()
                            .filter(|&other| {
                                other.as_str() != course_key
                                    && graph.get(other).is_some_and(|n| {
                                        n.prerequisites.iter().any(|e| e.prerequisite == opt)
                                    })
                            })
                            .count()
                    })
                    .copied();

                if let Some(prereq) = best_prereq {
                    dag.add_prerequisite(course_key.clone(), prereq);
                }
            }
        }
    }

    dag
}

/// Find an equivalent course that is in the plan set.
///
/// Returns the first equivalent course found, or None if no equivalent is in the plan.
fn find_equivalent_in_plan_set<'a>(
    course: &str,
    equivalences: &HashMap<String, HashSet<String>>,
    plan_courses: &HashSet<&'a str>,
) -> Option<&'a str> {
    equivalences.get(course).and_then(|equivs| {
        equivs
            .iter()
            .find_map(|eq| plan_courses.get(eq.as_str()).copied())
    })
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
    fn test_resolve_convert_output_branches() {
        let input = Path::new("degrees/neu.json");
        assert_eq!(
            resolve_convert_output(input, None, false),
            PathBuf::from("degrees/neu.unified.json")
        );
        assert_eq!(
            resolve_convert_output(input, Some(Path::new("out")), true),
            PathBuf::from("out/neu.unified.json")
        );
        assert_eq!(
            resolve_convert_output(input, Some(Path::new("out/explicit.json")), false),
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
}
