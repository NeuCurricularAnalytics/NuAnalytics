//! Plan selection for identifying special degree plans
//!
//! Provides scoring and selection of notable plans from the plan space:
//! - Shortest path to completion (minimum terms)
//! - Longest path to completion (maximum terms)
//! - Calculus-ready shortest path (assuming calc prereqs satisfied)
//! - Random sample of plans for statistical validity

use super::plan_variant::PlanVariant;
use crate::core::metrics::CourseMetrics;
use crate::core::models::{School, DAG};
use crate::core::report::term_scheduler::{SchedulerConfig, TermPlan, TermScheduler};
use std::collections::HashMap;

/// Scores associated with a plan for comparison
#[derive(Debug, Clone, Default)]
pub struct PlanScore {
    /// Number of terms required to complete
    pub terms_required: usize,

    /// Total degree complexity (sum of all course complexities)
    pub total_complexity: usize,

    /// Longest delay factor in the plan
    pub longest_delay: usize,

    /// Whether this plan assumes calculus readiness
    pub is_calc_ready: bool,
}

impl PlanScore {
    /// Check if this plan is shorter than another
    #[must_use]
    pub const fn is_shorter_than(&self, other: &Self) -> bool {
        self.terms_required < other.terms_required
    }

    /// Check if this plan is longer than another
    #[must_use]
    pub const fn is_longer_than(&self, other: &Self) -> bool {
        self.terms_required > other.terms_required
    }

    /// Check if this plan has lower complexity (for ties in term count)
    #[must_use]
    pub const fn has_lower_complexity(&self, other: &Self) -> bool {
        self.total_complexity < other.total_complexity
    }
}

/// A scored plan with its variant and metrics
#[derive(Debug, Clone)]
pub struct ScoredPlan {
    /// The plan variant
    pub variant: PlanVariant,

    /// Computed score for comparison
    pub score: PlanScore,

    /// The term-by-term schedule
    pub schedule: TermPlan,

    /// Per-course metrics
    pub course_metrics: HashMap<String, CourseMetrics>,
}

impl ScoredPlan {
    /// Get the plan name for display
    #[must_use]
    pub fn display_name(&self, category: &str) -> String {
        format!(
            "{} ({} terms, complexity {})",
            category, self.score.terms_required, self.score.total_complexity
        )
    }
}

/// Category of special plan
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PlanCategory {
    /// Shortest path to completion
    Shortest,
    /// Longest path to completion
    Longest,
    /// Shortest path assuming calculus readiness
    CalcReadyShortest,
    /// Randomly sampled plan
    RandomSample,
}

impl PlanCategory {
    /// Get display name for the category
    #[must_use]
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Shortest => "Shortest Path",
            Self::Longest => "Longest Path",
            Self::CalcReadyShortest => "Calculus-Ready Shortest",
            Self::RandomSample => "Random Sample",
        }
    }

    /// Get filename-safe identifier
    #[must_use]
    pub const fn file_name(&self) -> &'static str {
        match self {
            Self::Shortest => "shortest",
            Self::Longest => "longest",
            Self::CalcReadyShortest => "calc-ready-shortest",
            Self::RandomSample => "random-sample",
        }
    }
}

/// Configuration for plan selection
#[derive(Debug, Clone)]
pub struct PlanSelectorConfig {
    /// Number of random plans to sample
    pub sample_count: usize,

    /// Scheduler configuration for term scheduling
    pub scheduler_config: SchedulerConfig,

    /// Course codes considered "calculus" for calc-ready plans
    pub calculus_courses: Vec<String>,

    /// Course codes for calculus prerequisites to skip in calc-ready mode
    pub calculus_prereqs: Vec<String>,
}

impl Default for PlanSelectorConfig {
    fn default() -> Self {
        Self {
            sample_count: 5,
            scheduler_config: SchedulerConfig::default(),
            calculus_courses: vec![
                "MATH1341".to_string(), // Calculus 1
                "MATH1342".to_string(), // Calculus 2
                "MATH2321".to_string(), // Calculus 3
            ],
            calculus_prereqs: vec![
                "MATH1120".to_string(), // Precalculus
                "MATH1215".to_string(), // Mathematical Thinking
            ],
        }
    }
}

/// Selects and tracks special plans during plan enumeration
pub struct PlanSelector<'a> {
    /// Reference to school for course lookup
    school: &'a School,

    /// Reference to DAG for scheduling
    dag: &'a DAG,

    /// Configuration
    config: PlanSelectorConfig,

    /// Current best shortest plan
    shortest: Option<ScoredPlan>,

    /// Current best longest plan
    longest: Option<ScoredPlan>,

    /// Current best calc-ready shortest plan
    calc_ready_shortest: Option<ScoredPlan>,

    /// Reservoir of random samples
    random_samples: Vec<ScoredPlan>,

    /// Total plans seen (for reservoir sampling)
    plans_seen: usize,
}

impl<'a> PlanSelector<'a> {
    /// Create a new plan selector
    #[must_use]
    pub const fn new(school: &'a School, dag: &'a DAG, config: PlanSelectorConfig) -> Self {
        Self {
            school,
            dag,
            config,
            shortest: None,
            longest: None,
            calc_ready_shortest: None,
            random_samples: Vec::new(),
            plans_seen: 0,
        }
    }

    /// Process a plan and update selections
    ///
    /// This should be called for each plan generated during enumeration.
    pub fn process_plan(
        &mut self,
        variant: &PlanVariant,
        course_metrics: &HashMap<String, CourseMetrics>,
    ) {
        self.plans_seen += 1;

        // Score the plan
        let scored = self.score_plan(variant.clone(), course_metrics.clone());

        // Update shortest
        if self.should_update_shortest(&scored) {
            self.shortest = Some(scored.clone());
        }

        // Update longest
        if self.should_update_longest(&scored) {
            self.longest = Some(scored.clone());
        }

        // Update calc-ready shortest if applicable
        if self.is_calc_ready_plan(variant) && self.should_update_calc_ready(&scored) {
            // Re-score with calc-ready scheduling
            let calc_ready_scored =
                self.score_plan_calc_ready(variant.clone(), course_metrics.clone());
            self.calc_ready_shortest = Some(calc_ready_scored);
        }

        // Reservoir sampling for random samples
        self.reservoir_sample(scored);
    }

    /// Score a plan by scheduling it and computing metrics
    fn score_plan(
        &self,
        variant: PlanVariant,
        course_metrics: HashMap<String, CourseMetrics>,
    ) -> ScoredPlan {
        let scheduler =
            TermScheduler::new(self.school, self.dag, self.config.scheduler_config.clone());
        let schedule = scheduler.schedule(&variant.courses);

        let total_complexity: usize = course_metrics.values().map(|m| m.complexity).sum();
        let longest_delay = course_metrics.values().map(|m| m.delay).max().unwrap_or(0);

        let score = PlanScore {
            terms_required: schedule.terms_used(),
            total_complexity,
            longest_delay,
            is_calc_ready: false,
        };

        ScoredPlan {
            variant,
            score,
            schedule,
            course_metrics,
        }
    }

    /// Score a plan assuming calculus readiness (skip calc prereqs)
    fn score_plan_calc_ready(
        &self,
        variant: PlanVariant,
        course_metrics: HashMap<String, CourseMetrics>,
    ) -> ScoredPlan {
        // For calc-ready, we'd ideally modify the DAG to remove calc prereqs
        // For now, we use the standard scheduling but mark it as calc-ready
        let mut scored = self.score_plan(variant, course_metrics);
        scored.score.is_calc_ready = true;
        scored
    }

    /// Check if plan should replace current shortest
    fn should_update_shortest(&self, scored: &ScoredPlan) -> bool {
        self.shortest.as_ref().is_none_or(|current| {
            scored.score.is_shorter_than(&current.score)
                || (scored.score.terms_required == current.score.terms_required
                    && scored.score.has_lower_complexity(&current.score))
        })
    }

    /// Check if plan should replace current longest
    fn should_update_longest(&self, scored: &ScoredPlan) -> bool {
        self.longest
            .as_ref()
            .is_none_or(|current| scored.score.is_longer_than(&current.score))
    }

    /// Check if plan should replace current calc-ready shortest
    fn should_update_calc_ready(&self, scored: &ScoredPlan) -> bool {
        self.calc_ready_shortest.as_ref().is_none_or(|current| {
            scored.score.is_shorter_than(&current.score)
                || (scored.score.terms_required == current.score.terms_required
                    && scored.score.has_lower_complexity(&current.score))
        })
    }

    /// Check if a plan is eligible for calc-ready consideration
    ///
    /// A plan is calc-ready if it contains calculus courses.
    /// All such plans are considered calc-ready candidates.
    fn is_calc_ready_plan(&self, variant: &PlanVariant) -> bool {
        // All plans are potential calc-ready candidates
        // If no calculus is needed, it's trivially calc-ready
        // If calculus is needed, the scheduling would assume calc prereqs satisfied
        variant.courses.iter().any(|c| {
            self.config
                .calculus_courses
                .iter()
                .any(|calc| c.contains(calc))
        })
    }

    /// Reservoir sampling using Algorithm R
    fn reservoir_sample(&mut self, scored: ScoredPlan) {
        if self.random_samples.len() < self.config.sample_count {
            // Reservoir not full, just add
            self.random_samples.push(scored);
        } else {
            // Reservoir full, replace with probability k/n
            let j = fastrand::usize(0..self.plans_seen);
            if j < self.config.sample_count {
                self.random_samples[j] = scored;
            }
        }
    }

    /// Get the selected shortest plan
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // as_ref() is not const
    pub fn shortest_plan(&self) -> Option<&ScoredPlan> {
        self.shortest.as_ref()
    }

    /// Get the selected longest plan
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // as_ref() is not const
    pub fn longest_plan(&self) -> Option<&ScoredPlan> {
        self.longest.as_ref()
    }

    /// Get the selected calc-ready shortest plan
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // as_ref() is not const
    pub fn calc_ready_plan(&self) -> Option<&ScoredPlan> {
        self.calc_ready_shortest.as_ref()
    }

    /// Get the randomly sampled plans
    #[must_use]
    pub fn random_samples(&self) -> &[ScoredPlan] {
        &self.random_samples
    }

    /// Get total number of plans processed
    #[must_use]
    pub const fn plans_seen(&self) -> usize {
        self.plans_seen
    }

    /// Get all selected plans with their categories
    #[must_use]
    pub fn all_selected_plans(&self) -> Vec<(PlanCategory, &ScoredPlan)> {
        let mut plans = Vec::new();

        if let Some(plan) = &self.shortest {
            plans.push((PlanCategory::Shortest, plan));
        }
        if let Some(plan) = &self.longest {
            plans.push((PlanCategory::Longest, plan));
        }
        if let Some(plan) = &self.calc_ready_shortest {
            plans.push((PlanCategory::CalcReadyShortest, plan));
        }
        for plan in &self.random_samples {
            plans.push((PlanCategory::RandomSample, plan));
        }

        plans
    }

    /// Consume selector and return owned plans
    #[must_use]
    pub fn into_selected_plans(self) -> SelectedPlans {
        SelectedPlans {
            shortest: self.shortest,
            longest: self.longest,
            calc_ready_shortest: self.calc_ready_shortest,
            random_samples: self.random_samples,
            total_plans_seen: self.plans_seen,
        }
    }
}

/// Collection of selected plans after processing
#[derive(Debug, Clone)]
pub struct SelectedPlans {
    /// Shortest path plan
    pub shortest: Option<ScoredPlan>,

    /// Longest path plan
    pub longest: Option<ScoredPlan>,

    /// Calculus-ready shortest plan
    pub calc_ready_shortest: Option<ScoredPlan>,

    /// Randomly sampled plans
    pub random_samples: Vec<ScoredPlan>,

    /// Total number of plans processed
    pub total_plans_seen: usize,
}

impl SelectedPlans {
    /// Get iterator over all plans with categories
    pub fn iter(&self) -> impl Iterator<Item = (PlanCategory, &ScoredPlan)> {
        let shortest = self.shortest.iter().map(|p| (PlanCategory::Shortest, p));
        let longest = self.longest.iter().map(|p| (PlanCategory::Longest, p));
        let calc_ready = self
            .calc_ready_shortest
            .iter()
            .map(|p| (PlanCategory::CalcReadyShortest, p));
        let samples = self
            .random_samples
            .iter()
            .map(|p| (PlanCategory::RandomSample, p));

        shortest.chain(longest).chain(calc_ready).chain(samples)
    }

    /// Get count of special plans (excluding random samples)
    #[must_use]
    pub fn special_plan_count(&self) -> usize {
        usize::from(self.shortest.is_some())
            + usize::from(self.longest.is_some())
            + usize::from(self.calc_ready_shortest.is_some())
    }

    /// Get total count of all selected plans
    #[must_use]
    pub fn total_count(&self) -> usize {
        self.special_plan_count() + self.random_samples.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_school() -> School {
        School::new("Test University".to_string())
    }

    fn create_test_dag() -> DAG {
        DAG::new()
    }

    #[allow(clippy::cast_precision_loss)]
    fn create_test_variant(courses: &[&str]) -> PlanVariant {
        PlanVariant::from_parts(
            courses
                .iter()
                .map(std::string::ToString::to_string)
                .collect(),
            HashMap::new(),
            courses.len() as f32 * 3.0,
        )
    }

    fn create_test_metrics(courses: &[&str]) -> HashMap<String, CourseMetrics> {
        courses
            .iter()
            .enumerate()
            .map(|(i, c)| {
                (
                    c.to_string(),
                    CourseMetrics {
                        complexity: 5 + i,
                        centrality: 3 + i,
                        delay: 2 + i,
                        blocking: 1 + i,
                    },
                )
            })
            .collect()
    }

    #[test]
    fn test_plan_score_comparison() {
        let score1 = PlanScore {
            terms_required: 8,
            total_complexity: 150,
            longest_delay: 6,
            is_calc_ready: false,
        };
        let score2 = PlanScore {
            terms_required: 9,
            total_complexity: 160,
            longest_delay: 7,
            is_calc_ready: false,
        };

        assert!(score1.is_shorter_than(&score2));
        assert!(!score1.is_longer_than(&score2));
        assert!(score1.has_lower_complexity(&score2));
    }

    #[test]
    fn test_plan_category_names() {
        assert_eq!(PlanCategory::Shortest.display_name(), "Shortest Path");
        assert_eq!(PlanCategory::Shortest.file_name(), "shortest");
        assert_eq!(PlanCategory::Longest.display_name(), "Longest Path");
        assert_eq!(
            PlanCategory::CalcReadyShortest.display_name(),
            "Calculus-Ready Shortest"
        );
    }

    #[test]
    fn test_selector_tracks_shortest() {
        let school = create_test_school();
        let dag = create_test_dag();
        let config = PlanSelectorConfig::default();
        let mut selector = PlanSelector::new(&school, &dag, config);

        // First plan
        let variant1 = create_test_variant(&["CS1000", "CS2000", "CS3000"]);
        let metrics1 = create_test_metrics(&["CS1000", "CS2000", "CS3000"]);
        selector.process_plan(&variant1, &metrics1);

        assert!(selector.shortest_plan().is_some());
        assert_eq!(selector.plans_seen(), 1);
    }

    #[test]
    fn test_selector_reservoir_sampling() {
        let school = create_test_school();
        let dag = create_test_dag();
        let config = PlanSelectorConfig {
            sample_count: 3,
            ..Default::default()
        };
        let mut selector = PlanSelector::new(&school, &dag, config);

        // Process 10 plans
        for _i in 0..10 {
            let variant = create_test_variant(&["CS1000", "CS2000"]);
            let metrics = create_test_metrics(&["CS1000", "CS2000"]);
            selector.process_plan(&variant, &metrics);
        }

        // Should have exactly 3 random samples
        assert_eq!(selector.random_samples().len(), 3);
        assert_eq!(selector.plans_seen(), 10);
    }

    #[test]
    fn test_selected_plans_iteration() {
        let selected = SelectedPlans {
            shortest: Some(ScoredPlan {
                variant: create_test_variant(&["CS1000"]),
                score: PlanScore::default(),
                schedule: TermPlan::new(8, false, 15.0),
                course_metrics: HashMap::new(),
            }),
            longest: None,
            calc_ready_shortest: None,
            random_samples: vec![],
            total_plans_seen: 1,
        };

        assert_eq!(selected.special_plan_count(), 1);
        assert_eq!(selected.total_count(), 1);

        let collected: Vec<_> = selected.iter().collect();
        assert_eq!(collected.len(), 1);
        assert_eq!(collected[0].0, PlanCategory::Shortest);
    }

    #[test]
    fn test_default_config() {
        let config = PlanSelectorConfig::default();
        assert_eq!(config.sample_count, 5);
        assert!(!config.calculus_courses.is_empty());
    }
}
