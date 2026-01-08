//! Planner command handler - CSV metrics export

use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan},
    planner::parse_curriculum_csv,
};
use nu_analytics::{error, info};
use std::path::{Path, PathBuf};

/// Run CSV export for a single input file
///
/// # Arguments
/// * `input_file` - Path to input CSV file
/// * `output_file` - Optional explicit output path
/// * `metrics_dir` - Directory for output when `output_file` is None
/// * `verbose` - Whether to show detailed metrics output
pub fn run_single(input_file: &Path, output_file: Option<&Path>, metrics_dir: &str, verbose: bool) {
    if let Err(err) = export_csv(input_file, output_file, metrics_dir, verbose) {
        error!("Planner failed for {}: {err}", input_file.display());
        eprintln!("{err}");
    }
}

fn export_csv(
    input_file: &Path,
    output_file: Option<&Path>,
    metrics_dir: &str,
    verbose: bool,
) -> Result<(), String> {
    let school = parse_curriculum_csv(input_file).map_err(|e| {
        error!("Failed to load curriculum {}: {e}", input_file.display());
        format!("✗ Failed to load {}: {e}", input_file.display())
    })?;

    if verbose {
        println!(
            "✓ Curriculum loaded successfully from: {}",
            input_file.display()
        );
    } else {
        info!("Curriculum loaded: {}", input_file.display());
    }

    let dag = school.build_dag();

    let all_metrics = metrics::compute_all_metrics(&dag).map_err(|e| {
        error!(
            "Metrics computation failed for {}: {e}",
            input_file.display()
        );
        format!(
            "✗ Failed to compute metrics for {}: {e}",
            input_file.display()
        )
    })?;

    let plan = if let Some(p) = school.plans.first() {
        p.clone()
    } else {
        // If no explicit plans are defined, create a default plan that includes all courses.
        // This ensures metrics can be computed for the entire curriculum even if individual
        // plans haven't been specified in the input file. The default plan is named "All Courses"
        // and associated with the first degree in the school (if any).
        let mut default_plan = Plan::new(
            "All Courses".to_string(),
            school.degrees.first().map_or_else(String::new, Degree::id),
        );
        for course in &dag.courses {
            default_plan.add_course(course.clone());
        }
        default_plan
    };

    let final_output_path: PathBuf = if let Some(output) = output_file {
        // Ensure parent directory exists
        if let Some(parent) = output.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "✗ Failed to create output directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        output.to_path_buf()
    } else {
        let metrics_path = PathBuf::from(metrics_dir);
        std::fs::create_dir_all(&metrics_path).map_err(|e| {
            format!(
                "✗ Failed to create metrics directory {}: {e}",
                metrics_path.display()
            )
        })?;

        let filename = input_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("curriculum")
            .to_string();
        let output_filename = format!("{filename}_w_metrics.csv");
        metrics_path.join(output_filename)
    };

    let plan_name = plan.name.clone();
    let degree_label = school
        .degrees
        .first()
        .map_or_else(|| "Unknown Degree".to_string(), Degree::id);
    let institution_label = plan
        .institution
        .clone()
        .unwrap_or_else(|| school.name.clone());

    match metrics_export::export_metrics_csv(&school, &plan, &all_metrics, &final_output_path) {
        Ok(summary) => {
            println!("✓ Metrics exported to: {}", final_output_path.display());
            info!(
                "Exported curriculum metrics to: {}",
                final_output_path.display()
            );

            if verbose {
                println!("\n=== Plan Summary for {plan_name} ({degree_label}) at {institution_label} ===");
                println!("Total Structural Complexity: {}", summary.total_complexity);
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
            Ok(())
        }
        Err(e) => Err(format!(
            "✗ Failed to export metrics to {}: {e}",
            final_output_path.display()
        )),
    }
}
