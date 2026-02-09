//! Statistics module for degree analysis
//!
//! Provides calculation strategies, descriptive statistics, and streaming
//! aggregation for collecting metrics across multiple degree plans.

pub mod aggregator;
pub mod descriptive;
pub mod strategy;
pub mod streaming;

pub use aggregator::{
    new_shared_aggregator, AggregatedCourseStats, AggregatedDegreeStats, AggregatorConfig,
    CourseAggregator, MetricStats, MetricsAggregator, SharedAggregator,
};
pub use descriptive::DescriptiveStats;
pub use strategy::{CalculationStrategy, MeanStrategy, MedianStrategy};
