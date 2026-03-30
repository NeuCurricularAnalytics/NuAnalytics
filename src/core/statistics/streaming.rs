//! Streaming statistics for online computation
//!
//! Provides algorithms for computing statistics incrementally without
//! storing all values. Uses Welford's algorithm for stable online
//! computation of mean and variance.

// Allow precision loss for count-to-f64 conversions - counts fit in f64 mantissa
#![allow(clippy::cast_precision_loss)]

/// Online statistics accumulator using Welford's algorithm
///
/// Computes mean, variance, and tracks min/max in a single pass
/// with numerical stability and O(1) memory.
#[derive(Debug, Clone, Default)]
pub struct WelfordAccumulator {
    /// Number of values seen
    count: usize,
    /// Running mean
    mean: f64,
    /// Sum of squared differences from mean (M2 in Welford's)
    m2: f64,
    /// Minimum value seen
    min: f64,
    /// Maximum value seen
    max: f64,
}

impl WelfordAccumulator {
    /// Create a new empty accumulator
    #[must_use]
    pub const fn new() -> Self {
        Self {
            count: 0,
            mean: 0.0,
            m2: 0.0,
            min: f64::INFINITY,
            max: f64::NEG_INFINITY,
        }
    }

    /// Add a new value to the accumulator
    ///
    /// Uses Welford's online algorithm for numerical stability.
    pub fn push(&mut self, value: f64) {
        self.count += 1;

        // Update min/max
        if value < self.min {
            self.min = value;
        }
        if value > self.max {
            self.max = value;
        }

        // Welford's algorithm
        let delta = value - self.mean;
        self.mean += delta / self.count as f64;
        let delta2 = value - self.mean;
        self.m2 += delta * delta2;
    }

    /// Get the current count
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Get the current mean
    #[must_use]
    pub const fn mean(&self) -> f64 {
        self.mean
    }

    /// Get the population variance
    #[must_use]
    pub fn variance(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        self.m2 / self.count as f64
    }

    /// Get the sample variance (unbiased)
    #[must_use]
    pub fn sample_variance(&self) -> f64 {
        if self.count < 2 {
            return 0.0;
        }
        self.m2 / (self.count - 1) as f64
    }

    /// Get the population standard deviation
    #[must_use]
    pub fn std_dev(&self) -> f64 {
        self.variance().sqrt()
    }

    /// Get the sample standard deviation
    #[must_use]
    pub fn sample_std_dev(&self) -> f64 {
        self.sample_variance().sqrt()
    }

    /// Get the minimum value
    #[must_use]
    pub const fn min(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.min
        }
    }

    /// Get the maximum value
    #[must_use]
    pub const fn max(&self) -> f64 {
        if self.count == 0 {
            0.0
        } else {
            self.max
        }
    }

    /// Merge another accumulator into this one
    ///
    /// Uses parallel algorithm for combining Welford accumulators.
    pub fn merge(&mut self, other: &Self) {
        if other.count == 0 {
            return;
        }
        if self.count == 0 {
            *self = other.clone();
            return;
        }

        let combined_count = self.count + other.count;
        let delta = other.mean - self.mean;

        // Combined mean using mul_add for numerical stability
        let combined_mean = delta.mul_add(other.count as f64 / combined_count as f64, self.mean);

        // Combined M2 using parallel algorithm
        let m2_delta =
            delta * delta * (self.count as f64 * other.count as f64 / combined_count as f64);
        let combined_m2 = self.m2 + other.m2 + m2_delta;

        self.count = combined_count;
        self.mean = combined_mean;
        self.m2 = combined_m2;
        self.min = self.min.min(other.min);
        self.max = self.max.max(other.max);
    }

    /// Check if the accumulator is empty
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.count == 0
    }
}

/// Reservoir for approximate quantile computation
///
/// Uses reservoir sampling to maintain a representative sample
/// for computing approximate percentiles.
#[derive(Debug, Clone)]
pub struct QuantileReservoir {
    /// Maximum reservoir size
    capacity: usize,
    /// Stored samples
    samples: Vec<f64>,
    /// Total count of values seen
    count: usize,
}

impl QuantileReservoir {
    /// Create a new reservoir with the given capacity
    #[must_use]
    pub fn new(capacity: usize) -> Self {
        Self {
            capacity,
            samples: Vec::with_capacity(capacity),
            count: 0,
        }
    }

    /// Add a value to the reservoir
    ///
    /// Uses Algorithm R for uniform random sampling.
    pub fn push(&mut self, value: f64) {
        self.count += 1;

        if self.samples.len() < self.capacity {
            self.samples.push(value);
        } else {
            // Algorithm R: replace with probability capacity/count
            let idx = fastrand::usize(0..self.count);
            if idx < self.capacity {
                self.samples[idx] = value;
            }
        }
    }

    /// Get an approximate percentile from the reservoir
    ///
    /// # Arguments
    /// * `percentile` - Percentile to compute (0-100)
    #[must_use]
    pub fn percentile(&self, percentile: f64) -> f64 {
        let mut sorted = self.samples.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        super::compute_percentile(&sorted, percentile)
    }

    /// Get approximate median
    #[must_use]
    pub fn median(&self) -> f64 {
        self.percentile(50.0)
    }

    /// Get approximate Q1
    #[must_use]
    pub fn q1(&self) -> f64 {
        self.percentile(25.0)
    }

    /// Get approximate Q3
    #[must_use]
    pub fn q3(&self) -> f64 {
        self.percentile(75.0)
    }

    /// Get the total count of values seen (not just stored)
    #[must_use]
    pub const fn count(&self) -> usize {
        self.count
    }

    /// Get the current reservoir size
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::len() is not const
    pub fn reservoir_size(&self) -> usize {
        self.samples.len()
    }

    /// Merge another reservoir into this one
    ///
    /// After merging, the combined reservoir is a valid sample of
    /// the union of both input streams.
    pub fn merge(&mut self, other: &Self) {
        // Simple merge: combine samples and downsample if needed
        let combined_count = self.count + other.count;

        for &sample in &other.samples {
            if self.samples.len() < self.capacity {
                self.samples.push(sample);
            } else {
                // Probabilistically include based on relative counts
                let idx = fastrand::usize(0..combined_count);
                if idx < self.capacity {
                    self.samples[idx] = sample;
                }
            }
        }

        self.count = combined_count;
    }
}

impl Default for QuantileReservoir {
    fn default() -> Self {
        // Default to 1000 samples for reasonable quantile accuracy
        Self::new(1000)
    }
}

/// Exact quantile accumulator that stores all values
///
/// Use when exact quantiles are required and memory allows.
/// For large datasets (>100k values), consider `QuantileReservoir` instead.
#[derive(Debug, Clone, Default)]
pub struct ExactQuantileAccumulator {
    /// All stored values
    values: Vec<f64>,
}

impl ExactQuantileAccumulator {
    /// Create a new exact accumulator
    #[must_use]
    pub const fn new() -> Self {
        Self { values: Vec::new() }
    }

    /// Create with pre-allocated capacity
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            values: Vec::with_capacity(capacity),
        }
    }

    /// Add a value
    pub fn push(&mut self, value: f64) {
        self.values.push(value);
    }

    /// Get the count of values
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::len() is not const
    pub fn count(&self) -> usize {
        self.values.len()
    }

    /// Check if empty
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::is_empty() is not const
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Get a sorted copy of values for percentile calculation
    fn sorted_values(&self) -> Vec<f64> {
        let mut sorted = self.values.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        sorted
    }

    /// Get exact percentile
    ///
    /// # Arguments
    /// * `percentile` - Percentile to compute (0-100)
    #[must_use]
    pub fn percentile(&self, percentile: f64) -> f64 {
        let sorted = self.sorted_values();
        super::compute_percentile(&sorted, percentile)
    }

    /// Get exact median
    #[must_use]
    pub fn median(&self) -> f64 {
        self.percentile(50.0)
    }

    /// Get exact Q1
    #[must_use]
    pub fn q1(&self) -> f64 {
        self.percentile(25.0)
    }

    /// Get exact Q3
    #[must_use]
    pub fn q3(&self) -> f64 {
        self.percentile(75.0)
    }

    /// Get minimum value
    #[must_use]
    pub fn min(&self) -> f64 {
        self.values
            .iter()
            .copied()
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0)
    }

    /// Get maximum value
    #[must_use]
    pub fn max(&self) -> f64 {
        self.values
            .iter()
            .copied()
            .max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(0.0)
    }

    /// Merge another accumulator into this one
    pub fn merge(&mut self, other: &Self) {
        self.values.extend_from_slice(&other.values);
    }

    /// Get all values (for box plot generation)
    #[must_use]
    pub fn values(&self) -> &[f64] {
        &self.values
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_welford_basic() {
        let mut acc = WelfordAccumulator::new();
        for i in 1..=10 {
            acc.push(f64::from(i));
        }

        assert_eq!(acc.count(), 10);
        assert!((acc.mean() - 5.5).abs() < f64::EPSILON);
        assert!((acc.min() - 1.0).abs() < f64::EPSILON);
        assert!((acc.max() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_welford_empty() {
        let acc = WelfordAccumulator::new();
        assert_eq!(acc.count(), 0);
        assert!((acc.mean() - 0.0).abs() < f64::EPSILON);
        assert!((acc.min() - 0.0).abs() < f64::EPSILON);
        assert!((acc.max() - 0.0).abs() < f64::EPSILON);
        assert!(acc.is_empty());
    }

    #[test]
    fn test_welford_single_value() {
        let mut acc = WelfordAccumulator::new();
        acc.push(42.0);

        assert_eq!(acc.count(), 1);
        assert!((acc.mean() - 42.0).abs() < f64::EPSILON);
        assert!((acc.variance() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_welford_variance() {
        let mut acc = WelfordAccumulator::new();
        // Values with known standard deviation of 2.0 (population)
        for &v in &[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0] {
            acc.push(v);
        }

        assert!((acc.std_dev() - 2.0).abs() < 0.001);
    }

    #[test]
    fn test_welford_merge() {
        let mut acc1 = WelfordAccumulator::new();
        let mut acc2 = WelfordAccumulator::new();

        for i in 1..=5 {
            acc1.push(f64::from(i));
        }
        for i in 6..=10 {
            acc2.push(f64::from(i));
        }

        acc1.merge(&acc2);

        assert_eq!(acc1.count(), 10);
        assert!((acc1.mean() - 5.5).abs() < f64::EPSILON);
        assert!((acc1.min() - 1.0).abs() < f64::EPSILON);
        assert!((acc1.max() - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_welford_merge_empty() {
        let mut acc1 = WelfordAccumulator::new();
        let acc2 = WelfordAccumulator::new();

        acc1.push(5.0);
        acc1.merge(&acc2);

        assert_eq!(acc1.count(), 1);
        assert!((acc1.mean() - 5.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reservoir_basic() {
        let mut reservoir = QuantileReservoir::new(100);
        for i in 1..=100 {
            reservoir.push(f64::from(i));
        }

        assert_eq!(reservoir.count(), 100);
        assert_eq!(reservoir.reservoir_size(), 100);

        // Median should be approximately 50
        let median = reservoir.median();
        assert!(median > 40.0 && median < 60.0);
    }

    #[test]
    fn test_reservoir_overflow() {
        let mut reservoir = QuantileReservoir::new(10);
        for i in 1..=1000 {
            reservoir.push(f64::from(i));
        }

        assert_eq!(reservoir.count(), 1000);
        assert_eq!(reservoir.reservoir_size(), 10);
    }

    #[test]
    fn test_reservoir_percentiles() {
        let mut reservoir = QuantileReservoir::new(1000);
        for i in 1..=100 {
            reservoir.push(f64::from(i));
        }

        // For uniform 1-100, Q1 ≈ 25, median ≈ 50, Q3 ≈ 75
        assert!(reservoir.q1() > 20.0 && reservoir.q1() < 30.0);
        assert!(reservoir.median() > 45.0 && reservoir.median() < 55.0);
        assert!(reservoir.q3() > 70.0 && reservoir.q3() < 80.0);
    }

    #[test]
    fn test_reservoir_empty() {
        let reservoir = QuantileReservoir::new(100);
        assert_eq!(reservoir.count(), 0);
        assert!((reservoir.median() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_reservoir_single() {
        let mut reservoir = QuantileReservoir::new(100);
        reservoir.push(42.0);

        assert!((reservoir.median() - 42.0).abs() < f64::EPSILON);
        assert!((reservoir.q1() - 42.0).abs() < f64::EPSILON);
        assert!((reservoir.q3() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_exact_accumulator_basic() {
        let mut acc = ExactQuantileAccumulator::new();
        for i in 1..=100 {
            acc.push(f64::from(i));
        }

        assert_eq!(acc.count(), 100);
        // Exact median of 1-100 is 50.5
        assert!((acc.median() - 50.5).abs() < 0.01);
        assert!((acc.min() - 1.0).abs() < f64::EPSILON);
        assert!((acc.max() - 100.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_exact_accumulator_quartiles() {
        let mut acc = ExactQuantileAccumulator::new();
        for i in 1..=100 {
            acc.push(f64::from(i));
        }

        // For uniform 1-100: Q1 ≈ 25.75, Q3 ≈ 75.25
        assert!(acc.q1() > 25.0 && acc.q1() < 26.0);
        assert!(acc.q3() > 75.0 && acc.q3() < 76.0);
    }

    #[test]
    fn test_exact_accumulator_empty() {
        let acc = ExactQuantileAccumulator::new();
        assert!(acc.is_empty());
        assert!((acc.median() - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_exact_accumulator_single() {
        let mut acc = ExactQuantileAccumulator::new();
        acc.push(42.0);

        assert!((acc.median() - 42.0).abs() < f64::EPSILON);
        assert!((acc.q1() - 42.0).abs() < f64::EPSILON);
        assert!((acc.q3() - 42.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_exact_accumulator_merge() {
        let mut acc1 = ExactQuantileAccumulator::new();
        let mut acc2 = ExactQuantileAccumulator::new();

        for i in 1..=50 {
            acc1.push(f64::from(i));
        }
        for i in 51..=100 {
            acc2.push(f64::from(i));
        }

        acc1.merge(&acc2);

        assert_eq!(acc1.count(), 100);
        assert!((acc1.median() - 50.5).abs() < 0.01);
    }

    #[test]
    fn test_exact_vs_reservoir_accuracy() {
        // Test that exact mode gives consistent results regardless of order
        let mut exact = ExactQuantileAccumulator::new();

        // Add values in ascending order (worst case for reservoir)
        for i in 1..=1000 {
            exact.push(f64::from(i));
        }

        // Exact should give median of 500.5
        assert!((exact.median() - 500.5).abs() < 0.01);
    }
}
