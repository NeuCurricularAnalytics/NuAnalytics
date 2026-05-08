//! Degree command handler for validating degree program YAML files

use std::collections::HashMap as StdHashMap;

use nu_analytics::config::Config;
use nu_analytics::core::degree::audit::{
    detect_lowest_course_level, find_deep_chains, find_upper_level_without_prereqs,
};
use nu_analytics::core::degree::{
    load_degree_from_yaml, PlanGenerator, PlanGeneratorConfig, PlanSelector, PlanSelectorConfig,
    PlanValidator, PlanValidatorConfig, PlanVariant, SamplingStrategy,
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
    let program = load_degree_from_yaml(degree_path).map_err(|e| {
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

    let program = load_degree_from_yaml(degree_path).map_err(|e| {
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
    let program = load_degree_from_yaml(degree_path).map_err(|e| {
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

/// Options for the degree command
#[derive(Debug, Default, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct DegreeOptions {
    /// Whether to validate the degree program
    pub validate: bool,
    /// Whether to print the prerequisite graph
    pub print_graph: bool,
    /// Whether to run an audit report
    pub audit: bool,
    /// Whether to run full degree analysis
    pub analyze: bool,
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
}

/// Run the degree command for one or more YAML files.
///
/// Each file is processed independently in order. Per-file failures are
/// reported but do not abort the batch; the process exits non-zero if any
/// file failed. Non-YAML paths (e.g., from a `samples/degrees/*` glob that
/// matches `.md` or other files) are skipped with a warning.
///
/// # Arguments
/// * `files`   - Paths to degree YAML files (length 0 prints usage and exits)
/// * `options` - Degree command options applied uniformly to every file
/// * `config`  - Application configuration
pub fn run(files: &[PathBuf], options: &DegreeOptions, config: &Config) {
    if files.is_empty() {
        eprintln!("Error: No degree file specified.");
        eprintln!("Usage: nuanalytics degree [OPTIONS] <FILES>...");
        eprintln!("Run 'nuanalytics degree --help' for usage information.");
        process::exit(1);
    }

    // Default to analyze if no action flag is set — applied once for the batch.
    let options = if !options.validate && !options.print_graph && !options.audit && !options.analyze
    {
        DegreeOptions {
            analyze: true,
            ..options.clone()
        }
    } else {
        options.clone()
    };

    let yaml_files: Vec<&Path> = files
        .iter()
        .filter_map(|p| {
            if is_yaml_path(p) {
                Some(p.as_path())
            } else {
                eprintln!("Skipping non-YAML file: {}", p.display());
                None
            }
        })
        .collect();

    if yaml_files.is_empty() {
        eprintln!("Error: No YAML files to process after filtering.");
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
        if run_one(path, &options, config).is_err() {
            had_failure = true;
        }
    }

    if had_failure {
        process::exit(1);
    }
}

/// Returns `true` if the path has a `.yaml` or `.yml` extension (case-insensitive).
fn is_yaml_path(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|ext| ext.eq_ignore_ascii_case("yaml") || ext.eq_ignore_ascii_case("yml"))
}

/// Process a single degree YAML file, running every requested action.
///
/// Returns `Err(())` if any action failed for this file. Errors are written
/// to stderr inline so the batch caller does not need to format them.
fn run_one(degree_path: &Path, options: &DegreeOptions, config: &Config) -> Result<(), ()> {
    let mut state = ActionRunState::default();

    if options.validate {
        state.run(|| validate_degree(degree_path, options.verbose));
    }
    if options.print_graph {
        state.run(|| print_graph(degree_path, options.verbose));
    }
    if options.audit {
        state.run(|| audit_degree(degree_path, config, options.verbose));
    }
    if options.analyze {
        state.run(|| analyze_degree(degree_path, options, config));
    }

    if state.has_error {
        Err(())
    } else {
        Ok(())
    }
}

/// Tracks action sequencing within `run_one`: prints a separator between
/// successive actions and records whether any action failed.
#[derive(Default)]
struct ActionRunState {
    actions_run: usize,
    has_error: bool,
}

impl ActionRunState {
    /// Run one action, prefixing it with a separator if a prior action ran,
    /// and capturing any error to stderr without aborting.
    fn run<E: std::fmt::Display>(&mut self, action: impl FnOnce() -> Result<(), E>) {
        if self.actions_run > 0 {
            print_separator();
        }
        if let Err(e) = action() {
            eprintln!("Error: {e}");
            self.has_error = true;
        }
        self.actions_run += 1;
    }
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
    options: &DegreeOptions,
    config: &Config,
) -> Result<(), String> {
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

    Ok(())
}

/// Load and validate a degree program from YAML
fn load_degree_program(
    degree_path: &Path,
    verbose: bool,
) -> Result<nu_analytics::core::DegreeProgram, String> {
    if verbose {
        eprintln!("Starting degree analysis...");
        eprintln!("Loading degree program from: {}", degree_path.display());
    }

    let program = load_degree_from_yaml(degree_path).map_err(|e| {
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
    options: &DegreeOptions,
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

/// Generate HTML report
fn generate_html_report(
    ctx: &AnalysisContext<'_>,
    options: &DegreeOptions,
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
    options: &DegreeOptions,
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
    sorted_courses.sort_by(|a, b| b.1.cmp(&a.1)); // Descending by depth

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
                    if usages.is_none() || usages.is_some_and(std::vec::Vec::is_empty) {
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
}
