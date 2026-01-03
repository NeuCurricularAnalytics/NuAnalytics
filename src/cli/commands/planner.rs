//! Planner command handler

use nu_analytics::core::planner::parse_curriculum_csv;

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
            println!("\n{school:#?}");

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
