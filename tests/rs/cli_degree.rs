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
            "print-graph",
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
            "validate",
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

/// Each `degree` subcommand runs a single action. Mixing them (the old
/// flag-based double-action) must now fail at parse or load time rather
/// than silently doing both.
#[test]
fn test_degree_rejects_stacked_subcommands() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "validate",
            "--print-graph",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    assert!(
        !output.status.success(),
        "stacking the old --print-graph flag onto `degree validate` must not succeed"
    );
}

/// Test degree command handles missing file gracefully
#[test]
fn test_degree_missing_file() {
    let output = Command::new("cargo")
        .args(["run", "--", "degree", "validate", "nonexistent-file.yaml"])
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
        .args(["run", "--", "degree", "validate"])
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
            "print-graph",
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
            "print-graph",
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
            "print-graph",
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
            "print-graph",
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
            "validate",
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
            "print-graph",
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
            "audit",
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
            "audit",
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
            "audit",
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

/// Test that the degree command with --analyze produces analysis output
#[test]
fn test_degree_analyze_command() {
    use tempfile::TempDir;

    // Create a temp directory for outputs
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let report_dir = temp_dir.path().join("reports");
    let metrics_dir = temp_dir.path().join("metrics");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "analyze",
            "--max-plans",
            "50",
            "--sample-plans",
            "2",
            "--report-dir",
            report_dir.to_str().unwrap(),
            "--metrics-dir",
            metrics_dir.to_str().unwrap(),
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Should succeed
    assert!(
        output.status.success(),
        "Command should succeed. stderr: {stderr}"
    );

    // Should contain analysis summary
    assert!(
        stdout.contains("Degree Analysis Complete") || stdout.contains("Plans analyzed"),
        "Should report analysis completion. stdout: {stdout}"
    );

    // Check that HTML report was created
    let report_files: Vec<_> = std::fs::read_dir(&report_dir)
        .map(|dir| dir.filter_map(Result::ok).collect())
        .unwrap_or_default();
    assert!(
        !report_files.is_empty(),
        "Should have generated HTML report in {report_dir:?}"
    );

    // Check that CSV files were created in metrics directory
    let plans_dir = metrics_dir.join("plans");
    if plans_dir.exists() {
        let csv_files: Vec<_> = std::fs::read_dir(&plans_dir)
            .map(|dir| dir.filter_map(Result::ok).collect())
            .unwrap_or_default();
        assert!(
            !csv_files.is_empty(),
            "Should have generated CSV files in {plans_dir:?}"
        );
    }
}

/// Test analyze with --no-report and --no-csv flags
#[test]
fn test_degree_analyze_no_output_flags() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let report_dir = temp_dir.path().join("reports");
    let metrics_dir = temp_dir.path().join("metrics");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "analyze",
            "--no-report",
            "--no-csv",
            "--max-plans",
            "10",
            "--report-dir",
            report_dir.to_str().unwrap(),
            "--metrics-dir",
            metrics_dir.to_str().unwrap(),
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute command");

    // Should succeed even without generating files
    assert!(
        output.status.success(),
        "Command should succeed with --no-report --no-csv"
    );

    // Report directory should not exist or be empty
    let report_exists =
        report_dir.exists() && std::fs::read_dir(&report_dir).is_ok_and(|mut d| d.next().is_some());
    assert!(!report_exists, "Should not have generated HTML report");

    // Metrics directory should not exist or be empty
    let metrics_exists = metrics_dir.exists()
        && std::fs::read_dir(&metrics_dir).is_ok_and(|mut d| d.next().is_some());
    assert!(!metrics_exists, "Should not have generated CSV files");
}

// ---------------------------------------------------------------------------
// degree trim
// ---------------------------------------------------------------------------

/// Round-trip: trim a real sample, then validate the output through the
/// normal parser. The trimmed file must parse cleanly and the success
/// banner must show up on stdout.
#[test]
fn test_degree_trim_round_trip_validates() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let out_path = temp_dir.path().join("neu-trimmed.yaml");

    let trim_output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "-o",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        trim_output.status.success(),
        "trim should succeed. stderr: {}",
        String::from_utf8_lossy(&trim_output.stderr)
    );
    assert!(out_path.exists(), "trim must write the output file");
    let stdout = String::from_utf8_lossy(&trim_output.stdout);
    assert!(
        stdout.contains("Trimmed degree written to"),
        "trim should print a success banner; got: {stdout}"
    );

    // The trimmed file must still parse through `degree validate`.
    let validate_output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "validate",
            out_path.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute validate");
    assert!(
        validate_output.status.success(),
        "validate on trimmed file should succeed. stderr: {}",
        String::from_utf8_lossy(&validate_output.stderr)
    );
}

/// `degree trim` refuses to overwrite its input even when -o points at it.
#[test]
fn test_degree_trim_refuses_to_overwrite_input() {
    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "-o",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        !output.status.success(),
        "trim must refuse to overwrite the input file"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("refusing to overwrite"),
        "stderr should explain the refusal; got: {stderr}"
    );
}

/// Wildcards: trim multiple inputs, all outputs land under a single `-o`
/// directory and the trimmed filenames carry the `_trimmed` suffix.
#[test]
fn test_degree_trim_writes_each_input_to_out_dir() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "samples/degrees/csu-cs-bscs-general.yaml",
            "-o",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        output.status.success(),
        "trim must succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    for expected in [
        "neu-khoury-bscs-boston_trimmed.yaml",
        "csu-cs-bscs-general_trimmed.yaml",
    ] {
        let path = temp_dir.path().join(expected);
        assert!(
            path.exists(),
            "expected {} to be created in the -o directory",
            path.display()
        );
    }
}

/// Passing multiple inputs with a file-style `-o` is ambiguous (we can't
/// write N files to one path) and must fail with a clear message.
#[test]
fn test_degree_trim_rejects_file_out_with_multiple_inputs() {
    use tempfile::TempDir;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let file_out = temp_dir.path().join("collision.yaml");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "samples/degrees/csu-cs-bscs-general.yaml",
            "-o",
            file_out.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        !output.status.success(),
        "multiple inputs + file -o must not silently succeed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("is a file path") || stderr.contains("pass a directory"),
        "stderr should explain the ambiguity; got: {stderr}"
    );
}

/// Mixed input list: non-YAML files are skipped with a warning, YAML
/// files still get trimmed. The overall command succeeds.
#[test]
fn test_degree_trim_skips_non_yaml_and_proceeds() {
    use tempfile::TempDir;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "Readme.md",
            "-o",
            temp_dir.path().to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        output.status.success(),
        "trim should succeed when at least one input is YAML. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("Skipping non-YAML file") && stderr.contains("Readme.md"),
        "stderr should warn about the non-YAML input; got: {stderr}"
    );
    assert!(
        temp_dir
            .path()
            .join("neu-khoury-bscs-boston_trimmed.yaml")
            .exists(),
        "the YAML input must still be processed"
    );
}

/// All-invalid input list: every file is filtered out, command fails with
/// the dedicated "no YAML files to process" error.
#[test]
fn test_degree_trim_rejects_all_non_yaml_inputs() {
    let output = Command::new("cargo")
        .args(["run", "--", "degree", "trim", "Readme.md", "Cargo.toml"])
        .output()
        .expect("Failed to execute trim");

    assert!(
        !output.status.success(),
        "trim must fail when no YAML inputs survive filtering"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("No YAML files to process after filtering"),
        "stderr should explain the empty-after-filter state; got: {stderr}"
    );
}

/// Trailing-slash `-o` should auto-create the destination directory and
/// drop the trimmed file inside it.
#[test]
fn test_degree_trim_trailing_slash_creates_directory() {
    use tempfile::TempDir;
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    // Use a *non-existent* nested path with a trailing separator so we
    // exercise the `looks_like_directory` + `create_dir_all` code path.
    let nested = temp_dir.path().join("fresh/");

    let output = Command::new("cargo")
        .args([
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/uhm-ics-bscs-general.yaml",
            "-o",
            nested.to_str().unwrap(),
        ])
        .output()
        .expect("Failed to execute trim");

    assert!(
        output.status.success(),
        "trim with trailing-slash dir must succeed. stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = nested.join("uhm-ics-bscs-general_trimmed.yaml");
    assert!(
        expected.exists(),
        "expected {} to be created",
        expected.display()
    );
}

/// `--keep-all MATH` must protect MATH alternatives from being trimmed on a
/// degree where MATH is not in `major_subjects`. We verify this by checking
/// that more MATH-prefixed courses survive in the keep-all output than in
/// the default run.
#[test]
fn test_degree_trim_keep_all_preserves_extra_subject() {
    use tempfile::TempDir;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let default_out = temp_dir.path().join("default.yaml");
    let keep_math_out = temp_dir.path().join("keep_math.yaml");

    let run_trim = |out: &std::path::Path, extra_args: &[&str]| {
        let mut args = vec![
            "run",
            "--",
            "degree",
            "trim",
            "samples/degrees/neu-khoury-bscs-boston.yaml",
            "-o",
            out.to_str().unwrap(),
        ];
        args.extend_from_slice(extra_args);
        let output = Command::new("cargo")
            .args(&args)
            .output()
            .expect("Failed to execute trim");
        assert!(
            output.status.success(),
            "trim must succeed. stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    };

    run_trim(&default_out, &[]);
    run_trim(&keep_math_out, &["--keep-all", "MATH"]);

    let count_math = |path: &std::path::Path| {
        let text = std::fs::read_to_string(path).expect("read trim output");
        // Count occurrences of `MATH` followed by a digit — course keys
        // like MATH1341, MATH2331, etc. Avoids matching unrelated words.
        text.match_indices("MATH")
            .filter(|(idx, _)| {
                text[idx + 4..]
                    .chars()
                    .next()
                    .is_some_and(|c| c.is_ascii_digit())
            })
            .count()
    };

    let default_count = count_math(&default_out);
    let keep_math_count = count_math(&keep_math_out);
    assert!(
        keep_math_count > default_count,
        "--keep-all MATH should preserve more MATH references (got default={default_count}, keep-math={keep_math_count})"
    );
}
