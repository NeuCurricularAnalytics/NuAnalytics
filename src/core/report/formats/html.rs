//! HTML report generator
//!
//! Generates curriculum reports in HTML format with grid-based visualization.
//! The generated HTML is self-contained with embedded CSS and JavaScript.

use crate::core::metrics::CourseMetrics;
use crate::core::report::visualization::{
    spec_from_report_context, CurriculumGraphRenderer, VanillaJsRenderer,
};
use crate::core::report::{ReportContext, ReportGenerator};
use std::error::Error;
use std::fmt::Write;
use std::fs;
use std::path::Path;

/// Embedded HTML report template
const HTML_TEMPLATE: &str = include_str!("../templates/report.html");

/// HTML report generator with interactive visualizations
pub struct HtmlReporter;

impl HtmlReporter {
    /// Create a new HTML reporter
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Render the report using template substitution
    #[allow(clippy::unused_self)]
    fn render_template(&self, ctx: &ReportContext) -> String {
        let mut output = HTML_TEMPLATE.to_string();

        // Substitute header metadata
        output = output.replace("{{plan_name}}", &ctx.plan.name);
        output = output.replace("{{institution}}", ctx.institution_name());
        output = output.replace("{{degree_name}}", &ctx.degree_name());
        output = output.replace("{{system_type}}", ctx.system_type());
        output = output.replace("{{cip_code}}", ctx.cip_code());
        output = output.replace("{{years}}", &format!("{:.0}", ctx.years()));
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

        // Generate term schedule HTML
        let schedule_html = Self::generate_schedule_html(ctx);
        output = output.replace("{{term_schedule}}", &schedule_html);

        // Generate course metrics HTML
        let metrics_html = Self::generate_metrics_html(ctx);
        output = output.replace("{{course_metrics}}", &metrics_html);

        // Generate curriculum graph HTML (term columns + SVG edges + JS)
        let spec = spec_from_report_context(ctx, "main");
        let graph_html = VanillaJsRenderer.render(&spec);
        output = output.replace("{{curriculum_graph}}", &graph_html);

        output
    }

    /// Generate the term-by-term schedule as HTML table rows
    fn generate_schedule_html(ctx: &ReportContext) -> String {
        let mut html = String::new();

        for term in &ctx.term_plan.terms {
            if term.courses.is_empty() {
                continue;
            }

            let courses_html: Vec<String> = term
                .courses
                .iter()
                .map(|key| {
                    let name = ctx.school.get_course(key).map_or(key.as_str(), |c| &c.name);
                    format!("<span class=\"course-badge\">{key}</span> {name}")
                })
                .collect();

            let _ = writeln!(
                html,
                "<tr><td>{}</td><td>{}</td><td>{:.1}</td></tr>",
                term.number,
                courses_html.join("<br>"),
                term.total_credits
            );
        }

        // Add unscheduled courses if any
        if !ctx.term_plan.unscheduled.is_empty() {
            let _ = writeln!(
                html,
                "<tr class=\"unscheduled\"><td>⚠️</td><td>{}</td><td>-</td></tr>",
                ctx.term_plan.unscheduled.join(", ")
            );
        }

        html
    }

    /// Generate the course metrics as HTML table rows
    fn generate_metrics_html(ctx: &ReportContext) -> String {
        let mut html = String::new();

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

            // Add complexity class for color coding
            let complexity_class = match complexity {
                0..=5 => "low",
                6..=15 => "medium",
                _ => "high",
            };

            let _ = writeln!(
                html,
                "<tr class=\"complexity-{complexity_class}\"><td>{course_key}</td><td>{name}</td><td>{credits:.1}</td><td>{complexity}</td><td>{blocking}</td><td>{delay}</td><td>{centrality}</td></tr>"
            );
        }

        html
    }
}

impl Default for HtmlReporter {
    fn default() -> Self {
        Self::new()
    }
}

impl ReportGenerator for HtmlReporter {
    fn generate(&self, ctx: &ReportContext, output_path: &Path) -> Result<(), Box<dyn Error>> {
        let report_content = self.render(ctx)?;
        fs::write(output_path, report_content)?;
        Ok(())
    }

    fn render(&self, ctx: &ReportContext) -> Result<String, Box<dyn Error>> {
        Ok(self.render_template(ctx))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metrics::CourseMetrics;
    use crate::core::metrics_export::CurriculumSummary;
    use crate::core::models::{Course, Degree, Plan, School, DAG};
    use crate::core::report::term_scheduler::TermPlan;
    use std::collections::HashMap;

    fn create_test_context() -> (
        School,
        Plan,
        Degree,
        HashMap<String, CourseMetrics>,
        CurriculumSummary,
        DAG,
        TermPlan,
    ) {
        let mut school = School::new("Test University".to_string());

        let cs101 = Course::new(
            "Intro to CS".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );
        let mut cs201 = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "201".to_string(),
            4.0,
        );
        cs201.add_prerequisite("CS101".to_string());

        school.add_course(cs101);
        school.add_course(cs201);

        let degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        );

        let mut plan = Plan::new("CS Plan".to_string(), degree.degree_id());
        plan.add_course("CS101".to_string());
        plan.add_course("CS201".to_string());

        let mut metrics = HashMap::new();
        metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                complexity: 3,
                blocking: 1,
                delay: 1,
                centrality: 1,
                chain_length: 1,
            },
        );
        metrics.insert(
            "CS201".to_string(),
            CourseMetrics {
                complexity: 5,
                blocking: 0,
                delay: 2,
                centrality: 1,
                chain_length: 2,
            },
        );

        let summary = CurriculumSummary {
            total_complexity: 8,
            highest_centrality: 1,
            highest_centrality_course: "CS101".to_string(),
            longest_delay: 2,
            longest_delay_course: "CS201".to_string(),
            longest_delay_path: vec!["CS101".to_string(), "CS201".to_string()],
        };

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());
        dag.add_course("CS201".to_string());
        dag.add_prerequisite("CS201".to_string(), "CS101");

        let mut term_plan = TermPlan::new(8, false, 15.0);
        term_plan.terms[0].add_course("CS101".to_string(), 3.0);
        term_plan.terms[1].add_course("CS201".to_string(), 4.0);

        (school, plan, degree, metrics, summary, dag, term_plan)
    }

    #[test]
    fn test_html_reporter_new() {
        let reporter = HtmlReporter::new();
        // Verifies construction works - use in actual render test
        let (school, plan, degree, metrics, summary, dag, term_plan) = create_test_context();
        let ctx = ReportContext::new(
            &school,
            &plan,
            Some(&degree),
            &metrics,
            &summary,
            &dag,
            &term_plan,
        );
        let result = reporter.render(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_html_reporter_default() {
        let reporter = HtmlReporter;
        let (school, plan, degree, metrics, summary, dag, term_plan) = create_test_context();
        let ctx = ReportContext::new(
            &school,
            &plan,
            Some(&degree),
            &metrics,
            &summary,
            &dag,
            &term_plan,
        );
        let result = reporter.render(&ctx);
        assert!(result.is_ok());
    }

    #[test]
    fn test_render_produces_html() {
        let (school, plan, degree, metrics, summary, dag, term_plan) = create_test_context();

        let ctx = ReportContext::new(
            &school,
            &plan,
            Some(&degree),
            &metrics,
            &summary,
            &dag,
            &term_plan,
        );

        let reporter = HtmlReporter::new();
        let html = reporter.render(&ctx).unwrap();

        // Verify key elements are present
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Test University"));
        assert!(html.contains("CS Plan"));
        assert!(html.contains("CS101"));
        assert!(html.contains("CS201"));
    }

    #[test]
    fn test_render_contains_graph_components() {
        let (school, plan, degree, metrics, summary, dag, term_plan) = create_test_context();
        let ctx = ReportContext::new(
            &school,
            &plan,
            Some(&degree),
            &metrics,
            &summary,
            &dag,
            &term_plan,
        );
        let reporter = HtmlReporter::new();
        let html = reporter.render(&ctx).unwrap();
        assert!(html.contains("data-course-id=\"CS101\""));
        assert!(html.contains("data-course-id=\"CS201\""));
        assert!(html.contains("nuGraphs.register"));
        assert!(html.contains("\"CS101\""));
        assert!(html.contains("\"CS201\""));
    }
}
