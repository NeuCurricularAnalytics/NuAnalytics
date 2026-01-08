//! Markdown report generator
//!
//! Generates curriculum reports in Markdown format with embedded Mermaid diagrams
//! for visualization. These reports render well in GitHub, GitLab, and VS Code.

use crate::core::metrics::CourseMetrics;
use crate::core::report::visualization::MermaidGenerator;
use crate::core::report::{ReportContext, ReportGenerator};
use std::error::Error;
use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Embedded Markdown report template
const MARKDOWN_TEMPLATE: &str = include_str!("../templates/report.md");

/// Markdown report generator
pub struct MarkdownReporter;

impl MarkdownReporter {
    /// Create a new Markdown reporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Render the report using template substitution
    #[allow(clippy::unused_self)]
    fn render_template(&self, ctx: &ReportContext) -> String {
        let mut output = MARKDOWN_TEMPLATE.to_string();

        // Substitute header metadata
        output = output.replace("{{plan_name}}", &ctx.plan.name);
        output = output.replace("{{institution}}", ctx.institution_name());
        output = output.replace("{{degree_name}}", &ctx.degree_name());
        output = output.replace("{{system_type}}", ctx.system_type());
        output = output.replace("{{years}}", &format!("{:.0}", ctx.years()));
        output = output.replace("{{cip_code}}", ctx.cip_code());
        output = output.replace("{{total_credits}}", &format!("{:.1}", ctx.total_credits()));
        output = output.replace("{{course_count}}", &ctx.course_count().to_string());

        // Substitute summary metrics
        output = output.replace(
            "{{total_complexity}}",
            &ctx.summary.total_complexity.to_string(),
        );
        output = output.replace("{{longest_delay}}", &ctx.summary.longest_delay.to_string());
        output = output.replace(
            "{{longest_delay_course}}",
            &ctx.summary.longest_delay_course,
        );
        output = output.replace(
            "{{highest_centrality}}",
            &ctx.summary.highest_centrality.to_string(),
        );
        output = output.replace(
            "{{highest_centrality_course}}",
            &ctx.summary.highest_centrality_course,
        );

        // Generate longest delay path
        let delay_path = if ctx.summary.longest_delay_path.is_empty() {
            "N/A".to_string()
        } else {
            ctx.summary.longest_delay_path.join(" → ")
        };
        output = output.replace("{{longest_delay_path}}", &delay_path);

        // Generate term schedule table
        let schedule_table = Self::generate_schedule_table(ctx);
        output = output.replace("{{term_schedule}}", &schedule_table);

        // Generate course metrics table
        let metrics_table = Self::generate_metrics_table(ctx);
        output = output.replace("{{course_metrics}}", &metrics_table);

        // Generate Mermaid diagram
        let mermaid_diagram = MermaidGenerator::generate_term_diagram(
            ctx.term_plan,
            ctx.dag,
            ctx.school,
            ctx.metrics,
        );
        output = output.replace("{{mermaid_diagram}}", &mermaid_diagram);

        output
    }

    /// Generate the term-by-term schedule table
    fn generate_schedule_table(ctx: &ReportContext) -> String {
        let mut table = String::new();
        let term_label = ctx.term_plan.term_label();

        let _ = writeln!(table, "| {term_label} | Courses | Credits |");
        table.push_str("|---|---|---|\n");

        for term in &ctx.term_plan.terms {
            if term.courses.is_empty() {
                continue;
            }

            let courses_str: Vec<String> = term
                .courses
                .iter()
                .map(|key| {
                    ctx.school
                        .get_course(key)
                        .map_or_else(|| key.clone(), |c| format!("{key} - {}", c.name))
                })
                .collect();

            let _ = writeln!(
                table,
                "| {} | {} | {:.1} |",
                term.number,
                courses_str.join(", "),
                term.total_credits
            );
        }

        // Add unscheduled courses if any
        if !ctx.term_plan.unscheduled.is_empty() {
            let _ = writeln!(
                table,
                "| ⚠️ Unscheduled | {} | - |",
                ctx.term_plan.unscheduled.join(", ")
            );
        }

        table
    }

    /// Generate the course metrics table
    fn generate_metrics_table(ctx: &ReportContext) -> String {
        let mut table = String::new();

        table
            .push_str("| Course | Name | Credits | Complexity | Blocking | Delay | Centrality |\n");
        table.push_str("|---|---|---|---|---|---|---|\n");

        // Sort courses by complexity (descending)
        let mut courses: Vec<_> = ctx.plan.courses.iter().collect();
        courses.sort_by(|a, b| {
            let ma = ctx.metrics.get(*a).map_or(0, |m| m.complexity);
            let mb = ctx.metrics.get(*b).map_or(0, |m| m.complexity);
            mb.cmp(&ma)
        });

        for course_key in courses {
            let course = ctx.school.get_course(course_key);
            let metrics = ctx.metrics.get(course_key);

            let name = course.map_or("-", |c| &c.name);
            let credits = course.map_or(0.0, |c| c.credit_hours);
            let (complexity, blocking, delay, centrality) =
                metrics.map_or((0, 0, 0, 0), CourseMetrics::as_export_tuple);

            let _ = writeln!(
                table,
                "| {course_key} | {name} | {credits:.1} | {complexity} | {blocking} | {delay} | {centrality} |"
            );
        }

        table
    }
}

impl Default for MarkdownReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportGenerator for MarkdownReporter {
    fn generate(&self, ctx: &ReportContext, output_path: &Path) -> Result<(), Box<dyn Error>> {
        let report_content = self.render(ctx)?;
        fs::write(output_path, report_content)?;
        Ok(())
    }

    fn render(&self, ctx: &ReportContext) -> Result<String, Box<dyn Error>> {
        Ok(self.render_template(ctx))
    }
}
