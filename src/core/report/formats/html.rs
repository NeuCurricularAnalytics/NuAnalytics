//! HTML report generator
//!
//! Generates curriculum reports in HTML format with interactive vis.js graphs.
//! The generated HTML is self-contained with embedded CSS and JavaScript.

use crate::core::metrics::CourseMetrics;
use crate::core::report::{ReportContext, ReportGenerator};
use std::collections::HashMap;
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

        // Generate vis.js graph data
        let (nodes, edges) = Self::generate_graph_data(ctx);
        output = output.replace("{{graph_nodes}}", &nodes);
        output = output.replace("{{graph_edges}}", &edges);

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

    /// Generate vis.js node and edge data as JSON arrays
    /// Nodes are positioned by term (x-axis) with courses stacked vertically within each term
    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
    fn generate_graph_data(ctx: &ReportContext) -> (String, String) {
        let mut nodes = Vec::new();
        let mut edges = Vec::new();

        // Track y-positions within each term (reset per term)
        let mut term_y_count: HashMap<usize, usize> = HashMap::new();

        // Horizontal spacing per term
        let x_spacing = 250;
        // Vertical spacing per course within a term
        let y_spacing = 100;

        // Create nodes grouped by term
        for term in &ctx.term_plan.terms {
            let term_num = term.number;
            let x = (term_num as i32) * x_spacing;

            for course_key in &term.courses {
                let course = ctx.school.get_course(course_key);
                let metrics = ctx.metrics.get(course_key);

                // Use short label for readability
                let label = course.map_or(course_key.as_str(), |c| {
                    if c.name.len() > 20 {
                        course_key.as_str()
                    } else {
                        &c.name
                    }
                });

                let complexity = metrics.map_or(0, |m| m.complexity);

                // Calculate y position within this term
                let y_idx = term_y_count.entry(term_num).or_insert(0);
                let y = (*y_idx as i32) * y_spacing;
                *term_y_count.get_mut(&term_num).unwrap() += 1;

                // Color based on complexity
                let color = match complexity {
                    0..=5 => "#4CAF50",  // Green - low complexity
                    6..=15 => "#FF9800", // Orange - medium
                    _ => "#F44336",      // Red - high
                };

                // Add term number to title for hover info
                let title = format!("{course_key} - Term {term_num}\\nComplexity: {complexity}");

                nodes.push(format!(
                    "{{ id: '{course_key}', label: '{course_key}\\n{label}\\nC:{complexity}', x: {x}, y: {y}, color: {{ background: '{color}' }}, title: '{title}', level: {term_num} }}"
                ));
            }
        }

        // Handle unscheduled courses (if any)
        if !ctx.term_plan.unscheduled.is_empty() {
            let unscheduled_x = ((ctx.term_plan.terms.len() + 1) as i32) * x_spacing;
            for (idx, course_key) in ctx.term_plan.unscheduled.iter().enumerate() {
                let y = (idx as i32) * y_spacing;
                nodes.push(format!(
                    "{{ id: '{course_key}', label: '{course_key}\\n⚠️ Unscheduled', x: {unscheduled_x}, y: {y}, color: {{ background: '#9E9E9E' }}, level: 0 }}"
                ));
            }
        }

        // Create edges for prerequisites
        for (course, prereqs) in &ctx.dag.dependencies {
            if !ctx.plan.courses.contains(course) {
                continue;
            }
            for prereq in prereqs {
                if !ctx.plan.courses.contains(prereq) {
                    continue;
                }
                edges.push(format!(
                    "{{ from: '{prereq}', to: '{course}', arrows: 'to' }}"
                ));
            }
        }

        // Create edges for corequisites (dashed)
        for (course, coreqs) in &ctx.dag.corequisites {
            if !ctx.plan.courses.contains(course) {
                continue;
            }
            for coreq in coreqs {
                if !ctx.plan.courses.contains(coreq) {
                    continue;
                }
                edges.push(format!(
                    "{{ from: '{coreq}', to: '{course}', arrows: 'to', dashes: true, color: {{ color: '#2196F3' }} }}"
                ));
            }
        }

        (
            format!("[{}]", nodes.join(",\n        ")),
            format!("[{}]", edges.join(",\n        ")),
        )
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
