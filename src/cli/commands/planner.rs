//! Planner command handler

use logger::info;
use nu_analytics::config::Config;
use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan},
    planner::parse_curriculum_csv,
};
use std::path::{Path, PathBuf};

/// Run the planner command
///
/// # Arguments
/// * `input_file` - Path to the input CSV file
/// * `output_file` - Optional path to output file
/// * `config` - Configuration containing default output directory
/// * `verbose` - Whether to show detailed metrics output
pub fn run(input_file: &Path, output_file: Option<&Path>, config: &Config, verbose: bool) {
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

                    // Determine output path: use provided file or config out_dir
                    let final_output_path = output_file.map_or_else(
                        || {
                            // Use config out_dir with a filename based on input file
                            let out_dir = PathBuf::from(&config.paths.out_dir);
                            std::fs::create_dir_all(&out_dir).unwrap_or_else(|e| {
                                eprintln!("⚠ Failed to create output directory: {e}");
                            });

                            // Extract filename from input and append _w_metrics
                            let filename = input_file
                                .file_stem()
                                .and_then(|stem| stem.to_str())
                                .unwrap_or("curriculum")
                                .to_string();
                            let output_filename = format!("{filename}_w_metrics.csv");
                            out_dir.join(output_filename)
                        },
                        Path::to_path_buf,
                    );

                    match metrics_export::export_metrics_csv(
                        &school,
                        &plan,
                        &all_metrics,
                        &final_output_path,
                    ) {
                        Ok(summary) => {
                            println!("✓ Metrics exported to: {}", final_output_path.display());
                            info!(
                                "Exported curriculum metrics to: {}",
                                final_output_path.display()
                            );

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
                                    summary.highest_centrality, summary.highest_centrality_course
                                );
                            }
                        }
                        Err(e) => {
                            eprintln!("✗ Failed to export metrics: {e}");
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
