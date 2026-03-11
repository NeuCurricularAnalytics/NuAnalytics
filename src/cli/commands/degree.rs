//! Degree command handler for validating degree program YAML files

use nu_analytics::config::Config;
use nu_analytics::core::degree::{
    load_degree_from_yaml, PlanGenerator, PlanGeneratorConfig, PlanSelector, PlanSelectorConfig,
    PlanValidator, PlanValidatorConfig, PlanVariant, SamplingStrategy,
};
use nu_analytics::core::metrics::compute_all_metrics;
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
use std::path::Path;
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
        for (course, chain_lengths, chain_str) in &deep_chains {
            println!("  • {course} (chains: {chain_lengths})");
            if verbose {
                println!("    Chain: {chain_str}");
            }
        }
    }
    println!();
    // Convert to expected format (use max chain length for sorting/summary)
    deep_chains
        .into_iter()
        .map(|(c, lens, s)| {
            let max_len = lens
                .split(", ")
                .filter_map(|n| n.parse::<usize>().ok())
                .max()
                .unwrap_or(0);
            (c, max_len, s)
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

/// Detect the lowest course level in the degree program
///
/// Parses course keys to find numeric level indicators (e.g., CS1000 → 1000, CS100 → 100)
fn detect_lowest_course_level(program: &nu_analytics::core::DegreeProgram) -> u32 {
    let mut lowest = u32::MAX;

    for key in program.courses.keys() {
        if let Some(level) = extract_course_level(key) {
            if level < lowest {
                lowest = level;
            }
        }
    }

    // Default to 100 if no levels detected
    if lowest == u32::MAX {
        100
    } else {
        lowest
    }
}

/// Extract numeric course level from a course key
///
/// Examples: CS1000 → 1000, MATH156 → 100, CS2510 → 2000
fn extract_course_level(key: &str) -> Option<u32> {
    // Find the first digit sequence in the key
    let digits: String = key.chars().filter(char::is_ascii_digit).collect();

    if digits.is_empty() {
        return None;
    }

    // Parse and round to nearest level (100 or 1000)
    let num: u32 = digits.parse().ok()?;

    // Determine if it's a 4-digit system (1000s) or 3-digit system (100s)
    if num >= 1000 {
        Some((num / 1000) * 1000)
    } else {
        Some((num / 100) * 100)
    }
}

/// Find upper-level courses that have no prerequisites defined
fn find_upper_level_without_prereqs(
    graph_result: &nu_analytics::core::models::CourseGraphResult,
    lowest_level: u32,
) -> Vec<(String, u32)> {
    let mut missing = Vec::new();

    for key in graph_result.graph.course_keys() {
        if let Some(level) = extract_course_level(key) {
            // Skip lowest level courses (they typically don't need prereqs)
            if level <= lowest_level {
                continue;
            }

            // Check if this course has prerequisites
            if let Some(node) = graph_result.graph.get(key) {
                if node.prerequisites.is_empty() {
                    missing.push((key.to_string(), level));
                }
            }
        }
    }

    // Sort by level, then by course key
    missing.sort_by(|a, b| a.1.cmp(&b.1).then_with(|| a.0.cmp(&b.0)));
    missing
}

/// Find courses with deep prerequisite chains (above threshold)
///
/// Only includes courses that match the degree's major subjects (if defined),
/// or courses that appear in requirements. Uses the structured prerequisite
/// chain to properly represent parallel branches and overlapping requirements.
fn find_deep_chains(
    program: &nu_analytics::core::DegreeProgram,
    graph_result: &nu_analytics::core::models::CourseGraphResult,
    threshold: usize,
) -> Vec<(String, String, String)> {
    let mut deep = Vec::new();

    // Get major subjects if defined
    let major_subjects = program.degree.major_subjects.as_ref();

    // Get courses from requirements as fallback filter
    let requirement_courses = collect_requirement_courses(program);

    for key in graph_result.graph.course_keys() {
        // Filter: only include courses matching major subjects or in requirements
        if !is_course_in_scope(key, major_subjects, &requirement_courses) {
            continue;
        }

        // Get the structured prerequisite chain (considering OR alternatives and overlap)
        if let Some(chain) = graph_result.graph.structured_prerequisite_chain(key) {
            // Check if any branch meets the threshold
            let max_branch_len = chain.branch_lengths().into_iter().max().unwrap_or(0);
            if max_branch_len >= threshold {
                deep.push((key.to_string(), chain.format_lengths(), chain.format()));
            }
        }
    }

    // Sort by max chain length (descending), then by course key
    deep.sort_by(|a, b| {
        let a_max = parse_max_chain_length(&a.1);
        let b_max = parse_max_chain_length(&b.1);
        b_max.cmp(&a_max).then_with(|| a.0.cmp(&b.0))
    });
    deep
}

/// Parse the maximum chain length from a formatted length string (e.g., "5, 3" → 5)
fn parse_max_chain_length(lengths_str: &str) -> usize {
    lengths_str
        .split(", ")
        .filter_map(|n| n.parse::<usize>().ok())
        .max()
        .unwrap_or(0)
}

/// Collect all course keys referenced in requirements
fn collect_requirement_courses(program: &nu_analytics::core::DegreeProgram) -> HashSet<String> {
    let mut courses = HashSet::new();
    for req in program.requirements.values() {
        collect_courses_from_requirement(req, &mut courses);
    }
    courses
}

/// Recursively collect course keys from a requirement
fn collect_courses_from_requirement(req: &Requirement, courses: &mut HashSet<String>) {
    // Add direct courses
    if let Some(req_courses) = &req.courses {
        for course in req_courses {
            courses.insert(course.clone());
        }
    }

    // Add courses from "from" clause
    if let Some(from) = &req.from {
        if let Some(from_courses) = &from.courses {
            for course in from_courses {
                courses.insert(course.clone());
            }
        }
        // Add courses from groups
        if let Some(groups) = &from.groups {
            for group in groups {
                for course in &group.courses {
                    courses.insert(course.clone());
                }
            }
        }
    }

    // Add courses from options (one_of requirements)
    if let Some(options) = &req.options {
        for option in options {
            for nested_req in &option.requirements {
                collect_courses_from_requirement(nested_req, courses);
            }
        }
    }
}

/// Check if a course is in scope (matches major subjects or is in requirements)
fn is_course_in_scope(
    course_key: &str,
    major_subjects: Option<&Vec<String>>,
    requirement_courses: &HashSet<String>,
) -> bool {
    // If major subjects are defined, check if course matches
    if let Some(subjects) = major_subjects {
        if let Some(subject) = extract_subject(course_key) {
            if subjects.iter().any(|s| s.eq_ignore_ascii_case(subject)) {
                return true;
            }
        }
    }

    // Fallback: check if course is in requirements
    if major_subjects.is_none() {
        return requirement_courses.contains(course_key);
    }

    false
}

/// Extract subject code from a course key (e.g., "CS312" → "CS")
fn extract_subject(course_key: &str) -> Option<&str> {
    // Find where the digits start
    let digit_pos = course_key.find(|c: char| c.is_ascii_digit())?;
    if digit_pos > 0 {
        Some(&course_key[..digit_pos])
    } else {
        None
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
}

/// Run the degree command
///
/// Dispatches to the appropriate handler based on the provided options.
///
/// # Arguments
/// * `file` - Optional path to degree YAML file
/// * `options` - Degree command options
/// * `config` - Application configuration
pub fn run(file: Option<&Path>, options: &DegreeOptions, config: &Config) {
    // Check if a file was provided
    let Some(degree_path) = file else {
        eprintln!("Error: No degree file specified.");
        eprintln!("Usage: nuanalytics degree [OPTIONS] <FILE>");
        eprintln!("Run 'nuanalytics degree --help' for usage information.");
        process::exit(1);
    };

    // Default to analyze if no action specified
    let options = if !options.validate && !options.print_graph && !options.audit && !options.analyze
    {
        DegreeOptions {
            analyze: true,
            ..options.clone()
        }
    } else {
        options.clone()
    };

    let mut has_error = false;
    let mut actions_run = 0;

    // Run validation if requested
    if options.validate {
        if actions_run > 0 {
            print_separator();
        }
        match validate_degree(degree_path, options.verbose) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                has_error = true;
            }
        }
        actions_run += 1;
    }

    // Print graph if requested
    if options.print_graph {
        if actions_run > 0 {
            print_separator();
        }
        match print_graph(degree_path, options.verbose) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                has_error = true;
            }
        }
        actions_run += 1;
    }

    // Run audit if requested
    if options.audit {
        if actions_run > 0 {
            print_separator();
        }
        match audit_degree(degree_path, config, options.verbose) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                has_error = true;
            }
        }
        actions_run += 1;
    }

    // Run full analysis if requested
    if options.analyze {
        if actions_run > 0 {
            print_separator();
        }
        match analyze_degree(degree_path, &options, config) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                has_error = true;
            }
        }
    }

    if has_error {
        process::exit(1);
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
            ..Default::default()
        },
        verbose,
        equivalences,
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
        let expanded_courses =
            expand_courses_with_prerequisites(&variant.courses, ctx.graph, &ctx.equivalences);

        // Build plan-specific DAG and compute metrics
        let plan_dag = build_dag_for_plan(&expanded_courses, ctx.graph);
        let course_metrics = match compute_all_metrics(&plan_dag) {
            Ok(metrics) => metrics,
            Err(e) => {
                if ctx.verbose {
                    eprintln!("  Warning: Failed to compute metrics for plan: {e}");
                }
                continue;
            }
        };

        // Aggregate metrics using expanded course credits
        // Calculate total credits from expanded courses
        let total_credits = expanded_courses
            .iter()
            .filter_map(|key| ctx.school.get_course(key))
            .map(|c| f64::from(c.credit_hours))
            .sum::<f64>();
        aggregator.add_plan(&course_metrics, total_credits);

        // Update plan selection (pass expanded variant and plan-specific DAG)
        let expanded_variant = create_expanded_variant(
            &variant,
            &expanded_courses,
            &ctx.school,
            ctx.gen_config.target_credits,
        );
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
fn expand_courses_with_prerequisites(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
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
        if let Some(prereq_chain) =
            graph.min_prerequisite_chain_with_context(&course_key, &expanded)
        {
            for prereq in prereq_chain {
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
    let expanded_clone = expanded.clone();
    let redundant = find_redundant_prerequisites(&expanded_clone, graph, equivalences);
    for course in redundant {
        expanded.remove(&course);
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
    let mut prereq_usage: HashMap<String, Vec<String>> = HashMap::new();

    // Build a map of which courses use which prerequisites
    for course_key in courses {
        if let Some(node) = graph.get(course_key) {
            for edge in &node.prerequisites {
                if courses.contains(&edge.prerequisite) {
                    prereq_usage
                        .entry(edge.prerequisite.clone())
                        .or_default()
                        .push(course_key.clone());
                }
            }
        }
    }

    // Check for courses that are redundant because an equivalent is in the plan
    for course in courses {
        if let Some(equivs) = equivalences.get(course) {
            // If another equivalent course is also in the plan, one might be redundant
            for equiv in equivs {
                if equiv != course && courses.contains(equiv) {
                    // Check if this course is only used as a prerequisite
                    // If so, and the equivalent satisfies those same prereqs, it's redundant
                    let usages = prereq_usage.get(course);
                    if usages.is_none() || usages.is_some_and(std::vec::Vec::is_empty) {
                        // This course has no dependents - check if it was added as a prereq
                        // for something that the equivalent would satisfy
                        let equiv_satisfies_same =
                            prereq_usage.get(equiv).is_some_and(|equiv_usages| {
                                // Check if equivalent is used by any course
                                !equiv_usages.is_empty()
                            });

                        if equiv_satisfies_same {
                            // The equivalent course is being used - this one might be redundant
                            // Only mark redundant if this course was added as a prereq, not from requirements
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
                    // We have multiple options - find which ones are only used here
                    for &option in &in_plan {
                        if let Some(usages) = prereq_usage.get(option) {
                            // If this prereq is only used by this one course's OR-group,
                            // and another option exists, it's redundant
                            if usages.len() == 1 && usages[0] == *course_key {
                                // Check if this course has other prereqs depending on it
                                let has_dependents = courses.iter().any(|other| {
                                    if other == option {
                                        return false;
                                    }
                                    graph.get(other).is_some_and(|other_node| {
                                        other_node.prerequisites.iter().any(|e| {
                                            e.prerequisite == option
                                                && e.prereq_type
                                                    == nu_analytics::core::models::course_graph::PrerequisiteType::Required
                                        })
                                    })
                                });

                                if !has_dependents {
                                    // This option is redundant - but only mark it if
                                    // it's not the "best" option (prefer keeping the
                                    // one that's used by other courses)
                                    let better_exists = in_plan.iter().any(|&other| {
                                        other != option
                                            && prereq_usage.get(other).is_some_and(|u| u.len() > 1)
                                    });
                                    if better_exists {
                                        redundant.push(option.to_string());
                                    }
                                }
                            }
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
fn build_dag_for_plan(courses: &[String], graph: &CourseGraph) -> DAG {
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

            // Add required prerequisites that are in the plan
            for prereq in required_prereqs {
                if plan_courses.contains(prereq) {
                    dag.add_prerequisite(course_key.clone(), prereq);
                }
            }

            // For each OR-group, add the prerequisite that's in the plan (if any)
            for (_group, options) in or_groups {
                // Find options that are in the plan
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| plan_courses.contains(opt))
                    .copied()
                    .collect();

                // Add edges for all options that are in the plan
                // (Usually just one, but if multiple, all should be connected)
                for prereq in in_plan {
                    dag.add_prerequisite(course_key.clone(), prereq);
                }
            }
        }
    }

    dag
}

/// Print a separator between sections
fn print_separator() {
    println!();
    println!("================================================================================");
    println!();
}
