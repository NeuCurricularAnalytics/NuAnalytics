//! Term scheduler for distributing courses across semesters/quarters
//!
//! This module implements a greedy topological scheduling algorithm that:
//! 1. Respects prerequisite constraints (prerequisites must come before dependents)
//! 2. Groups corequisites and strict corequisites into the same term
//! 3. Attempts to balance credit hours across terms (~15 credits/term for semesters)

use crate::core::models::{School, DAG};
use std::collections::{HashMap, HashSet};

/// Default target credits per semester
pub const DEFAULT_SEMESTER_CREDITS: f32 = 15.0;

/// Default target credits per quarter
pub const DEFAULT_QUARTER_CREDITS: f32 = 15.0;

/// Number of terms in a standard 4-year semester plan
pub const SEMESTER_TERMS: usize = 8;

/// Number of terms in a standard 4-year quarter plan
pub const QUARTER_TERMS: usize = 12;

/// A single term in the schedule with its assigned courses
#[derive(Debug, Clone, Default)]
pub struct Term {
    /// Term number (1-indexed for display)
    pub number: usize,
    /// Course keys assigned to this term
    pub courses: Vec<String>,
    /// Total credit hours for this term
    pub total_credits: f32,
}

impl Term {
    /// Create a new empty term
    #[must_use]
    pub const fn new(number: usize) -> Self {
        Self {
            number,
            courses: Vec::new(),
            total_credits: 0.0,
        }
    }

    /// Add a course to the term
    pub fn add_course(&mut self, course_key: String, credits: f32) {
        self.courses.push(course_key);
        self.total_credits += credits;
    }
}

/// Complete term-by-term plan
#[derive(Debug, Clone)]
pub struct TermPlan {
    /// All terms in the plan
    pub terms: Vec<Term>,
    /// Whether this uses quarter system
    pub is_quarter_system: bool,
    /// Target credits per term
    pub target_credits: f32,
    /// Courses that couldn't be scheduled (if any)
    pub unscheduled: Vec<String>,
}

impl TermPlan {
    /// Create a new empty term plan
    #[must_use]
    pub fn new(num_terms: usize, is_quarter_system: bool, target_credits: f32) -> Self {
        let terms = (1..=num_terms).map(Term::new).collect();
        Self {
            terms,
            is_quarter_system,
            target_credits,
            unscheduled: Vec::new(),
        }
    }

    /// Add a new term to the plan
    pub fn add_term(&mut self) {
        let next_number = self.terms.len() + 1;
        self.terms.push(Term::new(next_number));
    }

    /// Get display name for terms (Semester/Quarter)
    #[must_use]
    pub const fn term_label(&self) -> &'static str {
        if self.is_quarter_system {
            "Quarter"
        } else {
            "Semester"
        }
    }

    /// Get the total number of terms actually used
    #[must_use]
    pub fn terms_used(&self) -> usize {
        self.terms.iter().filter(|t| !t.courses.is_empty()).count()
    }
}

/// Configuration for the term scheduler
#[derive(Debug, Clone)]
pub struct SchedulerConfig {
    /// Target credits per term
    pub target_credits: f32,
    /// Maximum credits per term (hard limit)
    pub max_credits: f32,
    /// Number of terms to schedule
    pub num_terms: usize,
    /// Whether using quarter system
    pub is_quarter_system: bool,
}

impl SchedulerConfig {
    /// Create config for semester system
    #[must_use]
    pub fn semester(target_credits: f32) -> Self {
        Self {
            target_credits,
            max_credits: target_credits + 6.0, // Allow some overflow
            num_terms: SEMESTER_TERMS,
            is_quarter_system: false,
        }
    }

    /// Create config for quarter system
    #[must_use]
    pub fn quarter(target_credits: f32) -> Self {
        Self {
            target_credits,
            max_credits: target_credits + 4.0,
            num_terms: QUARTER_TERMS,
            is_quarter_system: true,
        }
    }
}

impl Default for SchedulerConfig {
    fn default() -> Self {
        Self::semester(DEFAULT_SEMESTER_CREDITS)
    }
}

/// Term scheduler that distributes courses across terms
pub struct TermScheduler<'a> {
    school: &'a School,
    dag: &'a DAG,
    config: SchedulerConfig,
}

impl<'a> TermScheduler<'a> {
    /// Create a new term scheduler
    #[must_use]
    pub const fn new(school: &'a School, dag: &'a DAG, config: SchedulerConfig) -> Self {
        Self {
            school,
            dag,
            config,
        }
    }

    /// Schedule courses into terms
    ///
    /// Uses a greedy topological approach:
    /// 1. Build in-degree map from DAG
    /// 2. Process courses in topological order
    /// 3. Assign to earliest term that satisfies:
    ///    - All prerequisites completed in earlier terms
    ///    - Corequisites placed in same term
    ///    - Credit limit not exceeded (soft target, hard max)
    #[must_use]
    pub fn schedule(&self, course_keys: &[String]) -> TermPlan {
        let mut plan = TermPlan::new(
            self.config.num_terms,
            self.config.is_quarter_system,
            self.config.target_credits,
        );

        // Build prerequisite and corequisite maps for the courses in this plan
        let course_set: HashSet<_> = course_keys.iter().collect();

        // Map course -> earliest term it can be placed (based on prereqs)
        let mut earliest_term: HashMap<String, usize> = HashMap::new();

        // Group corequisites that must be scheduled together
        let coreq_groups = self.build_corequisite_groups(course_keys);

        // Track which courses have been scheduled
        let mut scheduled: HashSet<String> = HashSet::new();

        // Get topological order from DAG
        let topo_order = self.topological_sort(course_keys);

        // Process courses in topological order
        for course_key in &topo_order {
            if scheduled.contains(course_key) {
                continue;
            }

            // Find the corequisite group for this course
            let group = coreq_groups
                .iter()
                .find(|g| g.contains(course_key))
                .cloned()
                .unwrap_or_else(|| vec![course_key.clone()]);

            // Calculate the earliest term for this group
            let min_term = self.calculate_earliest_term(&group, &earliest_term, &course_set);

            // Calculate total credits for the group
            let group_credits: f32 = group
                .iter()
                .filter_map(|k| self.school.get_course(k))
                .map(|c| c.credit_hours)
                .sum();

            // Find the best term to place this group (will expand plan if needed)
            let term_idx = self.find_best_term(&mut plan, min_term, group_credits);

            // Schedule all courses in the group to this term
            for key in &group {
                if let Some(course) = self.school.get_course(key) {
                    plan.terms[term_idx].add_course(key.clone(), course.credit_hours);
                    scheduled.insert(key.clone());
                    earliest_term.insert(key.clone(), term_idx);
                }
            }
        }

        plan
    }

    /// Build groups of courses that must be in the same term (corequisites/strict corequisites)
    fn build_corequisite_groups(&self, course_keys: &[String]) -> Vec<Vec<String>> {
        let course_set: HashSet<_> = course_keys.iter().cloned().collect();
        let mut visited: HashSet<String> = HashSet::new();
        let mut groups: Vec<Vec<String>> = Vec::new();

        for key in course_keys {
            if visited.contains(key) {
                continue;
            }

            let mut group = vec![key.clone()];
            let mut to_check = vec![key.clone()];
            visited.insert(key.clone());

            // BFS to find all connected corequisites
            while let Some(current) = to_check.pop() {
                if let Some(course) = self.school.get_course(&current) {
                    // Add strict corequisites (must be same term)
                    for coreq in &course.strict_corequisites {
                        if course_set.contains(coreq) && !visited.contains(coreq) {
                            group.push(coreq.clone());
                            to_check.push(coreq.clone());
                            visited.insert(coreq.clone());
                        }
                    }

                    // Add regular corequisites (should be same term when possible)
                    for coreq in &course.corequisites {
                        if course_set.contains(coreq) && !visited.contains(coreq) {
                            group.push(coreq.clone());
                            to_check.push(coreq.clone());
                            visited.insert(coreq.clone());
                        }
                    }
                }
            }

            if !group.is_empty() {
                groups.push(group);
            }
        }

        groups
    }

    /// Calculate the earliest term a group can be placed (based on prerequisites)
    fn calculate_earliest_term(
        &self,
        group: &[String],
        scheduled: &HashMap<String, usize>,
        _course_set: &HashSet<&String>,
    ) -> usize {
        let mut min_term = 0;

        for key in group {
            if let Some(prereqs) = self.dag.dependencies.get(key) {
                for prereq in prereqs {
                    if let Some(&prereq_term) = scheduled.get(prereq) {
                        // Must be after the prerequisite's term
                        min_term = min_term.max(prereq_term + 1);
                    }
                }
            }
        }

        min_term
    }

    /// Find the best term to place a group, starting from `min_term`
    /// Expands the plan if needed to fit all courses
    fn find_best_term(&self, plan: &mut TermPlan, min_term: usize, group_credits: f32) -> usize {
        // Ensure we have enough terms
        while min_term >= plan.terms.len() {
            plan.add_term();
        }

        // First, try to find a term at or after min_term that fits within target
        for term_idx in min_term..plan.terms.len() {
            let projected = plan.terms[term_idx].total_credits + group_credits;
            if projected <= self.config.target_credits {
                return term_idx;
            }
        }

        // If no ideal fit, find term at or after min_term under max credits
        for term_idx in min_term..plan.terms.len() {
            let projected = plan.terms[term_idx].total_credits + group_credits;
            if projected <= self.config.max_credits {
                return term_idx;
            }
        }

        // If still no fit, add a new term and place there
        plan.add_term();
        plan.terms.len() - 1
    }

    /// Get topological order of courses
    fn topological_sort(&self, course_keys: &[String]) -> Vec<String> {
        let course_set: HashSet<_> = course_keys.iter().cloned().collect();

        // Build in-degree map (only counting edges within our course set)
        let mut in_degree: HashMap<String, usize> = HashMap::new();
        for key in course_keys {
            in_degree.insert(key.clone(), 0);
        }

        for key in course_keys {
            if let Some(prereqs) = self.dag.dependencies.get(key) {
                for prereq in prereqs {
                    if course_set.contains(prereq) {
                        *in_degree.entry(key.clone()).or_insert(0) += 1;
                    }
                }
            }
        }

        // Kahn's algorithm
        let mut queue: Vec<_> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(k, _)| k.clone())
            .collect();

        let mut result = Vec::new();

        while let Some(course) = queue.pop() {
            result.push(course.clone());

            // Decrease in-degree of dependents
            if let Some(dependents) = self.dag.dependents.get(&course) {
                for dep in dependents {
                    if let Some(deg) = in_degree.get_mut(dep) {
                        *deg = deg.saturating_sub(1);
                        if *deg == 0 && !result.contains(dep) {
                            queue.push(dep.clone());
                        }
                    }
                }
            }
        }

        // Add any courses not in topo order (should not happen in valid DAG)
        for key in course_keys {
            if !result.contains(key) {
                result.push(key.clone());
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::models::Course;

    fn create_test_school() -> School {
        let mut school = School::new("Test University".to_string());

        // Create courses with prerequisites
        let cs101 = Course::new(
            "Intro to CS".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );

        let mut cs201 = Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "201".to_string(),
            3.0,
        );
        cs201.add_prerequisite("CS101".to_string());

        let mut cs301 = Course::new(
            "Algorithms".to_string(),
            "CS".to_string(),
            "301".to_string(),
            3.0,
        );
        cs301.add_prerequisite("CS201".to_string());

        let math101 = Course::new(
            "Calculus I".to_string(),
            "MATH".to_string(),
            "101".to_string(),
            4.0,
        );

        school.add_course(cs101);
        school.add_course(cs201);
        school.add_course(cs301);
        school.add_course(math101);

        school
    }

    #[test]
    fn test_basic_scheduling() {
        let school = create_test_school();
        let mut dag = DAG::new();

        dag.add_course("CS101".to_string());
        dag.add_course("CS201".to_string());
        dag.add_course("CS301".to_string());
        dag.add_course("MATH101".to_string());

        dag.add_prerequisite("CS201".to_string(), "CS101");
        dag.add_prerequisite("CS301".to_string(), "CS201");

        let config = SchedulerConfig::semester(15.0);
        let scheduler = TermScheduler::new(&school, &dag, config);

        let courses = vec![
            "CS101".to_string(),
            "CS201".to_string(),
            "CS301".to_string(),
            "MATH101".to_string(),
        ];

        let plan = scheduler.schedule(&courses);

        // CS101 should be before CS201
        let cs101_term = plan
            .terms
            .iter()
            .position(|t| t.courses.contains(&"CS101".to_string()));
        let cs201_term = plan
            .terms
            .iter()
            .position(|t| t.courses.contains(&"CS201".to_string()));
        let cs301_term = plan
            .terms
            .iter()
            .position(|t| t.courses.contains(&"CS301".to_string()));

        assert!(cs101_term.is_some());
        assert!(cs201_term.is_some());
        assert!(cs301_term.is_some());
        assert!(cs101_term < cs201_term);
        assert!(cs201_term < cs301_term);
    }

    #[test]
    fn test_term_plan_creation() {
        let plan = TermPlan::new(8, false, 15.0);
        assert_eq!(plan.terms.len(), 8);
        assert_eq!(plan.term_label(), "Semester");
        assert_eq!(plan.terms_used(), 0);
    }
}
