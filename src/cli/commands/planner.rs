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
pub fn run(input_file: &std::path::Path, output_file: Option<&std::path::Path>) {
    match parse_curriculum_csv(input_file) {
        Ok(school) => {
            println!(
                "✓ Curriculum loaded successfully from: {}",
                input_file.display()
            );

            // Build and display the prerequisite DAG
            let dag = school.build_dag();
            println!("\n{dag}");

            // Compute all metrics at once
            match metrics::compute_all_metrics(&dag) {
                Ok(all_metrics) => {
                    println!("\nDelay factors (longest requisite path lengths in vertices):");
                    let mut entries: Vec<_> = all_metrics.iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(b.0));
                    for (course, m) in &entries {
                        println!("  {course}: {}", m.delay);
                    }

                    println!("\nBlocking factors (number of courses blocked by each course):");
                    for (course, m) in &entries {
                        println!("  {course}: {}", m.blocking);
                    }

                    println!("\nStructural complexity (delay + blocking):");
                    for (course, m) in &entries {
                        println!("  {course}: {}", m.complexity);
                    }

                    println!("\nCentrality (sum of path lengths through each course):");
                    for (course, m) in &entries {
                        println!("  {course}: {}", m.centrality);
                    }

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
                            Ok(()) => {
                                println!("\n✓ Metrics exported to: {}", output.display());
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
