//! Planner command handler

use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan},
    planner::parse_curriculum_csv,
};

/// Run the planner command
///
/// # Arguments
/// * `input_file` - Path to the input CSV file
/// * `output_file` - Optional path to output file
/// * `verbose` - Whether to show detailed metrics output
pub fn run(input_file: &std::path::Path, output_file: Option<&std::path::Path>, verbose: bool) {
    match parse_curriculum_csv(input_file) {
        Ok(school) => {
            println!(
                "✓ Curriculum loaded successfully from: {}",
                input_file.display()
            );

            // Build the prerequisite DAG
            let dag = school.build_dag();

            // Compute all metrics at once
            match metrics::compute_all_metrics(&dag) {
                Ok(all_metrics) => {
                    // Export metrics to CSV if output file is specified
                    if let Some(output) = output_file {
                        // If no plans exist, create a default plan with all courses
                        let plan = if let Some(p) = school.plans.first() {
                            p.clone()
                        } else {
                            let mut default_plan = Plan::new(
                                "All Courses".to_string(),
                                school.degrees.first().map(Degree::id).unwrap_or_default(),
                            );
                            for course in &dag.courses {
                                default_plan.add_course(course.clone());
                            }
                            default_plan
                        };

                        match metrics_export::export_metrics_csv(
                            &school,
                            &plan,
                            &all_metrics,
                            output,
                        ) {
                            Ok(summary) => {
                                println!("✓ Metrics exported to: {}", output.display());

                                // Show plan-level summary if verbose is enabled
                                if verbose {
                                    println!("\n=== Plan Summary ===");
                                    println!(
                                        "Total Structural Complexity: {}",
                                        summary.total_complexity
                                    );
                                    println!(
                                        "Longest Delay: {} ({})",
                                        summary.longest_delay,
                                        summary.longest_delay_path.join("->")
                                    );
                                    println!(
                                        "Highest Centrality: {} ({})",
                                        summary.highest_centrality,
                                        summary.highest_centrality_course
                                    );
                                }
                            }
                            Err(e) => {
                                eprintln!("✗ Failed to export metrics: {e}");
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("✗ Failed to compute metrics: {e}");
                }
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to load curriculum: {e}");
        }
    }
}
