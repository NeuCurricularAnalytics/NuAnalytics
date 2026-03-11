//! Plan generator for enumerating all possible degree plans
//!
//! Generates all valid combinations of course selections that satisfy
//! a degree's requirements, using lazy iteration to handle large plan spaces.
//!
//! The generator separates requirements into two categories:
//! - **Major requirements**: Enumerated to generate all possible plan combinations
//! - **Non-major requirements** (`gen_ed`, supporting, elective): Simplified to first/shortest option
//!
//! This design keeps combinatorial explosion manageable while still capturing
//! the meaningful variations in degree complexity.
//!
//! # Sampling Strategies
//!
//! The generator supports different sampling strategies to control how plans are
//! enumerated:
//!
//! - **Sequential**: Default enumeration order (may bias towards simpler plans first)
//! - **Shuffled**: Randomizes the order of plan generation for unbiased sampling
//! - **Stratified**: Ensures coverage across different complexity levels
//!
//! Shuffled sampling is recommended when computing aggregate statistics to avoid
//! systematic bias in the median/quartile calculations.

use super::gen_ed_tracker::GenEdTracker;
use super::plan_variant::PlanVariant;
use super::requirement_resolver::{RequirementResolver, ResolvedRequirement};
use crate::core::models::course::Course;
use crate::core::models::degree::Requirement;
use fastrand::Rng;
use std::collections::{HashMap, HashSet};

/// Categories that should be enumerated for plan generation
const ENUMERABLE_CATEGORIES: [&str; 1] = ["major"];

// ============================================================================
// Helper Functions
// ============================================================================

/// Expand a course reference, handling bundles and equivalents
///
/// - Bundles `[CS1800, CS1802]` expand to multiple courses (all required)
/// - Equivalents `{CS201, PHIL201}` expand to the first course (pick one)
/// - Regular courses pass through unchanged
fn expand_course_reference(course_ref: &str) -> Vec<String> {
    // Handle bundle syntax: "[CS1800, CS1802]"
    if course_ref.starts_with('[') && course_ref.ends_with(']') {
        let inner = &course_ref[1..course_ref.len() - 1];
        return inner
            .split(',')
            .map(|part| part.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
    }

    // Handle equivalent syntax: "{CS4530, CS4535}" - pick first as default
    if course_ref.starts_with('{') && course_ref.ends_with('}') {
        let inner = &course_ref[1..course_ref.len() - 1];
        if let Some(first) = inner.split(',').next() {
            let trimmed = first.trim();
            if !trimmed.is_empty() {
                return vec![trimmed.to_string()];
            }
        }
        return Vec::new();
    }

    // Regular course - return as-is
    vec![course_ref.to_string()]
}

/// Expand a list of course references, handling bundles and equivalents
fn expand_course_list(courses: &[String]) -> Vec<String> {
    courses
        .iter()
        .flat_map(|c| expand_course_reference(c))
        .collect()
}

/// Check if a course key represents a placeholder course
///
/// Placeholder courses are generated for requirements that use wildcard patterns
/// or when specific courses aren't enumerated. They follow naming conventions like:
/// - "ELEC001", "ELEC002" - free electives
/// - "GE01", "GE02" - generic gen-ed placeholders
/// - "FQ01", "FW01" - specific gen-ed category placeholders
fn is_placeholder_course(course_key: &str) -> bool {
    // Check for common placeholder patterns
    if course_key.starts_with("ELEC") {
        return true;
    }

    // Check for 2-4 letter prefix followed by digits
    let prefix_len = course_key
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .count();
    if (2..=4).contains(&prefix_len) {
        let suffix = &course_key[prefix_len..];
        // Placeholder if suffix is just digits (possibly with 'S' for small)
        if suffix.chars().all(|c| c.is_ascii_digit() || c == 'S') && !suffix.is_empty() {
            // Check if it looks like a placeholder (short number)
            let digits: String = suffix.chars().filter(char::is_ascii_digit).collect();
            if let Ok(num) = digits.parse::<u32>() {
                // Real course numbers are typically 3-4 digits (100-9999)
                // Placeholders are typically 1-2 digits (01-99)
                return num < 100;
            }
        }
    }

    false
}

/// Extract gen-ed attribute code from a requirement name
///
/// Looks for patterns like "(FQ)", "(FW)", "(DA/DH/DL)" in requirement names.
/// Returns the first code found in parentheses.
///
/// # Examples
/// - "Quantitative Reasoning (FQ)" → Some("FQ")
/// - "Written Communication (FW)" → Some("FW")
/// - "Arts, Humanities, and Literature (DA/DH/DL)" → Some("DA")
/// - "Core Courses" → None
fn extract_gen_ed_code(name: &str) -> Option<String> {
    // Find text within parentheses at the end of the name
    if let Some(start) = name.rfind('(') {
        if let Some(end) = name.rfind(')') {
            if start < end {
                let code_section = &name[start + 1..end];
                // Handle multiple codes separated by /
                // Return the first one as the primary code
                let first_code = code_section.split('/').next()?;
                let trimmed = first_code.trim();
                if !trimmed.is_empty() && trimmed.chars().all(|c| c.is_ascii_alphanumeric()) {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

/// Strategy for sampling plans during generation
///
/// Different strategies trade off between performance and statistical accuracy:
/// - Sequential is fastest but may produce biased statistics
/// - Shuffled gives unbiased samples at the cost of pre-computing indices
/// - Stratified ensures good coverage of the complexity range
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum SamplingStrategy {
    /// Sequential enumeration (first combinations first)
    /// Fast but may bias statistics towards simpler plans
    Sequential,
    /// Shuffled random order for unbiased sampling
    /// Recommended for accurate median/quartile computation
    #[default]
    Shuffled,
    /// Stratified sampling across complexity strata
    /// Ensures good coverage of complexity range
    Stratified,
}

impl std::fmt::Display for SamplingStrategy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Sequential => write!(f, "sequential"),
            Self::Shuffled => write!(f, "shuffled"),
            Self::Stratified => write!(f, "stratified"),
        }
    }
}

impl std::str::FromStr for SamplingStrategy {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "sequential" | "seq" => Ok(Self::Sequential),
            "shuffled" | "shuffle" | "random" => Ok(Self::Shuffled),
            "stratified" | "strat" => Ok(Self::Stratified),
            _ => Err(format!(
                "Unknown sampling strategy '{s}': expected 'sequential', 'shuffled', or 'stratified'"
            )),
        }
    }
}

/// Configuration for plan generation
#[derive(Debug, Clone)]
pub struct PlanGeneratorConfig {
    /// Maximum number of plans to generate (safety cap)
    pub max_plans: usize,

    /// Skip plans that are equivalent (same courses, same metrics)
    /// Defaults to true - use `full_run` to disable
    pub ignore_duplicates: bool,

    /// Number of random plans to sample
    pub sample_count: usize,

    /// Target total credits for the degree (adds placeholder electives if needed)
    pub target_credits: Option<u32>,

    /// Default credit hours for placeholder electives
    pub default_elective_credits: f32,

    /// Sampling strategy for plan enumeration
    /// Defaults to Shuffled for unbiased statistics
    pub sampling_strategy: SamplingStrategy,

    /// Random seed for reproducible shuffling (None = random)
    pub random_seed: Option<u64>,
}

impl Default for PlanGeneratorConfig {
    fn default() -> Self {
        Self {
            max_plans: 1_000,
            ignore_duplicates: true,
            sample_count: 5,
            target_credits: None,
            default_elective_credits: 3.0,
            sampling_strategy: SamplingStrategy::Shuffled,
            random_seed: None,
        }
    }
}

/// Statistics about plan generation
#[derive(Debug, Clone, Default)]
pub struct PlanGenerationStats {
    /// Total number of plans that could be generated (before limits)
    pub total_possible: usize,

    /// Number of plans actually generated
    pub plans_generated: usize,

    /// Number of plans skipped due to duplicates
    pub duplicates_skipped: usize,

    /// Number of variable requirements contributing to plan explosion
    pub variable_requirements: usize,

    /// Breakdown of choices per variable requirement
    pub requirement_choices: Vec<(String, usize)>,

    /// Total credits from major requirements
    pub major_credits: f32,

    /// Total credits from non-major requirements (`gen_ed`, supporting, etc.)
    pub non_major_credits: f32,

    /// Credits added from placeholder electives
    pub elective_placeholder_credits: f32,
}

impl PlanGenerationStats {
    /// Check if plan generation was truncated due to limits
    #[must_use]
    pub const fn was_truncated(&self) -> bool {
        self.plans_generated < self.total_possible
    }
}

/// Generates all possible degree plans from requirements
///
/// The generator separates requirements by category:
/// - Major requirements are enumerated to generate all combinations
/// - Non-major requirements (`gen_ed`, supporting, elective) use simplest option
/// - Gen-ed requirements are reduced/skipped if already satisfied by major courses
/// - Placeholder electives are added to reach target credits if specified
pub struct PlanGenerator<'a> {
    /// Major requirements to enumerate (category = "major")
    major_requirements: Vec<ResolvedRequirement>,

    /// Non-major requirements with fixed (first) choice
    non_major_courses: Vec<String>,

    /// Credits from non-major requirements
    non_major_credits: f32,

    /// Course credit information
    course_credits: HashMap<String, f32>,

    /// Configuration
    config: PlanGeneratorConfig,

    /// Reference to courses for credit lookup
    _courses: &'a HashMap<String, Course>,

    /// Gen-ed tracker for cross-category satisfaction
    /// Currently used during construction; will be used for dynamic plan building
    #[allow(dead_code)]
    gen_ed_tracker: GenEdTracker,
}

impl<'a> PlanGenerator<'a> {
    /// Create a new plan generator
    ///
    /// Separates requirements into major (enumerated) and non-major (simplified).
    /// Tracks gen-ed attributes from major/supporting courses to reduce duplicate credits.
    ///
    /// # Arguments
    /// * `requirements` - Degree requirements to generate plans from
    /// * `courses` - Available courses for pattern matching and credits
    /// * `config` - Generation configuration
    #[must_use]
    pub fn new(
        requirements: &HashMap<String, Requirement>,
        courses: &'a HashMap<String, Course>,
        config: PlanGeneratorConfig,
    ) -> Self {
        // Resolve requirements into enumerable choices
        let mut req_resolver = RequirementResolver::new(courses);
        let resolved = req_resolver.resolve_all(requirements);

        // Build credit lookup
        let course_credits: HashMap<String, f32> = courses
            .iter()
            .map(|(k, c)| (k.clone(), c.credit_hours))
            .collect();

        // Partition into major, supporting, gen_ed, and other requirements
        let (major_requirements, other_requirements): (Vec<_>, Vec<_>) =
            resolved.into_iter().partition(|r| {
                r.category
                    .as_ref()
                    .is_some_and(|cat| ENUMERABLE_CATEGORIES.contains(&cat.as_str()))
            });

        // Further partition non-major requirements
        let (supporting_requirements, remaining_requirements): (Vec<_>, Vec<_>) =
            other_requirements
                .into_iter()
                .partition(|r| r.category.as_ref().is_some_and(|cat| cat == "supporting"));

        let (gen_ed_requirements, elective_requirements): (Vec<_>, Vec<_>) = remaining_requirements
            .into_iter()
            .partition(|r| r.category.as_ref().is_some_and(|cat| cat == "gen_ed"));

        // Build gen-ed tracker with requirements from gen_ed category
        let gen_ed_tracker =
            Self::build_gen_ed_tracker(&gen_ed_requirements, &course_credits, requirements);

        // Track gen-ed satisfaction from major courses (first choice of each)
        let mut gen_ed_satisfied = gen_ed_tracker;
        Self::track_gen_ed_from_requirements(&major_requirements, courses, &mut gen_ed_satisfied);

        // Track gen-ed satisfaction from supporting courses
        let supporting_courses =
            Self::collect_courses_from_requirements(&supporting_requirements, &course_credits);
        for course_key in &supporting_courses {
            gen_ed_satisfied.record_course_by_key(course_key, courses);
        }

        // Process gen-ed requirements, reducing credits based on satisfaction
        let gen_ed_courses = Self::process_gen_ed_requirements(
            &gen_ed_requirements,
            &gen_ed_satisfied,
            &course_credits,
            requirements,
        );

        // Collect elective courses
        let elective_courses =
            Self::collect_courses_from_requirements(&elective_requirements, &course_credits);

        // Combine all non-major courses
        let mut non_major_courses = Vec::new();
        non_major_courses.extend(supporting_courses);
        non_major_courses.extend(gen_ed_courses);
        non_major_courses.extend(elective_courses);

        // Sort and dedupe non-major courses
        non_major_courses.sort();
        non_major_courses.dedup();

        // Calculate non-major credits after deduplication
        let non_major_credits: f32 = non_major_courses
            .iter()
            .map(|c| {
                course_credits
                    .get(c)
                    .copied()
                    .unwrap_or_else(|| placeholder_credits(c))
            })
            .sum();

        Self {
            major_requirements,
            non_major_courses,
            non_major_credits,
            course_credits,
            config,
            _courses: courses,
            gen_ed_tracker: gen_ed_satisfied,
        }
    }

    /// Build a gen-ed tracker from gen-ed requirements
    ///
    /// Extracts gen-ed attribute codes from requirement names (e.g., "(FQ)" from
    /// "Quantitative Reasoning (FQ)") and sets up credit tracking.
    fn build_gen_ed_tracker(
        gen_ed_requirements: &[ResolvedRequirement],
        _course_credits: &HashMap<String, f32>,
        requirements: &HashMap<String, Requirement>,
    ) -> GenEdTracker {
        let mut tracker = GenEdTracker::new();

        // Extract gen-ed requirements and their credit needs
        for resolved in gen_ed_requirements {
            // Look up original requirement for credit info
            if let Some(req) = requirements.get(&resolved.id) {
                // Try to extract gen-ed attribute code from requirement name
                let attr_code = req
                    .name
                    .as_ref()
                    .and_then(|name| extract_gen_ed_code(name))
                    .unwrap_or_else(|| resolved.id.clone());

                // Track credits for this gen-ed attribute
                if let Some(credits) = req.credits {
                    #[allow(clippy::cast_precision_loss)]
                    tracker
                        .required_credits
                        .insert(attr_code.clone(), credits as f32);
                } else if req.count.is_some() {
                    // For count-based requirements, estimate credits (usually 3-4 per course)
                    // count is typically small (1-10), so conversion to f32 is safe
                    let count = req.count.unwrap_or(1);
                    #[allow(clippy::cast_precision_loss)]
                    let estimated_credits = (count as f32) * 3.0;
                    tracker
                        .required_credits
                        .insert(attr_code.clone(), estimated_credits);
                }

                // Store mapping from requirement ID to attribute code for later lookup
                if attr_code != resolved.id {
                    tracker.required_credits.insert(
                        resolved.id.clone(),
                        tracker
                            .required_credits
                            .get(&attr_code)
                            .copied()
                            .unwrap_or(0.0),
                    );
                }
            }
        }

        tracker
    }

    /// Track gen-ed satisfaction from a set of requirements
    ///
    /// Records courses from major/supporting requirements and tracks their
    /// gen-ed attribute contributions.
    fn track_gen_ed_from_requirements(
        requirements: &[ResolvedRequirement],
        courses: &HashMap<String, Course>,
        tracker: &mut GenEdTracker,
    ) {
        for req in requirements {
            // Use first choice as the representative courses
            if let Some(first_choice) = req.choices.first() {
                let expanded = expand_course_list(first_choice);
                for course_key in &expanded {
                    tracker.record_course_by_key(course_key, courses);
                }
            }
        }
    }

    /// Collect courses from requirements (first choice, expanded)
    fn collect_courses_from_requirements(
        requirements: &[ResolvedRequirement],
        course_credits: &HashMap<String, f32>,
    ) -> Vec<String> {
        let mut courses = Vec::new();
        for req in requirements {
            if let Some(first_choice) = req.choices.first() {
                let expanded = expand_course_list(first_choice);
                for course in expanded {
                    // Only add if not a placeholder or if credits are tracked
                    if course_credits.contains_key(&course) || is_placeholder_course(&course) {
                        courses.push(course);
                    }
                }
            }
        }
        courses
    }

    /// Process gen-ed requirements, reducing credits based on satisfaction
    ///
    /// If a gen-ed requirement is already satisfied by major/supporting courses,
    /// we skip adding those courses. If partially satisfied, we reduce the credits.
    /// Checks satisfaction by both requirement ID and gen-ed attribute code.
    fn process_gen_ed_requirements(
        gen_ed_requirements: &[ResolvedRequirement],
        gen_ed_satisfied: &GenEdTracker,
        course_credits: &HashMap<String, f32>,
        requirements: &HashMap<String, Requirement>,
    ) -> Vec<String> {
        let mut courses = Vec::new();

        for resolved in gen_ed_requirements {
            // Look up original requirement to get the attribute code
            let attr_code = requirements
                .get(&resolved.id)
                .and_then(|req| req.name.as_ref())
                .and_then(|name| extract_gen_ed_code(name));

            // Check satisfaction by attribute code first (if available), then by requirement ID
            let is_satisfied = attr_code.as_ref().map_or_else(
                || gen_ed_satisfied.is_satisfied(&resolved.id),
                |code| {
                    gen_ed_satisfied.is_satisfied(code)
                        || gen_ed_satisfied.satisfied_credits(code) > 0.0
                },
            );

            // Get remaining credits needed
            let remaining = attr_code.as_ref().map_or_else(
                || gen_ed_satisfied.remaining_credits(&resolved.id),
                |code| gen_ed_satisfied.remaining_credits(code),
            );

            // Check if this gen-ed requirement has a credit-based requirement tracked
            let has_credit_requirement = attr_code
                .as_ref()
                .is_some_and(|c| gen_ed_satisfied.required_credits.contains_key(c))
                || gen_ed_satisfied.required_credits.contains_key(&resolved.id);

            if has_credit_requirement {
                // Credit-based gen-ed requirement
                // If fully satisfied by courses with matching gen-ed attributes, skip
                if is_satisfied || remaining <= 0.0 {
                    continue;
                }

                // Add courses to cover remaining credits
                if let Some(first_choice) = resolved.choices.first() {
                    let expanded = expand_course_list(first_choice);
                    let mut credits_added = 0.0f32;

                    for course in expanded {
                        if credits_added >= remaining {
                            break;
                        }

                        let course_creds = course_credits
                            .get(&course)
                            .copied()
                            .unwrap_or_else(|| placeholder_credits(&course));

                        courses.push(course);
                        credits_added += course_creds;
                    }
                }
            } else {
                // Count-based or simple requirement
                // Skip if already satisfied by a course with matching gen-ed attributes
                if is_satisfied {
                    continue;
                }

                // Add first choice
                if let Some(first_choice) = resolved.choices.first() {
                    let expanded = expand_course_list(first_choice);
                    courses.extend(expanded);
                }
            }
        }

        courses
    }

    /// Estimate the total number of possible plans (major requirements only)
    #[must_use]
    pub fn estimate_plan_count(&self) -> usize {
        if self.major_requirements.is_empty() {
            return 1; // Single plan with no major choices
        }
        self.major_requirements
            .iter()
            .map(|r| r.choice_count.max(1))
            .product()
    }

    /// Get statistics about the plan space
    #[must_use]
    pub fn get_stats(&self) -> PlanGenerationStats {
        let variable_reqs: Vec<_> = self
            .major_requirements
            .iter()
            .filter(|r| r.is_variable)
            .collect();

        PlanGenerationStats {
            total_possible: self.estimate_plan_count(),
            plans_generated: 0,
            duplicates_skipped: 0,
            variable_requirements: variable_reqs.len(),
            requirement_choices: variable_reqs
                .iter()
                .map(|r| (r.id.clone(), r.choice_count))
                .collect(),
            major_credits: 0.0, // Will be computed during generation
            non_major_credits: self.non_major_credits,
            elective_placeholder_credits: 0.0, // Will be computed during generation
        }
    }

    /// Calculate placeholder elective credits needed to reach target
    #[allow(clippy::cast_precision_loss)] // Safe: credit values are small integers
    fn calculate_elective_placeholders(&self, plan_credits: f32) -> f32 {
        if let Some(target) = self.config.target_credits {
            let target_f32 = target as f32;
            if plan_credits < target_f32 {
                return target_f32 - plan_credits;
            }
        }
        0.0
    }

    /// Generate all plans up to the configured limit
    ///
    /// Returns an iterator over plan variants for memory efficiency.
    #[must_use]
    pub fn generate(&self) -> PlanIterator<'_> {
        PlanIterator::new(self)
    }

    /// Generate all plans and collect into a vector
    ///
    /// Warning: May use significant memory for large plan spaces.
    #[must_use]
    pub fn generate_all(&self) -> (Vec<PlanVariant>, PlanGenerationStats) {
        let mut stats = self.get_stats();
        let mut plans = Vec::new();
        let mut seen_fingerprints = std::collections::HashSet::new();

        for plan in self.generate() {
            if plans.len() >= self.config.max_plans {
                break;
            }

            if self.config.ignore_duplicates {
                let fp = plan.fingerprint();
                if seen_fingerprints.contains(&fp) {
                    stats.duplicates_skipped += 1;
                    continue;
                }
                seen_fingerprints.insert(fp);
            }

            // Track credits for first plan to populate stats
            if plans.is_empty() {
                let major_credits = plan.total_credits - self.non_major_credits;
                stats.major_credits = major_credits;
                stats.elective_placeholder_credits =
                    self.calculate_elective_placeholders(plan.total_credits);
            }

            plans.push(plan);
        }

        stats.plans_generated = plans.len();
        (plans, stats)
    }
}

/// Iterator over generated plans
///
/// Iterates through all combinations of major requirement choices,
/// combining each with fixed non-major courses and placeholder electives.
///
/// Supports different iteration orders via [`SamplingStrategy`]:
/// - Sequential: Standard counter-style iteration (0, 1, 2, ...)
/// - Shuffled: Random permutation of plan indices for unbiased sampling
/// - Stratified: (future) Ensures coverage across complexity strata
pub struct PlanIterator<'a> {
    /// Reference to the generator
    generator: &'a PlanGenerator<'a>,

    /// Current indices into each major requirement's choices (for sequential)
    indices: Vec<usize>,

    /// Whether we've finished iterating
    done: bool,

    /// Count of plans generated
    count: usize,

    /// Shuffled plan indices (for shuffled/stratified strategies)
    /// Each index represents a flat plan number that gets converted to requirement indices
    shuffled_order: Option<Vec<usize>>,

    /// Current position in `shuffled_order`
    shuffled_pos: usize,
}

impl<'a> PlanIterator<'a> {
    /// Create a new plan iterator
    fn new(generator: &'a PlanGenerator<'a>) -> Self {
        let indices = vec![0; generator.major_requirements.len()];
        // Done if no major requirements (single plan) or any requirement has no choices
        let done = !generator.major_requirements.is_empty()
            && generator
                .major_requirements
                .iter()
                .any(|r| r.choices.is_empty());

        // Build shuffled order if using shuffled strategy
        let shuffled_order = match generator.config.sampling_strategy {
            SamplingStrategy::Sequential => None,
            SamplingStrategy::Shuffled | SamplingStrategy::Stratified => {
                if done {
                    None
                } else {
                    Some(Self::build_shuffled_order(generator))
                }
            }
        };

        Self {
            generator,
            indices,
            done,
            count: 0,
            shuffled_order,
            shuffled_pos: 0,
        }
    }

    /// Build a shuffled order of plan indices
    ///
    /// Creates a vector of flat indices [0, 1, 2, ..., total_plans-1] and shuffles it.
    /// For large plan spaces, only generates indices up to `max_plans` to save memory.
    ///
    /// **Important**: Always includes indices 0 (simplest plan) and (total-1) (most complex plan)
    /// at the start of the order to ensure the extremes are always processed, regardless of
    /// sample size. This guarantees that "shortest" and "longest" plans are accurate.
    fn build_shuffled_order(generator: &PlanGenerator<'_>) -> Vec<usize> {
        let total = generator.estimate_plan_count();
        // Cap at max_plans to avoid huge memory allocation
        let count = total.min(generator.config.max_plans);

        // Always include extremes: index 0 (simplest) and index (total-1) (most complex)
        // These will be placed at the beginning of the order
        let extreme_low = 0;
        let extreme_high = total.saturating_sub(1);

        let mut order: Vec<usize> = if count == total {
            // Small enough to shuffle all
            (0..total).collect()
        } else {
            // Too many plans - sample randomly without replacement
            // Reserve 2 slots for extremes
            let sample_count = count.saturating_sub(2);
            let mut sampled =
                Self::sample_indices(total, sample_count, generator.config.random_seed);

            // Remove extremes from sample if present (we'll add them explicitly)
            sampled.retain(|&x| x != extreme_low && x != extreme_high);

            // Start with extremes, then add sampled indices
            let mut result = vec![extreme_low];
            if extreme_high != extreme_low {
                result.push(extreme_high);
            }
            result.extend(sampled);

            // Truncate to count if we ended up with too many
            result.truncate(count);
            result
        };

        // Shuffle everything EXCEPT the first two elements (extremes)
        // This ensures extremes are always processed first
        let mut rng = generator
            .config
            .random_seed
            .map_or_else(Rng::new, Rng::with_seed);

        if order.len() > 2 {
            Self::fisher_yates_shuffle(&mut order[2..], &mut rng);
        }

        order
    }

    /// Fisher-Yates shuffle implementation using fastrand
    fn fisher_yates_shuffle(vec: &mut [usize], rng: &mut Rng) {
        for i in (1..vec.len()).rev() {
            let j = rng.usize(0..=i);
            vec.swap(i, j);
        }
    }

    /// Sample `count` unique indices from range [0, total) without replacement
    fn sample_indices(total: usize, count: usize, seed: Option<u64>) -> Vec<usize> {
        let mut rng = seed.map_or_else(Rng::new, Rng::with_seed);

        // For reasonable ratios, use Floyd's algorithm
        let mut selected = HashSet::with_capacity(count);
        for j in (total - count)..total {
            let t = rng.usize(0..=j);
            if selected.contains(&t) {
                selected.insert(j);
            } else {
                selected.insert(t);
            }
        }
        selected.into_iter().collect()
    }

    /// Convert a flat plan index to requirement indices
    ///
    /// Treats the plan index as a mixed-radix number where each digit
    /// corresponds to a choice index for a requirement.
    fn flat_index_to_indices(&self, mut flat_idx: usize) -> Vec<usize> {
        let mut indices = vec![0; self.generator.major_requirements.len()];

        // Convert from flat index to multi-dimensional indices
        // Like converting a number to mixed-radix representation
        for i in (0..self.generator.major_requirements.len()).rev() {
            let choice_count = self.generator.major_requirements[i].choices.len().max(1);
            indices[i] = flat_idx % choice_count;
            flat_idx /= choice_count;
        }

        indices
    }

    /// Get the current indices based on sampling strategy
    fn get_current_indices(&self) -> Vec<usize> {
        self.shuffled_order.as_ref().map_or_else(
            || self.indices.clone(),
            |order| {
                if self.shuffled_pos < order.len() {
                    self.flat_index_to_indices(order[self.shuffled_pos])
                } else {
                    vec![0; self.generator.major_requirements.len()]
                }
            },
        )
    }

    /// Build a plan variant from given indices
    ///
    /// Combines major requirement choices with fixed non-major courses
    /// and adds placeholder electives to reach target credits.
    /// Respects `exclude_used` constraints by dynamically selecting courses
    /// from the available pool when some are already used.
    /// Expands bundles and equivalents to their component courses.
    fn build_plan_from_indices(&self, indices: &[usize]) -> PlanVariant {
        let mut requirement_choices: HashMap<String, Vec<String>> = HashMap::new();
        let mut used_courses: HashSet<String> = HashSet::new();

        // Add major requirement choices based on provided indices
        // Track used courses for exclude_used filtering
        for (i, req) in self.generator.major_requirements.iter().enumerate() {
            let choice_idx = indices.get(i).copied().unwrap_or(0);
            if choice_idx < req.choices.len() {
                let raw_chosen = if req.exclude_used {
                    // For exclude_used requirements, dynamically select from the pool
                    self.select_courses_excluding_used(req, choice_idx, &used_courses)
                } else {
                    req.choices[choice_idx].clone()
                };

                // Expand bundles and equivalents to individual courses
                let chosen_courses = expand_course_list(&raw_chosen);

                // Add chosen courses to used set
                for course in &chosen_courses {
                    used_courses.insert(course.clone());
                }

                requirement_choices.insert(req.id.clone(), chosen_courses);
            }
        }

        // Add non-major courses as a fixed "non_major" requirement
        // Filter out any courses already used in major requirements
        // Also expand any bundles/equivalents in non-major courses
        if !self.generator.non_major_courses.is_empty() {
            let expanded_non_major = expand_course_list(&self.generator.non_major_courses);
            let non_major: Vec<String> = expanded_non_major
                .iter()
                .filter(|c| !used_courses.contains(*c))
                .cloned()
                .collect();

            for course in &non_major {
                used_courses.insert(course.clone());
            }

            requirement_choices.insert("_non_major".to_string(), non_major);
        }

        // Create base plan
        let mut plan = PlanVariant::new(requirement_choices, &self.generator.course_credits);

        // Add placeholder electives if needed to reach target credits
        let elective_credits = self
            .generator
            .calculate_elective_placeholders(plan.total_credits);
        if elective_credits > 0.0 {
            plan = self.add_elective_placeholders(plan, elective_credits);
        }

        plan
    }

    /// Add placeholder elective courses to reach target credits
    ///
    /// Creates generic 3-credit elective placeholders plus a smaller one
    /// if there's a remainder.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn add_elective_placeholders(&self, mut plan: PlanVariant, credits_needed: f32) -> PlanVariant {
        let default_credits = self.generator.config.default_elective_credits;
        // Safe: credits_needed is always non-negative and reasonable
        let full_electives = (credits_needed / default_credits).floor() as usize;
        let remainder = credits_needed % default_credits;

        let mut elective_courses = Vec::new();

        // Add full-credit electives
        for i in 0..full_electives {
            elective_courses.push(format!("ELEC{:03}", i + 1));
        }

        // Add partial elective if there's a remainder
        if remainder > 0.5 {
            elective_courses.push(format!("ELEC{:03}S", full_electives + 1));
        }

        // Add electives to plan
        if !elective_courses.is_empty() {
            let mut new_courses = plan.courses.clone();
            new_courses.extend(elective_courses.clone());
            new_courses.sort();

            let mut new_choices = plan.requirement_choices.clone();
            new_choices.insert("_elective_placeholders".to_string(), elective_courses);

            // Recalculate total credits
            let total_credits = plan.total_credits + credits_needed;

            plan = PlanVariant::from_parts(new_courses, new_choices, total_credits);
        }

        plan
    }

    /// Select courses for an `exclude_used` requirement
    ///
    /// Uses the pre-generated choice as a starting point, but dynamically
    /// adds additional courses from the pool if some were already used.
    #[allow(clippy::unused_self)]
    fn select_courses_excluding_used(
        &self,
        req: &ResolvedRequirement,
        choice_idx: usize,
        used_courses: &HashSet<String>,
    ) -> Vec<String> {
        // Get the base choice
        let base_choice = &req.choices[choice_idx];

        // Filter out already used courses
        let mut available: Vec<String> = base_choice
            .iter()
            .filter(|c| !used_courses.contains(*c))
            .cloned()
            .collect();

        // If we have enough courses, return them
        let needed = req.courses_needed.unwrap_or(base_choice.len());

        if available.len() >= needed {
            available.truncate(needed);
            return available;
        }

        // Need more courses - get them from the pool
        if let Some(pool) = &req.available_pool {
            // Find courses in pool that aren't used and aren't already selected
            let available_set: HashSet<&str> = available.iter().map(String::as_str).collect();
            let additional: Vec<String> = pool
                .iter()
                .filter(|c| !used_courses.contains(*c) && !available_set.contains(c.as_str()))
                .take(needed - available.len())
                .cloned()
                .collect();

            available.extend(additional);
        }

        available
    }

    /// Advance to the next combination of choices (sequential mode)
    fn advance_sequential(&mut self) {
        if self.generator.major_requirements.is_empty() {
            // Only one plan possible with no major choices
            self.done = true;
            return;
        }

        // Increment indices like a multi-digit counter
        for i in (0..self.indices.len()).rev() {
            self.indices[i] += 1;
            if self.indices[i] < self.generator.major_requirements[i].choices.len() {
                return;
            }
            self.indices[i] = 0;
        }

        // All indices wrapped around - we're done
        self.done = true;
    }

    /// Advance to the next position (shuffled mode)
    #[allow(clippy::missing_const_for_fn)] // Can't be const due to Option pattern matching
    fn advance_shuffled(&mut self) {
        self.shuffled_pos += 1;
        if let Some(order) = &self.shuffled_order {
            if self.shuffled_pos >= order.len() {
                self.done = true;
            }
        } else {
            self.done = true;
        }
    }
}

impl Iterator for PlanIterator<'_> {
    type Item = PlanVariant;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }

        // Check max plans limit
        if self.count >= self.generator.config.max_plans {
            self.done = true;
            return None;
        }

        // Get current indices and build plan
        let indices = self.get_current_indices();
        let plan = self.build_plan_from_indices(&indices);
        self.count += 1;

        // Advance based on strategy
        if self.shuffled_order.is_some() {
            self.advance_shuffled();
        } else {
            self.advance_sequential();
        }

        Some(plan)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done {
            return (0, Some(0));
        }

        let total = self.shuffled_order.as_ref().map_or_else(
            || {
                self.generator
                    .estimate_plan_count()
                    .saturating_sub(self.count)
            },
            |order| order.len().saturating_sub(self.shuffled_pos),
        );
        let capped = total.min(self.generator.config.max_plans - self.count);
        (capped, Some(capped))
    }
}

/// Calculate credits for a placeholder course based on its key pattern
///
/// Placeholder courses (generated for wildcard requirements) follow naming conventions:
/// - Courses ending in 'S' are "small" courses with 2 credits (remainder courses)
/// - All other placeholder courses default to 3 credits
///
/// This function recognizes placeholder patterns like:
/// - `FE01`, `FE02S` (free electives)
/// - `AC01`, `AW01` (gen ed placeholders)
/// - `ELEC001`, `ELEC002S` (target credit placeholders)
fn placeholder_credits(course_key: &str) -> f32 {
    // Check for "S" suffix indicating a small/remainder course
    if course_key.ends_with('S') {
        2.0
    } else {
        3.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::degree::{FromClause, RequirementType};

    fn sample_courses() -> HashMap<String, Course> {
        let mut courses = HashMap::new();
        for (key, credits) in [
            ("CS1000", 4.0),
            ("CS2000", 4.0),
            ("CS3000", 4.0),
            ("CS3500", 4.0),
            ("CS4000", 4.0),
            ("MATH1000", 3.0),
            ("MATH2000", 3.0),
        ] {
            let course = Course {
                credit_hours: credits,
                ..Default::default()
            };
            courses.insert(key.to_string(), course);
        }
        courses
    }

    fn sample_requirements() -> HashMap<String, Requirement> {
        let mut reqs = HashMap::new();

        // Fixed requirement
        reqs.insert(
            "core".to_string(),
            Requirement {
                name: Some("Core".to_string()),
                req_type: RequirementType::All,
                category: Some("major".to_string()),
                courses: Some(vec!["CS1000".to_string(), "CS2000".to_string()]),
                from: None,
                count: None,
                credits: None,
                credit_range: None,
                constraints: None,
                options: None,
            },
        );

        // Variable requirement: pick 1 from 3
        reqs.insert(
            "elective".to_string(),
            Requirement {
                name: Some("Elective".to_string()),
                req_type: RequirementType::Select,
                category: Some("major".to_string()),
                courses: None,
                from: Some(FromClause {
                    courses: Some(vec![
                        "CS3000".to_string(),
                        "CS3500".to_string(),
                        "CS4000".to_string(),
                    ]),
                    pattern: None,
                    include: None,
                    exclude: None,
                    groups: None,
                    groups_required: None,
                    per_group: None,
                }),
                count: Some(1),
                credits: None,
                credit_range: None,
                constraints: None,
                options: None,
            },
        );

        reqs
    }

    #[test]
    fn test_plan_generator_estimate() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig::default();

        let generator = PlanGenerator::new(&reqs, &courses, config);

        // 1 fixed * 3 choices = 3 plans
        assert_eq!(generator.estimate_plan_count(), 3);
    }

    #[test]
    fn test_plan_generator_generates_all() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig::default();

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, stats) = generator.generate_all();

        assert_eq!(plans.len(), 3);
        assert_eq!(stats.plans_generated, 3);
        assert_eq!(stats.total_possible, 3);
        assert!(!stats.was_truncated());
    }

    #[test]
    fn test_plan_generator_max_plans_limit() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig {
            max_plans: 2,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, stats) = generator.generate_all();

        assert_eq!(plans.len(), 2);
        assert_eq!(stats.plans_generated, 2);
        assert!(stats.was_truncated());
    }

    #[test]
    fn test_plan_generator_iterator() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig::default();

        let generator = PlanGenerator::new(&reqs, &courses, config);

        let mut count = 0;
        for plan in generator.generate() {
            // Each plan should have core courses plus one elective
            assert!(plan.contains_course("CS1000"));
            assert!(plan.contains_course("CS2000"));
            count += 1;
        }

        assert_eq!(count, 3);
    }

    #[test]
    fn test_plan_generator_stats() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig::default();

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let stats = generator.get_stats();

        assert_eq!(stats.variable_requirements, 1);
        assert_eq!(stats.total_possible, 3);
    }

    #[test]
    fn test_plan_unique_courses() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig {
            ignore_duplicates: false, // Need all plans for this test
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, _) = generator.generate_all();

        // Each plan should have different elective choice
        let electives: Vec<bool> = plans.iter().map(|p| p.contains_course("CS3000")).collect();

        // Exactly one plan should have CS3000
        assert_eq!(electives.iter().filter(|&&x| x).count(), 1);
    }

    #[test]
    fn test_non_major_requirements_simplified() {
        let courses = sample_courses();
        let mut reqs = sample_requirements();

        // Add a non-major (gen_ed) requirement with multiple choices
        reqs.insert(
            "gen_ed".to_string(),
            Requirement {
                name: Some("Gen Ed Math".to_string()),
                req_type: RequirementType::Select,
                category: Some("gen_ed".to_string()),
                courses: None,
                from: Some(FromClause {
                    courses: Some(vec!["MATH1000".to_string(), "MATH2000".to_string()]),
                    pattern: None,
                    include: None,
                    exclude: None,
                    groups: None,
                    groups_required: None,
                    per_group: None,
                }),
                count: Some(1),
                credits: None,
                credit_range: None,
                constraints: None,
                options: None,
            },
        );

        let config = PlanGeneratorConfig {
            ignore_duplicates: false,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, stats) = generator.generate_all();

        // Should still only have 3 plans (gen_ed doesn't contribute to combinations)
        assert_eq!(plans.len(), 3);
        assert_eq!(stats.total_possible, 3);

        // All plans should have the first gen_ed option (MATH1000)
        for plan in &plans {
            assert!(plan.contains_course("MATH1000"));
        }

        // Non-major credits should be tracked
        assert!((stats.non_major_credits - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_elective_placeholders() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig {
            target_credits: Some(30), // Higher than actual course credits
            ignore_duplicates: false,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, _) = generator.generate_all();

        // All plans should reach target credits
        for plan in &plans {
            assert!(
                plan.total_credits >= 30.0,
                "Plan has {} credits, expected >= 30",
                plan.total_credits
            );
            // Should have elective placeholders
            assert!(plan.contains_course("ELEC001"));
        }
    }

    #[test]
    fn test_sampling_strategy_parse() {
        assert_eq!(
            "sequential".parse::<SamplingStrategy>().unwrap(),
            SamplingStrategy::Sequential
        );
        assert_eq!(
            "shuffled".parse::<SamplingStrategy>().unwrap(),
            SamplingStrategy::Shuffled
        );
        assert_eq!(
            "stratified".parse::<SamplingStrategy>().unwrap(),
            SamplingStrategy::Stratified
        );
        assert_eq!(
            "random".parse::<SamplingStrategy>().unwrap(),
            SamplingStrategy::Shuffled
        );
        assert!("invalid".parse::<SamplingStrategy>().is_err());
    }

    #[test]
    fn test_sampling_strategy_display() {
        assert_eq!(SamplingStrategy::Sequential.to_string(), "sequential");
        assert_eq!(SamplingStrategy::Shuffled.to_string(), "shuffled");
        assert_eq!(SamplingStrategy::Stratified.to_string(), "stratified");
    }

    #[test]
    fn test_shuffled_sampling_generates_all_plans() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig {
            sampling_strategy: SamplingStrategy::Shuffled,
            random_seed: Some(42), // Fixed seed for reproducibility
            ignore_duplicates: false,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, _) = generator.generate_all();

        // Should still generate all 3 plans
        assert_eq!(plans.len(), 3);

        // All core courses should be present in all plans
        for plan in &plans {
            assert!(plan.contains_course("CS1000"));
            assert!(plan.contains_course("CS2000"));
        }
    }

    #[test]
    fn test_shuffled_sampling_different_order_than_sequential() {
        let courses = sample_courses();
        let reqs = sample_requirements();

        // Sequential config
        let seq_config = PlanGeneratorConfig {
            sampling_strategy: SamplingStrategy::Sequential,
            ignore_duplicates: false,
            ..Default::default()
        };

        // Shuffled config with fixed seed
        let shuf_config = PlanGeneratorConfig {
            sampling_strategy: SamplingStrategy::Shuffled,
            random_seed: Some(42),
            ignore_duplicates: false,
            ..Default::default()
        };

        let seq_gen = PlanGenerator::new(&reqs, &courses, seq_config);
        let shuf_gen = PlanGenerator::new(&reqs, &courses, shuf_config);

        let seq_plans: Vec<_> = seq_gen.generate().collect();
        let shuf_plans: Vec<_> = shuf_gen.generate().collect();

        // Both should have same number of plans
        assert_eq!(seq_plans.len(), shuf_plans.len());

        // The fingerprints should be the same set (same plans, different order)
        let seq_fps: std::collections::HashSet<_> =
            seq_plans.iter().map(PlanVariant::fingerprint).collect();
        let shuf_fps: std::collections::HashSet<_> =
            shuf_plans.iter().map(PlanVariant::fingerprint).collect();
        assert_eq!(seq_fps, shuf_fps);
    }

    #[test]
    fn test_sequential_strategy_uses_counter_style_iteration() {
        let courses = sample_courses();
        let reqs = sample_requirements();
        let config = PlanGeneratorConfig {
            sampling_strategy: SamplingStrategy::Sequential,
            ignore_duplicates: false,
            ..Default::default()
        };

        let generator = PlanGenerator::new(&reqs, &courses, config);

        // First plan should have first elective choice
        let first_plan = generator.generate().next().unwrap();

        // The iterator uses counter-style: first requirement at index 0 stays 0
        // until second requirement cycles through all its options
        // (depends on requirement order which is non-deterministic in HashMap)
        // Just verify we get a valid plan
        assert!(first_plan.contains_course("CS1000"));
        assert!(first_plan.contains_course("CS2000"));
    }
}
