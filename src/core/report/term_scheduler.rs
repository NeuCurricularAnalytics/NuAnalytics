//! Term scheduler for distributing courses across semesters/quarters
//!
//! This module implements a scheduling algorithm that:
//! 1. Prioritizes courses with long prerequisite chains (high delay factor)
//! 2. Groups corequisites and strict corequisites into the same term
//! 3. Respects prerequisite constraints (prerequisites must come before dependents)
//! 4. Balances credit hours across terms (~15 credits/term for semesters)
//! 5. Fills in low-complexity courses to balance underloaded terms

use crate::core::metrics::compute_delay;
use crate::core::models::{School, DAG};
use std::collections::{BinaryHeap, HashMap, HashSet};

/// Priority queue item for topological group ordering
///
/// Used in Kahn's algorithm to order corequisite groups by priority.
/// Higher priority groups (longer chains) are processed first, with
/// lexicographic ordering as a tiebreaker for deterministic results.
#[derive(Eq, PartialEq)]
struct GroupPQItem {
    /// Priority score (higher = more important)
    pri: usize,
    /// Lexicographically smallest course key in the group (for tiebreaking)
    name_hint: String,
    /// Index into the groups array
    idx: usize,
}

impl Ord for GroupPQItem {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        match self.pri.cmp(&other.pri) {
            std::cmp::Ordering::Equal => other.name_hint.cmp(&self.name_hint),
            ord => ord,
        }
    }
}

impl PartialOrd for GroupPQItem {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

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
    /// Algorithm:
    /// 1. Compute delay factors and blocking factors for prioritization
    /// 2. Build corequisite groups (courses that must be in the same term)
    /// 3. Process groups in topological order, but prioritize chain starters
    /// 4. Place each group in the earliest valid term respecting prerequisites
    /// 5. Rebalance by moving low-complexity filler courses to underloaded terms
    #[must_use]
    pub fn schedule(&self, course_keys: &[String]) -> TermPlan {
        let mut plan = TermPlan::new(
            self.config.num_terms,
            self.config.is_quarter_system,
            self.config.target_credits,
        );

        let course_set: HashSet<_> = course_keys.iter().collect();
        let delay_factors = compute_delay(self.dag).unwrap_or_default();
        let chain_priority = self.compute_chain_priority(course_keys, &course_set, &delay_factors);

        let mut course_term: HashMap<String, usize> = HashMap::new();
        let coreq_groups = self.build_corequisite_groups(course_keys);

        // Separate filler courses (no prerequisites, no dependents in plan)
        let (filler_groups, priority_groups) =
            self.separate_filler_groups(coreq_groups, &course_set);

        // Compute a dependency-respecting order of priority groups (topological by prerequisites)
        let ordered_priority_groups =
            self.order_groups_by_dependencies(&priority_groups, &course_set, &chain_priority);

        // Schedule priority groups first (courses with prerequisites/chains) in dependency order
        self.schedule_priority_groups(
            &ordered_priority_groups,
            &mut plan,
            &mut course_term,
            &course_set,
        );

        // Now fill in filler courses to balance terms
        self.schedule_filler_groups(&filler_groups, &mut plan, &mut course_term);

        // Final rebalancing pass
        self.rebalance_terms(&mut plan, &delay_factors);

        plan
    }

    /// Compute chain priority scores for course scheduling
    fn compute_chain_priority(
        &self,
        course_keys: &[String],
        course_set: &HashSet<&String>,
        delay_factors: &HashMap<String, usize>,
    ) -> HashMap<String, usize> {
        course_keys
            .iter()
            .map(|k| {
                let delay = delay_factors.get(k).copied().unwrap_or(0);
                let has_prereqs_in_plan = self
                    .dag
                    .dependencies
                    .get(k)
                    .is_some_and(|deps| deps.iter().any(|d| course_set.contains(d)));
                let has_dependents_in_plan = self
                    .dag
                    .dependents
                    .get(k)
                    .is_some_and(|deps| deps.iter().any(|d| course_set.contains(d)));

                // Chain starters: no prereqs in plan + have dependents + high delay = very important
                let priority = if !has_prereqs_in_plan && has_dependents_in_plan {
                    delay * 10 + 100 // Chain starters get big bonus
                } else if has_dependents_in_plan {
                    delay * 5 // Mid-chain courses
                } else {
                    delay // End-of-chain or standalone
                };

                (k.clone(), priority)
            })
            .collect()
    }

    /// Sort corequisite groups by chain priority (descending)
    #[allow(clippy::unused_self)]
    fn sort_groups_by_priority(
        &self,
        groups: &mut [Vec<String>],
        chain_priority: &HashMap<String, usize>,
    ) {
        groups.sort_by(|a, b| {
            let max_priority_a = a
                .iter()
                .map(|k| chain_priority.get(k).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            let max_priority_b = b
                .iter()
                .map(|k| chain_priority.get(k).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);

            match max_priority_b.cmp(&max_priority_a) {
                std::cmp::Ordering::Equal => {
                    let min_key_a = a.iter().min().cloned().unwrap_or_default();
                    let min_key_b = b.iter().min().cloned().unwrap_or_default();
                    min_key_a.cmp(&min_key_b)
                }
                other => other,
            }
        });
    }

    /// Compute a dependency-respecting order of groups using topological sort.
    /// Ties (multiple groups with no incoming edges) are broken by group priority
    /// derived from `chain_priority` (higher first), then lexicographically.
    #[allow(clippy::too_many_lines)]
    fn order_groups_by_dependencies(
        &self,
        groups: &[Vec<String>],
        course_set: &HashSet<&String>,
        chain_priority: &HashMap<String, usize>,
    ) -> Vec<Vec<String>> {
        if groups.is_empty() {
            return Vec::new();
        }

        // Map each course to its group index
        let mut course_to_group: HashMap<&str, usize> = HashMap::new();
        for (idx, g) in groups.iter().enumerate() {
            for k in g {
                course_to_group.insert(k.as_str(), idx);
            }
        }

        // Compute group priorities as max of member priorities
        let mut group_priority: Vec<usize> = Vec::with_capacity(groups.len());
        for g in groups {
            let pri = g
                .iter()
                .map(|k| chain_priority.get(k).copied().unwrap_or(0))
                .max()
                .unwrap_or(0);
            group_priority.push(pri);
        }

        // Build group dependency graph: edge A -> B if any course in B depends (transitively) on a course in A
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
        let mut indeg: Vec<usize> = vec![0; groups.len()];

        // Cache for transitive prerequisites restricted to `course_set`
        let mut prereq_cache: HashMap<String, HashSet<String>> = HashMap::new();

        for (b_idx, group) in groups.iter().enumerate() {
            let mut prereq_groups: HashSet<usize> = HashSet::new();
            for key in group {
                let deps = self.collect_transitive_prereqs_in_set(
                    key.as_str(),
                    course_set,
                    &mut prereq_cache,
                );
                for dep in deps {
                    if let Some(&a_idx) = course_to_group.get(dep.as_str()) {
                        if a_idx != b_idx {
                            prereq_groups.insert(a_idx);
                        }
                    }
                }
            }

            for a_idx in prereq_groups {
                adj[a_idx].push(b_idx);
                indeg[b_idx] += 1;
            }
        }

        // Kahn's algorithm with priority queue to break ties by group_priority (desc), then by lexicographic min key
        let mut heap = BinaryHeap::new();
        for i in 0..groups.len() {
            if indeg[i] == 0 {
                let name_hint = groups[i].iter().min().cloned().unwrap_or_else(String::new);
                heap.push(GroupPQItem {
                    pri: group_priority[i],
                    name_hint,
                    idx: i,
                });
            }
        }

        let mut order: Vec<usize> = Vec::with_capacity(groups.len());
        while let Some(GroupPQItem { idx, .. }) = heap.pop() {
            order.push(idx);
            for &v in &adj[idx] {
                indeg[v] -= 1;
                if indeg[v] == 0 {
                    let name_hint = groups[v].iter().min().cloned().unwrap_or_else(String::new);
                    heap.push(GroupPQItem {
                        pri: group_priority[v],
                        name_hint,
                        idx: v,
                    });
                }
            }
        }

        // If cycle detected (shouldn't happen in DAG), fall back to priority sort
        if order.len() != groups.len() {
            let mut fallback = groups.to_vec();
            self.sort_groups_by_priority(&mut fallback, chain_priority);
            return fallback;
        }

        // Reorder groups by computed order
        let mut result = Vec::with_capacity(groups.len());
        for i in order {
            result.push(groups[i].clone());
        }
        result
    }

    /// Collect transitive prerequisites of `course` restricted to `course_set`.
    /// Uses DFS with caching to avoid recomputation.
    fn collect_transitive_prereqs_in_set(
        &self,
        course: &str,
        course_set: &HashSet<&String>,
        cache: &mut HashMap<String, HashSet<String>>,
    ) -> HashSet<String> {
        if let Some(cached) = cache.get(course) {
            return cached.clone();
        }

        let mut visited: HashSet<String> = HashSet::new();
        let mut stack: Vec<&str> = Vec::new();
        stack.push(course);

        while let Some(cur) = stack.pop() {
            if let Some(prs) = self.dag.dependencies.get(cur) {
                for p in prs {
                    if !visited.contains(p) {
                        // Only traverse within the plan's course set
                        if course_set.contains(p) {
                            stack.push(p);
                        }
                        visited.insert(p.to_string());
                    }
                }
            }
        }

        cache.insert(course.to_string(), visited.clone());
        visited
    }

    /// Separate groups into filler (isolated) and priority (connected) groups
    fn separate_filler_groups(
        &self,
        groups: Vec<Vec<String>>,
        course_set: &HashSet<&String>,
    ) -> (Vec<Vec<String>>, Vec<Vec<String>>) {
        let mut filler_groups = Vec::new();
        let mut priority_groups = Vec::new();

        for group in groups {
            let is_filler = group.iter().all(|k| {
                let has_prereqs = self
                    .dag
                    .dependencies
                    .get(k)
                    .is_some_and(|p| p.iter().any(|d| course_set.contains(d)));
                let has_dependents = self
                    .dag
                    .dependents
                    .get(k)
                    .is_some_and(|d| d.iter().any(|dep| course_set.contains(dep)));
                !has_prereqs && !has_dependents
            });

            if is_filler {
                filler_groups.push(group);
            } else {
                priority_groups.push(group);
            }
        }

        // Sort filler groups by course key
        filler_groups.sort_by(|a, b| {
            let min_a = a.iter().min().cloned().unwrap_or_default();
            let min_b = b.iter().min().cloned().unwrap_or_default();
            min_a.cmp(&min_b)
        });

        (filler_groups, priority_groups)
    }

    /// Schedule priority groups (courses with dependencies)
    fn schedule_priority_groups(
        &self,
        groups: &[Vec<String>],
        plan: &mut TermPlan,
        course_term: &mut HashMap<String, usize>,
        course_set: &HashSet<&String>,
    ) {
        for group in groups {
            let min_term = self.calculate_earliest_term(group, course_term, course_set);
            let group_credits: f32 = group
                .iter()
                .filter_map(|k| self.school.get_course(k))
                .map(|c| c.credit_hours)
                .sum();

            let term_idx = self.find_best_term(plan, min_term, group_credits);

            for key in group {
                if let Some(course) = self.school.get_course(key) {
                    plan.terms[term_idx].add_course(key.clone(), course.credit_hours);
                    course_term.insert(key.clone(), term_idx);
                }
            }
        }
    }

    /// Schedule filler groups to balance term loads
    fn schedule_filler_groups(
        &self,
        groups: &[Vec<String>],
        plan: &mut TermPlan,
        course_term: &mut HashMap<String, usize>,
    ) {
        for group in groups {
            let group_credits: f32 = group
                .iter()
                .filter_map(|k| self.school.get_course(k))
                .map(|c| c.credit_hours)
                .sum();

            let term_idx = self.find_underloaded_term(plan, group_credits);

            for key in group {
                if let Some(course) = self.school.get_course(key) {
                    plan.terms[term_idx].add_course(key.clone(), course.credit_hours);
                    course_term.insert(key.clone(), term_idx);
                }
            }
        }
    }

    /// Find the term with lowest credits that can accommodate the group
    fn find_underloaded_term(&self, plan: &mut TermPlan, group_credits: f32) -> usize {
        // Find the term with minimum credits that won't exceed max
        let mut best_term = 0;
        let mut min_credits = f32::MAX;

        for (idx, term) in plan.terms.iter().enumerate() {
            let projected = term.total_credits + group_credits;
            if projected <= self.config.max_credits && term.total_credits < min_credits {
                min_credits = term.total_credits;
                best_term = idx;
            }
        }

        // If no term fits, add a new one
        if min_credits == f32::MAX {
            plan.add_term();
            plan.terms.len() - 1
        } else {
            best_term
        }
    }

    /// Rebalance terms by moving low-complexity courses from overloaded to underloaded terms
    fn rebalance_terms(&self, plan: &mut TermPlan, delay_factors: &HashMap<String, usize>) {
        let target = self.config.target_credits;

        // Multiple passes to iteratively balance
        for _ in 0..3 {
            // Find overloaded and underloaded terms
            let mut overloaded: Vec<usize> = Vec::new();
            let mut underloaded: Vec<usize> = Vec::new();

            for (idx, term) in plan.terms.iter().enumerate() {
                if term.total_credits > target + 3.0 {
                    overloaded.push(idx);
                } else if term.total_credits < target - 3.0 && !term.courses.is_empty() {
                    underloaded.push(idx);
                }
            }

            // Try to move courses from overloaded to underloaded
            for &over_idx in &overloaded {
                if underloaded.is_empty() {
                    break;
                }

                // Find movable courses (low delay, no prerequisite issues)
                let movable: Vec<(String, f32)> = plan.terms[over_idx]
                    .courses
                    .iter()
                    .filter_map(|k| {
                        let delay = delay_factors.get(k).copied().unwrap_or(0);
                        let credits = self.school.get_course(k).map_or(0.0, |c| c.credit_hours);
                        // Only move low-complexity courses with no dependents in later terms
                        if delay <= 1 && !self.has_dependents_in_later_terms(k, over_idx, plan) {
                            Some((k.clone(), credits))
                        } else {
                            None
                        }
                    })
                    .collect();

                for (course_key, credits) in movable {
                    // Find an underloaded term that can accept this course
                    for &under_idx in &underloaded {
                        if under_idx == over_idx {
                            continue;
                        }

                        let projected = plan.terms[under_idx].total_credits + credits;
                        let over_projected = plan.terms[over_idx].total_credits - credits;

                        // Only move if it improves balance
                        if projected <= target + 1.0 && over_projected >= target - 3.0 {
                            // Move the course
                            plan.terms[over_idx].courses.retain(|k| k != &course_key);
                            plan.terms[over_idx].total_credits -= credits;
                            plan.terms[under_idx].add_course(course_key.clone(), credits);
                            break;
                        }
                    }
                }
            }
        }
    }

    /// Check if a course has dependents scheduled in later terms
    fn has_dependents_in_later_terms(
        &self,
        course_key: &str,
        term_idx: usize,
        plan: &TermPlan,
    ) -> bool {
        if let Some(dependents) = self.dag.dependents.get(course_key) {
            for dep in dependents {
                for (idx, term) in plan.terms.iter().enumerate() {
                    if idx > term_idx && term.courses.contains(dep) {
                        return true;
                    }
                }
            }
        }
        false
    }

    /// Build groups of courses that must be in the same term (corequisites/strict corequisites)
    /// This performs bidirectional search: if A has B as coreq, or B has A as coreq, they're grouped
    fn build_corequisite_groups(&self, course_keys: &[String]) -> Vec<Vec<String>> {
        let course_set: HashSet<_> = course_keys.iter().cloned().collect();
        let mut visited: HashSet<String> = HashSet::new();
        let mut groups: Vec<Vec<String>> = Vec::new();

        // Build reverse corequisite map: for each course, find courses that list it as corequisite
        let mut reverse_coreqs: HashMap<String, Vec<String>> = HashMap::new();
        for key in course_keys {
            if let Some(course) = self.school.get_course(key) {
                for coreq in &course.corequisites {
                    if course_set.contains(coreq) {
                        reverse_coreqs
                            .entry(coreq.clone())
                            .or_default()
                            .push(key.clone());
                    }
                }
                for coreq in &course.strict_corequisites {
                    if course_set.contains(coreq) {
                        reverse_coreqs
                            .entry(coreq.clone())
                            .or_default()
                            .push(key.clone());
                    }
                }
            }
        }

        for key in course_keys {
            if visited.contains(key) {
                continue;
            }

            let mut group = vec![key.clone()];
            let mut to_check = vec![key.clone()];
            visited.insert(key.clone());

            // BFS to find all connected corequisites (bidirectional)
            while let Some(current) = to_check.pop() {
                // Forward direction: courses this one lists as corequisites
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

                // Reverse direction: courses that list this one as corequisite
                if let Some(rev_coreqs) = reverse_coreqs.get(&current) {
                    for rev_coreq in rev_coreqs {
                        if !visited.contains(rev_coreq) {
                            group.push(rev_coreq.clone());
                            to_check.push(rev_coreq.clone());
                            visited.insert(rev_coreq.clone());
                        }
                    }
                }
            }

            if !group.is_empty() {
                // Sort group by course key for consistent ordering
                group.sort();
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

    #[test]
    fn test_term_plan_quarter_system() {
        let plan = TermPlan::new(12, true, 15.0);
        assert_eq!(plan.terms.len(), 12);
        assert_eq!(plan.term_label(), "Quarter");
        assert!(plan.is_quarter_system);
    }

    #[test]
    fn test_term_add_course() {
        let mut term = Term::new(1);
        assert_eq!(term.courses.len(), 0);
        assert!((term.total_credits - 0.0).abs() < f32::EPSILON);

        term.add_course("CS101".to_string(), 3.0);
        assert_eq!(term.courses.len(), 1);
        assert!((term.total_credits - 3.0).abs() < f32::EPSILON);

        term.add_course("CS102".to_string(), 4.0);
        assert_eq!(term.courses.len(), 2);
        assert!((term.total_credits - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_term_plan_add_term() {
        let mut plan = TermPlan::new(2, false, 15.0);
        assert_eq!(plan.terms.len(), 2);

        plan.add_term();
        assert_eq!(plan.terms.len(), 3);
        assert_eq!(plan.terms[2].number, 3);
    }

    #[test]
    fn test_terms_used_counts_nonempty() {
        let mut plan = TermPlan::new(4, false, 15.0);
        assert_eq!(plan.terms_used(), 0);

        plan.terms[0].add_course("CS101".to_string(), 3.0);
        assert_eq!(plan.terms_used(), 1);

        plan.terms[2].add_course("CS201".to_string(), 3.0);
        assert_eq!(plan.terms_used(), 2);
    }

    #[test]
    fn test_scheduler_config_semester() {
        let config = SchedulerConfig::semester(15.0);
        assert!((config.target_credits - 15.0).abs() < f32::EPSILON);
        assert!((config.max_credits - 21.0).abs() < f32::EPSILON); // 15 + 6
        assert_eq!(config.num_terms, 8);
        assert!(!config.is_quarter_system);
    }

    #[test]
    fn test_scheduler_config_quarter() {
        let config = SchedulerConfig::quarter(15.0);
        assert!((config.target_credits - 15.0).abs() < f32::EPSILON);
        assert!((config.max_credits - 19.0).abs() < f32::EPSILON); // 15 + 4
        assert_eq!(config.num_terms, 12);
        assert!(config.is_quarter_system);
    }

    #[test]
    fn test_corequisites_same_term() {
        let mut school = School::new("Test".to_string());

        let cs101 = Course::new(
            "Intro".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );
        let mut cs101l = Course::new(
            "Intro Lab".to_string(),
            "CS".to_string(),
            "101L".to_string(),
            1.0,
        );
        cs101l.add_strict_corequisite("CS101".to_string());

        school.add_course(cs101);
        school.add_course(cs101l);

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());
        dag.add_course("CS101L".to_string());
        dag.add_corequisite("CS101L".to_string(), "CS101");

        let config = SchedulerConfig::semester(15.0);
        let scheduler = TermScheduler::new(&school, &dag, config);

        let courses = vec!["CS101".to_string(), "CS101L".to_string()];
        let plan = scheduler.schedule(&courses);

        // Both should be in the same term
        let main_course_term = plan
            .terms
            .iter()
            .position(|t| t.courses.contains(&"CS101".to_string()));
        let lab_course_term = plan
            .terms
            .iter()
            .position(|t| t.courses.contains(&"CS101L".to_string()));

        assert_eq!(main_course_term, lab_course_term);
    }

    #[test]
    fn test_schedule_respects_credit_limits() {
        let mut school = School::new("Test".to_string());

        // Create 6 courses, each 4 credits
        for i in 1..=6 {
            let course = Course::new(
                format!("Course {i}"),
                "CS".to_string(),
                format!("{i}00"),
                4.0,
            );
            school.add_course(course);
        }

        let mut dag = DAG::new();
        for i in 1..=6 {
            dag.add_course(format!("CS{i}00"));
        }

        let config = SchedulerConfig::semester(15.0); // max ~21
        let max_credits = config.max_credits;
        let scheduler = TermScheduler::new(&school, &dag, config);

        let courses: Vec<String> = (1..=6).map(|i| format!("CS{i}00")).collect();
        let plan = scheduler.schedule(&courses);

        // No term should exceed max credits
        for term in &plan.terms {
            assert!(term.total_credits <= max_credits + 0.01);
        }
    }

    #[test]
    fn test_filler_courses_balanced() {
        let mut school = School::new("Test".to_string());

        // Create a chain course and several standalone courses
        let cs101 = Course::new(
            "Intro".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        );
        let mut cs201 = Course::new(
            "Advanced".to_string(),
            "CS".to_string(),
            "201".to_string(),
            3.0,
        );
        cs201.add_prerequisite("CS101".to_string());

        // Filler courses (no prereqs or dependents)
        let gen_ed1 = Course::new(
            "Gen Ed 1".to_string(),
            "GEN".to_string(),
            "101".to_string(),
            3.0,
        );
        let gen_ed2 = Course::new(
            "Gen Ed 2".to_string(),
            "GEN".to_string(),
            "102".to_string(),
            3.0,
        );

        school.add_course(cs101);
        school.add_course(cs201);
        school.add_course(gen_ed1);
        school.add_course(gen_ed2);

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());
        dag.add_course("CS201".to_string());
        dag.add_course("GEN101".to_string());
        dag.add_course("GEN102".to_string());
        dag.add_prerequisite("CS201".to_string(), "CS101");

        let config = SchedulerConfig::semester(15.0);
        let scheduler = TermScheduler::new(&school, &dag, config);

        let courses = vec![
            "CS101".to_string(),
            "CS201".to_string(),
            "GEN101".to_string(),
            "GEN102".to_string(),
        ];
        let plan = scheduler.schedule(&courses);

        // All courses should be scheduled
        let total_scheduled: usize = plan.terms.iter().map(|t| t.courses.len()).sum();
        assert_eq!(total_scheduled, 4);
    }
}
