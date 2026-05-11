//! Degree analysis report generation
//!
//! Generates HTML reports for degree analysis with box plots, per-course
//! statistics, and selected plan details.

use crate::core::degree::plan_selector::{PlanCategory, ScoredPlan, SelectedPlans};
use crate::core::models::{Degree, School, DAG};
use crate::core::report::visualization::renderer::escape_html;
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

        // Unified tabbed plans section — one tab per selected plan
        // (named plans + each random sample).
        html = html.replace("{{plan_tabs_section}}", &Self::render_plan_tabs(ctx));

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
                    escape_html(course_name),
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

    /// Render the unified tabbed plans section.
    ///
    /// Emits a tab strip plus one panel per selected plan. The first plan's
    /// curriculum graph carries the shared `GRAPH_VANILLA_JS` library inline;
    /// subsequent panels re-use the already-loaded library via
    /// `render_without_library`, which drops ~20 KB per fragment.
    fn render_plan_tabs(ctx: &DegreeReportContext) -> String {
        let tabbed = collect_tabbed_plans(ctx.selected_plans);
        if tabbed.is_empty() {
            return String::from(
                "<p class=\"plan-tabs-empty\">No selected plans available for this degree.</p>",
            );
        }

        let mut html = String::new();
        let _ = writeln!(html, "<div class=\"nu-plan-tabs\">");

        // Tab strip — first button is the active default.
        let _ = writeln!(html, "<div class=\"tab-strip\" role=\"tablist\">");
        for (idx, entry) in tabbed.iter().enumerate() {
            let active_class = if idx == 0 { " tab-btn--active" } else { "" };
            let aria_selected = if idx == 0 { "true" } else { "false" };
            let _ = writeln!(
                html,
                "<button type=\"button\" class=\"tab-btn{active_class}\" role=\"tab\" aria-selected=\"{aria_selected}\" data-tab-target=\"{}\">{}</button>",
                entry.graph_id,
                escape_html(&entry.tab_label)
            );
        }
        let _ = writeln!(html, "</div>"); // end tab-strip

        // Panels.
        for (idx, entry) in tabbed.iter().enumerate() {
            let active_class = if idx == 0 { " tab-panel--active" } else { "" };
            let include_library = idx == 0;
            let _ = writeln!(
                html,
                "<div class=\"tab-panel{active_class}\" role=\"tabpanel\" data-tab-id=\"{}\">",
                entry.graph_id
            );
            let _ = write!(
                html,
                "{}",
                Self::render_plan_panel_body(ctx, entry.plan, &entry.graph_id, include_library)
            );
            let _ = writeln!(html, "</div>"); // end tab-panel
        }

        let _ = writeln!(html, "</div>"); // end nu-plan-tabs
        html
    }

    /// Render one plan's panel contents: overview stats, critical path,
    /// curriculum graph, and term schedule.
    ///
    /// `include_library` controls whether the graph fragment inlines the
    /// shared `GRAPH_VANILLA_JS` library — set true only for the first panel
    /// on the page; subsequent panels reuse the already-loaded namespace.
    fn render_plan_panel_body(
        ctx: &DegreeReportContext,
        plan: &ScoredPlan,
        graph_id: &str,
        include_library: bool,
    ) -> String {
        let mut html = String::new();

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

        // Curriculum graph
        let _ = write!(
            html,
            "{}",
            Self::render_curriculum_graph(ctx, plan, graph_id, include_library)
        );

        // Term schedule (grid of per-term cards)
        let _ = write!(html, "{}", Self::render_term_schedule_cards(ctx, plan));

        html
    }

    /// Render the term schedule as a grid of per-term cards.
    ///
    /// Each non-empty term becomes a card with a header (number + total
    /// credits) and a list of its courses, color-coded by whether the
    /// course belongs to one of the degree's `major_subjects`. The grid
    /// wraps as the viewport narrows so the layout still scans easily in
    /// print and on small screens.
    fn render_term_schedule_cards(ctx: &DegreeReportContext, plan: &ScoredPlan) -> String {
        let mut html = String::new();
        let _ = writeln!(html, "<div class=\"term-schedule\">");
        let _ = writeln!(html, "<h4>Term Schedule</h4>");

        // Show the legend only when the degree declares major subjects;
        // without it every course gets the same (neutral) accent so the
        // legend would be misleading.
        let has_major_subjects = !ctx.major_subjects().is_empty();
        if has_major_subjects {
            let _ = writeln!(
                html,
                "<div class=\"term-courses-legend\">\n\
<span><span class=\"legend-swatch legend-swatch--major\"></span>Major</span>\n\
<span><span class=\"legend-swatch legend-swatch--other\"></span>Supporting / gen-ed / elective</span>\n\
</div>"
            );
        }

        let _ = writeln!(html, "<div class=\"term-grid\">");
        for term in &plan.schedule.terms {
            if term.courses.is_empty() {
                continue;
            }
            let _ = writeln!(html, "<div class=\"term-card\">");
            let _ = writeln!(
                html,
                "<div class=\"term-card-header\">\n\
<span>Term {}</span>\n\
<span class=\"term-credits\">{:.1} cr</span>\n\
</div>",
                term.number, term.total_credits
            );
            let _ = writeln!(html, "<ul class=\"term-courses\">");
            for key in &term.courses {
                let name = ctx.school.get_course(key).map_or(key.as_str(), |c| &c.name);
                let credits = ctx.school.get_course(key).map_or(0.0, |c| c.credit_hours);
                let extra_class = if has_major_subjects && ctx.is_major_course(key) {
                    " term-course--major"
                } else {
                    ""
                };
                let credits_cell = if credits > 0.0 {
                    format!("<span class=\"course-credits\">{credits:.1}</span>")
                } else {
                    "<span class=\"course-credits\"></span>".to_string()
                };
                let _ = writeln!(
                    html,
                    "<li class=\"term-course{extra_class}\">\n\
<span class=\"course-id\">{}</span>\n\
<span class=\"course-name\" title=\"{}\">{}</span>\n\
{credits_cell}\n\
</li>",
                    escape_html(key),
                    escape_html(name),
                    escape_html(name),
                );
            }
            let _ = writeln!(html, "</ul>");
            let _ = writeln!(html, "</div>"); // end term-card
        }
        let _ = writeln!(html, "</div>"); // end term-grid

        let _ = writeln!(html, "</div>"); // end term-schedule
        html
    }

    /// Render the curriculum graph fragment for a plan.
    ///
    /// `include_library` controls whether the inline `<script>` block carries
    /// the shared `GRAPH_VANILLA_JS` namespace — pass `true` for the first
    /// graph on the page and `false` for the rest so the ~20 KB library is
    /// loaded once.
    fn render_curriculum_graph(
        ctx: &DegreeReportContext,
        plan: &ScoredPlan,
        plan_id: &str,
        include_library: bool,
    ) -> String {
        use crate::core::report::visualization::{
            spec_from_scored_plan, CurriculumGraphRenderer, VanillaJsRenderer,
        };
        let spec = spec_from_scored_plan(
            ctx.school,
            ctx.equivalences,
            plan,
            Some(ctx.aggregator),
            plan_id,
        );
        if include_library {
            VanillaJsRenderer.render(&spec)
        } else {
            VanillaJsRenderer.render_without_library(&spec)
        }
    }
}

/// One entry in the tabbed plans section: the tab label users see, the DOM
/// id that ties the tab button to its panel, and the `ScoredPlan` whose
/// graph + schedule populate the panel body.
struct TabbedPlan<'a> {
    /// Visible label shown on the tab button.
    tab_label: String,
    /// Filename-safe identifier used for `data-tab-target` / `data-tab-id` and
    /// as the graph's DOM id.
    graph_id: String,
    /// The plan whose body fills the panel.
    plan: &'a ScoredPlan,
}

/// Flatten [`SelectedPlans`] into the ordered list of tabbed entries the
/// report renders. Named plans come first (Shortest, Longest,
/// Calculus-Ready Shortest — each only if present) followed by the random
/// samples labelled `"Sample 1"`, `"Sample 2"`, … in their vec order.
fn collect_tabbed_plans(selected: &SelectedPlans) -> Vec<TabbedPlan<'_>> {
    let mut entries: Vec<TabbedPlan<'_>> = Vec::new();

    let named = [
        (PlanCategory::Shortest, selected.shortest.as_ref()),
        (PlanCategory::Longest, selected.longest.as_ref()),
        (
            PlanCategory::CalcReadyShortest,
            selected.calc_ready_shortest.as_ref(),
        ),
    ];
    for (category, plan_opt) in named {
        if let Some(plan) = plan_opt {
            entries.push(TabbedPlan {
                tab_label: category.display_name().to_string(),
                graph_id: category.file_name().to_string(),
                plan,
            });
        }
    }

    for (idx, plan) in selected.random_samples.iter().enumerate() {
        let n = idx + 1;
        entries.push(TabbedPlan {
            tab_label: format!("Sample {n}"),
            graph_id: format!("sample-{n}"),
            plan,
        });
    }

    entries
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

    /// Build a `ScoredPlan` with one course and the given headline stats.
    /// Helper for assembling test fixtures with multiple plans.
    fn make_test_scored_plan(course: &str, terms: usize, complexity: usize) -> ScoredPlan {
        ScoredPlan {
            variant: PlanVariant::from_parts(vec![course.to_string()], HashMap::new(), 3.0),
            score: PlanScore {
                terms_required: terms,
                total_complexity: complexity,
                longest_delay: 1,
                longest_delay_chain: Vec::new(),
                is_calc_ready: false,
            },
            schedule: TermPlan::new(terms, false, 15.0),
            course_metrics: HashMap::new(),
        }
    }

    #[test]
    fn test_collect_tabbed_plans_orders_named_then_samples() {
        // Two named plans + two random samples = four entries in the order
        // Shortest, Longest, Sample 1, Sample 2. Sample labels are 1-indexed.
        let selected = SelectedPlans {
            shortest: Some(make_test_scored_plan("CS1000", 8, 50)),
            longest: Some(make_test_scored_plan("CS1000", 12, 80)),
            calc_ready_shortest: None,
            random_samples: vec![
                make_test_scored_plan("CS1000", 9, 60),
                make_test_scored_plan("CS1000", 10, 65),
            ],
            total_plans_seen: 100,
        };
        let entries = collect_tabbed_plans(&selected);
        let labels: Vec<&str> = entries.iter().map(|e| e.tab_label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["Shortest Path", "Longest Path", "Sample 1", "Sample 2"]
        );
        let ids: Vec<&str> = entries.iter().map(|e| e.graph_id.as_str()).collect();
        assert_eq!(ids, vec!["shortest", "longest", "sample-1", "sample-2"]);
    }

    #[test]
    fn test_collect_tabbed_plans_skips_absent_named_plans() {
        let selected = SelectedPlans {
            shortest: None,
            longest: None,
            calc_ready_shortest: Some(make_test_scored_plan("CS1000", 8, 50)),
            random_samples: vec![make_test_scored_plan("CS1000", 9, 60)],
            total_plans_seen: 1,
        };
        let entries = collect_tabbed_plans(&selected);
        let labels: Vec<&str> = entries.iter().map(|e| e.tab_label.as_str()).collect();
        assert_eq!(labels, vec!["Calculus-Ready Shortest", "Sample 1"]);
    }

    #[test]
    fn test_report_emits_one_tab_per_selected_plan() {
        // Two named plans + three random samples → 5 tab buttons + 5 panels.
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let dag = create_test_dag();
        let equivalences = std::collections::HashMap::new();
        let selected = SelectedPlans {
            shortest: Some(make_test_scored_plan("CS1000", 8, 50)),
            longest: Some(make_test_scored_plan("CS1000", 12, 80)),
            calc_ready_shortest: None,
            random_samples: vec![
                make_test_scored_plan("CS1000", 9, 60),
                make_test_scored_plan("CS1000", 10, 65),
                make_test_scored_plan("CS1000", 11, 70),
            ],
            total_plans_seen: 200,
        };
        let ctx = DegreeReportContext::new(
            &school,
            &degree,
            &aggregator,
            &selected,
            &dag,
            &equivalences,
        );
        let html = DegreeReportGenerator::new().render(&ctx).unwrap();

        assert_eq!(
            html.matches("class=\"tab-btn").count(),
            5,
            "expected 5 tab-btn occurrences (one per selected plan)"
        );
        assert_eq!(
            html.matches("class=\"tab-panel").count(),
            5,
            "expected 5 tab-panel occurrences (one per selected plan)"
        );
        // Every label appears at least once (we don't count occurrences here
        // because the labels may also leak into aria attributes or panel
        // sub-content in future edits).
        assert!(html.contains(">Shortest Path<"));
        assert!(html.contains(">Longest Path<"));
        assert!(html.contains(">Sample 1<"));
        assert!(html.contains(">Sample 2<"));
        assert!(html.contains(">Sample 3<"));
        // First button + first panel must carry the active class.
        assert!(html.contains("tab-btn tab-btn--active"));
        assert!(html.contains("tab-panel tab-panel--active"));
        // Library is included once (first panel) and dropped on later ones.
        // The GRAPH_VANILLA_JS namespace literal `window.nuGraphs =` appears
        // inside the library only — exactly once across the whole report.
        assert_eq!(
            html.matches("window.nuGraphs =").count(),
            1,
            "GRAPH_VANILLA_JS library must be inlined exactly once across all tabs"
        );
    }

    #[test]
    fn test_report_emits_empty_state_when_no_selected_plans() {
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let dag = create_test_dag();
        let equivalences = std::collections::HashMap::new();
        let selected = SelectedPlans {
            shortest: None,
            longest: None,
            calc_ready_shortest: None,
            random_samples: vec![],
            total_plans_seen: 0,
        };
        let ctx = DegreeReportContext::new(
            &school,
            &degree,
            &aggregator,
            &selected,
            &dag,
            &equivalences,
        );
        let html = DegreeReportGenerator::new().render(&ctx).unwrap();
        assert!(html.contains("plan-tabs-empty"));
        assert!(!html.contains("class=\"tab-btn"));
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
