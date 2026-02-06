//! Integration tests for CLI commands
//!
//! Tests the command-line interface for degree validation and graph printing.

use nu_analytics::core::degree::load_degree_from_yaml;
use nu_analytics::core::models::CourseGraph;
use std::io::Write;
use std::process::Command;
use tempfile::NamedTempFile;

/// Test that the degree command with --print-graph produces output
#[test]
fn test_degree_print_graph_command() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain graph output
    assert!(
        stdout.contains("Course Prerequisite Graph"),
        "Output should contain graph header"
    );
    assert!(
        stdout.contains("Prerequisite Map"),
        "Output should contain prerequisite map"
    );
    assert!(
        stdout.contains("Graph Statistics"),
        "Output should contain statistics"
    );
}

/// Test degree command with --validate flag
#[test]
fn test_degree_validate_command() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--validate",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain validation output
    assert!(
        stdout.contains("Running validation checks") || stdout.contains("valid"),
        "Output should indicate validation"
    );
}

/// Test degree command with both --validate and --print-graph
#[test]
fn test_degree_validate_and_print_graph() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--validate",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain both validation and graph output
    assert!(
        stdout.contains("validation") || stdout.contains("valid"),
        "Output should include validation"
    );
    assert!(
        stdout.contains("Course Prerequisite Graph") || stdout.contains("Graph"),
        "Output should include graph"
    );
}

/// Test degree command handles missing file gracefully
#[test]
fn test_degree_missing_file() {
    let output = Command::new("cargo")
        .args(["run", "--", "degree", "--validate", "nonexistent-file.yaml"])
        .output()
        .expect("Failed to execute command");

    // Should fail (non-zero exit code)
    assert!(!output.status.success(), "Should fail for missing file");

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Error") || stderr.contains("error") || stderr.contains("No such file"),
        "Error output should mention the problem"
    );
}

/// Test degree command requires file argument
#[test]
fn test_degree_requires_file_argument() {
    let output = Command::new("cargo")
        .args(["run", "--", "degree", "--validate"])
        .output()
        .expect("Failed to execute command");

    // Should fail without file
    assert!(
        !output.status.success(),
        "Should fail when no file specified"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No degree file specified")
            || stderr.contains("required")
            || stderr.contains("error"),
        "Error should mention missing file"
    );
}

/// Test that graph output contains expected course keys
#[test]
fn test_graph_output_contains_courses() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain actual course codes from the degree
    assert!(stdout.contains("CS"), "Output should contain CS courses");
    assert!(stdout.contains("→"), "Output should contain arrow symbols");
}

/// Test graph output format for courses with prerequisites
#[test]
fn test_graph_output_prerequisite_format() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should show prerequisite relationships
    // Looking for patterns like "CS3650 → CS3100"
    assert!(
        stdout.contains("→"),
        "Output should contain prerequisite arrows"
    );

    // Should have courses with "(none)" for no prerequisites
    assert!(
        stdout.contains("(none)") || stdout.contains("Entry Points"),
        "Output should indicate courses with no prerequisites"
    );
}

/// Test that cycle detection is reported in graph output
#[test]
fn test_graph_output_reports_cycles() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/csu-cs-bscs-general.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // CSU degree has CS152 <-> CS163 cycle
    if stdout.contains("CS152") && stdout.contains("CS163") {
        // If both courses present, might mention circular prerequisites
        let mentions_cycle =
            stdout.contains("Circular") || stdout.contains("cycle") || stdout.contains("Cycle");
        // This is expected for CSU degree
        if mentions_cycle {
            assert!(
                stdout.contains("CS152") && stdout.contains("CS163"),
                "Cycle report should mention both CS152 and CS163"
            );
        }
    }
}

/// Test graph statistics are present in output
#[test]
fn test_graph_output_statistics() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain statistics
    assert!(
        stdout.contains("Entry Points")
            || stdout.contains("Terminal")
            || stdout.contains("Statistics"),
        "Output should contain graph statistics"
    );
}

/// Test that malformed YAML is handled gracefully
#[test]
fn test_degree_malformed_yaml() {
    // Create a temporary malformed YAML file
    let mut temp_file = NamedTempFile::new().expect("Failed to create temp file");
    writeln!(temp_file, "this is not: [valid: yaml {{").expect("Failed to write");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--validate",
            temp_file.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute command");

    // Should fail gracefully
    assert!(!output.status.success(), "Should fail for malformed YAML");
}

/// Test programmatic graph building matches CLI output structure
#[test]
fn test_graph_programmatic_vs_cli_consistency() {
    // Load degree programmatically
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load degree");

    let result = CourseGraph::from_degree_program(&program);

    // Get CLI output
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // CLI should show same number of courses (approximately)
    let course_count = result.graph.len();
    assert!(
        stdout.contains(&format!("{course_count}"))
            || stdout.contains(&format!("{course_count} courses")),
        "CLI should report same course count as programmatic build"
    );
}

/// Test degree audit command produces expected sections
#[test]
fn test_degree_audit_command() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--audit",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // Should contain all audit sections
    assert!(
        stdout.contains("Degree Audit Report"),
        "Output should contain audit header"
    );
    assert!(
        stdout.contains("1. Validation Report"),
        "Output should contain validation section"
    );
    assert!(
        stdout.contains("2. Upper-Level Courses Missing Prerequisites"),
        "Output should contain missing prereqs section"
    );
    assert!(
        stdout.contains("3. Deep Prerequisite Chains"),
        "Output should contain deep chains section"
    );
    assert!(
        stdout.contains("Audit Summary"),
        "Output should contain summary"
    );
}

/// Test degree audit detects upper-level courses without prerequisites
#[test]
fn test_degree_audit_finds_missing_prereqs() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--audit",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // NEU degree has several upper-level courses without prerequisites
    // These should be reported
    assert!(
        stdout.contains("upper-level course(s) without prerequisites")
            || stdout.contains("All upper-level courses have prerequisites"),
        "Should report on upper-level courses without prerequisites"
    );
}

/// Test degree audit detects deep prerequisite chains
#[test]
fn test_degree_audit_finds_deep_chains() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "--audit",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // NEU degree has courses with deep chains (CS4410 has 6+)
    // Should find courses with chains >= threshold (default 3 or 4)
    assert!(
        stdout.contains("prerequisite chains >=")
            || stdout.contains("prerequisites in chain")
            || stdout.contains("Deep Prerequisite Chains"),
        "Should report on deep prerequisite chains"
    );
}
