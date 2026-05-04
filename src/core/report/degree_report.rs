//! Degree analysis report generation
//!
//! Generates HTML reports for degree analysis with box plots, per-course
//! statistics, and selected plan details.

use crate::core::degree::plan_selector::{PlanCategory, ScoredPlan, SelectedPlans};
use crate::core::models::{Degree, School, DAG};
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
    /// DAG for prerequisite/corequisite edges
    pub dag: &'a DAG,
    /// Map from course key to equivalent courses
    pub equivalences: &'a std::collections::HashMap<String, std::collections::HashSet<String>>,
}

impl<'a> DegreeReportContext<'a> {
    /// Create a new degree report context
    #[must_use]
    pub const fn new(
        school: &'a School,
        degree: &'a Degree,
        aggregator: &'a MetricsAggregator,
        selected_plans: &'a SelectedPlans,
        dag: &'a DAG,
        equivalences: &'a std::collections::HashMap<String, std::collections::HashSet<String>>,
    ) -> Self {
        Self {
            school,
            degree,
            aggregator,
            selected_plans,
            dag,
            equivalences,
        }
    }

    /// Get major subject codes from the degree configuration
    ///
    /// Returns the list of subject codes that are considered part of the major.
    fn major_subjects(&self) -> Vec<&str> {
        self.degree
            .major_subjects
            .as_ref()
            .map_or_else(Vec::new, |subjects| {
                subjects.iter().map(String::as_str).collect()
            })
    }

    /// Check if a course ID belongs to a major subject
    fn is_major_course(&self, course_id: &str) -> bool {
        let subjects = self.major_subjects();
        if subjects.is_empty() {
            return false;
        }
        subjects.iter().any(|subj| course_id.starts_with(subj))
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

        // Special plans section (shortest, longest, calc-ready with full details)
        html = html.replace(
            "{{special_plans_section}}",
            &Self::render_special_plans(ctx),
        );

        // Random samples section (compact view)
        html = html.replace(
            "{{random_samples_section}}",
            &Self::render_random_samples(ctx),
        );

        // Major subjects JSON for JavaScript sorting
        html = html.replace(
            "{{major_subjects_json}}",
            &Self::render_major_subjects_json(ctx),
        );

        Ok(html)
    }

    /// Render major subjects as JSON array for JavaScript
    fn render_major_subjects_json(ctx: &DegreeReportContext) -> String {
        let subjects = ctx.major_subjects();
        if subjects.is_empty() {
            return "[]".to_string();
        }
        let quoted: Vec<String> = subjects.iter().map(|s| format!("\"{s}\"")).collect();
        format!("[{}]", quoted.join(", "))
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

    /// Render per-course statistics table with sortable headers
    fn render_course_stats_table(ctx: &DegreeReportContext) -> String {
        let mut html = String::new();

        // Table header with sortable columns
        let _ = writeln!(
            html,
            "<table class=\"metrics-table\" id=\"course-stats-table\">\n\
<thead>\n\
    <tr>\n\
        <th class=\"sortable\">Course</th>\n\
        <th class=\"sortable\">Plans</th>\n\
        <th class=\"sortable\">Complexity</th>\n\
        <th class=\"sortable\">Centrality</th>\n\
        <th class=\"sortable\">Delay</th>\n\
        <th class=\"sortable\">Blocking</th>\n\
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

        // Get course IDs and sort: major courses first by complexity (descending)
        let mut course_ids = ctx.aggregator.course_ids();
        course_ids.sort_by(|a, b| {
            let a_major = ctx.is_major_course(a);
            let b_major = ctx.is_major_course(b);

            // Major courses come first
            if a_major != b_major {
                return if a_major {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }

            // Within same category, sort by complexity (descending)
            let a_complexity = ctx
                .aggregator
                .course_stats(a)
                .map_or(0.0, |s| s.complexity.median);
            let b_complexity = ctx
                .aggregator
                .course_stats(b)
                .map_or(0.0, |s| s.complexity.median);

            b_complexity
                .partial_cmp(&a_complexity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        for course_id in course_ids {
            if let Some(stats) = ctx.aggregator.course_stats(&course_id) {
                let course_name = ctx
                    .school
                    .get_course(&course_id)
                    .map_or(&course_id, |c| &c.name);

                let row_class = if ctx.is_major_course(&course_id) {
                    " class=\"major-course\""
                } else {
                    ""
                };

                let _ = writeln!(
                    html,
                    "<tr{row_class}>\n\
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

    /// Render special plans section with full details (term schedule, critical path, curriculum graph)
    fn render_special_plans(ctx: &DegreeReportContext) -> String {
        let mut html = String::new();

        let special_plans = [
            (PlanCategory::Shortest, &ctx.selected_plans.shortest),
            (PlanCategory::Longest, &ctx.selected_plans.longest),
            (
                PlanCategory::CalcReadyShortest,
                &ctx.selected_plans.calc_ready_shortest,
            ),
        ];

        for (category, plan_opt) in special_plans {
            if let Some(plan) = plan_opt {
                let _ = write!(
                    html,
                    "{}",
                    Self::render_single_special_plan(ctx, plan, category)
                );
            }
        }

        html
    }

    /// Render one special plan: overview stats, critical path, graph, and term schedule.
    fn render_single_special_plan(
        ctx: &DegreeReportContext,
        plan: &ScoredPlan,
        category: PlanCategory,
    ) -> String {
        let mut html = String::new();
        let plan_id = category.file_name().replace('-', "_");

        let _ = writeln!(html, "<div class=\"special-plan\" id=\"plan-{plan_id}\">");
        let _ = writeln!(html, "<h3>{}</h3>", category.display_name());

        // Overview stats
        let _ = writeln!(html, "<div class=\"plan-overview\">");
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Terms</div>\n\
    <div class=\"value\">{}</div>\n\
</div>",
            plan.score.terms_required
        );
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Complexity</div>\n\
    <div class=\"value\">{}</div>\n\
</div>",
            plan.score.total_complexity
        );
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Longest Delay</div>\n\
    <div class=\"value\">{}</div>\n\
</div>",
            plan.score.longest_delay
        );

        let total_credits: f32 = plan.schedule.terms.iter().map(|t| t.total_credits).sum();
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Total Credits</div>\n\
    <div class=\"value\">{total_credits:.0}</div>\n\
</div>"
        );

        let course_count: usize = plan.schedule.terms.iter().map(|t| t.courses.len()).sum();
        let _ = writeln!(
            html,
            "<div class=\"stat-card\">\n\
    <div class=\"label\">Courses</div>\n\
    <div class=\"value\">{course_count}</div>\n\
</div>"
        );
        let _ = writeln!(html, "</div>"); // end plan-overview

        // Critical path
        if !plan.score.longest_delay_chain.is_empty() {
            let path_str = plan.score.longest_delay_chain.join(" → ");
            let _ = writeln!(
                html,
                "<div class=\"critical-path\"><strong>Critical Path:</strong> {path_str}</div>"
            );
        }

        // Curriculum Graph (includes legend, term columns, SVG overlay, JS)
        let graph_html = Self::render_curriculum_graph(ctx, plan, &plan_id);
        let _ = write!(html, "{graph_html}");

        // Term schedule table
        let _ = write!(html, "{}", Self::render_term_schedule_table(ctx, plan));
        let _ = writeln!(html, "</div>"); // end special-plan

        html
    }

    /// Render the term schedule table for a plan.
    fn render_term_schedule_table(ctx: &DegreeReportContext, plan: &ScoredPlan) -> String {
        let mut html = String::new();
        let _ = writeln!(html, "<div class=\"term-schedule\">");
        let _ = writeln!(html, "<h4>Term Schedule</h4>");
        let _ = writeln!(
            html,
            "<table>\n\
<thead><tr><th>Term</th><th>Courses</th><th>Credits</th></tr></thead>\n\
<tbody>"
        );

        for term in &plan.schedule.terms {
            if term.courses.is_empty() {
                continue;
            }

            let courses_html: Vec<String> = term
                .courses
                .iter()
                .map(|key| {
                    let name = ctx.school.get_course(key).map_or(key.as_str(), |c| &c.name);
                    format!(
                        "<span class=\"course-badge\">{key}</span> {}",
                        Self::escape_html(name)
                    )
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

        let _ = writeln!(html, "</tbody></table>");
        let _ = writeln!(html, "</div>"); // end term-schedule
        html
    }

    /// Render the curriculum graph for a special plan.
    ///
    /// Delegates to [`VanillaJsRenderer`] via [`spec_from_scored_plan`].
    fn render_curriculum_graph(
        ctx: &DegreeReportContext,
        plan: &ScoredPlan,
        plan_id: &str,
    ) -> String {
        use crate::core::report::visualization::{
            spec_from_scored_plan, CurriculumGraphRenderer, VanillaJsRenderer,
        };
        let spec = spec_from_scored_plan(ctx.school, ctx.equivalences, plan, plan_id);
        VanillaJsRenderer.render(&spec)
    }

    /// Render random samples section with compact course lists
    fn render_random_samples(ctx: &DegreeReportContext) -> String {
        let mut html = String::new();

        for (idx, plan) in ctx.selected_plans.random_samples.iter().enumerate() {
            let _ = writeln!(html, "<div class=\"plan-card\">");
            let _ = writeln!(html, "<h3>Random Sample {}</h3>", idx + 1);

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

            // Collapsible course list
            let _ = writeln!(html, "<details><summary>View Courses</summary><ul>");
            for course in &plan.variant.courses {
                let name = ctx
                    .school
                    .get_course(course)
                    .map_or("", |c| c.name.as_str());
                if name.is_empty() {
                    let _ = writeln!(html, "<li>{course}</li>");
                } else {
                    let _ = writeln!(
                        html,
                        "<li><strong>{course}</strong> - {}</li>",
                        Self::escape_html(name)
                    );
                }
            }
            let _ = writeln!(html, "</ul></details>");
            let _ = writeln!(html, "</div>");
        }

        if ctx.selected_plans.random_samples.is_empty() {
            let _ = writeln!(
                html,
                "<p style=\"color: #666; font-style: italic;\">No random samples collected</p>"
            );
        }

        html
    }

    /// Render selected plans section (legacy - kept for compatibility)
    #[allow(dead_code)]
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

    /// Render details for a single plan (legacy - kept for compatibility)
    #[allow(dead_code)]
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

    fn create_test_dag() -> DAG {
        DAG::new()
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
        let dag = create_test_dag();
        let equivalences = std::collections::HashMap::new();

        let ctx = DegreeReportContext::new(
            &school,
            &degree,
            &aggregator,
            &selected,
            &dag,
            &equivalences,
        );
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
        let dag = create_test_dag();
        let equivalences = std::collections::HashMap::new();

        let ctx = DegreeReportContext::new(
            &school,
            &degree,
            &aggregator,
            &selected,
            &dag,
            &equivalences,
        );
        assert_eq!(ctx.degree.name, "Computer Science");
        assert_eq!(ctx.school.name, "Test University");
    }
}
