//! The aggregated statistics a report consumes, decoupled from how they were produced.
//!
//! The HTML report and the curriculum-graph renderer between them ask for exactly three
//! things: the degree-level summary, a per-course summary, and the list of courses that
//! have one. Nothing asks for a statistic that a [`MetricsAggregator`] has not already
//! reduced, so taking the *outputs* here rather than the aggregator itself is what lets a
//! report be built from stored rows as well as from a live analysis run.
//!
//! That matters because the aggregator cannot be reconstructed: its
//! `WelfordAccumulator`s and quantile reservoirs hold every per-plan observation, and the
//! database stores only the reduced five-number summaries. Those summaries are, however,
//! precisely what the report renders — including the box plots, which need nothing beyond
//! min/Q1/median/Q3/max and the mean.

use std::collections::HashMap;

use crate::core::statistics::aggregator::{
    AggregatedCourseStats, AggregatedDegreeStats, MetricsAggregator,
};

/// Degree- and course-level statistics for one analysis run.
#[derive(Debug, Clone)]
pub struct ReportStats {
    degree: AggregatedDegreeStats,
    courses: HashMap<String, AggregatedCourseStats>,
}

impl ReportStats {
    /// Build from already-reduced statistics, e.g. rows read back from the database.
    #[must_use]
    pub const fn new(
        degree: AggregatedDegreeStats,
        courses: HashMap<String, AggregatedCourseStats>,
    ) -> Self {
        Self { degree, courses }
    }

    /// Reduce a live aggregator to just what a report reads.
    #[must_use]
    pub fn from_aggregator(aggregator: &MetricsAggregator) -> Self {
        let courses = aggregator
            .course_ids()
            .into_iter()
            .filter_map(|id| aggregator.course_stats(&id).map(|stats| (id, stats)))
            .collect();
        Self {
            degree: aggregator.degree_stats(),
            courses,
        }
    }

    /// Degree-level summary: complexity, delay, credits, chain length.
    #[must_use]
    pub const fn degree_stats(&self) -> &AggregatedDegreeStats {
        &self.degree
    }

    /// Summary for one course, or `None` if it appeared in no analysed plan.
    #[must_use]
    pub fn course_stats(&self, course_id: &str) -> Option<&AggregatedCourseStats> {
        self.courses.get(course_id)
    }

    /// Every course that has a summary, in a stable order.
    ///
    /// Sorted, not raw `HashMap` order. The report sorts these by (is-major, complexity)
    /// with a *stable* sort, so whatever order arrives here decides every tie — and
    /// unsorted map keys made the generated HTML differ between two runs of the same
    /// binary on the same input.
    #[must_use]
    pub fn course_ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.courses.keys().cloned().collect();
        ids.sort();
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metrics::CourseMetrics;
    use crate::core::statistics::aggregator::AggregatorConfig;

    fn metrics(complexity: usize) -> CourseMetrics {
        CourseMetrics {
            complexity,
            centrality: 5,
            delay: 3,
            blocking: 2,
            chain_length: 2,
        }
    }

    #[test]
    fn from_aggregator_carries_every_course_the_aggregator_knows() {
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
        let mut plan = HashMap::new();
        plan.insert("CS101".to_string(), metrics(10));
        plan.insert("CS102".to_string(), metrics(20));
        agg.add_plan(&plan, 120.0);

        let stats = ReportStats::from_aggregator(&agg);
        let mut ids = stats.course_ids();
        ids.sort();
        assert_eq!(
            ids,
            ["CS101", "CS102"],
            "a course was dropped in the reduction"
        );
        assert!(stats.course_stats("CS101").is_some());
    }

    #[test]
    fn from_aggregator_preserves_the_degree_summary_the_box_plots_render() {
        // The five-number summary is the whole input to BoxPlotData::from_metric_stats,
        // so a reduction that lost a quartile would silently flatten every box plot.
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
        for c in [10_usize, 20, 30] {
            let mut plan = HashMap::new();
            plan.insert("CS101".to_string(), metrics(c));
            agg.add_plan(&plan, f64::from(u32::try_from(c).expect("small")) * 4.0);
        }
        let direct = agg.degree_stats();
        let stats = ReportStats::from_aggregator(&agg);
        let via = stats.degree_stats();
        assert!((via.total_complexity.min - direct.total_complexity.min).abs() < f64::EPSILON);
        assert!((via.total_complexity.q1 - direct.total_complexity.q1).abs() < f64::EPSILON);
        assert!(
            (via.total_complexity.median - direct.total_complexity.median).abs() < f64::EPSILON
        );
        assert!((via.total_complexity.q3 - direct.total_complexity.q3).abs() < f64::EPSILON);
        assert!((via.total_complexity.max - direct.total_complexity.max).abs() < f64::EPSILON);
        assert_eq!(via.plan_count, direct.plan_count);
    }

    #[test]
    fn an_unknown_course_has_no_stats_rather_than_zeroes() {
        let stats = ReportStats::new(
            MetricsAggregator::new(AggregatorConfig::default()).degree_stats(),
            HashMap::new(),
        );
        assert!(stats.course_stats("NOPE").is_none());
        assert!(stats.course_ids().is_empty());
    }
}
