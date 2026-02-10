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

use super::plan_variant::PlanVariant;
use super::requirement_resolver::{RequirementResolver, ResolvedRequirement};
use crate::core::models::course::Course;
use crate::core::models::degree::Requirement;
use std::collections::HashMap;

/// Categories that should be enumerated for plan generation
const ENUMERABLE_CATEGORIES: [&str; 1] = ["major"];

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
}

impl Default for PlanGeneratorConfig {
    fn default() -> Self {
        Self {
            max_plans: 1_000_000,
            ignore_duplicates: true, // Default to true per user request
            sample_count: 5,
            target_credits: None,
            default_elective_credits: 3.0,
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
}

impl<'a> PlanGenerator<'a> {
    /// Create a new plan generator
    ///
    /// Separates requirements into major (enumerated) and non-major (simplified).
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

        // Separate major vs non-major requirements
        let (major_requirements, non_major_requirements): (Vec<_>, Vec<_>) =
            resolved.into_iter().partition(|r| {
                r.category
                    .as_ref()
                    .is_some_and(|cat| ENUMERABLE_CATEGORIES.contains(&cat.as_str()))
            });

        // For non-major requirements, pick the first/simplest choice
        let mut non_major_courses = Vec::new();
        let mut non_major_credits = 0.0f32;

        for req in &non_major_requirements {
            if let Some(first_choice) = req.choices.first() {
                for course in first_choice {
                    let credits = course_credits.get(course).copied().unwrap_or(3.0);
                    non_major_credits += credits;
                    non_major_courses.push(course.clone());
                }
            }
        }

        // Sort and dedupe non-major courses
        non_major_courses.sort();
        non_major_courses.dedup();

        Self {
            major_requirements,
            non_major_courses,
            non_major_credits,
            course_credits,
            config,
            _courses: courses,
        }
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
pub struct PlanIterator<'a> {
    /// Reference to the generator
    generator: &'a PlanGenerator<'a>,

    /// Current indices into each major requirement's choices
    indices: Vec<usize>,

    /// Whether we've finished iterating
    done: bool,

    /// Count of plans generated
    count: usize,
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

        Self {
            generator,
            indices,
            done,
            count: 0,
        }
    }

    /// Build a plan variant from current indices
    ///
    /// Combines major requirement choices with fixed non-major courses
    /// and adds placeholder electives to reach target credits.
    fn build_current_plan(&self) -> PlanVariant {
        let mut requirement_choices: HashMap<String, Vec<String>> = HashMap::new();

        // Add major requirement choices based on current indices
        for (i, req) in self.generator.major_requirements.iter().enumerate() {
            let choice_idx = self.indices[i];
            if choice_idx < req.choices.len() {
                requirement_choices.insert(req.id.clone(), req.choices[choice_idx].clone());
            }
        }

        // Add non-major courses as a fixed "non_major" requirement
        if !self.generator.non_major_courses.is_empty() {
            requirement_choices.insert(
                "_non_major".to_string(),
                self.generator.non_major_courses.clone(),
            );
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

    /// Advance to the next combination of choices
    fn advance(&mut self) {
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

        // Build current plan
        let plan = self.build_current_plan();
        self.count += 1;

        // Advance for next iteration
        self.advance();

        Some(plan)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        if self.done {
            return (0, Some(0));
        }

        let remaining = self
            .generator
            .estimate_plan_count()
            .saturating_sub(self.count);
        let capped = remaining.min(self.generator.config.max_plans - self.count);
        (capped, Some(capped))
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
}
