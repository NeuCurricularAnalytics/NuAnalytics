//! Planner command handler

use logger::{error, info};
use nu_analytics::config::Config;
use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan},
    planner::parse_curriculum_csv,
};
use std::path::{Path, PathBuf};

/// Run the planner command for one or more input files.
///
/// # Arguments
/// * `input_files` - Paths to input CSV files
/// * `output_files` - Optional output paths; must match inputs 1:1 when provided
/// * `config` - Configuration containing default output directory
/// * `verbose` - Whether to show detailed metrics output
pub fn run(input_files: &[PathBuf], output_files: &[PathBuf], config: &Config, verbose: bool) {
    if input_files.is_empty() {
        eprintln!("✗ No input files provided.");
        return;
    }

    if !output_files.is_empty() && output_files.len() != input_files.len() {
        eprintln!(
            "✗ When using -o/--output, provide one output path per input file ({} inputs, {} outputs).",
            input_files.len(),
            output_files.len()
        );
        return;
    }

    for (idx, input_file) in input_files.iter().enumerate() {
        let output_file = output_files.get(idx).map(PathBuf::as_path);
        if let Err(err) = export_single(input_file, output_file, config, verbose) {
            error!("Planner failed for {}: {err}", input_file.display());
            eprintln!("{err}");
        }
    }
}

fn export_single(
    input_file: &Path,
    output_file: Option<&Path>,
    config: &Config,
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
        output.to_path_buf()
    } else {
        let out_dir = PathBuf::from(&config.paths.out_dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            format!(
                "✗ Failed to create output directory {}: {e}",
                out_dir.display()
            )
        })?;

        let filename = input_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("curriculum")
            .to_string();
        let output_filename = format!("{filename}_w_metrics.csv");
        out_dir.join(output_filename)
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
