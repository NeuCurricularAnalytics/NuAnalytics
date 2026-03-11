//! Gen-ed tracking for cross-category course sharing
//!
//! Tracks which general education attributes have been satisfied by courses
//! selected for major/supporting requirements, allowing gen-ed requirements
//! to be reduced or skipped when already satisfied.
//!
//! # Problem
//!
//! Courses often satisfy requirements in multiple categories. For example:
//! - ICS141 (Discrete Math) has `gen_ed_attributes: [FQ]` - satisfies FQ gen-ed
//! - MATH241 (Calculus I) satisfies both calculus requirement AND FQ gen-ed
//!
//! Without tracking, these get counted twice, inflating credit totals.
//!
//! # Solution
//!
//! This module provides a tracker that:
//! 1. Records which gen-ed attributes are satisfied as courses are selected
//! 2. Calculates remaining credits needed for each gen-ed requirement
//! 3. Allows gen-ed requirements to be skipped when already satisfied

use crate::core::models::course::Course;
use std::collections::{HashMap, HashSet};

/// Tracks gen-ed attribute satisfaction during plan building
///
/// As courses are selected for major and supporting requirements,
/// their gen-ed attributes are recorded here. Gen-ed requirements
/// can then check if they're already satisfied before adding more courses.
#[derive(Debug, Clone, Default)]
pub struct GenEdTracker {
    /// Map from gen-ed attribute code to courses that satisfy it
    /// Example: `"FQ" -> ["MATH241", "ICS141"]`
    satisfied_by: HashMap<String, Vec<String>>,

    /// Map from gen-ed attribute code to total credits satisfying it
    /// Example: `"FQ" -> 7.0` (from MATH241=4 + ICS141=3)
    satisfied_credits: HashMap<String, f32>,

    /// Gen-ed requirements and their target credits
    /// Example: `"FQ" -> 3.0`, `"FW" -> 3.0`
    pub required_credits: HashMap<String, f32>,

    /// Gen-ed requirements that need multiple categories
    /// Example: `"FG" -> 2` (must have courses from 2 different FG subcategories)
    min_categories: HashMap<String, usize>,

    /// Track which sub-categories have been satisfied for grouped gen-eds
    /// Example: `"FG" -> {"FGA", "FGB"}` (two of three FG categories satisfied)
    satisfied_subcategories: HashMap<String, HashSet<String>>,
}

impl GenEdTracker {
    /// Create a new empty tracker
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a tracker with known gen-ed requirements
    ///
    /// # Arguments
    /// * `requirements` - Map from gen-ed code to required credits
    #[must_use]
    pub fn with_requirements(requirements: HashMap<String, f32>) -> Self {
        Self {
            required_credits: requirements,
            ..Default::default()
        }
    }

    /// Set the minimum number of categories required for a grouped gen-ed
    ///
    /// Some gen-ed requirements (like "Global Perspectives") require courses
    /// from multiple subcategories (e.g., FGA, FGB, FGC - need 2 of 3).
    pub fn set_min_categories(&mut self, gen_ed_code: &str, min: usize) {
        self.min_categories.insert(gen_ed_code.to_string(), min);
    }

    /// Record that a course has been selected, tracking its gen-ed contributions
    ///
    /// # Arguments
    /// * `course_key` - The course identifier (e.g., "MATH241")
    /// * `course` - The course data containing gen-ed attributes and credits
    pub fn record_course(&mut self, course_key: &str, course: &Course) {
        let Some(attrs) = &course.gen_ed_attributes else {
            return;
        };

        for attr in attrs {
            // Track which course satisfies this attribute
            self.satisfied_by
                .entry(attr.clone())
                .or_default()
                .push(course_key.to_string());

            // Track credits for this specific attribute
            *self.satisfied_credits.entry(attr.clone()).or_default() += course.credit_hours;

            // Track subcategories for grouped gen-eds
            // E.g., "FGA" contributes to parent "FG"
            if let Some(parent) = get_parent_category(attr) {
                // Track which subcategory was satisfied
                self.satisfied_subcategories
                    .entry(parent.clone())
                    .or_default()
                    .insert(attr.clone());

                // Also track credits against the parent category
                *self.satisfied_credits.entry(parent.clone()).or_default() += course.credit_hours;

                // Track that this course satisfies the parent too
                self.satisfied_by
                    .entry(parent)
                    .or_default()
                    .push(course_key.to_string());
            }
        }
    }

    /// Record a course by key, looking up its data from a course map
    ///
    /// # Arguments
    /// * `course_key` - The course identifier
    /// * `courses` - Map of all available courses
    pub fn record_course_by_key(&mut self, course_key: &str, courses: &HashMap<String, Course>) {
        if let Some(course) = courses.get(course_key) {
            self.record_course(course_key, course);
        }
    }

    /// Check if a gen-ed attribute is fully satisfied
    ///
    /// Returns true if the satisfied credits meet or exceed required credits.
    #[must_use]
    pub fn is_satisfied(&self, gen_ed_code: &str) -> bool {
        let required = self
            .required_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0);
        let satisfied = self
            .satisfied_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0);

        // Check credit requirement
        if satisfied < required {
            return false;
        }

        // Check category requirement if applicable
        if let Some(&min_cats) = self.min_categories.get(gen_ed_code) {
            let satisfied_cats = self
                .satisfied_subcategories
                .get(gen_ed_code)
                .map_or(0, std::collections::HashSet::len);
            if satisfied_cats < min_cats {
                return false;
            }
        }

        true
    }

    /// Get the remaining credits needed for a gen-ed requirement
    ///
    /// Returns 0.0 if already satisfied or if the requirement isn't tracked.
    #[must_use]
    pub fn remaining_credits(&self, gen_ed_code: &str) -> f32 {
        let required = self
            .required_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0);
        let satisfied = self
            .satisfied_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0);

        (required - satisfied).max(0.0)
    }

    /// Get the credits already satisfied for a gen-ed attribute
    #[must_use]
    pub fn satisfied_credits(&self, gen_ed_code: &str) -> f32 {
        self.satisfied_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0)
    }

    /// Get all courses that satisfy a gen-ed attribute
    #[must_use]
    pub fn courses_satisfying(&self, gen_ed_code: &str) -> Vec<String> {
        self.satisfied_by
            .get(gen_ed_code)
            .cloned()
            .unwrap_or_default()
    }

    /// Get all satisfied gen-ed codes
    #[must_use]
    pub fn all_satisfied_codes(&self) -> Vec<String> {
        self.satisfied_credits.keys().cloned().collect()
    }

    /// Check if any credits have been recorded for a gen-ed attribute
    #[must_use]
    pub fn has_any_credits(&self, gen_ed_code: &str) -> bool {
        self.satisfied_credits
            .get(gen_ed_code)
            .copied()
            .unwrap_or(0.0)
            > 0.0
    }

    /// Get a summary of gen-ed satisfaction status
    #[must_use]
    pub fn summary(&self) -> GenEdSummary {
        let mut fully_satisfied = Vec::new();
        let mut partially_satisfied = Vec::new();
        let mut unsatisfied = Vec::new();

        for (code, &required) in &self.required_credits {
            let satisfied = self.satisfied_credits.get(code).copied().unwrap_or(0.0);

            if self.is_satisfied(code) {
                fully_satisfied.push(code.clone());
            } else if satisfied > 0.0 {
                partially_satisfied.push((code.clone(), satisfied, required));
            } else {
                unsatisfied.push((code.clone(), required));
            }
        }

        GenEdSummary {
            fully_satisfied,
            partially_satisfied,
            unsatisfied,
        }
    }

    /// Merge another tracker into this one
    ///
    /// Used when combining gen-ed tracking from multiple requirement categories.
    pub fn merge(&mut self, other: &Self) {
        for (code, courses) in &other.satisfied_by {
            self.satisfied_by
                .entry(code.clone())
                .or_default()
                .extend(courses.iter().cloned());
        }

        for (code, credits) in &other.satisfied_credits {
            *self.satisfied_credits.entry(code.clone()).or_default() += credits;
        }

        for (code, subcats) in &other.satisfied_subcategories {
            self.satisfied_subcategories
                .entry(code.clone())
                .or_default()
                .extend(subcats.iter().cloned());
        }
    }
}

/// Summary of gen-ed satisfaction status
#[derive(Debug, Clone)]
pub struct GenEdSummary {
    /// Gen-ed codes that are fully satisfied
    pub fully_satisfied: Vec<String>,

    /// Gen-ed codes that are partially satisfied: (code, `satisfied_credits`, `required_credits`)
    pub partially_satisfied: Vec<(String, f32, f32)>,

    /// Gen-ed codes with no satisfaction: (code, `required_credits`)
    pub unsatisfied: Vec<(String, f32)>,
}

impl GenEdSummary {
    /// Check if all gen-ed requirements are satisfied
    #[must_use]
    pub const fn all_satisfied(&self) -> bool {
        self.partially_satisfied.is_empty() && self.unsatisfied.is_empty()
    }

    /// Get total remaining credits needed across all gen-ed requirements
    #[must_use]
    pub fn total_remaining_credits(&self) -> f32 {
        let partial: f32 = self
            .partially_satisfied
            .iter()
            .map(|(_, sat, req)| req - sat)
            .sum();
        let unsatisfied: f32 = self.unsatisfied.iter().map(|(_, req)| req).sum();
        partial + unsatisfied
    }
}

/// Extract parent category from a gen-ed subcategory code
///
/// Many gen-ed systems have grouped requirements where subcategories
/// like "FGA", "FGB", "FGC" all contribute to a parent "FG" requirement.
///
/// # Examples
/// - "FGA" -> Some("FG")
/// - "DA" -> Some("D") (Arts within Diversification)
/// - "FQ" -> None (standalone, not a subcategory)
fn get_parent_category(code: &str) -> Option<String> {
    // Common patterns for subcategories:
    // - 2-letter base + 1-letter suffix: "FGA" -> "FG"
    // - 1-letter base + 1-letter suffix: "DA" -> "D"

    if code.len() >= 3 {
        // Check for 2-letter parent + suffix pattern (e.g., "FGA" -> "FG")
        let parent = &code[..2];
        let suffix = &code[2..];
        if suffix.len() == 1 && suffix.chars().all(|c| c.is_ascii_alphabetic()) {
            return Some(parent.to_string());
        }
    }

    if code.len() == 2 {
        // Check for 1-letter parent + suffix pattern (e.g., "DA" -> "D")
        // But avoid false positives like "FQ" which is standalone
        let first = &code[..1];
        let second = &code[1..];

        // Common single-letter parents: D (Diversification), G (Global)
        // These typically have alphabetic suffixes
        if matches!(first, "D" | "G") && second.chars().all(|c| c.is_ascii_alphabetic()) {
            return Some(first.to_string());
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_course(credits: f32, gen_ed: Vec<&str>) -> Course {
        Course {
            credit_hours: credits,
            gen_ed_attributes: Some(gen_ed.into_iter().map(String::from).collect()),
            ..Default::default()
        }
    }

    #[test]
    fn test_record_course_tracks_credits() {
        let mut tracker = GenEdTracker::new();

        let calc = make_course(4.0, vec!["FQ"]);
        tracker.record_course("MATH241", &calc);

        assert!((tracker.satisfied_credits("FQ") - 4.0).abs() < f32::EPSILON);
        assert_eq!(tracker.courses_satisfying("FQ"), vec!["MATH241"]);
    }

    #[test]
    fn test_multiple_courses_same_attribute() {
        let mut tracker = GenEdTracker::new();

        let calc = make_course(4.0, vec!["FQ"]);
        let discrete = make_course(3.0, vec!["FQ"]);

        tracker.record_course("MATH241", &calc);
        tracker.record_course("ICS141", &discrete);

        assert!((tracker.satisfied_credits("FQ") - 7.0).abs() < f32::EPSILON);
        assert_eq!(tracker.courses_satisfying("FQ").len(), 2);
    }

    #[test]
    fn test_is_satisfied_with_requirements() {
        let mut requirements = HashMap::new();
        requirements.insert("FQ".to_string(), 3.0);
        requirements.insert("FW".to_string(), 3.0);

        let mut tracker = GenEdTracker::with_requirements(requirements);

        // FQ not satisfied yet
        assert!(!tracker.is_satisfied("FQ"));
        assert!((tracker.remaining_credits("FQ") - 3.0).abs() < f32::EPSILON);

        // Add a course that satisfies FQ
        let calc = make_course(4.0, vec!["FQ"]);
        tracker.record_course("MATH241", &calc);

        // FQ now satisfied (4.0 >= 3.0)
        assert!(tracker.is_satisfied("FQ"));
        assert!((tracker.remaining_credits("FQ")).abs() < f32::EPSILON);

        // FW still not satisfied
        assert!(!tracker.is_satisfied("FW"));
    }

    #[test]
    fn test_partial_satisfaction() {
        let mut requirements = HashMap::new();
        requirements.insert("DA".to_string(), 6.0);

        let mut tracker = GenEdTracker::with_requirements(requirements);

        let art1 = make_course(3.0, vec!["DA"]);
        tracker.record_course("ART101", &art1);

        assert!(!tracker.is_satisfied("DA"));
        assert!((tracker.remaining_credits("DA") - 3.0).abs() < f32::EPSILON);
        assert!(tracker.has_any_credits("DA"));
    }

    #[test]
    fn test_subcategory_tracking() {
        let mut tracker = GenEdTracker::new();

        let course_a = make_course(3.0, vec!["FGA"]);
        let course_b = make_course(3.0, vec!["FGB"]);

        tracker.record_course("HIST101", &course_a);
        tracker.record_course("ANTH101", &course_b);

        // Both should contribute to parent "FG"
        let subcats = tracker.satisfied_subcategories.get("FG");
        assert!(subcats.is_some());
        let subcats = subcats.unwrap();
        assert!(subcats.contains("FGA"));
        assert!(subcats.contains("FGB"));
    }

    #[test]
    fn test_min_categories_requirement() {
        let mut requirements = HashMap::new();
        requirements.insert("FG".to_string(), 6.0);

        let mut tracker = GenEdTracker::with_requirements(requirements);
        tracker.set_min_categories("FG", 2);

        // Add two courses from same subcategory
        let course1 = make_course(3.0, vec!["FGA"]);
        let course2 = make_course(3.0, vec!["FGA"]);

        tracker.record_course("HIST101", &course1);
        tracker.record_course("HIST201", &course2);

        // Credits are met (6.0 >= 6.0) but only one category
        assert!(!tracker.is_satisfied("FG"));

        // Add course from different subcategory
        let course3 = make_course(3.0, vec!["FGB"]);
        tracker.record_course("ANTH101", &course3);

        // Now both requirements are met
        assert!(tracker.is_satisfied("FG"));
    }

    #[test]
    fn test_summary() {
        let mut requirements = HashMap::new();
        requirements.insert("FQ".to_string(), 3.0);
        requirements.insert("FW".to_string(), 3.0);
        requirements.insert("DA".to_string(), 6.0);

        let mut tracker = GenEdTracker::with_requirements(requirements);

        // Satisfy FQ completely
        let calc = make_course(4.0, vec!["FQ"]);
        tracker.record_course("MATH241", &calc);

        // Partially satisfy DA
        let art = make_course(3.0, vec!["DA"]);
        tracker.record_course("ART101", &art);

        // FW left unsatisfied

        let summary = tracker.summary();
        assert_eq!(summary.fully_satisfied, vec!["FQ"]);
        assert_eq!(summary.partially_satisfied.len(), 1);
        assert_eq!(summary.partially_satisfied[0].0, "DA");
        assert_eq!(summary.unsatisfied.len(), 1);
        assert_eq!(summary.unsatisfied[0].0, "FW");
    }

    #[test]
    fn test_get_parent_category() {
        assert_eq!(get_parent_category("FGA"), Some("FG".to_string()));
        assert_eq!(get_parent_category("FGB"), Some("FG".to_string()));
        assert_eq!(get_parent_category("DA"), Some("D".to_string()));
        assert_eq!(get_parent_category("DH"), Some("D".to_string()));
        assert_eq!(get_parent_category("FQ"), None); // Standalone
        assert_eq!(get_parent_category("FW"), None); // Standalone
    }

    #[test]
    fn test_merge_trackers() {
        let mut tracker1 = GenEdTracker::new();
        let mut tracker2 = GenEdTracker::new();

        let calc = make_course(4.0, vec!["FQ"]);
        let writing = make_course(3.0, vec!["FW"]);

        tracker1.record_course("MATH241", &calc);
        tracker2.record_course("ENG100", &writing);

        tracker1.merge(&tracker2);

        assert!((tracker1.satisfied_credits("FQ") - 4.0).abs() < f32::EPSILON);
        assert!((tracker1.satisfied_credits("FW") - 3.0).abs() < f32::EPSILON);
    }
}
