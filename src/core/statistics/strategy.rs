//! Calculation strategy pattern for aggregate metrics
//!
//! Provides extensible strategies for computing aggregate values from
//! collections of metrics. Default is median, but can be configured to
//! use mean or other strategies.

use std::fmt;

/// Strategy for calculating aggregate values from a collection of metrics
///
/// This trait enables different aggregation methods (median, mean, etc.)
/// to be used interchangeably when computing summary statistics.
pub trait CalculationStrategy: Send + Sync + fmt::Debug {
    /// Compute the aggregate value from a slice of values
    ///
    /// # Arguments
    /// * `values` - Slice of values to aggregate (must not be empty)
    ///
    /// # Returns
    /// The aggregated value according to this strategy
    fn aggregate(&self, values: &[f64]) -> f64;

    /// Get the name of this strategy for display purposes
    fn name(&self) -> &'static str;
}

/// Median calculation strategy (default)
///
/// Returns the middle value when sorted. For even-length collections,
/// returns the average of the two middle values.
#[derive(Debug, Clone, Copy, Default)]
pub struct MedianStrategy;

impl CalculationStrategy for MedianStrategy {
    fn aggregate(&self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }

        let mut sorted: Vec<f64> = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = sorted.len();
        if len.is_multiple_of(2) {
            f64::midpoint(sorted[len / 2 - 1], sorted[len / 2])
        } else {
            sorted[len / 2]
        }
    }

    fn name(&self) -> &'static str {
        "median"
    }
}

/// Mean (average) calculation strategy
///
/// Returns the arithmetic mean of all values.
#[derive(Debug, Clone, Copy, Default)]
pub struct MeanStrategy;

impl CalculationStrategy for MeanStrategy {
    #[allow(clippy::cast_precision_loss)]
    fn aggregate(&self, values: &[f64]) -> f64 {
        if values.is_empty() {
            return 0.0;
        }

        values.iter().sum::<f64>() / values.len() as f64
    }

    fn name(&self) -> &'static str {
        "mean"
    }
}

/// Parse a strategy name into a boxed strategy instance
///
/// # Arguments
/// * `name` - Strategy name ("median" or "mean")
///
/// # Returns
/// A boxed strategy instance, or None if the name is not recognized
#[must_use]
pub fn strategy_from_name(name: &str) -> Option<Box<dyn CalculationStrategy>> {
    match name.to_lowercase().as_str() {
        "median" => Some(Box::new(MedianStrategy)),
        "mean" => Some(Box::new(MeanStrategy)),
        _ => None,
    }
}

/// Get the default calculation strategy (median)
#[must_use]
pub fn default_strategy() -> Box<dyn CalculationStrategy> {
    Box::new(MedianStrategy)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_median_odd_count() {
        let strategy = MedianStrategy;
        let values = vec![1.0, 3.0, 2.0, 5.0, 4.0];
        assert!((strategy.aggregate(&values) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_median_even_count() {
        let strategy = MedianStrategy;
        let values = vec![1.0, 2.0, 3.0, 4.0];
        assert!((strategy.aggregate(&values) - 2.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_median_single_value() {
        let strategy = MedianStrategy;
        let values = vec![42.0];
        assert!((strategy.aggregate(&values) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_median_empty() {
        let strategy = MedianStrategy;
        let values: Vec<f64> = vec![];
        assert!((strategy.aggregate(&values) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mean_values() {
        let strategy = MeanStrategy;
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        assert!((strategy.aggregate(&values) - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mean_single_value() {
        let strategy = MeanStrategy;
        let values = vec![42.0];
        assert!((strategy.aggregate(&values) - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_mean_empty() {
        let strategy = MeanStrategy;
        let values: Vec<f64> = vec![];
        assert!((strategy.aggregate(&values) - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_strategy_names() {
        assert_eq!(MedianStrategy.name(), "median");
        assert_eq!(MeanStrategy.name(), "mean");
    }

    #[test]
    fn test_strategy_from_name() {
        assert!(strategy_from_name("median").is_some());
        assert!(strategy_from_name("mean").is_some());
        assert!(strategy_from_name("MEDIAN").is_some()); // case insensitive
        assert!(strategy_from_name("unknown").is_none());
    }

    #[test]
    fn test_default_strategy() {
        let strategy = default_strategy();
        assert_eq!(strategy.name(), "median");
    }
}
