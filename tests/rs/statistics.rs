//! Integration tests for statistics module
//!
//! Tests the statistics functionality in realistic scenarios that would
//! be used during degree analysis.

use nu_analytics::core::statistics::{
    strategy::{default_strategy, strategy_from_name, MeanStrategy, MedianStrategy},
    CalculationStrategy, DescriptiveStats,
};

/// Test that strategies can be selected by name from configuration
#[test]
fn test_strategy_selection_from_config() {
    // Simulate config values
    let config_values = vec!["median", "mean", "MEDIAN", "Mean"];

    for config_val in config_values {
        let strategy = strategy_from_name(config_val);
        assert!(
            strategy.is_some(),
            "Strategy '{config_val}' should be recognized"
        );
    }

    // Unknown strategy returns None
    assert!(strategy_from_name("unknown").is_none());
    assert!(strategy_from_name("average").is_none()); // Common mistake
}

/// Test statistics computation on realistic course metrics data
#[test]
fn test_statistics_on_course_metrics() {
    // Simulate complexity scores from multiple plans for a single course
    // These would come from computing metrics on different degree plans
    let complexity_scores: Vec<f64> = vec![
        5.0, 6.0, 6.0, 7.0, 7.0, 7.0, 8.0, 8.0, 9.0, 12.0, // Mostly 6-9, one outlier at 12
    ];

    let stats = DescriptiveStats::from_values(&complexity_scores);

    // Basic sanity checks
    assert_eq!(stats.count, 10);
    assert!((stats.min - 5.0).abs() < f64::EPSILON);
    assert!((stats.max - 12.0).abs() < f64::EPSILON);

    // Median should be around 7 (middle of sorted data)
    assert!(stats.median >= 7.0 && stats.median <= 7.5);

    // Mean should be pulled up by the outlier
    assert!(stats.mean > stats.median);

    // 12.0 should be detected as an outlier
    assert!(
        stats.is_outlier(12.0),
        "12.0 should be an outlier in this dataset"
    );
    assert!(
        !stats.is_outlier(7.0),
        "7.0 should not be an outlier in this dataset"
    );
}

/// Test that median and mean strategies give different results on skewed data
#[test]
fn test_strategy_differences_on_skewed_data() {
    // Highly skewed data - most values low, one very high
    let skewed_data = vec![1.0, 2.0, 2.0, 3.0, 3.0, 3.0, 4.0, 4.0, 5.0, 100.0];

    let median_strategy = MedianStrategy;
    let mean_strategy = MeanStrategy;

    let median_result = median_strategy.aggregate(&skewed_data);
    let mean_result = mean_strategy.aggregate(&skewed_data);

    // Median should be around 3.0 (unaffected by outlier)
    assert!(
        (3.0..=3.5).contains(&median_result),
        "Median was {median_result}, expected ~3.0"
    );

    // Mean should be much higher due to outlier
    assert!(
        mean_result > 10.0,
        "Mean was {mean_result}, expected >10 due to outlier"
    );

    // This demonstrates why median is the default for degree analysis
    assert!(mean_result > median_result * 3.0);
}

/// Test statistics on degree-level complexity sums
#[test]
fn test_degree_complexity_statistics() {
    // Simulate total complexity scores for different degree plans
    // Each value is the sum of all course complexities in one plan
    let degree_complexities: Vec<usize> = vec![
        145, 148, 150, 152, 155, 158, 160, 162, 165, 170, // Normal range
        180, 185, // Slightly higher plans
        220, // One outlier plan with many complex choices
    ];

    let stats = DescriptiveStats::from_usize_values(&degree_complexities);

    assert_eq!(stats.count, 13);

    // Check that we can identify the outlier plan
    assert!(stats.is_outlier(220.0));
    assert!(!stats.is_outlier(160.0));

    // IQR should be reasonable
    assert!(stats.iqr() > 0.0);

    // Standard deviation should reflect the spread
    assert!(stats.std_dev > 10.0);
}

/// Test that default strategy returns median
#[test]
fn test_default_strategy_is_median() {
    let strategy = default_strategy();
    assert_eq!(strategy.name(), "median");

    let values = vec![1.0, 2.0, 3.0, 4.0, 5.0];
    let result = strategy.aggregate(&values);

    // Median of [1,2,3,4,5] is 3
    assert!((result - 3.0).abs() < f64::EPSILON);
}

/// Test box plot data extraction
#[test]
fn test_box_plot_data_extraction() {
    let values: Vec<f64> = (1..=100).map(f64::from).collect();
    let stats = DescriptiveStats::from_values(&values);

    // For box plots we need: min, Q1, median, Q3, max
    assert!(stats.min < stats.q1);
    assert!(stats.q1 < stats.median);
    assert!(stats.median < stats.q3);
    assert!(stats.q3 < stats.max);

    // Also verify fences for whiskers
    assert!(stats.lower_fence() < stats.q1);
    assert!(stats.upper_fence() > stats.q3);
}

/// Test empty data handling
#[test]
fn test_empty_data_handling() {
    let empty: Vec<f64> = vec![];
    let stats = DescriptiveStats::from_values(&empty);

    assert_eq!(stats.count, 0);
    assert!((stats.min - 0.0).abs() < f64::EPSILON);
    assert!((stats.max - 0.0).abs() < f64::EPSILON);
    assert!((stats.median - 0.0).abs() < f64::EPSILON);

    // Strategies should also handle empty data
    let median = MedianStrategy.aggregate(&empty);
    let mean = MeanStrategy.aggregate(&empty);

    assert!((median - 0.0).abs() < f64::EPSILON);
    assert!((mean - 0.0).abs() < f64::EPSILON);
}

/// Test streaming aggregation matches batch computation
#[test]
fn test_streaming_vs_batch_statistics() {
    use nu_analytics::core::statistics::streaming::WelfordAccumulator;

    let values: Vec<f64> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];

    // Batch computation
    let batch_stats = DescriptiveStats::from_values(&values);

    // Streaming computation
    let mut streaming = WelfordAccumulator::new();
    for &v in &values {
        streaming.push(v);
    }

    // Results should match
    assert!((streaming.mean() - batch_stats.mean).abs() < 0.001);
    assert!((streaming.min() - batch_stats.min).abs() < f64::EPSILON);
    assert!((streaming.max() - batch_stats.max).abs() < f64::EPSILON);
    assert!((streaming.std_dev() - batch_stats.std_dev).abs() < 0.001);
}

/// Test metrics aggregation across simulated plans
#[test]
fn test_metrics_aggregation_integration() {
    use nu_analytics::core::metrics::CourseMetrics;
    use nu_analytics::core::statistics::{AggregatorConfig, MetricsAggregator};
    use std::collections::HashMap;

    let mut aggregator = MetricsAggregator::new(AggregatorConfig::default());

    // Simulate 50 different degree plans
    for plan_num in 1..=50 {
        let mut course_metrics = HashMap::new();

        // CS1000 appears in all plans with varying metrics
        course_metrics.insert(
            "CS1000".to_string(),
            CourseMetrics {
                complexity: 5 + (plan_num % 5),
                centrality: 3 + (plan_num % 3),
                delay: 2 + (plan_num % 4),
                blocking: 3 + (plan_num % 3),
            },
        );

        // CS2000 appears in all plans
        course_metrics.insert(
            "CS2000".to_string(),
            CourseMetrics {
                complexity: 8 + (plan_num % 4),
                centrality: 5 + (plan_num % 5),
                delay: 4 + (plan_num % 3),
                blocking: 4 + (plan_num % 4),
            },
        );

        // CS3000 appears in all plans (highest complexity)
        course_metrics.insert(
            "CS3000".to_string(),
            CourseMetrics {
                complexity: 12 + (plan_num % 6),
                centrality: 8 + (plan_num % 4),
                delay: 6 + (plan_num % 5),
                blocking: 6 + (plan_num % 6),
            },
        );

        aggregator.add_plan(&course_metrics, 12.0);
    }

    // Verify aggregation results
    assert_eq!(aggregator.plan_count(), 50);

    // Check degree-level stats
    let degree_stats = aggregator.degree_stats();
    assert_eq!(degree_stats.plan_count, 50);
    assert!(degree_stats.total_complexity.mean > 20.0); // Sum of 3 courses
    assert!(degree_stats.longest_delay.max >= 6.0); // At least CS3000's delay

    // Check course-level stats
    let cs1000_stats = aggregator
        .course_stats("CS1000")
        .expect("CS1000 should exist");
    assert_eq!(cs1000_stats.plan_count, 50);
    assert!(cs1000_stats.complexity.min >= 5.0);
    assert!(cs1000_stats.complexity.max <= 10.0);

    let cs3000_stats = aggregator
        .course_stats("CS3000")
        .expect("CS3000 should exist");
    assert!(cs3000_stats.complexity.mean > cs1000_stats.complexity.mean);

    // Verify all courses are tracked
    let course_ids = aggregator.course_ids();
    assert_eq!(course_ids.len(), 3);
}

/// Test reservoir sampling approximates true percentiles
#[test]
fn test_reservoir_percentile_accuracy() {
    use nu_analytics::core::statistics::streaming::QuantileReservoir;

    // Generate 10000 values
    let mut reservoir = QuantileReservoir::new(1000);
    for i in 1..=10000 {
        reservoir.push(f64::from(i));
    }

    // For uniform 1-10000:
    // Q1 ≈ 2500, Median ≈ 5000, Q3 ≈ 7500
    let q1 = reservoir.q1();
    let median = reservoir.median();
    let q3 = reservoir.q3();

    // Allow 10% error due to sampling
    assert!(q1 > 2000.0 && q1 < 3000.0, "Q1 was {q1}");
    assert!(median > 4500.0 && median < 5500.0, "Median was {median}");
    assert!(q3 > 7000.0 && q3 < 8000.0, "Q3 was {q3}");
}
