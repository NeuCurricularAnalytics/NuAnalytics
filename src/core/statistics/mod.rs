//! Statistics module for degree analysis
//!
//! Provides calculation strategies, descriptive statistics, streaming
//! aggregation, and visualization for collecting metrics across multiple
//! degree plans.

pub mod aggregator;
pub mod box_plot;
pub mod descriptive;
pub mod strategy;
pub mod streaming;

pub use aggregator::{
    new_shared_aggregator, AggregatedCourseStats, AggregatedDegreeStats, AggregatorConfig,
    CourseAggregator, MetricStats, MetricsAggregator, SharedAggregator,
};
pub use box_plot::{BoxPlotConfig, BoxPlotData, BoxPlotGenerator};
pub use descriptive::DescriptiveStats;
pub use strategy::{CalculationStrategy, MeanStrategy, MedianStrategy};

/// Compute a percentile from a pre-sorted slice using linear interpolation
///
/// # Arguments
/// * `sorted` - A pre-sorted (ascending) slice of f64 values
/// * `percentile` - The percentile to compute (0.0 to 100.0)
///
/// # Returns
/// The interpolated value at the given percentile, or 0.0 for empty slices
#[must_use]
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
pub fn compute_percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let n = sorted.len();
    let rank = (percentile / 100.0) * (n - 1) as f64;
    let lower_idx = rank.floor() as usize;
    let upper_idx = (lower_idx + 1).min(n - 1);

    if lower_idx == upper_idx {
        sorted[lower_idx]
    } else {
        let fraction = rank - lower_idx as f64;
        fraction.mul_add(sorted[upper_idx] - sorted[lower_idx], sorted[lower_idx])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_percentile_empty() {
        assert!((compute_percentile(&[], 50.0) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_percentile_single() {
        assert!((compute_percentile(&[42.0], 50.0) - 42.0).abs() < f64::EPSILON);
        assert!((compute_percentile(&[42.0], 0.0) - 42.0).abs() < f64::EPSILON);
        assert!((compute_percentile(&[42.0], 100.0) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_percentile_two_values() {
        let values = vec![10.0, 20.0];
        assert!((compute_percentile(&values, 0.0) - 10.0).abs() < f64::EPSILON);
        assert!((compute_percentile(&values, 50.0) - 15.0).abs() < f64::EPSILON);
        assert!((compute_percentile(&values, 100.0) - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_compute_percentile_quartiles() {
        let values: Vec<f64> = (1..=100).map(f64::from).collect();
        let median = compute_percentile(&values, 50.0);
        let q1 = compute_percentile(&values, 25.0);
        let q3 = compute_percentile(&values, 75.0);

        assert!((median - 50.5).abs() < 0.01);
        assert!(q1 > 25.0 && q1 < 26.0);
        assert!(q3 > 75.0 && q3 < 76.0);
    }
}
