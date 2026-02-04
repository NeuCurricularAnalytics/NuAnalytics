//! Degree command handler for validating degree program YAML files

use nu_analytics::core::degree::load_degree_from_yaml;
use nu_analytics::core::models::CourseGraph;
use nu_analytics::core::validate_degree_program;
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
        eprintln!("Building course graph...");
    }

    // Build the course graph
    let result = CourseGraph::from_degree_program(&program);

    // Print header
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

    // Report any cycles detected
    if !result.cycles.is_empty() {
        println!("⚠ Circular Prerequisites Detected:");
        for cycle in &result.cycles {
            println!("  {} → {}", cycle[0], cycle.join(" → "));
        }
        println!();
    }

    // Report missing courses (referenced but not defined)
    if !result.missing_courses.is_empty() {
        let mut missing = result.missing_courses.clone();
        missing.sort();
        println!("⚠ Missing Courses (referenced but not defined):");
        for course in &missing {
            println!("  {course}");
        }
        println!();
    }

    // Print graph statistics
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

    // Print the graph as an association list
    println!("Prerequisite Map (course → prerequisites):");
    println!("------------------------------------------");

    // Sort course keys for consistent output
    let mut keys: Vec<&str> = result.graph.course_keys();
    keys.sort_unstable();

    for key in keys {
        if let Some(node) = result.graph.get(key) {
            let mut parts = Vec::new();

            // Format prerequisite paths (DNF form)
            let prereq_str = node.format_prerequisite_paths();
            if !prereq_str.is_empty() {
                parts.push(prereq_str);
            }

            // Collect corequisites
            let coreqs: Vec<&str> = node.corequisites();
            if !coreqs.is_empty() {
                parts.push(format!("co: {}", coreqs.join(", ")));
            }

            // Format output
            if parts.is_empty() {
                println!("  {key} → (none)");
            } else {
                println!("  {key} → {}", parts.join(" + "));
            }
        }
    }

    if verbose {
        eprintln!("\n✓ Graph printed successfully");
    }

    Ok(())
}

/// Run the degree command
///
/// Dispatches to the appropriate handler based on the provided options.
///
/// # Arguments
/// * `file` - Optional path to degree YAML file
/// * `validate` - Whether to validate the degree program
/// * `print_graph` - Whether to print the prerequisite graph
/// * `verbose` - Whether to print verbose output
pub fn run(file: Option<&Path>, validate: bool, print_graph_flag: bool, verbose: bool) {
    // Check if a file was provided
    let Some(degree_path) = file else {
        eprintln!("Error: No degree file specified.");
        eprintln!("Usage: nuanalytics degree [OPTIONS] <FILE>");
        eprintln!("Run 'nuanalytics degree --help' for usage information.");
        process::exit(1);
    };

    // Check if at least one action was specified
    if !validate && !print_graph_flag {
        eprintln!("Error: No action specified. Use --validate and/or --print-graph.");
        eprintln!("Run 'nuanalytics degree --help' for usage information.");
        process::exit(1);
    }

    let mut has_error = false;

    // Run validation if requested
    if validate {
        match validate_degree(degree_path, verbose) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("Error: {e}");
                has_error = true;
            }
        }

        // Add separator if both actions are requested
        if print_graph_flag {
            println!();
            println!(
                "================================================================================"
            );
            println!();
        }
    }

    // Print graph if requested
    if print_graph_flag {
        match print_graph(degree_path, verbose) {
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
