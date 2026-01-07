//! Report command handler
//!
//! Generates curriculum reports in various formats (Markdown, HTML, PDF)
//! with metrics visualization and term scheduling.

use logger::{error, info};
use nu_analytics::config::Config;
use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan, School, DAG},
    planner::parse_curriculum_csv,
    report::{
        formats::ReportFormat, HtmlReporter, MarkdownReporter, ReportContext, ReportGenerator,
        SchedulerConfig, TermPlan, TermScheduler,
    },
};
use std::path::{Path, PathBuf};
use std::str::FromStr;

/// Default target credits per term
const DEFAULT_TERM_CREDITS: f32 = 15.0;

/// Run the report command.
///
/// # Arguments
/// * `input_file` - Path to input CSV file
/// * `output_file` - Optional output path
/// * `format_str` - Report format (markdown, html, pdf)
/// * `term_credits` - Optional target credits per term
/// * `config` - Configuration containing default output directory
pub fn run(
    input_file: &Path,
    output_file: Option<&Path>,
    format_str: &str,
    term_credits: Option<f32>,
    config: &Config,
) {
    if let Err(err) = generate_report(input_file, output_file, format_str, term_credits, config) {
        error!(
            "Report generation failed for {}: {err}",
            input_file.display()
        );
        eprintln!("{err}");
    }
}

/// Prepared report data ready for rendering
struct ReportData {
    school: School,
    plan: Plan,
    dag: DAG,
    metrics: metrics::CurriculumMetrics,
    summary: metrics_export::CurriculumSummary,
    term_plan: TermPlan,
}

/// Load and prepare all data needed for report generation
fn prepare_report_data(input_file: &Path, term_credits: Option<f32>) -> Result<ReportData, String> {
    // Load curriculum
    let school = parse_curriculum_csv(input_file).map_err(|e| {
        error!("Failed to load curriculum {}: {e}", input_file.display());
        format!("✗ Failed to load {}: {e}", input_file.display())
    })?;

    info!("Curriculum loaded: {}", input_file.display());

    // Build DAG
    let dag = school.build_dag();

    // Compute metrics
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

    // Get or create plan
    let plan = school.plans.first().cloned().unwrap_or_else(|| {
        let mut default_plan = Plan::new(
            "All Courses".to_string(),
            school.degrees.first().map_or_else(String::new, Degree::id),
        );
        for course in &dag.courses {
            default_plan.add_course(course.clone());
        }
        default_plan
    });

    // Get degree for context
    let degree = school.degrees.first();

    // Compute summary
    let summary = metrics_export::CurriculumSummary::from_metrics(&plan, &school, &all_metrics)
        .with_delay_path(&dag, &all_metrics);

    // Configure term scheduler
    let is_quarter = degree.is_some_and(Degree::is_quarter_system);
    let credits = term_credits.unwrap_or(DEFAULT_TERM_CREDITS);
    let scheduler_config = if is_quarter {
        SchedulerConfig::quarter(credits)
    } else {
        SchedulerConfig::semester(credits)
    };

    // Schedule courses into terms
    let scheduler = TermScheduler::new(&school, &dag, scheduler_config);
    let term_plan = scheduler.schedule(&plan.courses);

    Ok(ReportData {
        school,
        plan,
        dag,
        metrics: all_metrics,
        summary,
        term_plan,
    })
}

/// Write the report to a file in the specified format
fn write_report(data: &ReportData, format: ReportFormat, output_path: &Path) -> Result<(), String> {
    let degree = data.school.degrees.first();
    let ctx = ReportContext::new(
        &data.school,
        &data.plan,
        degree,
        &data.metrics,
        &data.summary,
        &data.dag,
        &data.term_plan,
    );

    match format {
        ReportFormat::Markdown => {
            let reporter = MarkdownReporter::new();
            reporter
                .generate(&ctx, output_path)
                .map_err(|e| format!("✗ Failed to generate Markdown report: {e}"))?;
        }
        ReportFormat::Html => {
            let reporter = HtmlReporter::new();
            reporter
                .generate(&ctx, output_path)
                .map_err(|e| format!("✗ Failed to generate HTML report: {e}"))?;
        }
        ReportFormat::Pdf => {
            // For now, generate HTML and suggest conversion
            let html_path = output_path.with_extension("html");
            let reporter = HtmlReporter::new();
            reporter
                .generate(&ctx, &html_path)
                .map_err(|e| format!("✗ Failed to generate HTML for PDF: {e}"))?;
            println!(
                "ℹ PDF generation not yet implemented. HTML generated at: {}",
                html_path.display()
            );
            println!("  Use a browser or wkhtmltopdf to convert to PDF.");
        }
    }

    Ok(())
}

/// Print a summary of the report
fn print_summary(data: &ReportData) {
    println!("\n=== Summary ===");
    println!("Plan: {}", data.plan.name);
    println!(
        "Institution: {}",
        data.plan
            .institution
            .as_deref()
            .unwrap_or(&data.school.name)
    );
    println!("Total Courses: {}", data.plan.courses.len());
    println!("Total Complexity: {}", data.summary.total_complexity);
    println!(
        "Longest Delay: {} ({})",
        data.summary.longest_delay, data.summary.longest_delay_course
    );
    println!("Terms Used: {}", data.term_plan.terms_used());

    if !data.term_plan.unscheduled.is_empty() {
        println!(
            "⚠️  {} courses couldn't be scheduled in {} terms",
            data.term_plan.unscheduled.len(),
            data.term_plan.terms.len()
        );
    }
}

fn generate_report(
    input_file: &Path,
    output_file: Option<&Path>,
    format_str: &str,
    term_credits: Option<f32>,
    config: &Config,
) -> Result<(), String> {
    // Parse the format
    let format = ReportFormat::from_str(format_str)
        .map_err(|e| format!("✗ {e}. Use: markdown, html, or pdf"))?;

    // Prepare report data
    let data = prepare_report_data(input_file, term_credits)?;

    // Determine output path
    let final_output_path: PathBuf = if let Some(output) = output_file {
        output.to_path_buf()
    } else {
        let reports_dir = PathBuf::from(&config.paths.reports_dir);
        std::fs::create_dir_all(&reports_dir).map_err(|e| {
            format!(
                "✗ Failed to create reports directory {}: {e}",
                reports_dir.display()
            )
        })?;

        let filename = input_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("curriculum")
            .to_string();
        let output_filename = format!("{filename}_report.{}", format.extension());
        reports_dir.join(output_filename)
    };

    // Write the report
    write_report(&data, format, &final_output_path)?;

    if format != ReportFormat::Pdf {
        println!("✓ Report generated: {}", final_output_path.display());
        info!("Report exported to: {}", final_output_path.display());
    }

    print_summary(&data);

    Ok(())
}

/// Generate a report as part of the planner command
///
/// This is called when `--report` is passed to the planner command.
pub fn generate_from_planner(
    input_file: &Path,
    output_dir: &Path,
    format_str: &str,
    term_credits: Option<f32>,
) -> Result<PathBuf, String> {
    // Parse the format
    let format = ReportFormat::from_str(format_str)
        .map_err(|e| format!("✗ {e}. Use: markdown, html, or pdf"))?;

    // Prepare report data
    let data = prepare_report_data(input_file, term_credits)?;

    // Build output path
    let filename = input_file
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("curriculum")
        .to_string();
    let output_filename = format!("{filename}_report.{}", format.extension());
    let output_path = output_dir.join(output_filename);

    // Write the report
    write_report(&data, format, &output_path)?;

    Ok(output_path)
}
