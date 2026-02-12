//! Degree analysis report generation
//!
//! Generates HTML reports for degree analysis with box plots, per-course
//! statistics, and selected plan details.

use crate::core::degree::plan_selector::{PlanCategory, ScoredPlan, SelectedPlans};
use crate::core::models::{Degree, School};
use crate::core::statistics::aggregator::{AggregatedDegreeStats, MetricsAggregator};
use crate::core::statistics::box_plot::{BoxPlotData, BoxPlotGenerator};
use std::error::Error;
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::Path;

/// Degree report template embedded at compile time
const DEGREE_REPORT_TEMPLATE: &str = include_str!("templates/degree_report.html");

/// Context for degree report generation
#[derive(Debug)]
pub struct DegreeReportContext<'a> {
    /// School containing course catalog
    pub school: &'a School,
    /// Degree being analyzed
    pub degree: &'a Degree,
    /// Aggregated statistics from all plans
    pub aggregator: &'a MetricsAggregator,
    /// Selected special plans
    pub selected_plans: &'a SelectedPlans,
}

impl<'a> DegreeReportContext<'a> {
    /// Create a new degree report context
    #[must_use]
    pub const fn new(
        school: &'a School,
        degree: &'a Degree,
        aggregator: &'a MetricsAggregator,
        selected_plans: &'a SelectedPlans,
    ) -> Self {
        Self {
            school,
            degree,
            aggregator,
            selected_plans,
        }
    }
}

/// Generates HTML reports for degree analysis
pub struct DegreeReportGenerator;

impl DegreeReportGenerator {
    /// Create a new degree report generator
    #[must_use]
    pub const fn new() -> Self {
        Self
    }

    /// Generate HTML report to a file
    ///
    /// # Errors
    /// Returns an error if file writing fails
    pub fn generate(
        &self,
        ctx: &DegreeReportContext,
        output_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        let html = self.render(ctx)?;
        fs::write(output_path, html)?;
        Ok(())
    }

    /// Render HTML report as a string
    ///
    /// # Errors
    /// Returns an error if rendering fails
    pub fn render(&self, ctx: &DegreeReportContext) -> Result<String, Box<dyn Error>> {
        let mut html = DEGREE_REPORT_TEMPLATE.to_string();

        // Basic metadata
        html = html.replace("{{degree_name}}", &ctx.degree.name);
        html = html.replace("{{degree_type}}", &ctx.degree.degree_type);
        html = html.replace("{{degree_id}}", &ctx.degree.degree_id());
        html = html.replace("{{institution}}", &ctx.school.name);
        html = html.replace(
            "{{cip_code}}",
            ctx.degree.cip_code.as_deref().unwrap_or("N/A"),
        );
        html = html.replace("{{system_type}}", &ctx.degree.system_type);

        // Plan counts
        html = html.replace(
            "{{total_plans}}",
            &ctx.selected_plans.total_plans_seen.to_string(),
        );
        html = html.replace(
            "{{selected_plans_count}}",
            &ctx.selected_plans.total_count().to_string(),
        );

        // Degree statistics
        let degree_stats = ctx.aggregator.degree_stats();
        html = html.replace(
            "{{stats_section}}",
            &Self::render_degree_stats(&degree_stats),
        );

        // Box plots
        html = html.replace("{{box_plots}}", &Self::render_box_plots(&degree_stats));

        // Course statistics table
        html = html.replace(
            "{{course_stats_table}}",
            &Self::render_course_stats_table(ctx),
        );

        // Selected plans section
        html = html.replace(
            "{{selected_plans_section}}",
            &Self::render_selected_plans(ctx),
        );

        Ok(html)
    }

    /// Render degree-level statistics cards
    fn render_degree_stats(stats: &AggregatedDegreeStats) -> String {
        let mut html = String::new();

        // Plan count
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Plans Analyzed</div>\n\
    <div class=\"value\">{}</div>\n\
</div>",
            stats.plan_count
        );

        // Complexity stats
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Complexity (Median)</div>\n\
    <div class=\"value\">{:.1}</div>\n\
    <div class=\"detail\">Range: {:.1} - {:.1}</div>\n\
</div>",
            stats.total_complexity.median, stats.total_complexity.min, stats.total_complexity.max
        );

        // Delay stats
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Longest Delay (Median)</div>\n\
    <div class=\"value\">{:.1}</div>\n\
    <div class=\"detail\">Range: {:.1} - {:.1}</div>\n\
</div>",
            stats.longest_delay.median, stats.longest_delay.min, stats.longest_delay.max
        );

        // Credits stats
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Credits (Mean)</div>\n\
    <div class=\"value\">{:.1}</div>\n\
    <div class=\"detail\">Range: {:.1} - {:.1}</div>\n\
</div>",
            stats.total_credits.mean, stats.total_credits.min, stats.total_credits.max
        );

        html
    }

    /// Render SVG box plots for degree metrics
    fn render_box_plots(stats: &AggregatedDegreeStats) -> String {
        let mut html = String::new();
        let generator = BoxPlotGenerator::new();

        // Complexity box plot
        let complexity_data =
            BoxPlotData::from_metric_stats("Degree Complexity", &stats.total_complexity);
        let complexity_svg =
            generator.generate_single("Degree Complexity Distribution", &complexity_data);
        let _ = writeln!(
            html,
            "<div class=\"box-plot-container\">\n\
    <h3>Degree Complexity</h3>\n\
    {complexity_svg}\n\
    <div class=\"box-plot-legend\">\n\
        <span><span class=\"legend-box\"></span> Q1-Q3</span>\n\
        <span><span class=\"legend-median\"></span> Median</span>\n\
        <span><span class=\"legend-mean\"></span> Mean</span>\n\
    </div>\n\
</div>"
        );

        // Delay box plot
        let delay_data = BoxPlotData::from_metric_stats("Longest Delay", &stats.longest_delay);
        let delay_svg = generator.generate_single("Longest Delay Factor Distribution", &delay_data);
        let _ = writeln!(
            html,
            "<div class=\"box-plot-container\">\n\
    <h3>Longest Delay Factor</h3>\n\
    {delay_svg}\n\
    <div class=\"box-plot-legend\">\n\
        <span><span class=\"legend-box\"></span> Q1-Q3</span>\n\
        <span><span class=\"legend-median\"></span> Median</span>\n\
        <span><span class=\"legend-mean\"></span> Mean</span>\n\
    </div>\n\
</div>"
        );

        html
    }

    /// Render per-course statistics table
    fn render_course_stats_table(ctx: &DegreeReportContext) -> String {
        let mut html = String::new();

        // Table header
        let _ = writeln!(
            html,
            "<table class=\"metrics-table\">\n\
<thead>\n\
    <tr>\n\
        <th>Course</th>\n\
        <th>Plans</th>\n\
        <th>Complexity</th>\n\
        <th>Centrality</th>\n\
        <th>Delay</th>\n\
        <th>Blocking</th>\n\
    </tr>\n\
    <tr class=\"sub-header\">\n\
        <th></th>\n\
        <th></th>\n\
        <th>Med / Min / Max</th>\n\
        <th>Med / Min / Max</th>\n\
        <th>Med / Min / Max</th>\n\
        <th>Med / Min / Max</th>\n\
    </tr>\n\
</thead>\n\
<tbody>"
        );

        // Get course IDs and sort
        let mut course_ids = ctx.aggregator.course_ids();
        course_ids.sort();

        for course_id in course_ids {
            if let Some(stats) = ctx.aggregator.course_stats(&course_id) {
                let course_name = ctx
                    .school
                    .get_course(&course_id)
                    .map_or(&course_id, |c| &c.name);

                let _ = writeln!(
                    html,
                    "<tr>\n\
    <td><strong>{}</strong><br><small>{}</small></td>\n\
    <td>{}</td>\n\
    <td>{:.1} / {:.1} / {:.1}</td>\n\
    <td>{:.1} / {:.1} / {:.1}</td>\n\
    <td>{:.1} / {:.1} / {:.1}</td>\n\
    <td>{:.1} / {:.1} / {:.1}</td>\n\
</tr>",
                    course_id,
                    Self::escape_html(course_name),
                    stats.plan_count,
                    stats.complexity.median,
                    stats.complexity.min,
                    stats.complexity.max,
                    stats.centrality.median,
                    stats.centrality.min,
                    stats.centrality.max,
                    stats.delay.median,
                    stats.delay.min,
                    stats.delay.max,
                    stats.blocking.median,
                    stats.blocking.min,
                    stats.blocking.max,
                );
            }
        }

        let _ = writeln!(html, "</tbody></table>");
        html
    }

    /// Render selected plans section
    fn render_selected_plans(ctx: &DegreeReportContext) -> String {
        let mut html = String::new();

        for (category, plan) in ctx.selected_plans.iter() {
            let plan_details = Self::render_plan_details(plan, category);
            let _ = writeln!(
                html,
                "<div class=\"plan-card\">\n\
    <h3>{}</h3>\n\
    {plan_details}\n\
</div>",
                category.display_name()
            );
        }

        html
    }

    /// Render details for a single plan
    fn render_plan_details(plan: &ScoredPlan, category: PlanCategory) -> String {
        let mut html = String::new();

        // Summary stats
        let _ = writeln!(
            html,
            "<div class=\"plan-stats\">\n\
    <div class=\"plan-stat\"><span class=\"label\">Terms:</span> <span class=\"value\">{}</span></div>\n\
    <div class=\"plan-stat\"><span class=\"label\">Complexity:</span> <span class=\"value\">{}</span></div>\n\
    <div class=\"plan-stat\"><span class=\"label\">Longest Delay:</span> <span class=\"value\">{}</span></div>\n\
</div>",
            plan.score.terms_required, plan.score.total_complexity, plan.score.longest_delay
        );

        // Course list for non-random samples (keep random samples compact)
        if category != PlanCategory::RandomSample {
            let _ = writeln!(html, "<details><summary>View Courses</summary><ul>");
            for course in &plan.variant.courses {
                let _ = writeln!(html, "<li>{course}</li>");
            }
            let _ = writeln!(html, "</ul></details>");
        }

        html
    }

    /// Escape HTML special characters
    fn escape_html(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
            .replace('"', "&quot;")
    }
}

impl Default for DegreeReportGenerator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::degree::plan_selector::{PlanScore, ScoredPlan};
    use crate::core::degree::plan_variant::PlanVariant;
    use crate::core::report::term_scheduler::TermPlan;
    use crate::core::statistics::aggregator::AggregatorConfig;
    use std::collections::HashMap;

    fn create_test_school() -> School {
        School::new("Test University".to_string())
    }

    fn create_test_degree() -> Degree {
        Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        )
    }

    fn create_test_aggregator() -> MetricsAggregator {
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
        let mut metrics = HashMap::new();
        metrics.insert(
            "CS1000".to_string(),
            crate::core::metrics::CourseMetrics {
                complexity: 10,
                centrality: 5,
                delay: 3,
                blocking: 2,
            },
        );
        agg.add_plan(&metrics, 120.0);
        agg
    }

    fn create_test_selected_plans() -> SelectedPlans {
        SelectedPlans {
            shortest: Some(ScoredPlan {
                variant: PlanVariant::from_parts(vec!["CS1000".to_string()], HashMap::new(), 3.0),
                score: PlanScore {
                    terms_required: 8,
                    total_complexity: 100,
                    longest_delay: 5,
                    longest_delay_chain: Vec::new(),
                    is_calc_ready: false,
                },
                schedule: TermPlan::new(8, false, 15.0),
                course_metrics: HashMap::new(),
            }),
            longest: None,
            calc_ready_shortest: None,
            random_samples: vec![],
            total_plans_seen: 10,
        }
    }

    #[test]
    fn test_generator_new() {
        let gen = DegreeReportGenerator::new();
        // Just verify construction
        let _ = gen;
    }

    #[test]
    fn test_generator_default() {
        let gen = DegreeReportGenerator;
        let _ = gen;
    }

    #[test]
    fn test_render_produces_html() {
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected_plans();

        let ctx = DegreeReportContext::new(&school, &degree, &aggregator, &selected);
        let gen = DegreeReportGenerator::new();

        let result = gen.render(&ctx);
        assert!(result.is_ok());

        let html = result.unwrap();
        assert!(html.contains("<!DOCTYPE html>"));
        assert!(html.contains("Computer Science"));
        assert!(html.contains("Test University"));
    }

    #[test]
    fn test_escape_html() {
        assert_eq!(
            DegreeReportGenerator::escape_html("Test & <Data>"),
            "Test &amp; &lt;Data&gt;"
        );
    }

    #[test]
    fn test_context_creation() {
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected_plans();

        let ctx = DegreeReportContext::new(&school, &degree, &aggregator, &selected);
        assert_eq!(ctx.degree.name, "Computer Science");
        assert_eq!(ctx.school.name, "Test University");
    }
}
