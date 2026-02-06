//! Degree command handler for validating degree program YAML files

use nu_analytics::config::Config;
use nu_analytics::core::degree::load_degree_from_yaml;
use nu_analytics::core::models::degree::Requirement;
use nu_analytics::core::models::CourseGraph;
use nu_analytics::core::validate_degree_program;
use std::collections::HashSet;
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
#[derive(Debug, Default)]
#[allow(clippy::struct_excessive_bools)]
pub struct DegreeOptions {
    /// Whether to validate the degree program
    pub validate: bool,
    /// Whether to print the prerequisite graph
    pub print_graph: bool,
    /// Whether to run an audit report
    pub audit: bool,
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

    // Check if at least one action was specified
    if !options.validate && !options.print_graph && !options.audit {
        eprintln!("Error: No action specified. Use --validate, --print-graph, and/or --audit.");
        eprintln!("Run 'nuanalytics degree --help' for usage information.");
        process::exit(1);
    }

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
    }

    if has_error {
        process::exit(1);
    }
}

/// Print a separator between sections
fn print_separator() {
    println!();
    println!("================================================================================");
    println!();
}
