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

    /// Chain of courses that creates the longest delay (from start to end)
    pub longest_delay_chain: Vec<String>,

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

    /// Parse a user-supplied string into a [`PlanCategory`].
    ///
    /// Accepts the canonical `file_name` form (`"shortest"`,
    /// `"calc-ready-shortest"`, …), the `display_name` lowercased + dashed
    /// (`"shortest-path"`, `"calculus-ready-shortest"`), and the underscore
    /// variants (`"calc_ready_shortest"`, `"random_sample"`). Matching is
    /// case-insensitive. Returns `None` when the input matches no variant.
    #[must_use]
    pub fn from_user_input(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "shortest" | "shortest-path" => Some(Self::Shortest),
            "longest" | "longest-path" => Some(Self::Longest),
            "calc-ready-shortest" | "calculus-ready-shortest" | "calc_ready_shortest" => {
                Some(Self::CalcReadyShortest)
            }
            "sample" | "random-sample" | "random_sample" => Some(Self::RandomSample),
            _ => None,
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

    /// Patterns to detect calculus courses (regex-like: prefix contains these)
    pub calculus_patterns: Vec<String>,

    /// Seed for the reservoir-sample RNG. When `None`, the selector picks
    /// a non-deterministic seed from `fastrand`'s thread-local entropy (the
    /// legacy behaviour). Callers that want deterministic samples — e.g.
    /// `analyze_degree`'s default path — pass a seed derived from the input
    /// YAML so the same `(yaml, max_plans, include_courses)` tuple always
    /// produces the same Random Sample plan.
    pub random_seed: Option<u64>,
}

impl Default for PlanSelectorConfig {
    fn default() -> Self {
        Self {
            sample_count: 5,
            scheduler_config: SchedulerConfig::default(),
            // Common calculus course codes across institutions
            calculus_courses: vec![
                // NEU
                "MATH1341".to_string(),
                "MATH1342".to_string(),
                "MATH2321".to_string(),
                // CSU (Colorado State)
                "MATH155".to_string(),
                "MATH156".to_string(),
                "MATH160".to_string(),
                "MATH161".to_string(),
                // Generic patterns
                "CALC".to_string(),
            ],
            calculus_prereqs: vec![
                // NEU
                "MATH1120".to_string(),
                "MATH1215".to_string(),
                // CSU
                "MATH117".to_string(),
                "MATH118".to_string(),
                "MATH120".to_string(),
                "MATH124".to_string(),
                "MATH125".to_string(),
                "MATH126".to_string(),
                "MATH127".to_string(),
            ],
            // Patterns that indicate a calculus course (used for fuzzy matching)
            calculus_patterns: vec!["Calculus".to_string(), "CALC".to_string()],
            // Default to thread-local entropy so existing callers keep their
            // historical behaviour. The MCP analyze path overrides this with
            // a yaml-derived seed for deterministic reports.
            random_seed: None,
        }
    }
}

/// Selects and tracks special plans during plan enumeration
pub struct PlanSelector<'a> {
    /// Reference to school for course lookup
    school: &'a School,

    /// Configuration
    config: PlanSelectorConfig,

    /// Per-selector RNG. Seeded from `config.random_seed` when set,
    /// otherwise from `fastrand`'s thread-local entropy.
    rng: fastrand::Rng,

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
    ///
    /// Note: The DAG parameter is kept for API compatibility but is not used.
    /// Plan-specific DAGs are now passed to `process_plan` instead.
    #[must_use]
    pub fn new(school: &'a School, _dag: &'a DAG, config: PlanSelectorConfig) -> Self {
        let rng = config
            .random_seed
            .map_or_else(fastrand::Rng::new, fastrand::Rng::with_seed);
        Self {
            school,
            config,
            rng,
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
    /// The `plan_dag` should contain only courses in the plan with their
    /// resolved prerequisite edges.
    pub fn process_plan(
        &mut self,
        variant: &PlanVariant,
        course_metrics: &HashMap<String, CourseMetrics>,
        plan_dag: &DAG,
    ) {
        self.plans_seen += 1;

        // Score the plan using plan-specific DAG
        let scored = self.score_plan(variant.clone(), course_metrics.clone(), plan_dag);

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
                self.score_plan_calc_ready(variant.clone(), course_metrics.clone(), plan_dag);
            self.calc_ready_shortest = Some(calc_ready_scored);
        }

        // Reservoir sampling for random samples
        self.reservoir_sample(scored);
    }

    /// Score a plan by scheduling it and computing metrics
    ///
    /// Uses the plan-specific DAG for accurate term scheduling.
    fn score_plan(
        &self,
        variant: PlanVariant,
        course_metrics: HashMap<String, CourseMetrics>,
        plan_dag: &DAG,
    ) -> ScoredPlan {
        // Use plan-specific DAG for scheduling to ensure only courses in the plan
        // are considered for prerequisite ordering
        let scheduler =
            TermScheduler::new(self.school, plan_dag, self.config.scheduler_config.clone());
        let schedule = scheduler.schedule(&variant.courses);

        let total_complexity: usize = course_metrics.values().map(|m| m.complexity).sum();
        let longest_delay = course_metrics.values().map(|m| m.delay).max().unwrap_or(0);

        // Find the course(s) with the longest delay and trace back to find the chain
        // Use plan_dag for chain computation too
        let longest_delay_chain = self.compute_longest_delay_chain(
            &variant.courses,
            &course_metrics,
            longest_delay,
            plan_dag,
        );

        let score = PlanScore {
            terms_required: schedule.terms_used(),
            total_complexity,
            longest_delay,
            longest_delay_chain,
            is_calc_ready: false,
        };

        ScoredPlan {
            variant,
            score,
            schedule,
            course_metrics,
        }
    }

    /// Compute the chain of courses that creates the longest delay
    ///
    /// Traces forward from a course with high delay through its dependents
    /// to find the complete prerequisite chain that creates the longest path.
    /// The delay factor represents the longest path THROUGH a course, so courses
    /// at the START of long chains have high delay (they block many courses).
    #[allow(clippy::unused_self)]
    fn compute_longest_delay_chain(
        &self,
        courses: &[String],
        course_metrics: &HashMap<String, CourseMetrics>,
        longest_delay: usize,
        plan_dag: &DAG,
    ) -> Vec<String> {
        // Find course(s) with the longest delay
        let start_course = course_metrics
            .iter()
            .filter(|(_, m)| m.delay == longest_delay)
            .max_by_key(|(_, m)| m.blocking) // Prefer courses that block more (start of chain)
            .map(|(k, _)| k.clone());

        let Some(start) = start_course else {
            return Vec::new();
        };

        // Build set of courses in plan for filtering
        let plan_set: std::collections::HashSet<&str> =
            courses.iter().map(String::as_str).collect();

        // Trace forward through dependents to build the chain
        // Use plan_dag.dependents for accurate plan-specific edges
        let mut chain = vec![start.clone()];
        let mut current = start;

        while let Some(dependents) = plan_dag.get_dependents(&current) {
            // Find the dependent in this plan with the highest delay
            // (continues the longest path)
            let next = dependents
                .iter()
                .filter(|d| plan_set.contains(d.as_str()))
                .filter_map(|d| course_metrics.get(d).map(|m| (d, m.delay)))
                .max_by_key(|(_, delay)| *delay)
                .map(|(d, _)| d.clone());

            match next {
                Some(dependent) => {
                    chain.push(dependent.clone());
                    current = dependent;
                }
                None => break,
            }
        }

        chain
    }

    /// Score a plan assuming calculus readiness (skip calc prereqs)
    fn score_plan_calc_ready(
        &self,
        variant: PlanVariant,
        course_metrics: HashMap<String, CourseMetrics>,
        plan_dag: &DAG,
    ) -> ScoredPlan {
        // For calc-ready, we'd ideally modify the DAG to remove calc prereqs
        // For now, we use the standard scheduling but mark it as calc-ready
        let mut scored = self.score_plan(variant, course_metrics, plan_dag);
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
    ///
    /// Prefers plans with more terms required, using higher complexity as tiebreaker.
    fn should_update_longest(&self, scored: &ScoredPlan) -> bool {
        self.longest.as_ref().is_none_or(|current| {
            scored.score.is_longer_than(&current.score)
                || (scored.score.terms_required == current.score.terms_required
                    && scored.score.total_complexity > current.score.total_complexity)
        })
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
        // Check if any course matches calculus course codes or patterns
        variant.courses.iter().any(|course| {
            // Direct match with calculus courses
            self.config
                .calculus_courses
                .iter()
                .any(|calc| course.contains(calc))
                // Pattern match with calculus patterns (e.g., "Calculus" in course name)
                || self
                    .config
                    .calculus_patterns
                    .iter()
                    .any(|pattern| course.to_uppercase().contains(&pattern.to_uppercase()))
        })
    }

    /// Reservoir sampling using Algorithm R.
    ///
    /// Uses the per-selector `rng` so a seeded config produces deterministic
    /// samples across runs. Callers that want non-determinism leave
    /// `config.random_seed = None` and the selector falls back to fastrand's
    /// thread-local entropy.
    fn reservoir_sample(&mut self, scored: ScoredPlan) {
        if self.random_samples.len() < self.config.sample_count {
            // Reservoir not full, just add
            self.random_samples.push(scored);
        } else {
            // Reservoir full, replace with probability k/n
            let j = self.rng.usize(0..self.plans_seen);
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

    /// Consume selector and return owned plans.
    ///
    /// When the calc-ready shortest plan is structurally identical to the
    /// shortest plan — same score, same course set, same term schedule —
    /// drop the duplicate and record the fact in
    /// [`SelectedPlans::calc_ready_suppressed`]. Without this dedup, the
    /// MCP response surfaces "Shortest Path" and "Calculus-Ready Shortest"
    /// as separate entries that confuse reports because they're really one
    /// plan presented twice.
    #[must_use]
    pub fn into_selected_plans(self) -> SelectedPlans {
        let calc_ready_suppressed = match (&self.shortest, &self.calc_ready_shortest) {
            (Some(s), Some(c)) => plans_structurally_equal(s, c),
            _ => false,
        };
        let calc_ready_shortest = if calc_ready_suppressed {
            None
        } else {
            self.calc_ready_shortest
        };
        SelectedPlans {
            shortest: self.shortest,
            longest: self.longest,
            calc_ready_shortest,
            random_samples: self.random_samples,
            total_plans_seen: self.plans_seen,
            calc_ready_suppressed,
        }
    }
}

/// Two scored plans are "the same plan" for the dedup check when their
/// score (terms, complexity, longest delay) matches, their term schedule
/// is identical, and the course set is identical. The schedule check is
/// the strict signal — score equality alone would over-suppress.
fn plans_structurally_equal(a: &ScoredPlan, b: &ScoredPlan) -> bool {
    if a.score.terms_required != b.score.terms_required
        || a.score.total_complexity != b.score.total_complexity
        || a.score.longest_delay != b.score.longest_delay
    {
        return false;
    }
    if a.schedule.terms.len() != b.schedule.terms.len() {
        return false;
    }
    for (ta, tb) in a.schedule.terms.iter().zip(b.schedule.terms.iter()) {
        if ta.number != tb.number || ta.courses != tb.courses {
            return false;
        }
    }
    true
}

/// Collection of selected plans after processing
#[derive(Debug, Clone)]
pub struct SelectedPlans {
    /// Shortest path plan
    pub shortest: Option<ScoredPlan>,

    /// Longest path plan
    pub longest: Option<ScoredPlan>,

    /// Calculus-ready shortest plan. `None` when no calculus path exists in
    /// the program, or when it was structurally identical to the shortest
    /// path and dropped to avoid duplicate reporting — check
    /// `calc_ready_suppressed` to distinguish.
    pub calc_ready_shortest: Option<ScoredPlan>,

    /// Randomly sampled plans
    pub random_samples: Vec<ScoredPlan>,

    /// Total number of plans processed
    pub total_plans_seen: usize,

    /// `true` when a calc-ready candidate existed but matched the shortest
    /// path entry term-for-term. Callers can surface a note ("calc-ready
    /// suppressed as duplicate") instead of letting the duplicate confuse
    /// reports.
    pub calc_ready_suppressed: bool,
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
            longest_delay_chain: Vec::new(),
            is_calc_ready: false,
        };
        let score2 = PlanScore {
            terms_required: 9,
            total_complexity: 160,
            longest_delay: 7,
            longest_delay_chain: Vec::new(),
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
        selector.process_plan(&variant1, &metrics1, &dag);

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
            selector.process_plan(&variant, &metrics, &dag);
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
            calc_ready_suppressed: false,
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

    #[test]
    fn test_plans_structurally_equal_compares_scores_and_schedules() {
        let make_plan = |terms: usize, complexity: usize, term_courses: &[&str]| ScoredPlan {
            variant: create_test_variant(term_courses),
            score: PlanScore {
                terms_required: terms,
                total_complexity: complexity,
                longest_delay: 0,
                longest_delay_chain: vec![],
                is_calc_ready: false,
            },
            schedule: {
                use crate::core::report::term_scheduler::Term;
                TermPlan {
                    terms: vec![Term {
                        number: 1,
                        courses: term_courses.iter().map(|s| (*s).to_string()).collect(),
                        total_credits: 15.0,
                    }],
                    is_quarter_system: false,
                    target_credits: 15.0,
                    unscheduled: vec![],
                }
            },
            course_metrics: HashMap::new(),
        };

        let shortest = make_plan(8, 50, &["CS1000", "CS2000"]);
        let same = make_plan(8, 50, &["CS1000", "CS2000"]);
        let diff_score = make_plan(8, 51, &["CS1000", "CS2000"]);
        let diff_courses = make_plan(8, 50, &["CS1000", "CS3000"]);

        assert!(plans_structurally_equal(&shortest, &same));
        assert!(!plans_structurally_equal(&shortest, &diff_score));
        assert!(!plans_structurally_equal(&shortest, &diff_courses));
    }

    #[test]
    fn test_into_selected_plans_drops_duplicate_calc_ready() {
        // Two identical plans (same score, same schedule, same courses) —
        // calc-ready must be suppressed and the flag set.
        use crate::core::report::term_scheduler::Term;
        let make = |is_calc: bool| ScoredPlan {
            variant: create_test_variant(&["CS1000", "CS2000"]),
            score: PlanScore {
                terms_required: 6,
                total_complexity: 20,
                longest_delay: 2,
                longest_delay_chain: vec![],
                is_calc_ready: is_calc,
            },
            schedule: TermPlan {
                terms: vec![Term {
                    number: 1,
                    courses: vec!["CS1000".to_string(), "CS2000".to_string()],
                    total_credits: 8.0,
                }],
                is_quarter_system: false,
                target_credits: 8.0,
                unscheduled: vec![],
            },
            course_metrics: HashMap::new(),
        };

        let school = create_test_school();
        let dag = create_test_dag();
        let config = PlanSelectorConfig::default();
        let mut selector = PlanSelector::new(&school, &dag, config);
        selector.shortest = Some(make(false));
        selector.calc_ready_shortest = Some(make(true));
        let selected = selector.into_selected_plans();
        assert!(selected.calc_ready_suppressed);
        assert!(selected.calc_ready_shortest.is_none());
        assert!(selected.shortest.is_some());
    }
}
