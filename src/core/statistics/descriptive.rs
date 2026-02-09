//! Descriptive statistics for metrics aggregation
//!
//! Provides computation of standard descriptive statistics including
//! min, max, quartiles, mean, and standard deviation.

use super::strategy::{CalculationStrategy, MedianStrategy};

/// Descriptive statistics for a collection of values
///
/// Contains all standard measures needed for box plots and summary reports.
#[derive(Debug, Clone, PartialEq)]
pub struct DescriptiveStats {
    /// Minimum value
    pub min: f64,
    /// First quartile (25th percentile)
    pub q1: f64,
    /// Median (50th percentile)
    pub median: f64,
    /// Third quartile (75th percentile)
    pub q3: f64,
    /// Maximum value
    pub max: f64,
    /// Arithmetic mean
    pub mean: f64,
    /// Standard deviation (population)
    pub std_dev: f64,
    /// Number of values
    pub count: usize,
}

impl Default for DescriptiveStats {
    fn default() -> Self {
        Self {
            min: 0.0,
            q1: 0.0,
            median: 0.0,
            q3: 0.0,
            max: 0.0,
            mean: 0.0,
            std_dev: 0.0,
            count: 0,
        }
    }
}

impl DescriptiveStats {
    /// Compute descriptive statistics from a slice of values
    ///
    /// # Arguments
    /// * `values` - Slice of values to analyze
    ///
    /// # Returns
    /// `DescriptiveStats` with all computed measures
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_values(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }

        let count = values.len();
        let mut sorted: Vec<f64> = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let min = sorted[0];
        let max = sorted[count - 1];
        let median = compute_percentile(&sorted, 50.0);
        let q1 = compute_percentile(&sorted, 25.0);
        let q3 = compute_percentile(&sorted, 75.0);

        let mean = values.iter().sum::<f64>() / count as f64;
        let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / count as f64;
        let std_dev = variance.sqrt();

        Self {
            min,
            q1,
            median,
            q3,
            max,
            mean,
            std_dev,
            count,
        }
    }

    /// Compute descriptive statistics from a slice of usize values
    ///
    /// Convenience method for working with integer metrics.
    #[must_use]
    #[allow(clippy::cast_precision_loss)]
    pub fn from_usize_values(values: &[usize]) -> Self {
        let float_values: Vec<f64> = values.iter().map(|&v| v as f64).collect();
        Self::from_values(&float_values)
    }

    /// Get the interquartile range (IQR)
    #[must_use]
    pub fn iqr(&self) -> f64 {
        self.q3 - self.q1
    }

    /// Get the range (max - min)
    #[must_use]
    pub fn range(&self) -> f64 {
        self.max - self.min
    }

    /// Check if a value is an outlier using the 1.5*IQR rule
    #[must_use]
    pub fn is_outlier(&self, value: f64) -> bool {
        value < self.lower_fence() || value > self.upper_fence()
    }

    /// Get the lower fence for outlier detection
    #[must_use]
    pub fn lower_fence(&self) -> f64 {
        1.5_f64.mul_add(-self.iqr(), self.q1)
    }

    /// Get the upper fence for outlier detection
    #[must_use]
    pub fn upper_fence(&self) -> f64 {
        1.5_f64.mul_add(self.iqr(), self.q3)
    }

    /// Get the representative value using a calculation strategy
    ///
    /// # Arguments
    /// * `strategy` - The calculation strategy to use
    /// * `values` - Original values (needed for some strategies)
    #[must_use]
    pub fn representative_value(&self, strategy: &dyn CalculationStrategy) -> f64 {
        match strategy.name() {
            "mean" => self.mean,
            // Default to median for unknown strategies
            _ => self.median,
        }
    }

    /// Get the representative value using the default strategy (median)
    #[must_use]
    pub fn default_representative(&self) -> f64 {
        self.representative_value(&MedianStrategy)
    }
}

/// Compute a percentile from a sorted slice
///
/// Uses linear interpolation between adjacent values.
#[allow(
    clippy::cast_precision_loss,
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss
)]
fn compute_percentile(sorted: &[f64], percentile: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }

    let n = sorted.len();
    let rank = (percentile / 100.0) * (n - 1) as f64;
    let lower_idx = rank.floor() as usize;
    let upper_idx = rank.ceil() as usize;

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
    fn test_descriptive_stats_basic() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let stats = DescriptiveStats::from_values(&values);

        assert!((stats.min - 1.0).abs() < f64::EPSILON);
        assert!((stats.max - 10.0).abs() < f64::EPSILON);
        assert!((stats.mean - 5.5).abs() < f64::EPSILON);
        assert!((stats.median - 5.5).abs() < f64::EPSILON);
        assert_eq!(stats.count, 10);
    }

    #[test]
    fn test_descriptive_stats_quartiles() {
        // Using values 1-12 for clear quartile boundaries
        let values: Vec<f64> = (1..=12).map(f64::from).collect();
        let stats = DescriptiveStats::from_values(&values);

        // Q1 should be around 3.75, Q3 around 9.25
        assert!(stats.q1 > 3.0 && stats.q1 < 4.0);
        assert!(stats.q3 > 9.0 && stats.q3 < 10.0);
    }

    #[test]
    fn test_descriptive_stats_empty() {
        let values: Vec<f64> = vec![];
        let stats = DescriptiveStats::from_values(&values);

        assert_eq!(stats.count, 0);
        assert!((stats.min - 0.0).abs() < f64::EPSILON);
        assert!((stats.max - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_descriptive_stats_single_value() {
        let values = vec![42.0];
        let stats = DescriptiveStats::from_values(&values);

        assert!((stats.min - 42.0).abs() < f64::EPSILON);
        assert!((stats.max - 42.0).abs() < f64::EPSILON);
        assert!((stats.median - 42.0).abs() < f64::EPSILON);
        assert!((stats.mean - 42.0).abs() < f64::EPSILON);
        assert!((stats.std_dev - 0.0).abs() < f64::EPSILON);
        assert_eq!(stats.count, 1);
    }

    #[test]
    fn test_descriptive_stats_from_usize() {
        let values: Vec<usize> = vec![1, 2, 3, 4, 5];
        let stats = DescriptiveStats::from_usize_values(&values);

        assert!((stats.min - 1.0).abs() < f64::EPSILON);
        assert!((stats.max - 5.0).abs() < f64::EPSILON);
        assert!((stats.mean - 3.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_iqr() {
        let values: Vec<f64> = (1..=12).map(f64::from).collect();
        let stats = DescriptiveStats::from_values(&values);

        let iqr = stats.iqr();
        assert!(iqr > 5.0 && iqr < 6.0); // Q3 - Q1 ≈ 9.25 - 3.75 = 5.5
    }

    #[test]
    fn test_outlier_detection() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let stats = DescriptiveStats::from_values(&values);

        // Values within normal range should not be outliers
        assert!(!stats.is_outlier(5.0));

        // Extreme values should be outliers
        assert!(stats.is_outlier(-100.0));
        assert!(stats.is_outlier(100.0));
    }

    #[test]
    fn test_representative_value() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 100.0]; // Skewed data
        let stats = DescriptiveStats::from_values(&values);

        let median_strategy = super::super::strategy::MedianStrategy;
        let mean_strategy = super::super::strategy::MeanStrategy;

        // Median should be less affected by outlier
        assert!((stats.representative_value(&median_strategy) - 3.0).abs() < f64::EPSILON);
        // Mean should be higher due to outlier
        assert!(stats.representative_value(&mean_strategy) > 20.0);
    }

    #[test]
    fn test_std_dev() {
        // Values with known standard deviation
        let values = vec![2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0];
        let stats = DescriptiveStats::from_values(&values);

        // Population std dev should be 2.0
        assert!((stats.std_dev - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_range() {
        let values = vec![5.0, 10.0, 15.0, 20.0, 25.0];
        let stats = DescriptiveStats::from_values(&values);
        assert!((stats.range() - 20.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_fences() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        let stats = DescriptiveStats::from_values(&values);

        // Verify fences are computed correctly
        let iqr = stats.iqr();
        let expected_lower = 1.5_f64.mul_add(-iqr, stats.q1);
        let expected_upper = 1.5_f64.mul_add(iqr, stats.q3);

        assert!((stats.lower_fence() - expected_lower).abs() < 0.001);
        assert!((stats.upper_fence() - expected_upper).abs() < 0.001);
    }

    #[test]
    fn test_percentile_single_value() {
        // This tests the single-value branch in compute_percentile
        let values = vec![42.0];
        let stats = DescriptiveStats::from_values(&values);

        // All quartiles should equal the single value
        assert!((stats.q1 - 42.0).abs() < f64::EPSILON);
        assert!((stats.median - 42.0).abs() < f64::EPSILON);
        assert!((stats.q3 - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_default_representative() {
        let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
        let stats = DescriptiveStats::from_values(&values);

        // Default representative should be median
        assert!((stats.default_representative() - stats.median).abs() < f64::EPSILON);
    }
}
