//! Integration tests for metrics comparison against reference files
//!
//! These tests compare computed metrics against known-correct reference files
//! in `samples/correct/` to ensure our metric calculations match expected values.

use nu_analytics::core::metrics::{self, CurriculumMetrics};
use nu_analytics::core::planner::csv_parser::parse_curriculum_csv;
use std::collections::HashMap;
use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Represents expected metrics for a single course from the reference CSV
#[derive(Debug, Clone)]
struct ExpectedCourseMetrics {
    csv_id: String,
    course_name: String,
    complexity: f64,
    blocking: usize,
    delay: f64,
    centrality: usize,
}

/// Represents expected summary from the reference CSV
#[derive(Debug, Clone)]
struct ExpectedSummary {
    total_structural_complexity: f64,
    courses: Vec<ExpectedCourseMetrics>,
}

/// Parse the reference CSV to extract expected metrics
fn parse_expected_metrics(reference_path: &Path) -> ExpectedSummary {
    let contents = fs::read_to_string(reference_path).unwrap_or_else(|_| {
        panic!(
            "Failed to read reference file: {}",
            reference_path.display()
        )
    });

    let mut courses = Vec::new();
    let mut in_courses_section = false;
    let mut header_seen = false;

    for line in contents.lines() {
        // Check for Courses section start
        if line.starts_with("Courses") {
            in_courses_section = true;
            continue;
        }

        // Skip header row in courses section
        if in_courses_section && !header_seen && line.starts_with("Course ID,") {
            header_seen = true;
            continue;
        }

        // Parse course data rows
        if in_courses_section && header_seen && !line.is_empty() {
            if let Some(metrics) = parse_course_line(line) {
                courses.push(metrics);
            }
        }
    }

    // Compute total structural complexity from course complexities
    let total_structural_complexity: f64 = courses.iter().map(|c| c.complexity).sum();

    ExpectedSummary {
        total_structural_complexity,
        courses,
    }
}

/// Parse a single course line from the reference CSV
fn parse_course_line(line: &str) -> Option<ExpectedCourseMetrics> {
    // Parse CSV line handling quoted fields
    let fields = parse_csv_line(line);
    if fields.len() < 14 {
        return None;
    }

    // CSV columns:
    // 0: Course ID, 1: Course Name, 2: Prefix, 3: Number, 4: Prerequisites,
    // 5: Corequisites, 6: Strict-Corequisites, 7: Credit Hours, 8: Institution,
    // 9: Canonical Name, 10: Complexity, 11: Blocking, 12: Delay, 13: Centrality

    let csv_id = fields[0].clone();
    let course_name = fields[1].clone();
    let complexity: f64 = fields[10].parse().unwrap_or(0.0);
    let blocking: usize = fields[11].parse().unwrap_or(0);
    let delay: f64 = fields[12].parse().unwrap_or(0.0);
    let centrality: usize = fields[13].parse().unwrap_or(0);

    Some(ExpectedCourseMetrics {
        csv_id,
        course_name,
        complexity,
        blocking,
        delay,
        centrality,
    })
}

/// Parse a CSV line handling quoted fields
fn parse_csv_line(line: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut current = String::new();
    let mut in_quotes = false;

    for ch in line.chars() {
        match ch {
            '"' => in_quotes = !in_quotes,
            ',' if !in_quotes => {
                fields.push(current.trim().to_string());
                current = String::new();
            }
            _ => current.push(ch),
        }
    }
    fields.push(current.trim().to_string());
    fields
}

/// Compare computed metrics against expected metrics (with scaling for quarter systems)
fn compare_metrics(
    computed: &CurriculumMetrics,
    expected: &ExpectedSummary,
    csv_id_to_storage_key: &HashMap<String, String>,
    scale_factor: f64,
) -> Vec<String> {
    let mut differences = Vec::new();

    for expected_course in &expected.courses {
        // Look up storage key from CSV ID
        let storage_key = csv_id_to_storage_key
            .get(&expected_course.csv_id)
            .cloned()
            .unwrap_or_else(|| expected_course.csv_id.clone());

        if let Some(actual) = computed.get(&storage_key) {
            // Compare each metric
            #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
            let expected_delay = expected_course.delay as usize;

            // Scale our computed complexity to match reference format
            #[allow(clippy::cast_precision_loss)]
            let scaled_complexity = (actual.complexity as f64) * scale_factor;

            // Compare with tolerance for floating point
            let complexity_diff = (scaled_complexity - expected_course.complexity).abs();

            if actual.delay != expected_delay {
                differences.push(format!(
                    "Course {} ({}): delay {} vs expected {}",
                    expected_course.csv_id,
                    expected_course.course_name,
                    actual.delay,
                    expected_delay
                ));
            }
            if actual.blocking != expected_course.blocking {
                differences.push(format!(
                    "Course {} ({}): blocking {} vs expected {}",
                    expected_course.csv_id,
                    expected_course.course_name,
                    actual.blocking,
                    expected_course.blocking
                ));
            }
            // Compare complexity with tolerance for floating point rounding
            if complexity_diff > 0.1 {
                differences.push(format!(
                    "Course {} ({}): complexity {:.1} vs expected {:.1}",
                    expected_course.csv_id,
                    expected_course.course_name,
                    scaled_complexity,
                    expected_course.complexity
                ));
            }
            if actual.centrality != expected_course.centrality {
                differences.push(format!(
                    "Course {} ({}): centrality {} vs expected {}",
                    expected_course.csv_id,
                    expected_course.course_name,
                    actual.centrality,
                    expected_course.centrality
                ));
            }
        } else {
            differences.push(format!(
                "Course {} ({}) not found in computed metrics",
                expected_course.csv_id, expected_course.course_name
            ));
        }
    }

    differences
}

/// Compute total structural complexity from metrics
fn compute_total_complexity(metrics: &CurriculumMetrics) -> usize {
    metrics.values().map(|m| m.complexity).sum()
}

/// Run a metrics comparison test for a given plan
fn run_metrics_comparison_test(plan_name: &str) {
    let plan_path = format!("samples/plans/{plan_name}.csv");
    let reference_path = format!("samples/correct/{plan_name}_w_metrics.csv");

    // Parse the plan
    let school = parse_curriculum_csv(&plan_path)
        .unwrap_or_else(|e| panic!("Failed to parse plan {plan_name}: {e}"));

    // Build DAG and compute metrics
    let dag = school.build_dag();
    let metrics = metrics::compute_all_metrics(&dag)
        .unwrap_or_else(|e| panic!("Failed to compute metrics for {plan_name}: {e}"));

    // Parse expected metrics
    let expected = parse_expected_metrics(Path::new(&reference_path));

    // Build CSV ID to storage key mapping
    let mut csv_id_to_storage_key: HashMap<String, String> = HashMap::new();
    for (storage_key, course) in school.courses_with_keys() {
        if let Some(csv_id) = &course.csv_id {
            csv_id_to_storage_key.insert(csv_id.clone(), storage_key.clone());
        }
    }

    // Get scale factor from degree (for quarter systems)
    let scale_factor = school
        .degrees
        .first()
        .map_or(1.0, nu_analytics::models::Degree::complexity_scale_factor);

    // Compare total structural complexity (with scaling)
    let computed_total = compute_total_complexity(&metrics);
    #[allow(clippy::cast_precision_loss)]
    let scaled_total = (computed_total as f64) * scale_factor;
    let expected_total = expected.total_structural_complexity;

    // Compare individual course metrics
    let differences = compare_metrics(&metrics, &expected, &csv_id_to_storage_key, scale_factor);

    // Report results (allow small floating point tolerance)
    let total_diff = (scaled_total - expected_total).abs();
    if total_diff > 0.5 || !differences.is_empty() {
        let mut error_msg = format!("\n=== Metrics Comparison Failed for {plan_name} ===\n\n");

        let _ = writeln!(
            error_msg,
            "Total Structural Complexity: {scaled_total:.1} (computed) vs {expected_total:.1} (expected)\n"
        );

        if !differences.is_empty() {
            let _ = writeln!(error_msg, "Course differences ({}):", differences.len());
            for diff in &differences {
                let _ = writeln!(error_msg, "  - {diff}");
            }
        }

        panic!("{error_msg}");
    }
}

// ============================================================================
// Individual test cases for each curriculum file
// ============================================================================

#[test]
fn metrics_match_bscs_hawaii_manoa() {
    run_metrics_comparison_test("BSCS_Hawaii_Manoa");
}

#[test]
fn metrics_match_california_berkeley_v2() {
    run_metrics_comparison_test("California_Berkely_V2");
}

#[test]
fn metrics_match_colostate_cs_degree() {
    run_metrics_comparison_test("Colostate_CSDegree");
}

#[test]
fn metrics_match_colostate_cs_degree_2017() {
    run_metrics_comparison_test("Colostate_CSDegree_2017");
}

#[test]
fn metrics_match_colostate_cs_degree_2017_w_math() {
    run_metrics_comparison_test("Colostate_CSDegree_2017_w_MATH");
}

#[test]
fn metrics_match_kennesaw_state_university_cs() {
    run_metrics_comparison_test("Kennesaw_State_University_CS");
}

#[test]
fn metrics_match_metropolitan_state_university_cs() {
    run_metrics_comparison_test("Metropolitan_State_University_CS");
}

#[test]
fn metrics_match_michigan_ann_arbor_cs() {
    run_metrics_comparison_test("Michigan_Ann_Arbor_CS");
}

#[test]
fn metrics_match_u_of_colorado_boulder_cs() {
    run_metrics_comparison_test("U_of_Colorado_Boulder_CS");
}

// ============================================================================
// Aggregate test to run all comparisons and collect statistics
// ============================================================================

#[test]
fn aggregate_metrics_comparison() {
    let plans = [
        "BSCS_Hawaii_Manoa",
        "California_Berkely_V2",
        "Colostate_CSDegree",
        "Colostate_CSDegree_2017",
        "Colostate_CSDegree_2017_w_MATH",
        "Kennesaw_State_University_CS",
        "Metropolitan_State_University_CS",
        "Michigan_Ann_Arbor_CS",
        "U_of_Colorado_Boulder_CS",
    ];

    let mut passed = 0;
    let mut failed = 0;
    let mut failures: Vec<String> = Vec::new();

    for plan_name in &plans {
        let plan_path = format!("samples/plans/{plan_name}.csv");
        let reference_path = format!("samples/correct/{plan_name}_w_metrics.csv");

        // Skip if reference file doesn't exist
        if !Path::new(&reference_path).exists() {
            println!("Skipping {plan_name}: reference file not found");
            continue;
        }

        let school = match parse_curriculum_csv(&plan_path) {
            Ok(s) => s,
            Err(e) => {
                failures.push(format!("{plan_name}: Failed to parse - {e}"));
                failed += 1;
                continue;
            }
        };

        let dag = school.build_dag();
        let metrics = match metrics::compute_all_metrics(&dag) {
            Ok(m) => m,
            Err(e) => {
                failures.push(format!("{plan_name}: Failed to compute metrics - {e}"));
                failed += 1;
                continue;
            }
        };

        let expected = parse_expected_metrics(Path::new(&reference_path));

        // Build CSV ID to storage key mapping
        let mut csv_id_to_storage_key: HashMap<String, String> = HashMap::new();
        for (storage_key, course) in school.courses_with_keys() {
            if let Some(csv_id) = &course.csv_id {
                csv_id_to_storage_key.insert(csv_id.clone(), storage_key.clone());
            }
        }

        // Get scale factor from degree (for quarter systems)
        let scale_factor = school
            .degrees
            .first()
            .map_or(1.0, nu_analytics::models::Degree::complexity_scale_factor);

        let computed_total = compute_total_complexity(&metrics);
        #[allow(clippy::cast_precision_loss)]
        let scaled_total = (computed_total as f64) * scale_factor;
        let expected_total = expected.total_structural_complexity;
        let total_diff = (scaled_total - expected_total).abs();
        let differences =
            compare_metrics(&metrics, &expected, &csv_id_to_storage_key, scale_factor);

        if total_diff <= 0.5 && differences.is_empty() {
            passed += 1;
            println!("✅ {plan_name}: PASSED (complexity: {scaled_total:.1})");
        } else {
            failed += 1;
            let diff_summary = if total_diff > 0.5 {
                format!(
                    "complexity {:.1} vs {:.1}, {} course differences",
                    scaled_total,
                    expected_total,
                    differences.len()
                )
            } else {
                format!("{} course differences", differences.len())
            };
            failures.push(format!("{plan_name}: {diff_summary}"));
            println!("❌ {plan_name}: FAILED ({diff_summary})");
        }
    }

    println!("\n=== Summary ===");
    println!("Passed: {passed}/{}", passed + failed);
    println!("Failed: {failed}/{}", passed + failed);

    if !failures.is_empty() {
        println!("\nFailures:");
        for failure in &failures {
            println!("  - {failure}");
        }
    }

    // All 9 should pass now with proper quarter system scaling
    assert!(
        passed >= 9,
        "Expected all 9 plans to pass, but only {passed} passed"
    );
}
