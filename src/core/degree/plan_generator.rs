//! Plan generator for enumerating all possible degree plans
//!
//! Generates all valid combinations of course selections that satisfy
//! a degree's requirements, using lazy iteration to handle large plan spaces.

use super::plan_variant::PlanVariant;
use super::requirement_resolver::{RequirementResolver, ResolvedRequirement};
use crate::core::models::course::Course;
use crate::core::models::degree::Requirement;
use std::collections::HashMap;

/// Configuration for plan generation
#[derive(Debug, Clone)]
pub struct PlanGeneratorConfig {
    /// Maximum number of plans to generate (safety cap)
    pub max_plans: usize,

    /// Skip plans that are equivalent (same courses, same metrics)
    pub ignore_duplicates: bool,

    /// Number of random plans to sample
    pub sample_count: usize,
}

impl Default for PlanGeneratorConfig {
    fn default() -> Self {
        Self {
            max_plans: 1_000_000,
            ignore_duplicates: false,
            sample_count: 5,
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
}

impl PlanGenerationStats {
    /// Check if plan generation was truncated due to limits
    #[must_use]
    pub const fn was_truncated(&self) -> bool {
        self.plans_generated < self.total_possible
    }
}

/// Generates all possible degree plans from requirements
pub struct PlanGenerator<'a> {
    /// Resolved requirements with their choices
    resolved: Vec<ResolvedRequirement>,

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

        Self {
            resolved,
            course_credits,
            config,
            _courses: courses,
        }
    }

    /// Estimate the total number of possible plans
    #[must_use]
    pub fn estimate_plan_count(&self) -> usize {
        self.resolved
            .iter()
            .map(|r| r.choice_count.max(1))
            .product()
    }

    /// Get statistics about the plan space
    #[must_use]
    pub fn get_stats(&self) -> PlanGenerationStats {
        let variable_reqs: Vec<_> = self.resolved.iter().filter(|r| r.is_variable).collect();

        PlanGenerationStats {
            total_possible: self.estimate_plan_count(),
            plans_generated: 0,
            duplicates_skipped: 0,
            variable_requirements: variable_reqs.len(),
            requirement_choices: variable_reqs
                .iter()
                .map(|r| (r.id.clone(), r.choice_count))
                .collect(),
        }
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

            plans.push(plan);
        }

        stats.plans_generated = plans.len();
        (plans, stats)
    }
}

/// Iterator over generated plans
pub struct PlanIterator<'a> {
    /// Reference to the generator
    generator: &'a PlanGenerator<'a>,

    /// Current indices into each requirement's choices
    indices: Vec<usize>,

    /// Whether we've finished iterating
    done: bool,

    /// Count of plans generated
    count: usize,
}

impl<'a> PlanIterator<'a> {
    fn new(generator: &'a PlanGenerator<'a>) -> Self {
        let indices = vec![0; generator.resolved.len()];
        let done = generator.resolved.is_empty()
            || generator.resolved.iter().any(|r| r.choices.is_empty());

        Self {
            generator,
            indices,
            done,
            count: 0,
        }
    }

    /// Build a plan variant from current indices
    fn build_current_plan(&self) -> PlanVariant {
        let mut requirement_choices: HashMap<String, Vec<String>> = HashMap::new();

        for (i, req) in self.generator.resolved.iter().enumerate() {
            let choice_idx = self.indices[i];
            if choice_idx < req.choices.len() {
                requirement_choices.insert(req.id.clone(), req.choices[choice_idx].clone());
            }
        }

        PlanVariant::new(requirement_choices, &self.generator.course_credits)
    }

    /// Advance to the next combination of choices
    fn advance(&mut self) {
        // Increment indices like a multi-digit counter
        for i in (0..self.indices.len()).rev() {
            self.indices[i] += 1;
            if self.indices[i] < self.generator.resolved[i].choices.len() {
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
        let config = PlanGeneratorConfig::default();

        let generator = PlanGenerator::new(&reqs, &courses, config);
        let (plans, _) = generator.generate_all();

        // Each plan should have different elective choice
        let electives: Vec<bool> = plans.iter().map(|p| p.contains_course("CS3000")).collect();

        // Exactly one plan should have CS3000
        assert_eq!(electives.iter().filter(|&&x| x).count(), 1);
    }
}
