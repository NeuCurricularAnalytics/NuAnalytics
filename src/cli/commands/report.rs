//! Report generation utilities for CLI commands.
//!
//! This module provides shared report generation functionality used by
//! multiple CLI commands. It handles loading curriculum data, computing
//! metrics, scheduling terms, and rendering reports in various formats
//! (Markdown, HTML, PDF).
//!
//! The main entry point is [`generate_report_file`], which orchestrates
//! the full report generation pipeline from an input CSV file.

use crate::args::ReportFormatArg;
use nu_analytics::core::{
    metrics, metrics_export,
    models::{Degree, Plan, School, DAG},
    planner::parse_curriculum_csv,
    report::{
        formats::ReportFormat, HtmlReporter, MarkdownReporter, PdfReporter, ReportContext,
        ReportGenerator, SchedulerConfig, TermPlan, TermScheduler,
    },
};
use nu_analytics::{error, info};
use std::path::{Path, PathBuf};

/// Default target credits per term
const DEFAULT_TERM_CREDITS: f32 = 15.0;

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
fn write_report(
    data: &ReportData,
    format: ReportFormat,
    output_path: &Path,
    pdf_converter: Option<&str>,
) -> Result<(), String> {
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
            let reporter = pdf_converter.map_or_else(PdfReporter::new, PdfReporter::with_converter);
            reporter
                .generate(&ctx, output_path)
                .map_err(|e| format!("✗ Failed to generate PDF report: {e}"))?;
        }
    }

    Ok(())
}

/// Print a summary of the report to stdout
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

/// Convert CLI format arg to internal `ReportFormat`
const fn to_report_format(fmt: ReportFormatArg) -> ReportFormat {
    match fmt {
        ReportFormatArg::Html => ReportFormat::Html,
        ReportFormatArg::Md => ReportFormat::Markdown,
        ReportFormatArg::Pdf => ReportFormat::Pdf,
    }
}

/// Generate a report file from an input curriculum CSV
///
/// # Arguments
/// * `input_file` - Path to input CSV file
/// * `output_file` - Optional explicit output path (overrides `reports_dir`)
/// * `format` - Report format (Html, Md, Pdf)
/// * `reports_dir` - Directory for output when `output_file` is None
/// * `term_credits` - Optional target credits per term
/// * `pdf_converter` - Optional custom PDF converter command
///
/// # Returns
/// Path to the generated report file
pub fn generate_report_file(
    input_file: &Path,
    output_file: Option<&Path>,
    format: ReportFormatArg,
    reports_dir: &str,
    term_credits: Option<f32>,
    pdf_converter: Option<&str>,
) -> Result<PathBuf, String> {
    // Convert to internal format type
    let report_format = to_report_format(format);

    // Prepare report data
    let data = prepare_report_data(input_file, term_credits)?;

    // Determine output path
    let output_path: PathBuf = if let Some(explicit_path) = output_file {
        // Ensure parent directory exists
        if let Some(parent) = explicit_path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                format!(
                    "✗ Failed to create output directory {}: {e}",
                    parent.display()
                )
            })?;
        }
        explicit_path.to_path_buf()
    } else {
        // Use reports_dir with generated filename
        let reports_path = PathBuf::from(reports_dir);
        std::fs::create_dir_all(&reports_path).map_err(|e| {
            format!(
                "✗ Failed to create reports directory {}: {e}",
                reports_path.display()
            )
        })?;

        let filename = input_file
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or("curriculum")
            .to_string();
        let output_filename = format!("{filename}_report.{}", format.extension());
        reports_path.join(output_filename)
    };

    // Write the report
    write_report(&data, report_format, &output_path, pdf_converter)?;

    info!("Report exported to: {}", output_path.display());
    print_summary(&data);

    Ok(output_path)
}
