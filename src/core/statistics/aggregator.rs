//! Metrics aggregation for degree analysis
//!
//! Collects and aggregates curriculum metrics across multiple degree plans,
//! supporting both streaming (memory-efficient) and batch processing modes.
//!
//! # Exact vs Streaming Mode
//!
//! By default, exact mode is used which stores all values for precise quantile
//! computation. For very large plan counts (>100k), consider disabling `exact_mode`
//! to use reservoir sampling (note: reservoir sampling may give biased results
//! if plans are not processed in random order).

// Allow precision loss for metrics-to-f64 conversions - metrics fit in f64 mantissa
#![allow(clippy::cast_precision_loss)]

use super::streaming::{ExactQuantileAccumulator, QuantileReservoir, WelfordAccumulator};
use crate::core::metrics::CourseMetrics;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Configuration for metrics aggregation
#[derive(Debug, Clone)]
pub struct AggregatorConfig {
    /// Reservoir size for approximate quantiles (used when `exact_mode` is false)
    pub reservoir_size: usize,
    /// Whether to track per-course metrics
    pub track_per_course: bool,
    /// Whether to compute exact statistics (stores all values)
    /// Defaults to true for accurate quantile computation
    pub exact_mode: bool,
}

impl Default for AggregatorConfig {
    fn default() -> Self {
        Self {
            reservoir_size: 10000,
            track_per_course: true,
            exact_mode: true, // Default to exact for accurate quantiles
        }
    }
}

/// Per-course streaming statistics
#[derive(Debug, Clone)]
pub struct CourseAggregator {
    /// Complexity accumulator
    pub complexity: WelfordAccumulator,
    /// Centrality accumulator
    pub centrality: WelfordAccumulator,
    /// Delay accumulator
    pub delay: WelfordAccumulator,
    /// Blocking factor accumulator
    pub blocking: WelfordAccumulator,
    /// Reservoir for complexity quantiles
    pub complexity_reservoir: QuantileReservoir,
}

impl CourseAggregator {
    /// Create a new course aggregator
    #[must_use]
    pub fn new(reservoir_size: usize) -> Self {
        Self {
            complexity: WelfordAccumulator::new(),
            centrality: WelfordAccumulator::new(),
            delay: WelfordAccumulator::new(),
            blocking: WelfordAccumulator::new(),
            complexity_reservoir: QuantileReservoir::new(reservoir_size),
        }
    }

    /// Add metrics from a plan
    #[allow(clippy::cast_precision_loss)] // Metrics values fit in f64 mantissa
    pub fn add(&mut self, metrics: &CourseMetrics) {
        self.complexity.push(metrics.complexity as f64);
        self.centrality.push(metrics.centrality as f64);
        self.delay.push(metrics.delay as f64);
        self.blocking.push(metrics.blocking as f64);
        self.complexity_reservoir.push(metrics.complexity as f64);
    }

    /// Get the number of plans this course appeared in
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // WelfordAccumulator::count() is not const
    pub fn plan_count(&self) -> usize {
        self.complexity.count()
    }

    /// Merge another aggregator into this one
    pub fn merge(&mut self, other: &Self) {
        self.complexity.merge(&other.complexity);
        self.centrality.merge(&other.centrality);
        self.delay.merge(&other.delay);
        self.blocking.merge(&other.blocking);
        self.complexity_reservoir.merge(&other.complexity_reservoir);
    }
}

impl Default for CourseAggregator {
    fn default() -> Self {
        Self::new(1000)
    }
}

/// Aggregated statistics for a single course across all plans
#[derive(Debug, Clone)]
pub struct AggregatedCourseStats {
    /// Course identifier
    pub course_id: String,
    /// Number of plans this course appeared in
    pub plan_count: usize,
    /// Complexity statistics
    pub complexity: MetricStats,
    /// Centrality statistics
    pub centrality: MetricStats,
    /// Delay statistics
    pub delay: MetricStats,
    /// Blocking factor statistics
    pub blocking: MetricStats,
}

/// Statistics for a single metric
#[derive(Debug, Clone)]
pub struct MetricStats {
    /// Minimum value
    pub min: f64,
    /// Maximum value
    pub max: f64,
    /// Mean value
    pub mean: f64,
    /// Standard deviation
    pub std_dev: f64,
    /// Median value
    pub median: f64,
    /// First quartile (Q1)
    pub q1: f64,
    /// Third quartile (Q3)
    pub q3: f64,
}

impl MetricStats {
    /// Create from Welford accumulator and quantile reservoir (streaming mode)
    #[must_use]
    fn from_accumulators(welford: &WelfordAccumulator, reservoir: &QuantileReservoir) -> Self {
        Self {
            min: welford.min(),
            max: welford.max(),
            mean: welford.mean(),
            std_dev: welford.std_dev(),
            median: reservoir.median(),
            q1: reservoir.q1(),
            q3: reservoir.q3(),
        }
    }

    /// Create from Welford accumulator and quantile storage (supports exact or streaming)
    #[must_use]
    fn from_welford_and_quantile_storage(
        welford: &WelfordAccumulator,
        storage: &QuantileStorage,
    ) -> Self {
        Self {
            min: welford.min(),
            max: welford.max(),
            mean: welford.mean(),
            std_dev: welford.std_dev(),
            median: storage.median(),
            q1: storage.q1(),
            q3: storage.q3(),
        }
    }

    /// Create from Welford accumulator only (uses mean as median estimate)
    #[must_use]
    fn from_welford_only(welford: &WelfordAccumulator) -> Self {
        Self {
            min: welford.min(),
            max: welford.max(),
            mean: welford.mean(),
            std_dev: welford.std_dev(),
            // Without reservoir, use mean as rough median estimate
            median: welford.mean(),
            q1: welford.mean() - welford.std_dev(),
            q3: welford.mean() + welford.std_dev(),
        }
    }
}

/// Degree-level aggregated statistics
#[derive(Debug, Clone)]
pub struct AggregatedDegreeStats {
    /// Total plans processed
    pub plan_count: usize,
    /// Degree complexity (sum of course complexities) statistics
    pub total_complexity: MetricStats,
    /// Longest delay factor statistics
    pub longest_delay: MetricStats,
    /// Total credits statistics
    pub total_credits: MetricStats,
}

/// Quantile storage mode - either exact or approximate
#[derive(Debug)]
enum QuantileStorage {
    /// Exact mode stores all values
    Exact(ExactQuantileAccumulator),
    /// Streaming mode uses reservoir sampling
    Streaming(QuantileReservoir),
}

impl QuantileStorage {
    /// Create new storage based on exact mode setting
    fn new(exact_mode: bool, reservoir_size: usize) -> Self {
        if exact_mode {
            Self::Exact(ExactQuantileAccumulator::new())
        } else {
            Self::Streaming(QuantileReservoir::new(reservoir_size))
        }
    }

    /// Add a value
    fn push(&mut self, value: f64) {
        match self {
            Self::Exact(acc) => acc.push(value),
            Self::Streaming(res) => res.push(value),
        }
    }

    /// Get median
    fn median(&self) -> f64 {
        match self {
            Self::Exact(acc) => acc.median(),
            Self::Streaming(res) => res.median(),
        }
    }

    /// Get Q1
    fn q1(&self) -> f64 {
        match self {
            Self::Exact(acc) => acc.q1(),
            Self::Streaming(res) => res.q1(),
        }
    }

    /// Get Q3
    fn q3(&self) -> f64 {
        match self {
            Self::Exact(acc) => acc.q3(),
            Self::Streaming(res) => res.q3(),
        }
    }

    /// Merge another storage into this one
    fn merge(&mut self, other: &Self) {
        match (self, other) {
            (Self::Exact(acc), Self::Exact(other_acc)) => acc.merge(other_acc),
            (Self::Streaming(res), Self::Streaming(other_res)) => res.merge(other_res),
            _ => {
                // Mismatched modes - this shouldn't happen in practice
                // but we can handle it by converting streaming to exact
            }
        }
    }
}

/// Main aggregator for collecting metrics across plans
///
/// Thread-safe when wrapped in Arc<Mutex<>>.
#[derive(Debug)]
pub struct MetricsAggregator {
    /// Configuration
    config: AggregatorConfig,
    /// Per-course aggregators
    course_stats: HashMap<String, CourseAggregator>,
    /// Degree-level complexity accumulator (for mean/stddev)
    degree_complexity: WelfordAccumulator,
    /// Degree-level longest delay accumulator (for mean/stddev)
    degree_delay: WelfordAccumulator,
    /// Degree-level credits accumulator
    degree_credits: WelfordAccumulator,
    /// Storage for degree complexity quantiles
    degree_complexity_quantiles: QuantileStorage,
    /// Storage for degree delay quantiles
    degree_delay_quantiles: QuantileStorage,
    /// Total plans processed
    plan_count: usize,
}

impl MetricsAggregator {
    /// Create a new aggregator with the given configuration
    #[must_use]
    pub fn new(config: AggregatorConfig) -> Self {
        let exact_mode = config.exact_mode;
        let reservoir_size = config.reservoir_size;
        Self {
            config,
            course_stats: HashMap::new(),
            degree_complexity: WelfordAccumulator::new(),
            degree_delay: WelfordAccumulator::new(),
            degree_credits: WelfordAccumulator::new(),
            degree_complexity_quantiles: QuantileStorage::new(exact_mode, reservoir_size),
            degree_delay_quantiles: QuantileStorage::new(exact_mode, reservoir_size),
            plan_count: 0,
        }
    }

    /// Add metrics from a single plan
    ///
    /// # Arguments
    /// * `course_metrics` - Metrics for each course in the plan
    /// * `total_credits` - Total credits in the plan
    pub fn add_plan(
        &mut self,
        course_metrics: &HashMap<String, CourseMetrics>,
        total_credits: f64,
    ) {
        self.plan_count += 1;

        // Compute degree-level metrics
        let total_complexity: usize = course_metrics.values().map(|m| m.complexity).sum();
        let longest_delay: usize = course_metrics.values().map(|m| m.delay).max().unwrap_or(0);

        // Update Welford accumulators for mean/stddev
        self.degree_complexity.push(total_complexity as f64);
        self.degree_delay.push(longest_delay as f64);
        self.degree_credits.push(total_credits);

        // Update quantile storage
        self.degree_complexity_quantiles
            .push(total_complexity as f64);
        self.degree_delay_quantiles.push(longest_delay as f64);

        // Track per-course metrics if configured
        if self.config.track_per_course {
            for (course_id, metrics) in course_metrics {
                let aggregator = self
                    .course_stats
                    .entry(course_id.clone())
                    .or_insert_with(|| CourseAggregator::new(self.config.reservoir_size));
                aggregator.add(metrics);
            }
        }
    }

    /// Get the number of plans processed
    #[must_use]
    pub const fn plan_count(&self) -> usize {
        self.plan_count
    }

    /// Get degree-level aggregated statistics
    #[must_use]
    pub fn degree_stats(&self) -> AggregatedDegreeStats {
        AggregatedDegreeStats {
            plan_count: self.plan_count,
            total_complexity: MetricStats::from_welford_and_quantile_storage(
                &self.degree_complexity,
                &self.degree_complexity_quantiles,
            ),
            longest_delay: MetricStats::from_welford_and_quantile_storage(
                &self.degree_delay,
                &self.degree_delay_quantiles,
            ),
            total_credits: MetricStats::from_welford_only(&self.degree_credits),
        }
    }

    /// Get aggregated statistics for a specific course
    #[must_use]
    pub fn course_stats(&self, course_id: &str) -> Option<AggregatedCourseStats> {
        self.course_stats
            .get(course_id)
            .map(|agg| AggregatedCourseStats {
                course_id: course_id.to_string(),
                plan_count: agg.plan_count(),
                complexity: MetricStats::from_accumulators(
                    &agg.complexity,
                    &agg.complexity_reservoir,
                ),
                centrality: MetricStats::from_welford_only(&agg.centrality),
                delay: MetricStats::from_welford_only(&agg.delay),
                blocking: MetricStats::from_welford_only(&agg.blocking),
            })
    }

    /// Get all course IDs that have been tracked
    #[must_use]
    pub fn course_ids(&self) -> Vec<String> {
        self.course_stats.keys().cloned().collect()
    }

    /// Merge another aggregator into this one
    ///
    /// Used for combining results from parallel processing.
    pub fn merge(&mut self, other: &Self) {
        self.plan_count += other.plan_count;
        self.degree_complexity.merge(&other.degree_complexity);
        self.degree_delay.merge(&other.degree_delay);
        self.degree_credits.merge(&other.degree_credits);
        self.degree_complexity_quantiles
            .merge(&other.degree_complexity_quantiles);
        self.degree_delay_quantiles
            .merge(&other.degree_delay_quantiles);

        for (course_id, other_agg) in &other.course_stats {
            let agg = self
                .course_stats
                .entry(course_id.clone())
                .or_insert_with(|| CourseAggregator::new(self.config.reservoir_size));
            agg.merge(other_agg);
        }
    }
}

impl Default for MetricsAggregator {
    fn default() -> Self {
        Self::new(AggregatorConfig::default())
    }
}

/// Thread-safe wrapper for parallel aggregation
pub type SharedAggregator = Arc<Mutex<MetricsAggregator>>;

/// Create a new thread-safe aggregator
#[must_use]
pub fn new_shared_aggregator(config: AggregatorConfig) -> SharedAggregator {
    Arc::new(Mutex::new(MetricsAggregator::new(config)))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_metrics() -> CourseMetrics {
        CourseMetrics {
            complexity: 10,
            centrality: 5,
            delay: 3,
            blocking: 7,
        }
    }

    #[test]
    fn test_course_aggregator_basic() {
        let mut agg = CourseAggregator::default();
        agg.add(&sample_metrics());
        agg.add(&CourseMetrics {
            complexity: 20,
            centrality: 10,
            delay: 6,
            blocking: 14,
        });

        assert_eq!(agg.plan_count(), 2);
        assert!((agg.complexity.mean() - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_aggregator_single_plan() {
        let mut agg = MetricsAggregator::default();
        let mut course_metrics = HashMap::new();
        course_metrics.insert("CS1000".to_string(), sample_metrics());

        agg.add_plan(&course_metrics, 4.0);

        assert_eq!(agg.plan_count(), 1);
        let degree_stats = agg.degree_stats();
        assert_eq!(degree_stats.plan_count, 1);
    }

    #[test]
    fn test_metrics_aggregator_multiple_plans() {
        let mut agg = MetricsAggregator::default();

        for i in 1..=10 {
            let mut course_metrics = HashMap::new();
            course_metrics.insert(
                "CS1000".to_string(),
                CourseMetrics {
                    complexity: i * 10,
                    centrality: i * 5,
                    delay: i * 2,
                    blocking: i * 3,
                },
            );
            agg.add_plan(&course_metrics, (i * 4) as f64);
        }

        assert_eq!(agg.plan_count(), 10);

        let course_stats = agg.course_stats("CS1000").expect("Course should exist");
        assert_eq!(course_stats.plan_count, 10);
        assert!((course_stats.complexity.mean - 55.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_metrics_aggregator_merge() {
        let mut agg1 = MetricsAggregator::default();
        let mut agg2 = MetricsAggregator::default();

        let mut course_metrics = HashMap::new();
        course_metrics.insert("CS1000".to_string(), sample_metrics());

        agg1.add_plan(&course_metrics, 4.0);
        agg2.add_plan(&course_metrics, 4.0);

        agg1.merge(&agg2);

        assert_eq!(agg1.plan_count(), 2);
    }

    #[test]
    fn test_degree_stats_computation() {
        let mut agg = MetricsAggregator::default();

        for i in 1..=100 {
            let mut course_metrics = HashMap::new();
            course_metrics.insert(
                "CS1000".to_string(),
                CourseMetrics {
                    complexity: i,
                    centrality: i / 2,
                    delay: i % 10 + 1,
                    blocking: i / 3,
                },
            );
            course_metrics.insert(
                "CS2000".to_string(),
                CourseMetrics {
                    complexity: i * 2,
                    centrality: i,
                    delay: (i % 10) + 2,
                    blocking: i / 2,
                },
            );
            agg.add_plan(&course_metrics, 8.0);
        }

        let stats = agg.degree_stats();
        assert_eq!(stats.plan_count, 100);

        // Total complexity should be sum of both courses
        // Mean of (i + 2*i) = 3*i for i=1..100 => mean = 3 * 50.5 = 151.5
        assert!((stats.total_complexity.mean - 151.5).abs() < 0.1);
    }

    #[test]
    fn test_shared_aggregator() {
        let shared = new_shared_aggregator(AggregatorConfig::default());

        {
            let mut agg = shared.lock().unwrap();
            let mut course_metrics = HashMap::new();
            course_metrics.insert("CS1000".to_string(), sample_metrics());
            agg.add_plan(&course_metrics, 4.0);
        }

        let plan_count = shared.lock().unwrap().plan_count();
        assert_eq!(plan_count, 1);
    }

    #[test]
    fn test_course_ids() {
        let mut agg = MetricsAggregator::default();
        let mut course_metrics = HashMap::new();
        course_metrics.insert("CS1000".to_string(), sample_metrics());
        course_metrics.insert("CS2000".to_string(), sample_metrics());
        agg.add_plan(&course_metrics, 8.0);

        let ids = agg.course_ids();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&"CS1000".to_string()));
        assert!(ids.contains(&"CS2000".to_string()));
    }
}
