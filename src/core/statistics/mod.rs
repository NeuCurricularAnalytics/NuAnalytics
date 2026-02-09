//! Statistics module for degree analysis
//!
//! Provides calculation strategies and descriptive statistics for
//! aggregating metrics across multiple degree plans.

pub mod descriptive;
pub mod strategy;

pub use descriptive::DescriptiveStats;
pub use strategy::{CalculationStrategy, MeanStrategy, MedianStrategy};
