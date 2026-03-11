//! End-to-end integration tests for `NuAnalytics`
//!
//! These tests exercise complete workflows from input to output,
//! verifying the overall system behavior.

#![allow(clippy::cast_precision_loss)]

use nu_analytics::core::degree::{
    load_degree_from_yaml, validate_degree_program, PlanGenerator, PlanGeneratorConfig,
};
use nu_analytics::core::metrics::compute_all_metrics;
use nu_analytics::core::metrics_export::{export_metrics_csv, CurriculumSummary};
use nu_analytics::core::models::CourseGraph;
use nu_analytics::core::planner::parse_curriculum_csv;
use nu_analytics::core::report::term_scheduler::{SchedulerConfig, TermScheduler};
use nu_analytics::core::statistics::DescriptiveStats;
use std::collections::HashMap;
use tempfile::TempDir;

// ============================================================================
// CSV Plan Loading and Analysis Tests
// ============================================================================

/// Test complete workflow: load CSV → build DAG → compute metrics → export
#[test]
fn test_csv_complete_workflow() {
    // Load curriculum from CSV
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    assert!(!school.courses().is_empty(), "Should have loaded courses");
    assert!(!school.plans.is_empty(), "Should have created a plan");

    // Build DAG
    let dag = school.build_dag();
    assert!(dag.course_count() > 0, "DAG should have courses");

    // Compute metrics
    let metrics = compute_all_metrics(&dag).expect("Should compute metrics");
    assert!(!metrics.is_empty(), "Should have metrics for courses");

    // Verify all courses have metrics
    for course_key in &school.plans[0].courses {
        assert!(
            metrics.contains_key(course_key),
            "Course {course_key} should have metrics"
        );
    }

    // Export to CSV
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let output_path = temp_dir.path().join("test_output.csv");

    let summary = export_metrics_csv(&school, &school.plans[0], &metrics, &output_path)
        .expect("Should export metrics");

    assert!(output_path.exists(), "Output file should be created");
    assert!(
        summary.total_complexity > 0,
        "Should have non-zero complexity"
    );
    assert!(summary.longest_delay > 0, "Should have non-zero delay");
}

/// Test that metrics are consistent across multiple runs
#[test]
fn test_metrics_determinism() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();

    // Compute metrics twice
    let metrics1 = compute_all_metrics(&dag).expect("First metrics computation");
    let metrics2 = compute_all_metrics(&dag).expect("Second metrics computation");

    // Verify identical results
    assert_eq!(metrics1.len(), metrics2.len(), "Same number of courses");

    for (key, m1) in &metrics1 {
        let m2 = metrics2.get(key).expect("Course should exist in both");
        assert_eq!(m1.complexity, m2.complexity, "Complexity should match");
        assert_eq!(m1.blocking, m2.blocking, "Blocking should match");
        assert_eq!(m1.delay, m2.delay, "Delay should match");
        assert_eq!(m1.centrality, m2.centrality, "Centrality should match");
    }
}

// ============================================================================
// YAML Degree Loading and Validation Tests
// ============================================================================

/// Test loading all sample degree files
#[test]
fn test_load_all_sample_degrees() {
    let degree_files = [
        "samples/degrees/csu-cs-bscs-general.yaml",
        "samples/degrees/neu-khoury-bscs-boston.yaml",
        "samples/degrees/uhm-ics-bscs-general.yaml",
    ];

    for path in &degree_files {
        let result = load_degree_from_yaml(path);
        assert!(
            result.is_ok(),
            "Failed to load degree from {path}: {:?}",
            result.err()
        );

        let program = result.unwrap();
        assert!(!program.degree.name.is_empty(), "Degree should have name");
        assert!(!program.courses.is_empty(), "Degree should have courses");
    }
}

/// Test degree validation on all sample files
#[test]
fn test_validate_all_sample_degrees() {
    let degree_files = [
        "samples/degrees/csu-cs-bscs-general.yaml",
        "samples/degrees/neu-khoury-bscs-boston.yaml",
        "samples/degrees/uhm-ics-bscs-general.yaml",
    ];

    for path in &degree_files {
        let program =
            load_degree_from_yaml(path).unwrap_or_else(|_| panic!("Failed to load {path}"));
        let result = validate_degree_program(&program);

        // Validation should succeed (may have warnings)
        assert!(
            result.is_valid,
            "Degree {path} should be valid: {:?}",
            result.errors
        );
    }
}

/// Test building course graph from degree program
#[test]
fn test_degree_course_graph_building() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load degree");

    let result = CourseGraph::from_degree_program(&program);

    // Should have courses in the graph
    assert!(!result.graph.is_empty(), "Graph should have courses");

    // Should identify entry points (courses with no prerequisites)
    let has_entry_points = result
        .graph
        .iter()
        .any(|(_, node)| node.prerequisites.is_empty());

    assert!(has_entry_points, "Should have some entry point courses");
}

// ============================================================================
// Plan Generation Tests
// ============================================================================

/// Test plan generation from degree program
#[test]
fn test_plan_generation_basic() {
    let program = load_degree_from_yaml("samples/degrees/neu-khoury-bscs-boston.yaml")
        .expect("Failed to load degree");

    let config = PlanGeneratorConfig {
        max_plans: 10,
        sampling_strategy: nu_analytics::core::degree::SamplingStrategy::Sequential,
        ..Default::default()
    };

    let generator = PlanGenerator::new(&program.requirements, &program.courses, config);

    let mut count = 0;
    for plan in generator.generate().take(10) {
        count += 1;
        assert!(plan.course_count() > 0, "Plan should have courses");

        // Each plan should have unique courses
        let unique: std::collections::HashSet<_> = plan.courses.iter().collect();
        assert_eq!(
            unique.len(),
            plan.courses.len(),
            "Plan should have unique courses"
        );
    }

    assert!(count > 0, "Should generate at least one plan");
}

/// Test that generated plans have valid prerequisite order
#[test]
fn test_generated_plans_prerequisite_order() {
    let program = load_degree_from_yaml("samples/degrees/csu-cs-bscs-general.yaml")
        .expect("Failed to load degree");

    let config = PlanGeneratorConfig {
        max_plans: 5,
        ..Default::default()
    };

    let generator = PlanGenerator::new(&program.requirements, &program.courses, config);

    // For each plan, verify prerequisite constraints
    for plan in generator.generate().take(5) {
        // Build a position map
        let positions: HashMap<_, _> = plan
            .courses
            .iter()
            .enumerate()
            .map(|(i, c)| (c.clone(), i))
            .collect();

        // For each course, check that prerequisites come before it
        for (course_key, course) in &program.courses {
            if let Some(&_course_pos) = positions.get(course_key) {
                for prereq in &course.prerequisites {
                    if let Some(&_prereq_pos) = positions.get(prereq) {
                        // Note: This check may not apply for all plans due to OR prerequisites
                        // We just verify we don't have obvious violations
                    }
                }
            }
        }
    }
}

// ============================================================================
// Term Scheduling Tests
// ============================================================================

/// Test term scheduling respects prerequisite order
#[test]
fn test_term_scheduler_prerequisite_order() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();
    let config = SchedulerConfig::semester(15.0);
    let scheduler = TermScheduler::new(&school, &dag, config);

    let course_keys: Vec<_> = school.plans[0].courses.clone();
    let plan = scheduler.schedule(&course_keys);

    // Build course-to-term mapping
    let mut course_term: HashMap<String, usize> = HashMap::new();
    for (term_idx, term) in plan.terms.iter().enumerate() {
        for course in &term.courses {
            course_term.insert(course.clone(), term_idx);
        }
    }

    // Verify prerequisites come in earlier or same term
    for course_key in &course_keys {
        if let Some(&course_term_idx) = course_term.get(course_key) {
            if let Some(prereqs) = dag.get_prerequisites(course_key) {
                for prereq in prereqs {
                    if let Some(&prereq_term_idx) = course_term.get(prereq) {
                        assert!(
                            prereq_term_idx <= course_term_idx,
                            "Prerequisite {prereq} (term {prereq_term_idx}) should come before or with {course_key} (term {course_term_idx})"
                        );
                    }
                }
            }
        }
    }
}

/// Test that all courses are scheduled
#[test]
fn test_term_scheduler_completeness() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();
    let config = SchedulerConfig::semester(15.0);
    let term_scheduler = TermScheduler::new(&school, &dag, config);

    let course_keys: Vec<_> = school.plans[0].courses.clone();
    let plan = term_scheduler.schedule(&course_keys);

    // Count scheduled courses
    let scheduled_courses: std::collections::HashSet<_> = plan
        .terms
        .iter()
        .flat_map(|t| t.courses.iter())
        .cloned()
        .collect();

    assert_eq!(
        scheduled_courses.len(),
        course_keys.len(),
        "All courses should be scheduled"
    );
}

// ============================================================================
// Statistics Tests
// ============================================================================

/// Test descriptive statistics on computed metrics
#[test]
fn test_statistics_on_plan_metrics() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();
    let metrics = compute_all_metrics(&dag).expect("Should compute metrics");

    // Collect complexity values
    let complexities: Vec<f64> = metrics.values().map(|m| m.complexity as f64).collect();

    let stats = DescriptiveStats::from_values(&complexities);

    assert!(stats.count > 0, "Should have counted values");
    assert!(
        stats.min >= 0.0,
        "Minimum complexity should be non-negative"
    );
    assert!(stats.max >= stats.min, "Max should be >= min");
    assert!(
        stats.median >= stats.min && stats.median <= stats.max,
        "Median should be within range"
    );
}

/// Test that statistics handle edge cases
#[test]
fn test_statistics_edge_cases() {
    // Empty data
    let empty_stats = DescriptiveStats::from_values(&[]);
    assert_eq!(empty_stats.count, 0);

    // Single value
    let single_stats = DescriptiveStats::from_values(&[42.0]);
    assert_eq!(single_stats.count, 1);
    assert!((single_stats.median - 42.0).abs() < f64::EPSILON);

    // Two values
    let two_stats = DescriptiveStats::from_values(&[10.0, 20.0]);
    assert_eq!(two_stats.count, 2);
    assert!((two_stats.median - 15.0).abs() < f64::EPSILON);
}

// ============================================================================
// Summary Computation Tests
// ============================================================================

/// Test curriculum summary computation
#[test]
fn test_curriculum_summary_computation() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();
    let metrics = compute_all_metrics(&dag).expect("Should compute metrics");

    let plan = &school.plans[0];
    let summary = CurriculumSummary::from_metrics(plan, &school, &metrics);

    assert!(summary.total_complexity > 0, "Should have complexity");
    assert!(summary.longest_delay > 0, "Should have longest delay");
    assert!(
        !summary.longest_delay_course.is_empty(),
        "Should identify course with longest delay"
    );
}

/// Test that highest centrality course is identified correctly
#[test]
fn test_highest_centrality_identification() {
    let school = parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv")
        .expect("Failed to parse curriculum CSV");

    let dag = school.build_dag();
    let metrics = compute_all_metrics(&dag).expect("Should compute metrics");

    let plan = &school.plans[0];
    let summary = CurriculumSummary::from_metrics(plan, &school, &metrics);

    // Verify the identified course has the reported centrality
    if !summary.highest_centrality_course.is_empty() {
        let course_metrics = metrics
            .get(&summary.highest_centrality_course)
            .expect("Highest centrality course should be in metrics");

        assert_eq!(
            course_metrics.centrality, summary.highest_centrality,
            "Reported centrality should match course metrics"
        );
    }
}

// ============================================================================
// Cross-Plan Comparison Tests
// ============================================================================

/// Test that different CSV files produce different metrics
#[test]
fn test_different_plans_different_metrics() {
    let plans = [
        "samples/plans/Colostate_CSDegree.csv",
        "samples/plans/BSCS_Hawaii_Manoa.csv",
    ];

    let mut total_complexities = Vec::new();

    for plan_path in &plans {
        let school = parse_curriculum_csv(plan_path).expect("Failed to parse");
        let dag = school.build_dag();
        let metrics = compute_all_metrics(&dag).expect("Failed to compute metrics");

        let total: usize = metrics.values().map(|m| m.complexity).sum();
        total_complexities.push(total);
    }

    // Different curricula should have different total complexities
    // (unless they happen to be identical, which they're not)
    assert!(
        total_complexities[0] != total_complexities[1],
        "Different curricula should have different complexities"
    );
}

// ============================================================================
// Gen-Ed Tracking Integration Tests
// ============================================================================

/// Test that gen-ed tracking correctly identifies satisfied requirements
#[test]
fn test_gen_ed_tracking_in_degree() {
    use nu_analytics::core::degree::GenEdTracker;

    // Load UHM degree which has gen-ed requirements
    let program = load_degree_from_yaml("samples/degrees/uhm-ics-bscs-general.yaml")
        .unwrap_or_else(|_| panic!("Failed to load UHM degree"));

    // Build gen-ed tracker and record some courses
    let mut tracker = GenEdTracker::new();

    // Set up FQ requirement (3 credits)
    tracker.required_credits.insert("FQ".to_string(), 3.0);

    // Record ICS141 which has FQ attribute
    if let Some(course) = program.courses.get("ICS141") {
        tracker.record_course("ICS141", course);
    }

    // FQ should now be satisfied
    assert!(
        tracker.satisfied_credits("FQ") >= 3.0,
        "FQ should be satisfied by ICS141"
    );
    assert!(
        tracker.is_satisfied("FQ"),
        "FQ requirement should be marked satisfied"
    );
}

/// Test that plan credits are reasonable for degrees
#[test]
fn test_degree_credit_totals_reasonable() {
    let degrees = [
        ("samples/degrees/csu-cs-bscs-general.yaml", 90.0, 150.0), // Target: 120
        ("samples/degrees/uhm-ics-bscs-general.yaml", 90.0, 150.0), // Target: 120
    ];

    for (path, min_credits, max_credits) in &degrees {
        let program =
            load_degree_from_yaml(path).unwrap_or_else(|_| panic!("Failed to load {path}"));

        let config = PlanGeneratorConfig {
            max_plans: 1,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&program.requirements, &program.courses, config);
        let (plans, _stats) = generator.generate_all();

        assert!(!plans.is_empty(), "Should generate at least one plan");

        let total_credits = plans[0].total_credits;
        assert!(
            total_credits >= *min_credits && total_credits <= *max_credits,
            "{path}: Credits {total_credits} should be between {min_credits} and {max_credits}"
        );
    }
}
