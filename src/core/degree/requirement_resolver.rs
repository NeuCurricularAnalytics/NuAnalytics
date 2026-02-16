//! Requirement resolver for expanding degree requirements into course choices
//!
//! Converts degree requirements into concrete sets of course options that can
//! be combined to generate all possible degree plans.

use crate::core::models::course::Course;
use crate::core::models::degree::{FromClause, Requirement, RequirementType};
use std::collections::{HashMap, HashSet};

/// Represents the possible choices for a single requirement
#[derive(Debug, Clone)]
pub struct ResolvedRequirement {
    /// Requirement identifier
    pub id: String,

    /// Human-readable name
    pub name: String,

    /// Category (major, supporting, elective, etc.)
    pub category: Option<String>,

    /// All possible ways to satisfy this requirement
    /// Each inner Vec is one valid combination of courses
    pub choices: Vec<Vec<String>>,

    /// Whether this requirement contributes to plan explosion
    /// (has multiple choices that affect metrics)
    pub is_variable: bool,

    /// Estimated contribution to total plan count
    pub choice_count: usize,

    /// Whether to exclude courses already used in prior requirements
    pub exclude_used: bool,

    /// For `exclude_used` requirements: the full pool of available courses
    /// Used to dynamically select courses when some are already used
    pub available_pool: Option<Vec<String>>,

    /// For `exclude_used` requirements: the number of courses needed
    pub courses_needed: Option<usize>,
}

impl ResolvedRequirement {
    /// Create a fixed requirement with exactly one choice
    #[must_use]
    pub fn fixed(id: String, name: String, category: Option<String>, courses: Vec<String>) -> Self {
        Self {
            id,
            name,
            category,
            choices: vec![courses],
            is_variable: false,
            choice_count: 1,
            exclude_used: false,
            available_pool: None,
            courses_needed: None,
        }
    }

    /// Create a variable requirement with multiple choices
    #[must_use]
    #[allow(clippy::missing_const_for_fn)] // Vec::len() is not const
    pub fn variable(
        id: String,
        name: String,
        category: Option<String>,
        choices: Vec<Vec<String>>,
    ) -> Self {
        let choice_count = choices.len();
        Self {
            id,
            name,
            category,
            choices,
            is_variable: choice_count > 1,
            choice_count,
            exclude_used: false,
            available_pool: None,
            courses_needed: None,
        }
    }

    /// Set whether this requirement should exclude courses used in prior requirements
    #[must_use]
    pub const fn with_exclude_used(mut self, exclude: bool) -> Self {
        self.exclude_used = exclude;
        self
    }

    /// Set the available pool for dynamic selection when `exclude_used` is true
    #[must_use]
    pub fn with_available_pool(mut self, pool: Vec<String>, count: usize) -> Self {
        self.available_pool = Some(pool);
        self.courses_needed = Some(count);
        self
    }
}

/// Resolves degree requirements into enumerable course choices
pub struct RequirementResolver<'a> {
    /// Available courses for pattern matching
    courses: &'a HashMap<String, Course>,

    /// Cache of pattern match results
    pattern_cache: HashMap<String, Vec<String>>,

    /// Courses from fixed (type: all) requirements that should be excluded
    /// from select requirements with `exclude_used`
    fixed_courses: HashSet<String>,
}

impl<'a> RequirementResolver<'a> {
    /// Create a new requirement resolver
    #[must_use]
    pub fn new(courses: &'a HashMap<String, Course>) -> Self {
        Self {
            courses,
            pattern_cache: HashMap::new(),
            fixed_courses: HashSet::new(),
        }
    }

    /// Resolve all requirements in a degree program
    ///
    /// Requirements are processed in dependency order:
    /// 1. Requirements without `exclude_used` constraint come first
    /// 2. Requirements with `exclude_used` come later (they depend on prior selections)
    ///
    /// # Arguments
    /// * `requirements` - Map of requirement ID to requirement definition
    ///
    /// # Returns
    /// Vector of resolved requirements with all possible course choices
    pub fn resolve_all(
        &mut self,
        requirements: &HashMap<String, Requirement>,
    ) -> Vec<ResolvedRequirement> {
        // Clear fixed courses from any previous resolution
        self.fixed_courses.clear();

        // Sort requirements: fixed (type: all) first, then non-exclude_used, then exclude_used
        // This ensures we know which courses are fixed before processing select requirements
        let mut sorted_reqs: Vec<_> = requirements.iter().collect();
        sorted_reqs.sort_by(|(id_a, req_a), (id_b, req_b)| {
            // Fixed requirements (type: all) come first
            let a_is_fixed = req_a.req_type == RequirementType::All;
            let b_is_fixed = req_b.req_type == RequirementType::All;

            if a_is_fixed != b_is_fixed {
                return if a_is_fixed {
                    std::cmp::Ordering::Less
                } else {
                    std::cmp::Ordering::Greater
                };
            }

            // Then sort by exclude_used
            let a_excludes = req_a
                .constraints
                .as_ref()
                .and_then(|c| c.exclude_used)
                .unwrap_or(false);
            let b_excludes = req_b
                .constraints
                .as_ref()
                .and_then(|c| c.exclude_used)
                .unwrap_or(false);

            // Sort by exclude_used (false < true), then by ID for stability
            match (a_excludes, b_excludes) {
                (false, true) => std::cmp::Ordering::Less,
                (true, false) => std::cmp::Ordering::Greater,
                _ => id_a.cmp(id_b),
            }
        });

        sorted_reqs
            .into_iter()
            .map(|(id, req)| self.resolve_requirement(id, req))
            .collect()
    }

    /// Resolve a single requirement into course choices
    pub fn resolve_requirement(&mut self, id: &str, req: &Requirement) -> ResolvedRequirement {
        let name = req.name.clone().unwrap_or_else(|| id.to_string());
        let category = req.category.clone();

        match req.req_type {
            RequirementType::All => self.resolve_all_requirement(id, &name, category, req),
            RequirementType::Select => self.resolve_select_requirement(id, &name, category, req),
            RequirementType::OneOf => self.resolve_oneof_requirement(id, &name, category, req),
        }
    }

    /// Resolve an "all" requirement (fixed courses)
    ///
    /// Also tracks the courses in `fixed_courses` so that select requirements
    /// with `exclude_used` can filter them out of their choice pools.
    fn resolve_all_requirement(
        &mut self,
        id: &str,
        name: &str,
        category: Option<String>,
        req: &Requirement,
    ) -> ResolvedRequirement {
        let courses = req.courses.clone().unwrap_or_default();
        // Expand any bundles/equivalents in the course list
        let expanded = self.expand_course_list(&courses);

        // Track these as fixed courses for exclude_used filtering
        for course in &expanded {
            self.fixed_courses.insert(course.clone());
        }

        let exclude_used = req
            .constraints
            .as_ref()
            .and_then(|c| c.exclude_used)
            .unwrap_or(false);
        ResolvedRequirement::fixed(id.to_string(), name.to_string(), category, expanded)
            .with_exclude_used(exclude_used)
    }

    /// Resolve a "select" requirement (choose N from options)
    ///
    /// When `exclude_used` is true, filters out courses from fixed requirements
    /// before generating combinations to avoid overlap.
    fn resolve_select_requirement(
        &mut self,
        id: &str,
        name: &str,
        category: Option<String>,
        req: &Requirement,
    ) -> ResolvedRequirement {
        let exclude_used = req
            .constraints
            .as_ref()
            .and_then(|c| c.exclude_used)
            .unwrap_or(false);

        // Check if this is a pure wildcard requirement that should use placeholders
        // Pure wildcards are patterns like "*:*" with no explicit course list
        if self.is_pure_wildcard_requirement(req) {
            let placeholder_courses = self.generate_placeholder_courses(id, req);
            if !placeholder_courses.is_empty() {
                return ResolvedRequirement::fixed(
                    id.to_string(),
                    name.to_string(),
                    category,
                    placeholder_courses,
                )
                .with_exclude_used(exclude_used);
            }
        }

        // Get the pool of courses to select from
        let mut pool = self.get_selection_pool(req.from.as_ref());

        // If exclude_used is true, filter out courses from fixed requirements
        // This prevents generating combinations that include courses already required elsewhere
        if exclude_used && !self.fixed_courses.is_empty() {
            pool.retain(|c| !self.fixed_courses.contains(c));
        }

        // If pool is empty but credits are specified, generate placeholder courses
        if pool.is_empty() {
            let placeholder_courses = self.generate_placeholder_courses(id, req);
            if !placeholder_courses.is_empty() {
                return ResolvedRequirement::fixed(
                    id.to_string(),
                    name.to_string(),
                    category,
                    placeholder_courses,
                )
                .with_exclude_used(exclude_used);
            }
            return ResolvedRequirement::fixed(
                id.to_string(),
                name.to_string(),
                category,
                Vec::new(),
            )
            .with_exclude_used(exclude_used);
        }

        // Determine how many to select
        let count = self.determine_selection_count(req, &pool);

        if count == 0 || count >= pool.len() {
            // Select all - single choice
            return ResolvedRequirement::fixed(id.to_string(), name.to_string(), category, pool)
                .with_exclude_used(exclude_used);
        }

        // Generate all combinations of size `count`
        let combinations = self.generate_combinations(&pool, count);

        // For exclude_used requirements, also track the pool and count for dynamic selection
        let mut resolved =
            ResolvedRequirement::variable(id.to_string(), name.to_string(), category, combinations)
                .with_exclude_used(exclude_used);

        // If exclude_used is true, store the pool for dynamic selection during plan building
        if exclude_used {
            resolved = resolved.with_available_pool(pool, count);
        }

        resolved
    }

    /// Check if a requirement uses only wildcard patterns (no explicit courses)
    ///
    /// Pure wildcard requirements should generate placeholder courses since
    /// we can't meaningfully enumerate all possible courses from a wildcard.
    #[allow(clippy::unused_self)]
    fn is_pure_wildcard_requirement(&self, req: &Requirement) -> bool {
        // Must have credits specified (not count)
        if req.count.is_some() || (req.credits.is_none() && req.credit_range.is_none()) {
            return false;
        }

        // Check the from clause
        let Some(from) = &req.from else {
            return false;
        };

        // If there are explicit courses, it's not pure wildcard
        if from.courses.is_some() || from.groups.is_some() {
            return false;
        }

        // Check if pattern is a wildcard
        if let Some(pattern) = &from.pattern {
            if pattern.contains('*') || pattern == "*:*" {
                return true;
            }
        }

        // Check include patterns
        if let Some(includes) = &from.include {
            if includes.iter().any(|p| p.contains('*')) {
                return true;
            }
        }

        false
    }

    /// Generate placeholder courses for requirements with wildcard patterns
    ///
    /// When a requirement specifies credits but uses a wildcard pattern (like "*:*")
    /// that doesn't match any defined courses, we create placeholder courses to
    /// represent those credits in the plan.
    #[allow(clippy::unused_self)]
    fn generate_placeholder_courses(&self, req_id: &str, req: &Requirement) -> Vec<String> {
        // Determine how many credits are needed
        let credits_needed = req
            .credits
            .or_else(|| req.credit_range.as_ref().map(|r| r.min));

        let Some(credits) = credits_needed else {
            return Vec::new();
        };

        // Generate placeholder course keys based on requirement ID
        // Use 3-credit courses as default, with smaller courses for remainder
        let full_courses = credits / 3;
        let remainder = credits % 3;

        let mut placeholders = Vec::new();
        let prefix = sanitize_placeholder_prefix(req_id);

        for i in 0..full_courses {
            placeholders.push(format!("{prefix}{:02}", i + 1));
        }

        if remainder > 0 {
            placeholders.push(format!("{prefix}{:02}S", full_courses + 1));
        }

        placeholders
    }

    /// Resolve a `one_of` requirement (mutually exclusive paths)
    fn resolve_oneof_requirement(
        &mut self,
        id: &str,
        name: &str,
        category: Option<String>,
        req: &Requirement,
    ) -> ResolvedRequirement {
        let options = req.options.as_ref();

        if options.is_none() || options.unwrap().is_empty() {
            return ResolvedRequirement::fixed(
                id.to_string(),
                name.to_string(),
                category,
                Vec::new(),
            );
        }

        let mut all_choices: Vec<Vec<String>> = Vec::new();

        for option in options.unwrap() {
            // Resolve nested requirements for this option
            let nested_courses = self.resolve_nested_requirements(&option.requirements);

            // Each option produces one or more course combinations
            // For simplicity, we flatten nested choices into single combinations
            // More complex handling could preserve nested variability
            for combo in nested_courses {
                all_choices.push(combo);
            }
        }

        if all_choices.len() <= 1 {
            let courses = all_choices.into_iter().next().unwrap_or_default();
            ResolvedRequirement::fixed(id.to_string(), name.to_string(), category, courses)
        } else {
            ResolvedRequirement::variable(id.to_string(), name.to_string(), category, all_choices)
        }
    }

    /// Get the pool of courses for a select requirement
    ///
    /// Returns courses sorted by preference for shortest path:
    /// 1. Courses with no prerequisites
    /// 2. Courses with same major subject (CS, etc.)
    /// 3. Alphabetical order
    ///
    /// For credit-based requirements, bundles are kept as single units.
    /// For count-based requirements, bundles are expanded into individual courses.
    fn get_selection_pool(&mut self, from: Option<&FromClause>) -> Vec<String> {
        let Some(from) = from else {
            return Vec::new();
        };

        let mut pool: HashSet<String> = HashSet::new();

        // Add explicit course list - keep bundles as-is, don't expand them
        // This allows credit-based selection to work correctly with bundles
        if let Some(courses) = &from.courses {
            for course_ref in courses {
                pool.insert(course_ref.clone());
            }
        }

        // Add pattern matches (these are always individual courses)
        if let Some(pattern) = &from.pattern {
            for course in self.match_pattern(pattern) {
                pool.insert(course);
            }
        }

        // Add include patterns
        if let Some(includes) = &from.include {
            for pattern in includes {
                for course in self.match_pattern(pattern) {
                    pool.insert(course);
                }
            }
        }

        // Add courses from groups
        if let Some(groups) = &from.groups {
            for group in groups {
                for course in &group.courses {
                    pool.insert(course.clone());
                }
            }
        }

        // Remove excluded courses
        if let Some(excludes) = &from.exclude {
            for exclude in excludes {
                pool.remove(exclude);
            }
        }

        // Sort by preference: courses with fewer prerequisites first, then CS courses, then alphabetical
        let mut result: Vec<String> = pool.into_iter().collect();
        result.sort_by(|a, b| {
            // First: prefer items with known credits (not bundles without course data)
            let a_credits = self.get_item_credits(a);
            let b_credits = self.get_item_credits(b);
            let a_known = a_credits > 0;
            let b_known = b_credits > 0;
            if a_known != b_known {
                return b_known.cmp(&a_known); // known > unknown
            }

            // Second: for non-bundles, prefer courses with no prerequisites
            let a_is_bundle = a.starts_with('[') || a.starts_with('{');
            let b_is_bundle = b.starts_with('[') || b.starts_with('{');

            if !a_is_bundle && !b_is_bundle {
                let a_has_prereqs = self
                    .courses
                    .get(a)
                    .is_some_and(|c| c.prerequisites_raw.is_some() || !c.prerequisites.is_empty());
                let b_has_prereqs = self
                    .courses
                    .get(b)
                    .is_some_and(|c| c.prerequisites_raw.is_some() || !c.prerequisites.is_empty());
                if a_has_prereqs != b_has_prereqs {
                    return a_has_prereqs.cmp(&b_has_prereqs); // false < true (no prereqs first)
                }

                // Third: prefer major subjects (CS, CT, DSCI) for CS degrees
                let a_is_major = is_major_subject(a);
                let b_is_major = is_major_subject(b);
                if a_is_major != b_is_major {
                    return b_is_major.cmp(&a_is_major); // true > false
                }
            }

            // Finally: alphabetical
            a.cmp(b)
        });
        result
    }

    /// Determine how many courses/bundles to select
    ///
    /// For credit-based requirements, calculates based on actual course credits
    /// rather than assuming a fixed credit value. This handles bundles correctly.
    fn determine_selection_count(&self, req: &Requirement, pool: &[String]) -> usize {
        // Priority: count > credits-based > all
        if let Some(count) = req.count {
            return count as usize;
        }

        let credits_needed = req
            .credits
            .or_else(|| req.credit_range.as_ref().map(|r| r.min));

        if let Some(target_credits) = credits_needed {
            // Calculate how many items from pool are needed to reach target credits
            // Sort pool by credits (ascending) to prefer smaller combinations
            let mut pool_with_credits: Vec<(usize, &String, u32)> = pool
                .iter()
                .enumerate()
                .map(|(i, item)| (i, item, self.get_item_credits(item)))
                .collect();

            // Sort by credits (prefer items that get us closer to target)
            pool_with_credits.sort_by_key(|(_, _, credits)| *credits);

            // Greedily select items until we reach target credits
            let mut total = 0u32;
            let mut count = 0usize;
            for (_, _, credits) in &pool_with_credits {
                if total >= target_credits {
                    break;
                }
                total += credits;
                count += 1;
            }

            // Ensure we have at least 1 if credits are needed
            return count.max(1);
        }

        // Default: select all
        pool.len()
    }

    /// Get credits for a course or bundle
    ///
    /// For bundles like `[AA100, AA101]`, sums the credits of all courses.
    /// For equivalents like `{CS201, PHIL201}`, uses the first course's credits.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn get_item_credits(&self, item: &str) -> u32 {
        // Handle bundle syntax: "[CS1800, CS1802]"
        if item.starts_with('[') && item.ends_with(']') {
            let inner = &item[1..item.len() - 1];
            let total: f32 = inner
                .split(',')
                .filter_map(|part| {
                    let trimmed = part.trim();
                    self.courses.get(trimmed).map(|c| c.credit_hours)
                })
                .sum();
            return total.ceil() as u32;
        }

        // Handle equivalent syntax: "{CS4530, CS4535}" - use first course's credits
        if item.starts_with('{') && item.ends_with('}') {
            let inner = &item[1..item.len() - 1];
            if let Some(first) = inner.split(',').next() {
                let trimmed = first.trim();
                if let Some(course) = self.courses.get(trimmed) {
                    return course.credit_hours.ceil() as u32;
                }
            }
            return 4; // Default for equivalents
        }

        // Regular course - look up credits
        self.courses
            .get(item)
            .map_or(4, |c| c.credit_hours.ceil() as u32) // Default to 4 credits if unknown
    }

    /// Expand a course list, handling bundles and equivalents
    #[allow(clippy::unused_self)] // Keep as method for consistency
    fn expand_course_list(&self, courses: &[String]) -> Vec<String> {
        let mut result = Vec::new();

        for course_ref in courses {
            // Handle bundle syntax: "[CS1800, CS1802]"
            if course_ref.starts_with('[') && course_ref.ends_with(']') {
                let inner = &course_ref[1..course_ref.len() - 1];
                for part in inner.split(',') {
                    let trimmed = part.trim();
                    if !trimmed.is_empty() {
                        result.push(trimmed.to_string());
                    }
                }
            }
            // Handle equivalent syntax: "{CS4530, CS4535}" - pick first as default
            else if course_ref.starts_with('{') && course_ref.ends_with('}') {
                let inner = &course_ref[1..course_ref.len() - 1];
                if let Some(first) = inner.split(',').next() {
                    let trimmed = first.trim();
                    if !trimmed.is_empty() {
                        result.push(trimmed.to_string());
                    }
                }
            } else {
                result.push(course_ref.clone());
            }
        }

        result
    }

    /// Match a pattern against available courses
    fn match_pattern(&mut self, pattern: &str) -> Vec<String> {
        // Check cache first
        if let Some(cached) = self.pattern_cache.get(pattern) {
            return cached.clone();
        }

        let matches = self.do_pattern_match(pattern);
        self.pattern_cache
            .insert(pattern.to_string(), matches.clone());
        matches
    }

    /// Perform actual pattern matching
    fn do_pattern_match(&self, pattern: &str) -> Vec<String> {
        // Pattern format: "PREFIX:LEVEL" where LEVEL can be "*", "300+", "100-299", etc.
        let parts: Vec<&str> = pattern.split(':').collect();
        if parts.len() != 2 {
            return Vec::new();
        }

        let prefix = parts[0];
        let level_spec = parts[1];

        // Handle wildcard prefix "*" - match all prefixes
        let is_wildcard_prefix = prefix == "*";

        let mut matches = Vec::new();

        for key in self.courses.keys() {
            // Extract subject from course key (e.g., "CS" from "CS3000")
            let subject = extract_subject(key);

            // Skip if not matching the prefix (unless wildcard)
            if !is_wildcard_prefix && subject.as_deref() != Some(prefix) {
                continue;
            }

            // Extract course number
            let number = extract_number(key);
            if let Some(num) = number {
                if Self::matches_level_spec(num, level_spec) {
                    matches.push(key.clone());
                }
            }
        }

        matches.sort();
        matches
    }

    /// Check if a course number matches a level specification
    fn matches_level_spec(number: u32, spec: &str) -> bool {
        if spec == "*" {
            return true;
        }

        // Handle "N+" format (e.g., "2500+", "300+")
        if let Some(min_str) = spec.strip_suffix('+') {
            if let Ok(min) = min_str.parse::<u32>() {
                return number >= min;
            }
        }

        // Handle "N-M" format (e.g., "100-299", "3000-3999")
        if spec.contains('-') {
            let range_parts: Vec<&str> = spec.split('-').collect();
            if range_parts.len() == 2 {
                if let (Ok(min), Ok(max)) =
                    (range_parts[0].parse::<u32>(), range_parts[1].parse::<u32>())
                {
                    return number >= min && number <= max;
                }
            }
        }

        // Handle exact level (e.g., "3000")
        if let Ok(exact) = spec.parse::<u32>() {
            // Match level (e.g., 3000 matches 3000-3999)
            let level = exact / 1000 * 1000;
            return number >= level && number < level + 1000;
        }

        false
    }

    /// Generate all combinations of size k from a pool
    #[allow(clippy::unused_self)] // Keep as method for API consistency
    fn generate_combinations(&self, pool: &[String], k: usize) -> Vec<Vec<String>> {
        if k == 0 || k > pool.len() {
            return vec![Vec::new()];
        }

        let mut result = Vec::new();
        let mut indices = (0..k).collect::<Vec<usize>>();

        loop {
            // Add current combination
            let combo: Vec<String> = indices.iter().map(|&i| pool[i].clone()).collect();
            result.push(combo);

            // Find rightmost index that can be incremented
            let mut i = k;
            while i > 0 {
                i -= 1;
                if indices[i] < pool.len() - k + i {
                    break;
                }
            }

            if i == 0 && indices[0] >= pool.len() - k {
                break;
            }

            // Increment and reset subsequent indices
            indices[i] += 1;
            for j in (i + 1)..k {
                indices[j] = indices[j - 1] + 1;
            }
        }

        result
    }

    /// Resolve nested requirements and return all course combinations
    fn resolve_nested_requirements(&mut self, requirements: &[Requirement]) -> Vec<Vec<String>> {
        if requirements.is_empty() {
            return vec![Vec::new()];
        }

        // Resolve each nested requirement
        let resolved: Vec<ResolvedRequirement> = requirements
            .iter()
            .enumerate()
            .map(|(i, req)| self.resolve_requirement(&format!("nested_{i}"), req))
            .collect();

        // Compute cartesian product of all choices
        Self::cartesian_product_of_choices(&resolved)
    }

    /// Compute cartesian product of requirement choices
    fn cartesian_product_of_choices(requirements: &[ResolvedRequirement]) -> Vec<Vec<String>> {
        if requirements.is_empty() {
            return vec![Vec::new()];
        }

        let mut result = vec![Vec::new()];

        for req in requirements {
            let mut new_result = Vec::new();
            for existing in &result {
                for choice in &req.choices {
                    let mut combined = existing.clone();
                    combined.extend(choice.iter().cloned());
                    new_result.push(combined);
                }
            }
            result = new_result;
        }

        result
    }
}

/// Extract subject code from a course key (e.g., "CS" from "CS3000")
fn extract_subject(key: &str) -> Option<String> {
    let subject: String = key.chars().take_while(|c| c.is_alphabetic()).collect();
    if subject.is_empty() {
        None
    } else {
        Some(subject)
    }
}

/// Extract course number from a course key (e.g., 3000 from "CS3000", 310 from "CS310H")
///
/// Extracts leading digits after the prefix, ignoring any trailing letters (like "H" for honors)
fn extract_number(key: &str) -> Option<u32> {
    let number_str: String = key
        .chars()
        .skip_while(|c| c.is_alphabetic())
        .take_while(char::is_ascii_digit)
        .collect();
    number_str.parse().ok()
}

/// Sanitize a requirement ID to create a placeholder course prefix
///
/// Converts requirement IDs like `writing_composition` to "WRTC" (up to 4 chars, uppercase)
fn sanitize_placeholder_prefix(req_id: &str) -> String {
    // Take first letter of each word (snake_case), up to 4 chars
    let parts: Vec<&str> = req_id.split('_').collect();
    let prefix: String = parts
        .iter()
        .filter_map(|part| part.chars().next())
        .take(4)
        .collect::<String>()
        .to_uppercase();

    // Pad to at least 2 chars
    if prefix.len() < 2 {
        format!("{prefix}X")
    } else {
        prefix
    }
}

/// Check if a course key belongs to a major CS subject
///
/// Major subjects for CS degrees include: CS, CT, DSCI, ECE, MATH, STAT
/// These are preferred when selecting electives for shortest path plans.
fn is_major_subject(key: &str) -> bool {
    let subject = extract_subject(key);
    subject.is_some_and(|s| {
        matches!(
            s.to_uppercase().as_str(),
            "CS" | "CT" | "DSCI" | "ECE" | "MATH" | "STAT"
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_courses() -> HashMap<String, Course> {
        let mut courses = HashMap::new();
        for (key, credits) in [
            ("CS1000", 4.0),
            ("CS2000", 4.0),
            ("CS2500", 4.0),
            ("CS3000", 4.0),
            ("CS3500", 4.0),
            ("CS4000", 4.0),
            ("MATH1000", 3.0),
            ("MATH2000", 3.0),
        ] {
            let course = Course {
                prefix: extract_subject(key).unwrap_or_default(),
                number: key.chars().skip_while(|c| c.is_alphabetic()).collect(),
                credit_hours: credits,
                ..Default::default()
            };
            courses.insert(key.to_string(), course);
        }
        courses
    }

    #[test]
    fn test_resolve_all_requirement() {
        let courses = sample_courses();
        let mut req_resolver = RequirementResolver::new(&courses);

        let req = Requirement {
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
        };

        let result = req_resolver.resolve_requirement("core", &req);

        assert_eq!(result.id, "core");
        assert!(!result.is_variable);
        assert_eq!(result.choice_count, 1);
        assert_eq!(result.choices[0], vec!["CS1000", "CS2000"]);
    }

    #[test]
    fn test_resolve_select_requirement() {
        let courses = sample_courses();
        let mut req_resolver = RequirementResolver::new(&courses);

        let req = Requirement {
            name: Some("Electives".to_string()),
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
            count: Some(2),
            credits: None,
            credit_range: None,
            constraints: None,
            options: None,
        };

        let result = req_resolver.resolve_requirement("electives", &req);

        assert_eq!(result.id, "electives");
        assert!(result.is_variable);
        // C(3,2) = 3 combinations
        assert_eq!(result.choice_count, 3);
        assert_eq!(result.choices.len(), 3);
    }

    #[test]
    fn test_generate_combinations() {
        let courses = sample_courses();
        let combo_resolver = RequirementResolver::new(&courses);

        let pool = vec!["A".to_string(), "B".to_string(), "C".to_string()];
        let combos = combo_resolver.generate_combinations(&pool, 2);

        assert_eq!(combos.len(), 3);
        assert!(combos.contains(&vec!["A".to_string(), "B".to_string()]));
        assert!(combos.contains(&vec!["A".to_string(), "C".to_string()]));
        assert!(combos.contains(&vec!["B".to_string(), "C".to_string()]));
    }

    #[test]
    fn test_expand_course_list_bundles() {
        let courses = sample_courses();
        let resolver = RequirementResolver::new(&courses);

        let list = vec![
            "[CS1000, CS1001]".to_string(),
            "CS2000".to_string(),
            "{CS3000, CS3500}".to_string(),
        ];

        let expanded = resolver.expand_course_list(&list);

        // Bundle expands, equivalent picks first
        assert_eq!(expanded, vec!["CS1000", "CS1001", "CS2000", "CS3000"]);
    }

    #[test]
    fn test_pattern_matching() {
        let courses = sample_courses();
        let mut resolver = RequirementResolver::new(&courses);

        // Test "CS:*" - all CS courses
        let all_cs = resolver.match_pattern("CS:*");
        assert_eq!(all_cs.len(), 6);

        // Test "CS:3000+" - CS courses 3000 and above
        let upper_cs = resolver.match_pattern("CS:3000+");
        assert!(upper_cs.contains(&"CS3000".to_string()));
        assert!(upper_cs.contains(&"CS4000".to_string()));
        assert!(!upper_cs.contains(&"CS2000".to_string()));
    }

    #[test]
    fn test_extract_subject_and_number() {
        assert_eq!(extract_subject("CS3000"), Some("CS".to_string()));
        assert_eq!(extract_subject("MATH1234"), Some("MATH".to_string()));
        assert_eq!(extract_number("CS3000"), Some(3000));
        assert_eq!(extract_number("MATH1234"), Some(1234));
    }
}
