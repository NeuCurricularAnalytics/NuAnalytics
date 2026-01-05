//! Planner command handler

use nu_analytics::core::{metrics, planner::parse_curriculum_csv};

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

            let delay_result = metrics::compute_delay(&dag);
            let blocking_result = metrics::compute_blocking(&dag);

            match delay_result {
                Ok(ref delay_by_course) => {
                    println!("\nDelay factors (longest requisite path lengths in vertices):");

                    let mut entries: Vec<_> = delay_by_course.iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(b.0));

                    for (course, delay) in entries {
                        println!("  {course}: {delay}");
                    }
                }
                Err(ref err) => {
                    eprintln!("✗ Failed to compute delay factors: {err}");
                }
            }

            match blocking_result {
                Ok(ref blocking_by_course) => {
                    println!("\nBlocking factors (number of courses blocked by each course):");

                    let mut entries: Vec<_> = blocking_by_course.iter().collect();
                    entries.sort_by(|a, b| a.0.cmp(b.0));

                    for (course, blocking) in entries {
                        println!("  {course}: {blocking}");
                    }
                }
                Err(ref err) => {
                    eprintln!("✗ Failed to compute blocking factors: {err}");
                }
            }

            if let (Ok(delay), Ok(blocking)) = (delay_result, blocking_result) {
                match metrics::compute_complexity(&delay, &blocking) {
                    Ok(complexity_by_course) => {
                        println!("\nStructural complexity (delay + blocking):");

                        let mut entries: Vec<_> = complexity_by_course.into_iter().collect();
                        entries.sort_by(|a, b| a.0.cmp(&b.0));

                        for (course, complexity) in entries {
                            println!("  {course}: {complexity}");
                        }
                    }
                    Err(err) => {
                        eprintln!("✗ Failed to compute complexity: {err}");
                    }
                }
            }

            if let Some(output) = output_file {
                println!("\nOutput file specified: {}", output.display());
                println!("(Output functionality coming soon)");
            }
        }
        Err(e) => {
            eprintln!("✗ Failed to load curriculum: {e}");
        }
    }
}
