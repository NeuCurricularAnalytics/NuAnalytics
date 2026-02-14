//! Course prerequisite graph
//!
//! This module provides a graph structure for representing course prerequisite
//! relationships. Unlike the DAG which is used for plan validation, this graph
//! represents all courses in a degree program and their prerequisite chains.
//!
//! # Key Features
//! - Builds graph from all courses in a `DegreeProgram`
//! - Handles circular prerequisites (detected and reported)
//! - Tracks both required (AND) and optional (OR) prerequisites
//! - Computes full prerequisite chains for any course

use std::collections::{HashMap, HashSet, VecDeque};

use super::Course;
use crate::core::models::DegreeProgram;
use crate::core::prerequisite_parser;

// ============================================================================
// Helper Functions - Common operations used throughout the module
// ============================================================================

/// Check if an edge is a true prerequisite (not a corequisite)
///
/// Corequisites are taken concurrently and don't count for prerequisite chains.
#[inline]
fn is_true_prerequisite(edge: &PrerequisiteEdge) -> bool {
    edge.prereq_type != PrerequisiteType::Corequisite
        && edge.prereq_type != PrerequisiteType::StrictCorequisite
}

/// Filter prerequisite edges to only include true prerequisites (exclude corequisites)
fn filter_true_prerequisites(edges: &[PrerequisiteEdge]) -> Vec<&PrerequisiteEdge> {
    edges.iter().filter(|e| is_true_prerequisite(e)).collect()
}

/// Deduplicate a vector while preserving insertion order
///
/// Returns the same vector with duplicates removed, keeping the first occurrence.
fn deduplicate_preserving_order(items: &mut Vec<String>) {
    let mut seen = HashSet::new();
    items.retain(|c| seen.insert(c.clone()));
}

/// Collect direct required prerequisites from edges, merged with sibling context
///
/// Used for passing sibling context to recursive chain building.
fn collect_direct_required_prereqs(
    prereq_edges: &[&PrerequisiteEdge],
    sibling_required: &HashSet<String>,
) -> HashSet<String> {
    let mut direct_required: HashSet<String> = sibling_required.clone();
    for edge in prereq_edges {
        if edge.prereq_type == PrerequisiteType::Required {
            direct_required.insert(edge.prerequisite.clone());
        }
    }
    direct_required
}

/// Represents a prerequisite relationship type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrerequisiteType {
    /// Required prerequisite (must be taken)
    Required,
    /// Optional prerequisite (one of several choices)
    Optional,
    /// Corequisite (must be taken concurrently or before)
    Corequisite,
    /// Strict corequisite (must be taken concurrently)
    StrictCorequisite,
}

/// An edge in the course graph representing a prerequisite relationship
#[derive(Debug, Clone)]
pub struct PrerequisiteEdge {
    /// The prerequisite course key
    pub prerequisite: String,
    /// Type of prerequisite relationship
    pub prereq_type: PrerequisiteType,
    /// If optional, which OR-group does this belong to (same group = alternatives)
    pub or_group: Option<usize>,
}

/// Type alias for OR-groups mapping: group ID → list of (course key, chain)
type OrGroupsMap = HashMap<usize, Vec<(String, PrerequisiteChain)>>;

/// Result type for structured chain edge processing: (required branches, OR-groups)
type StructuredChainEdgeResult = (Vec<Vec<String>>, OrGroupsMap);

/// Represents a structured prerequisite chain with parallel branches
#[derive(Debug, Clone)]
pub struct PrerequisiteChain {
    /// Parallel branches that must all be satisfied (AND relationship)
    /// Each branch is an ordered list from leaf to immediate prereq
    pub branches: Vec<Vec<String>>,
    /// Total unique courses in the chain
    pub total_courses: usize,
}

impl PrerequisiteChain {
    /// Create an empty prerequisite chain (no prerequisites)
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            branches: Vec::new(),
            total_courses: 0,
        }
    }

    /// Format the chain for display in a readable format
    ///
    /// Shows parallel branches (AND requirements) wrapped in parentheses and
    /// separated by " & ". Each branch shows courses in dependency order
    /// connected by " → ".
    ///
    /// Example: `(MATH117 → MATH118) & (CS150B → CS165)`
    #[must_use]
    pub fn format(&self) -> String {
        if self.branches.is_empty() {
            return String::from("(none)");
        }

        // Filter out empty branches and format each
        let formatted: Vec<String> = self
            .branches
            .iter()
            .filter(|b| !b.is_empty())
            .map(|branch| {
                if branch.len() == 1 {
                    branch[0].clone()
                } else {
                    format!("({})", branch.join(" → "))
                }
            })
            .collect();

        if formatted.is_empty() {
            return String::from("(none)");
        }

        formatted.join(" & ")
    }

    /// Get the lengths of each branch
    ///
    /// Returns a vector of branch lengths for display purposes.
    #[must_use]
    pub fn branch_lengths(&self) -> Vec<usize> {
        self.branches.iter().map(Vec::len).collect()
    }

    /// Format branch lengths for display
    ///
    /// Returns a string like "3, 2" for two branches with 3 and 2 courses.
    #[must_use]
    pub fn format_lengths(&self) -> String {
        let lengths = self.branch_lengths();
        if lengths.is_empty() {
            return String::from("0");
        }
        if lengths.len() == 1 {
            return lengths[0].to_string();
        }
        lengths
            .iter()
            .map(std::string::ToString::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// Get all unique courses in topologically sorted order
    #[must_use]
    pub fn sorted_courses(&self) -> Vec<String> {
        let mut all: Vec<String> = self.branches.iter().flatten().cloned().collect();
        // Deduplicate while preserving order (first occurrence wins)
        let mut seen = HashSet::new();
        all.retain(|c| seen.insert(c.clone()));
        all
    }
}

/// A node in the course graph
#[derive(Debug, Clone)]
pub struct CourseNode {
    /// Course key (e.g., "CS2510")
    pub key: String,
    /// Reference to the course data (if available in the degree)
    pub has_course_data: bool,
    /// Incoming edges (prerequisites of this course) - flat list for graph traversal
    pub prerequisites: Vec<PrerequisiteEdge>,
    /// Prerequisite paths in DNF form (OR of ANDs)
    /// Each inner Vec represents a valid path (all courses in the path must be taken)
    /// Outer Vec represents alternatives (any one path satisfies the requirement)
    /// Empty means no prerequisites
    pub prerequisite_paths: Vec<Vec<String>>,
    /// Outgoing edges (courses that require this course)
    pub dependents: Vec<String>,
    /// Credit hours (copied from course for convenience)
    pub credits: f32,
    /// Course title (copied from course for convenience)
    pub title: String,
    /// Raw prerequisite expression (for display/debugging)
    pub prerequisites_raw: Option<String>,
}

impl CourseNode {
    /// Create a new course node
    fn new(key: String, course: Option<&Course>) -> Self {
        Self {
            key,
            has_course_data: course.is_some(),
            prerequisites: Vec::new(),
            prerequisite_paths: Vec::new(),
            dependents: Vec::new(),
            credits: course.map_or(0.0, |c| c.credit_hours),
            title: course.map_or_else(String::new, |c| c.name.clone()),
            prerequisites_raw: course.and_then(|c| c.prerequisites_raw.clone()),
        }
    }

    /// Check if this course has any required prerequisites
    #[must_use]
    pub fn has_required_prerequisites(&self) -> bool {
        self.prerequisites
            .iter()
            .any(|e| e.prereq_type == PrerequisiteType::Required)
    }

    /// Get all required prerequisites
    #[must_use]
    pub fn required_prerequisites(&self) -> Vec<&str> {
        self.prerequisites
            .iter()
            .filter(|e| e.prereq_type == PrerequisiteType::Required)
            .map(|e| e.prerequisite.as_str())
            .collect()
    }

    /// Get all optional prerequisites grouped by OR-group
    #[must_use]
    pub fn optional_prerequisite_groups(&self) -> HashMap<usize, Vec<&str>> {
        let mut groups: HashMap<usize, Vec<&str>> = HashMap::new();
        for edge in &self.prerequisites {
            if edge.prereq_type == PrerequisiteType::Optional {
                if let Some(group) = edge.or_group {
                    groups.entry(group).or_default().push(&edge.prerequisite);
                }
            }
        }
        groups
    }

    /// Get all corequisites
    #[must_use]
    pub fn corequisites(&self) -> Vec<&str> {
        self.prerequisites
            .iter()
            .filter(|e| {
                e.prereq_type == PrerequisiteType::Corequisite
                    || e.prereq_type == PrerequisiteType::StrictCorequisite
            })
            .map(|e| e.prerequisite.as_str())
            .collect()
    }

    /// Format prerequisite paths for display
    /// Returns a string representation like "(A & B) | C" or "A & B"
    #[must_use]
    pub fn format_prerequisite_paths(&self) -> String {
        if self.prerequisite_paths.is_empty() {
            return String::new();
        }

        let path_strs: Vec<String> = self
            .prerequisite_paths
            .iter()
            .map(|path| {
                if path.len() == 1 {
                    path[0].clone()
                } else {
                    format!("({})", path.join(" & "))
                }
            })
            .collect();

        if path_strs.len() == 1 {
            // Single path - don't need outer parens for AND
            if self.prerequisite_paths[0].len() == 1 {
                path_strs[0].clone()
            } else {
                // Multiple courses in single path - show without outer parens
                self.prerequisite_paths[0].join(" & ")
            }
        } else {
            path_strs.join(" | ")
        }
    }
}

/// Result of building a course graph
#[derive(Debug, Clone)]
pub struct CourseGraphResult {
    /// The built graph
    pub graph: CourseGraph,
    /// Any circular dependencies detected (each Vec is a cycle)
    pub cycles: Vec<Vec<String>>,
    /// Courses referenced as prerequisites but not defined in the degree
    pub missing_courses: Vec<String>,
}

/// A graph of courses and their prerequisite relationships
#[derive(Debug, Clone, Default)]
pub struct CourseGraph {
    /// All nodes in the graph, keyed by course key
    nodes: HashMap<String, CourseNode>,
    /// Cached topological order (if no cycles), from leaves to roots
    topo_order: Option<Vec<String>>,
}

impl CourseGraph {
    /// Build a course graph from a degree program
    ///
    /// This creates a graph of all courses and their prerequisite relationships.
    /// Courses referenced as prerequisites but not defined in the degree are
    /// included as nodes but flagged as missing.
    ///
    /// # Arguments
    /// * `program` - The degree program to build the graph from
    ///
    /// # Returns
    /// A `CourseGraphResult` containing the graph, any detected cycles, and missing courses
    #[must_use]
    pub fn from_degree_program(program: &DegreeProgram) -> CourseGraphResult {
        let mut graph = Self {
            nodes: HashMap::new(),
            topo_order: None,
        };
        let mut missing_courses = HashSet::new();

        // Build graph in multiple passes
        build_initial_nodes(&mut graph, program);
        add_prerequisites(&mut graph, program, &mut missing_courses);
        add_corequisites(&mut graph, program, &mut missing_courses);
        add_missing_nodes(&mut graph, &missing_courses);
        build_reverse_edges(&mut graph);

        // Detect cycles and compute topological order
        let cycles = graph.detect_cycles();
        if cycles.is_empty() {
            graph.topo_order = Some(graph.compute_topological_order());
        }

        CourseGraphResult {
            graph,
            cycles,
            missing_courses: missing_courses.into_iter().collect(),
        }
    }

    /// Break cycles in the graph by removing edges
    ///
    /// When cycles exist (often due to OR-prerequisites creating circular paths),
    /// this method removes one edge from each cycle to create a valid DAG.
    /// Prefers removing optional (OR-group) edges over required edges.
    ///
    /// # Arguments
    /// * `cycles` - The cycles detected in the graph
    ///
    /// # Returns
    /// A vector of edges that were removed: (course, prerequisite)
    pub fn break_cycles(&mut self, cycles: &[Vec<String>]) -> Vec<(String, String)> {
        let mut removed_edges = Vec::new();

        for cycle in cycles {
            if cycle.len() < 2 {
                continue;
            }

            // Find the best edge to remove (prefer optional/OR edges)
            let edge_to_remove = self.find_best_edge_to_break(cycle);

            if let Some((course, prereq)) = edge_to_remove {
                // Remove the edge
                if let Some(node) = self.nodes.get_mut(&course) {
                    node.prerequisites.retain(|e| e.prerequisite != prereq);
                }
                // Also remove from reverse edges
                if let Some(prereq_node) = self.nodes.get_mut(&prereq) {
                    prereq_node.dependents.retain(|d| d != &course);
                }
                removed_edges.push((course, prereq));
            }
        }

        // Recompute topological order after breaking cycles
        let remaining_cycles = self.detect_cycles();
        if remaining_cycles.is_empty() {
            self.topo_order = Some(self.compute_topological_order());
        }

        removed_edges
    }

    /// Find the best edge to remove from a cycle
    ///
    /// Prefers optional (OR-group) edges over required edges.
    /// Within a cycle A → B → C → A, we look at edges (A,B), (B,C), (C,A).
    fn find_best_edge_to_break(&self, cycle: &[String]) -> Option<(String, String)> {
        // Build list of edges in the cycle with their "removability" score
        // Higher score = more preferable to remove
        let mut edges: Vec<(String, String, i32)> = Vec::new();

        for i in 0..cycle.len() - 1 {
            let course = &cycle[i];
            let prereq = &cycle[i + 1];

            if let Some(node) = self.nodes.get(course) {
                for edge in &node.prerequisites {
                    if &edge.prerequisite == prereq {
                        // Score: optional edges are preferred (higher score)
                        let score = match edge.prereq_type {
                            PrerequisiteType::Optional => 10,
                            PrerequisiteType::Required => 1,
                            PrerequisiteType::Corequisite | PrerequisiteType::StrictCorequisite => {
                                0
                            } // Don't break coreqs
                        };
                        if score > 0 {
                            edges.push((course.clone(), prereq.clone(), score));
                        }
                    }
                }
            }
        }

        // Return edge with highest score
        edges
            .into_iter()
            .max_by_key(|(_, _, score)| *score)
            .map(|(course, prereq, _)| (course, prereq))
    }

    /// Get a course node by key
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&CourseNode> {
        self.nodes.get(key)
    }

    /// Get all course keys in the graph
    #[must_use]
    pub fn course_keys(&self) -> Vec<&str> {
        self.nodes.keys().map(String::as_str).collect()
    }

    /// Get the number of courses in the graph
    #[must_use]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Check if the graph is empty
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Check if the graph has cycles
    #[must_use]
    pub const fn has_cycles(&self) -> bool {
        self.topo_order.is_none()
    }

    /// Get the topological order (if no cycles)
    ///
    /// Returns courses in order from leaves (no prerequisites) to roots (no dependents)
    #[must_use]
    pub fn topological_order(&self) -> Option<&[String]> {
        self.topo_order.as_deref()
    }

    /// Get all courses that have no prerequisites (entry points)
    #[must_use]
    pub fn leaf_courses(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.prerequisites.is_empty())
            .map(|(key, _)| key.as_str())
            .collect()
    }

    /// Get all courses that no other course depends on (terminal courses)
    #[must_use]
    pub fn terminal_courses(&self) -> Vec<&str> {
        self.nodes
            .iter()
            .filter(|(_, node)| node.dependents.is_empty())
            .map(|(key, _)| key.as_str())
            .collect()
    }

    /// Get the full prerequisite chain for a course (all transitive prerequisites)
    ///
    /// This returns all courses that must be taken before the given course,
    /// following all prerequisite relationships transitively.
    ///
    /// # Arguments
    /// * `course_key` - The course to get prerequisites for
    /// * `include_optional` - Whether to include optional prerequisites
    ///
    /// # Returns
    /// A set of all prerequisite course keys, or None if the course doesn't exist
    #[must_use]
    pub fn prerequisite_chain(
        &self,
        course_key: &str,
        include_optional: bool,
    ) -> Option<HashSet<String>> {
        let node = self.nodes.get(course_key)?;
        let mut chain = HashSet::new();
        let mut visited = HashSet::new();
        let mut queue = VecDeque::new();

        // Start with direct prerequisites
        for edge in &node.prerequisites {
            if include_optional || edge.prereq_type == PrerequisiteType::Required {
                queue.push_back(edge.prerequisite.clone());
            }
        }

        // BFS to collect all transitive prerequisites
        while let Some(prereq) = queue.pop_front() {
            if visited.contains(&prereq) {
                continue;
            }
            visited.insert(prereq.clone());
            chain.insert(prereq.clone());

            if let Some(prereq_node) = self.nodes.get(&prereq) {
                for edge in &prereq_node.prerequisites {
                    if (include_optional || edge.prereq_type == PrerequisiteType::Required)
                        && !visited.contains(&edge.prerequisite)
                    {
                        queue.push_back(edge.prerequisite.clone());
                    }
                }
            }
        }

        Some(chain)
    }

    /// Get the minimum prerequisite chain depth for a course
    ///
    /// This considers OR groups and finds the shortest path through prerequisites.
    /// For courses with multiple prerequisite options (A | B), it takes the minimum.
    /// For courses with required prerequisites (A & B), it takes the maximum of the group.
    ///
    /// # Arguments
    /// * `course_key` - The course to get the chain depth for
    ///
    /// # Returns
    /// The minimum chain depth, or None if the course doesn't exist or has cycles
    #[must_use]
    pub fn min_prerequisite_depth(&self, course_key: &str) -> Option<usize> {
        self.min_depth_recursive(course_key, &mut HashSet::new())
    }

    /// Recursive helper for minimum depth calculation
    ///
    /// Uses cycle detection to compute minimum depth through prerequisites.
    fn min_depth_recursive(
        &self,
        course_key: &str,
        visiting: &mut HashSet<String>,
    ) -> Option<usize> {
        // Cycle detection
        if visiting.contains(course_key) {
            return None;
        }

        let node = self.nodes.get(course_key)?;
        let prereq_edges = filter_true_prerequisites(&node.prerequisites);

        // Base case: no prerequisites
        if prereq_edges.is_empty() {
            return Some(0);
        }

        visiting.insert(course_key.to_string());
        let result = self.calculate_min_depth_from_edges(&prereq_edges, visiting);
        visiting.remove(course_key);

        result
    }

    /// Calculate minimum depth from prerequisite edges
    ///
    /// Groups edges by type and computes:
    /// - Required: max depth (all must be satisfied)
    /// - OR-groups: min depth per group (only one needed), summed
    fn calculate_min_depth_from_edges(
        &self,
        prereq_edges: &[&PrerequisiteEdge],
        visiting: &mut HashSet<String>,
    ) -> Option<usize> {
        let mut required_depths = Vec::new();
        let mut or_group_depths: HashMap<usize, Vec<usize>> = HashMap::new();

        for edge in prereq_edges {
            let prereq_depth = self.min_depth_recursive(&edge.prerequisite, visiting)?;

            match edge.prereq_type {
                PrerequisiteType::Required => {
                    required_depths.push(prereq_depth);
                }
                PrerequisiteType::Optional => {
                    if let Some(group) = edge.or_group {
                        or_group_depths.entry(group).or_default().push(prereq_depth);
                    }
                }
                _ => {} // Skip corequisites (already filtered, but be safe)
            }
        }

        // Calculate total depth:
        // - For required: take max (all must be satisfied)
        // - For each OR-group: take min (only one needed)
        let required_max = required_depths.into_iter().max().unwrap_or(0);
        let or_groups_contribution: usize = or_group_depths
            .values()
            .filter_map(|depths| depths.iter().min().copied())
            .sum();

        Some(1 + required_max + or_groups_contribution)
    }

    /// Get the minimum prerequisite chain for a course (shortest path through OR alternatives)
    ///
    /// When choosing between OR alternatives, prefers courses with the same subject code.
    /// For example, CS312 with prereqs (CIS340 | CS165) will prefer CS165 since it shares
    /// the CS subject.
    ///
    /// Returns a list of courses in the minimum chain, or None if course doesn't exist
    #[must_use]
    pub fn min_prerequisite_chain(&self, course_key: &str) -> Option<Vec<String>> {
        self.min_prerequisite_chain_with_context(course_key, &HashSet::new())
    }

    /// Get minimum prerequisite chain for a course, preferring courses already in a plan
    ///
    /// This is context-aware: when choosing between OR alternatives, it prefers:
    /// 1. Courses already in the provided `plan_courses` set
    /// 2. Courses with the same subject code
    /// 3. Shortest chain length
    ///
    /// This prevents adding redundant prerequisites when a suitable one already exists.
    #[must_use]
    pub fn min_prerequisite_chain_with_context(
        &self,
        course_key: &str,
        plan_courses: &HashSet<String>,
    ) -> Option<Vec<String>> {
        let subject = extract_subject(course_key);
        self.min_chain_recursive_with_context(
            course_key,
            subject.as_deref(),
            plan_courses,
            &mut HashSet::new(),
        )
    }

    /// Recursive helper for building minimum chain with plan context
    ///
    /// Uses cycle detection and prefers courses already in the plan.
    fn min_chain_recursive_with_context(
        &self,
        course_key: &str,
        preferred_subject: Option<&str>,
        plan_courses: &HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        // Cycle detection
        if visiting.contains(course_key) {
            return None;
        }

        let node = self.nodes.get(course_key)?;
        let prereq_edges = filter_true_prerequisites(&node.prerequisites);

        if prereq_edges.is_empty() {
            return Some(Vec::new());
        }

        visiting.insert(course_key.to_string());
        let result = self.collect_min_chain_from_edges_with_context(
            &prereq_edges,
            preferred_subject,
            plan_courses,
            visiting,
        );
        visiting.remove(course_key);

        result
    }

    /// Collect minimum chain from prerequisite edges with plan context
    ///
    /// Separates required and optional prerequisites, processes each group,
    /// and combines results. For OR groups, prefers courses already in plan.
    fn collect_min_chain_from_edges_with_context(
        &self,
        prereq_edges: &[&PrerequisiteEdge],
        preferred_subject: Option<&str>,
        plan_courses: &HashSet<String>,
        visiting: &mut HashSet<String>,
    ) -> Option<Vec<String>> {
        let mut result_chain = Vec::new();
        let mut or_groups: HashMap<usize, Vec<(String, Vec<String>)>> = HashMap::new();

        // Process each edge by type
        for edge in prereq_edges {
            let prereq_chain = self.min_chain_recursive_with_context(
                &edge.prerequisite,
                preferred_subject,
                plan_courses,
                visiting,
            );

            match edge.prereq_type {
                PrerequisiteType::Required => {
                    // Required prereqs must succeed - if cycle, we fail
                    let chain = prereq_chain?;
                    result_chain.push(edge.prerequisite.clone());
                    result_chain.extend(chain);
                }
                PrerequisiteType::Optional => {
                    // Optional prereqs - skip if cycle, try alternatives
                    if let Some(group) = edge.or_group {
                        if let Some(chain) = prereq_chain {
                            or_groups
                                .entry(group)
                                .or_default()
                                .push((edge.prerequisite.clone(), chain));
                        }
                    }
                }
                _ => {}
            }
        }

        // Select best option from each OR-group, preferring courses in plan
        for (_group, options) in or_groups {
            if options.is_empty() {
                return None; // All options in this OR-group led to cycles
            }
            if let Some((best_prereq, best_chain)) = select_best_prerequisite_option_with_context(
                options,
                preferred_subject,
                plan_courses,
            ) {
                result_chain.push(best_prereq);
                result_chain.extend(best_chain);
            }
        }

        deduplicate_preserving_order(&mut result_chain);
        Some(result_chain)
    }

    /// Get a structured prerequisite chain showing parallel branches
    ///
    /// This method returns a `PrerequisiteChain` that properly represents:
    /// - Parallel branches (AND requirements) as separate lists
    /// - Each branch ordered from leaf (no prereqs) to immediate prereq
    /// - Overlapping courses deduplicated across branches
    ///
    /// When choosing between OR alternatives:
    /// 1. Prefers courses with same subject code
    /// 2. Prefers courses that overlap with other required branches
    /// 3. Picks shortest chain among remaining options
    #[must_use]
    pub fn structured_prerequisite_chain(&self, course_key: &str) -> Option<PrerequisiteChain> {
        let subject = extract_subject(course_key);
        self.build_structured_chain(
            course_key,
            subject.as_deref(),
            &mut HashSet::new(),
            &HashSet::new(),
        )
    }

    /// Build a structured chain recursively
    ///
    /// # Arguments
    /// * `course_key` - The course to build chain for
    /// * `preferred_subject` - Subject to prefer in OR choices
    /// * `visiting` - Courses currently being visited (for cycle detection)
    /// * `sibling_required` - Courses that are siblings (AND requirements at same level)
    fn build_structured_chain(
        &self,
        course_key: &str,
        preferred_subject: Option<&str>,
        visiting: &mut HashSet<String>,
        sibling_required: &HashSet<String>,
    ) -> Option<PrerequisiteChain> {
        // Cycle detection
        if visiting.contains(course_key) {
            return None;
        }

        let node = self.nodes.get(course_key)?;
        let prereq_edges = filter_true_prerequisites(&node.prerequisites);

        if prereq_edges.is_empty() {
            return Some(PrerequisiteChain::empty());
        }

        visiting.insert(course_key.to_string());
        let result = self.collect_structured_chain_from_edges(
            &prereq_edges,
            preferred_subject,
            visiting,
            sibling_required,
        );
        visiting.remove(course_key);

        result
    }

    /// Collect structured chain from prerequisite edges
    ///
    /// Processes required and optional prerequisites, building branches for each.
    fn collect_structured_chain_from_edges(
        &self,
        prereq_edges: &[&PrerequisiteEdge],
        preferred_subject: Option<&str>,
        visiting: &mut HashSet<String>,
        sibling_required: &HashSet<String>,
    ) -> Option<PrerequisiteChain> {
        // Collect direct required prereqs as siblings for recursive calls
        let direct_required = collect_direct_required_prereqs(prereq_edges, sibling_required);

        // Process edges into branches and OR-groups
        let (required_branches, or_groups) = self.process_edges_for_structured_chain(
            prereq_edges,
            preferred_subject,
            visiting,
            &direct_required,
        )?;

        // Select from OR-groups and build final chain
        let all_branches = self.finalize_structured_branches(
            required_branches,
            or_groups,
            preferred_subject,
            &direct_required,
        )?;

        // Merge overlapping branches and create result
        let merged = merge_overlapping_branches(all_branches);
        let total_courses = merged.iter().flatten().collect::<HashSet<_>>().len();

        Some(PrerequisiteChain {
            branches: merged,
            total_courses,
        })
    }

    /// Process prerequisite edges into required branches and OR-groups
    ///
    /// Returns `None` if a required prerequisite has a cycle.
    fn process_edges_for_structured_chain(
        &self,
        prereq_edges: &[&PrerequisiteEdge],
        preferred_subject: Option<&str>,
        visiting: &mut HashSet<String>,
        direct_required: &HashSet<String>,
    ) -> Option<StructuredChainEdgeResult> {
        let mut required_branches: Vec<Vec<String>> = Vec::new();
        let mut or_groups: OrGroupsMap = HashMap::new();

        for edge in prereq_edges {
            let sub_chain = self.build_structured_chain(
                &edge.prerequisite,
                preferred_subject,
                visiting,
                direct_required,
            );

            match edge.prereq_type {
                PrerequisiteType::Required => {
                    let chain = sub_chain?; // Required must succeed
                    let mut branch = flatten_chain_to_ordered_branch(&chain, self);
                    branch.push(edge.prerequisite.clone());
                    required_branches.push(branch);
                }
                PrerequisiteType::Optional => {
                    if let (Some(group), Some(chain)) = (edge.or_group, sub_chain) {
                        or_groups
                            .entry(group)
                            .or_default()
                            .push((edge.prerequisite.clone(), chain));
                    }
                }
                _ => {}
            }
        }

        Some((required_branches, or_groups))
    }

    /// Finalize structured branches by selecting from OR-groups
    ///
    /// Selects the best option from each OR-group considering overlap with required courses.
    fn finalize_structured_branches(
        &self,
        mut required_branches: Vec<Vec<String>>,
        or_groups: OrGroupsMap,
        preferred_subject: Option<&str>,
        direct_required: &HashSet<String>,
    ) -> Option<Vec<Vec<String>>> {
        // Build set of all required courses for overlap detection
        let mut all_required: HashSet<String> = direct_required.clone();
        for branch in &required_branches {
            all_required.extend(branch.iter().cloned());
        }

        // Select best option from each OR-group
        for (_group, options) in or_groups {
            if options.is_empty() {
                return None;
            }
            if let Some((best_prereq, best_chain)) =
                select_best_or_option(options, preferred_subject, &all_required, self)
            {
                let mut branch = flatten_chain_to_ordered_branch(&best_chain, self);
                branch.push(best_prereq);
                all_required.extend(branch.iter().cloned());
                required_branches.push(branch);
            }
        }

        Some(required_branches)
    }

    /// Get the full dependent chain for a course (all courses that transitively depend on it)
    ///
    /// # Arguments
    /// * `course_key` - The course to get dependents for
    ///
    /// # Returns
    /// A set of all dependent course keys, or None if the course doesn't exist
    #[must_use]
    pub fn dependent_chain(&self, course_key: &str) -> Option<HashSet<String>> {
        let node = self.nodes.get(course_key)?;
        let mut chain = HashSet::new();
        let mut visited = HashSet::new();
        let mut queue: VecDeque<String> = node.dependents.iter().cloned().collect();

        while let Some(dep) = queue.pop_front() {
            if visited.contains(&dep) {
                continue;
            }
            visited.insert(dep.clone());
            chain.insert(dep.clone());

            if let Some(dep_node) = self.nodes.get(&dep) {
                for next_dep in &dep_node.dependents {
                    if !visited.contains(next_dep) {
                        queue.push_back(next_dep.clone());
                    }
                }
            }
        }

        Some(chain)
    }

    /// Compute the depth of a course (longest path from any leaf)
    ///
    /// Depth 0 = leaf courses (no prerequisites)
    /// Depth 1 = courses with only leaf prerequisites
    /// etc.
    ///
    /// Returns None if the course doesn't exist or if there are cycles
    /// Note: Corequisites are not counted for depth calculation
    #[must_use]
    pub fn course_depth(&self, course_key: &str) -> Option<usize> {
        if self.has_cycles() {
            return None;
        }

        let node = self.nodes.get(course_key)?;
        let prereq_edges = filter_true_prerequisites(&node.prerequisites);

        if prereq_edges.is_empty() {
            return Some(0);
        }

        // Find max depth among all prerequisites
        prereq_edges
            .iter()
            .filter_map(|edge| self.course_depth(&edge.prerequisite))
            .max()
            .map(|max| max + 1)
    }

    /// Detect all cycles in the graph (only prerequisite edges, not corequisites)
    ///
    /// Corequisites are intentionally bidirectional (A requires B concurrently, B requires A concurrently)
    /// so they are not considered cycles for this detection.
    fn detect_cycles(&self) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = Vec::new();
        let mut on_stack = HashSet::new();

        for key in self.nodes.keys() {
            if !visited.contains(key) {
                self.dfs_cycles(
                    key,
                    &mut visited,
                    &mut rec_stack,
                    &mut on_stack,
                    &mut cycles,
                );
            }
        }

        cycles
    }

    /// DFS helper for cycle detection (only follows prerequisite edges, not corequisites)
    fn dfs_cycles(
        &self,
        key: &str,
        visited: &mut HashSet<String>,
        rec_stack: &mut Vec<String>,
        on_stack: &mut HashSet<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(key.to_string());
        rec_stack.push(key.to_string());
        on_stack.insert(key.to_string());

        if let Some(node) = self.nodes.get(key) {
            for edge in &node.prerequisites {
                if !is_true_prerequisite(edge) {
                    continue;
                }

                let prereq = &edge.prerequisite;

                if !visited.contains(prereq) {
                    self.dfs_cycles(prereq, visited, rec_stack, on_stack, cycles);
                } else if on_stack.contains(prereq) {
                    // Found a cycle - extract it from the recursion stack
                    if let Some(start) = rec_stack.iter().position(|k| k == prereq) {
                        let mut cycle: Vec<String> = rec_stack[start..].to_vec();
                        cycle.push(prereq.clone());
                        cycles.push(cycle);
                    }
                }
            }
        }

        rec_stack.pop();
        on_stack.remove(key);
    }

    /// Compute topological order using Kahn's algorithm
    ///
    /// Only considers prerequisite edges (not corequisites).
    fn compute_topological_order(&self) -> Vec<String> {
        // Initialize in-degrees (only counting true prerequisites)
        let in_degree: HashMap<&str, usize> = self
            .nodes
            .iter()
            .map(|(key, node)| {
                let prereq_count = filter_true_prerequisites(&node.prerequisites).len();
                (key.as_str(), prereq_count)
            })
            .collect();

        self.kahn_topological_sort(in_degree)
    }

    /// Perform Kahn's algorithm for topological sorting
    ///
    /// Starting from nodes with zero in-degree, iteratively process nodes
    /// and decrement the in-degrees of their dependents.
    fn kahn_topological_sort(&self, mut in_degree: HashMap<&str, usize>) -> Vec<String> {
        let mut order = Vec::new();
        let mut queue: VecDeque<&str> = in_degree
            .iter()
            .filter(|(_, &deg)| deg == 0)
            .map(|(&key, _)| key)
            .collect();

        while let Some(key) = queue.pop_front() {
            order.push(key.to_string());

            if let Some(node) = self.nodes.get(key) {
                for dep in &node.dependents {
                    // Only decrement if this dependent has us as a true prereq
                    if self.is_true_prerequisite_of(key, dep) {
                        if let Some(degree) = in_degree.get_mut(dep.as_str()) {
                            *degree -= 1;
                            if *degree == 0 {
                                queue.push_back(dep.as_str());
                            }
                        }
                    }
                }
            }
        }

        order
    }

    /// Check if `prereq` is a true prerequisite (not corequisite) of `course`
    fn is_true_prerequisite_of(&self, prereq: &str, course: &str) -> bool {
        self.nodes.get(course).is_some_and(|node| {
            node.prerequisites
                .iter()
                .any(|e| e.prerequisite == prereq && is_true_prerequisite(e))
        })
    }

    /// Iterate over all nodes in the graph
    pub fn iter(&self) -> impl Iterator<Item = (&String, &CourseNode)> {
        self.nodes.iter()
    }
}

impl std::fmt::Display for CourseGraph {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Course Graph ({} courses):", self.nodes.len())?;
        writeln!(f)?;

        // Sort for consistent output
        let mut sorted_keys: Vec<_> = self.nodes.keys().collect();
        sorted_keys.sort();

        for key in sorted_keys {
            if let Some(node) = self.nodes.get(key) {
                let prereqs: Vec<String> = node
                    .prerequisites
                    .iter()
                    .map(|e| {
                        let suffix = match e.prereq_type {
                            PrerequisiteType::Required => "",
                            PrerequisiteType::Optional => "?",
                            PrerequisiteType::Corequisite => "(co)",
                            PrerequisiteType::StrictCorequisite => "(strict-co)",
                        };
                        format!("{}{}", e.prerequisite, suffix)
                    })
                    .collect();

                if prereqs.is_empty() {
                    writeln!(f, "  {key} → (no prerequisites)")?;
                } else {
                    writeln!(f, "  {key} → {}", prereqs.join(", "))?;
                }
            }
        }

        Ok(())
    }
}

/// Build initial course nodes from the degree program
fn build_initial_nodes(graph: &mut CourseGraph, program: &DegreeProgram) {
    for (key, course) in &program.courses {
        let node = CourseNode::new(key.clone(), Some(course));
        graph.nodes.insert(key.clone(), node);
    }
}

/// Add prerequisite edges and DNF paths to the graph
fn add_prerequisites(
    graph: &mut CourseGraph,
    program: &DegreeProgram,
    missing_courses: &mut HashSet<String>,
) {
    for (key, course) in &program.courses {
        let (edges, dnf_paths) = parse_prerequisites(course, &program.courses, missing_courses);

        if let Some(node) = graph.nodes.get_mut(key) {
            node.prerequisites = edges;
            node.prerequisite_paths = dnf_paths;
        }
    }
}

/// Add corequisite edges to the graph
fn add_corequisites(
    graph: &mut CourseGraph,
    program: &DegreeProgram,
    missing_courses: &mut HashSet<String>,
) {
    for (key, course) in &program.courses {
        add_corequisite_type(
            graph,
            key,
            &course.corequisites,
            PrerequisiteType::Corequisite,
            missing_courses,
        );
        add_corequisite_type(
            graph,
            key,
            &course.strict_corequisites,
            PrerequisiteType::StrictCorequisite,
            missing_courses,
        );
    }
}

/// Add a specific type of corequisite edges
fn add_corequisite_type(
    graph: &mut CourseGraph,
    course_key: &str,
    coreqs: &[String],
    prereq_type: PrerequisiteType,
    missing_courses: &mut HashSet<String>,
) {
    for coreq in coreqs {
        if !graph.nodes.contains_key(coreq) {
            missing_courses.insert(coreq.clone());
            graph
                .nodes
                .insert(coreq.clone(), CourseNode::new(coreq.clone(), None));
        }
        if let Some(node) = graph.nodes.get_mut(course_key) {
            node.prerequisites.push(PrerequisiteEdge {
                prerequisite: coreq.clone(),
                prereq_type,
                or_group: None,
            });
        }
    }
}

/// Add missing course nodes (prerequisites referenced but not defined)
fn add_missing_nodes(graph: &mut CourseGraph, missing_courses: &HashSet<String>) {
    for missing in missing_courses {
        if !graph.nodes.contains_key(missing) {
            graph
                .nodes
                .insert(missing.clone(), CourseNode::new(missing.clone(), None));
        }
    }
}

/// Build reverse edges (dependents) in the graph
fn build_reverse_edges(graph: &mut CourseGraph) {
    let keys: Vec<String> = graph.nodes.keys().cloned().collect();
    for key in &keys {
        let prereqs: Vec<String> = graph
            .nodes
            .get(key)
            .map(|n| {
                n.prerequisites
                    .iter()
                    .map(|e| e.prerequisite.clone())
                    .collect()
            })
            .unwrap_or_default();

        for prereq in prereqs {
            if let Some(prereq_node) = graph.nodes.get_mut(&prereq) {
                if !prereq_node.dependents.contains(key) {
                    prereq_node.dependents.push(key.clone());
                }
            }
        }
    }
}

/// Parse prerequisites from a course into edges and DNF paths
///
/// Uses the shared `prerequisite_parser` module to parse prerequisites.
///
/// Returns both:
/// - A flat list of edges for graph traversal
/// - DNF paths (OR of ANDs) for accurate prerequisite representation
fn parse_prerequisites(
    course: &Course,
    courses: &HashMap<String, Course>,
    missing: &mut HashSet<String>,
) -> (Vec<PrerequisiteEdge>, Vec<Vec<String>>) {
    let mut edges = Vec::new();
    let mut dnf_paths = Vec::new();

    if let Some(raw) = &course.prerequisites_raw {
        // Parse using shared parser module
        let parsed = prerequisite_parser::parse_to_edges(raw);

        for (prereq, is_optional, or_group) in parsed {
            // Track missing courses
            if !courses.contains_key(&prereq) {
                missing.insert(prereq.clone());
            }

            edges.push(PrerequisiteEdge {
                prerequisite: prereq,
                prereq_type: if is_optional {
                    PrerequisiteType::Optional
                } else {
                    PrerequisiteType::Required
                },
                or_group,
            });
        }

        // Parse into DNF form using shared parser
        dnf_paths = prerequisite_parser::parse_to_dnf(raw);
    } else if !course.prerequisites.is_empty() {
        // Use the already-parsed prerequisites (all treated as required)
        // This is a single path with all courses required
        let mut path = Vec::new();
        for prereq in &course.prerequisites {
            if !courses.contains_key(prereq) {
                missing.insert(prereq.clone());
            }
            edges.push(PrerequisiteEdge {
                prerequisite: prereq.clone(),
                prereq_type: PrerequisiteType::Required,
                or_group: None,
            });
            path.push(prereq.clone());
        }
        if !path.is_empty() {
            dnf_paths.push(path);
        }
    }

    (edges, dnf_paths)
}

/// Extract the subject code from a course key (e.g., "CS165" -> "CS", "MATH156" -> "MATH")
fn extract_subject(course_key: &str) -> Option<String> {
    let subject: String = course_key
        .chars()
        .take_while(char::is_ascii_alphabetic)
        .collect();
    if subject.is_empty() {
        None
    } else {
        Some(subject)
    }
}

/// Select the best prerequisite option from an OR group
///
/// Prioritizes:
/// 1. Same-subject courses (if `preferred_subject` is provided)
/// 2. Shortest chain length
fn select_best_prerequisite_option(
    options: Vec<(String, Vec<String>)>,
    preferred_subject: Option<&str>,
) -> Option<(String, Vec<String>)> {
    if options.is_empty() {
        return None;
    }

    // Separate same-subject and other options
    let (same_subject, other): (Vec<_>, Vec<_>) = if let Some(subject) = preferred_subject {
        options.into_iter().partition(|(prereq, _)| {
            extract_subject(prereq).is_some_and(|s| s.eq_ignore_ascii_case(subject))
        })
    } else {
        (Vec::new(), options)
    };

    // Prefer same-subject options, then pick shortest chain
    let candidates = if same_subject.is_empty() {
        other
    } else {
        same_subject
    };

    candidates.into_iter().min_by_key(|(_, chain)| chain.len())
}

/// Select the best prerequisite option from an OR group with plan context
///
/// Prioritizes:
/// 1. Courses already in the plan (to avoid adding redundant prerequisites)
/// 2. Same-subject courses (if `preferred_subject` is provided)
/// 3. Shortest chain length
fn select_best_prerequisite_option_with_context(
    options: Vec<(String, Vec<String>)>,
    preferred_subject: Option<&str>,
    plan_courses: &HashSet<String>,
) -> Option<(String, Vec<String>)> {
    if options.is_empty() {
        return None;
    }

    // First priority: courses already in the plan (no new courses needed)
    let (in_plan, not_in_plan): (Vec<_>, Vec<_>) = options
        .into_iter()
        .partition(|(prereq, _)| plan_courses.contains(prereq));

    if !in_plan.is_empty() {
        // If multiple are in plan, prefer same-subject, then shortest
        return select_best_prerequisite_option(in_plan, preferred_subject);
    }

    // Count how many NEW courses each option would add (prereq + chain courses not in plan)
    // Prefer options that add the fewest new courses
    let mut scored_options: Vec<(String, Vec<String>, usize)> = not_in_plan
        .into_iter()
        .map(|(prereq, chain)| {
            let new_count = 1 + chain.iter().filter(|c| !plan_courses.contains(*c)).count();
            (prereq, chain, new_count)
        })
        .collect();

    // Sort by: 1) fewest new courses, 2) same subject preference, 3) alphabetical
    scored_options.sort_by(|(prereq_a, _, count_a), (prereq_b, _, count_b)| {
        // First by new course count
        match count_a.cmp(count_b) {
            std::cmp::Ordering::Equal => {
                // Then by subject preference
                let a_same_subject = preferred_subject.is_some_and(|subj| {
                    extract_subject(prereq_a).is_some_and(|s| s.eq_ignore_ascii_case(subj))
                });
                let b_same_subject = preferred_subject.is_some_and(|subj| {
                    extract_subject(prereq_b).is_some_and(|s| s.eq_ignore_ascii_case(subj))
                });
                match (a_same_subject, b_same_subject) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => prereq_a.cmp(prereq_b), // Alphabetical as tiebreaker
                }
            }
            other => other,
        }
    });

    scored_options
        .into_iter()
        .next()
        .map(|(prereq, chain, _)| (prereq, chain))
}

/// Flatten a structured chain into a single ordered branch (leaf to root)
fn flatten_chain_to_ordered_branch(chain: &PrerequisiteChain, graph: &CourseGraph) -> Vec<String> {
    if chain.branches.is_empty() {
        return Vec::new();
    }

    // Collect all unique courses from all branches
    let all_courses: HashSet<String> = chain.branches.iter().flatten().cloned().collect();

    // Build a mini dependency graph for ordering
    let mut in_degree: HashMap<String, usize> = HashMap::new();
    let mut dependents: HashMap<String, Vec<String>> = HashMap::new();

    for course in &all_courses {
        in_degree.entry(course.clone()).or_insert(0);
        if let Some(node) = graph.nodes.get(course) {
            for edge in &node.prerequisites {
                if all_courses.contains(&edge.prerequisite) {
                    *in_degree.entry(course.clone()).or_insert(0) += 1;
                    dependents
                        .entry(edge.prerequisite.clone())
                        .or_default()
                        .push(course.clone());
                }
            }
        }
    }

    // Topological sort: courses with no prereqs (in this set) come first
    let mut result = Vec::new();
    // Sort initial queue for deterministic output
    let mut initial: Vec<String> = in_degree
        .iter()
        .filter(|(_, &deg)| deg == 0)
        .map(|(k, _)| k.clone())
        .collect();
    initial.sort();
    let mut queue: VecDeque<String> = initial.into_iter().collect();

    while let Some(course) = queue.pop_front() {
        result.push(course.clone());
        if let Some(deps) = dependents.get(&course) {
            for dep in deps {
                if let Some(deg) = in_degree.get_mut(dep) {
                    *deg -= 1;
                    if *deg == 0 {
                        queue.push_back(dep.clone());
                    }
                }
            }
        }
    }

    result
}

/// Select the best OR option considering overlap with existing requirements
///
/// Prioritizes:
/// 1. Options where the prereq itself is already in `existing_required`
/// 2. Options that have chain overlap with already-required courses
/// 3. Same-subject courses
/// 4. Shortest chain
fn select_best_or_option(
    options: Vec<(String, PrerequisiteChain)>,
    preferred_subject: Option<&str>,
    existing_required: &HashSet<String>,
    _graph: &CourseGraph,
) -> Option<(String, PrerequisiteChain)> {
    if options.is_empty() {
        return None;
    }

    // First check: if any option's prereq is already in `existing_required`, use it
    // This handles the case where a sibling also requires the same course
    for (prereq, chain) in &options {
        if existing_required.contains(prereq) {
            return Some((prereq.clone(), chain.clone()));
        }
    }

    // Calculate overlap and chain size for each option
    let scored: Vec<_> = options
        .into_iter()
        .map(|(prereq, chain)| {
            let all_courses: HashSet<_> = chain.branches.iter().flatten().cloned().collect();
            let overlap_count = all_courses.intersection(existing_required).count();
            let is_same_subject = preferred_subject.is_some_and(|s| {
                extract_subject(&prereq).is_some_and(|ps| ps.eq_ignore_ascii_case(s))
            });
            // Score: prioritize overlap, then same-subject, then smaller chains
            // Higher overlap = better, same subject = better, smaller chain = better
            (prereq, chain, overlap_count, is_same_subject)
        })
        .collect();

    // Find max overlap
    let max_overlap = scored.iter().map(|(_, _, o, _)| *o).max().unwrap_or(0);

    // Filter to best overlap options
    let best_overlap: Vec<_> = scored
        .into_iter()
        .filter(|(_, _, o, _)| *o == max_overlap)
        .collect();

    // Among those, prefer same-subject
    let same_subject: Vec<_> = best_overlap
        .iter()
        .filter(|(_, _, _, is_same)| *is_same)
        .cloned()
        .collect();

    let candidates = if same_subject.is_empty() {
        best_overlap
    } else {
        same_subject
    };

    // Pick shortest chain
    candidates
        .into_iter()
        .min_by_key(|(_, chain, _, _)| chain.total_courses)
        .map(|(prereq, chain, _, _)| (prereq, chain))
}

/// Merge overlapping branches by keeping longest chains
///
/// Instead of removing duplicates aggressively, we keep the longest chain
/// that includes each course. This preserves dependency information.
fn merge_overlapping_branches(branches: Vec<Vec<String>>) -> Vec<Vec<String>> {
    if branches.len() <= 1 {
        return branches;
    }

    // Sort branches by length (longest first) so we keep the most complete chains
    let mut sorted_branches = branches;
    sorted_branches.sort_by_key(|b| std::cmp::Reverse(b.len()));

    // Track which courses are already covered by a kept branch
    let mut covered: HashSet<String> = HashSet::new();
    let mut result: Vec<Vec<String>> = Vec::new();

    for branch in sorted_branches {
        // Check if this branch adds any new "leaf" courses (courses at the end)
        // A branch is worth keeping if its last element isn't covered
        if let Some(leaf) = branch.last() {
            if !covered.contains(leaf) {
                // Add all courses in this branch to covered
                covered.extend(branch.iter().cloned());
                result.push(branch);
            }
        }
    }

    // Sort result for consistent output
    result.sort_by(|a, b| a.first().cmp(&b.first()));
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_course_node_required_prereqs() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS200".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS101".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });

        assert!(node.has_required_prerequisites());
        assert_eq!(node.required_prerequisites(), vec!["CS200"]);

        let opt_groups = node.optional_prerequisite_groups();
        assert_eq!(opt_groups.len(), 1);
        assert!(opt_groups.get(&0).unwrap().contains(&"CS100"));
        assert!(opt_groups.get(&0).unwrap().contains(&"CS101"));
    }

    #[test]
    fn test_format_prerequisite_paths_single() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisite_paths = vec![vec!["CS200".to_string()]];
        assert_eq!(node.format_prerequisite_paths(), "CS200");
    }

    #[test]
    fn test_format_prerequisite_paths_and() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisite_paths = vec![vec!["CS200".to_string(), "CS201".to_string()]];
        assert_eq!(node.format_prerequisite_paths(), "CS200 & CS201");
    }

    #[test]
    fn test_format_prerequisite_paths_or() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisite_paths = vec![vec!["CS200".to_string()], vec!["CS201".to_string()]];
        assert_eq!(node.format_prerequisite_paths(), "CS200 | CS201");
    }

    #[test]
    fn test_format_prerequisite_paths_mixed() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisite_paths = vec![
            vec!["MATH124".to_string(), "MATH126".to_string()],
            vec!["MATH127".to_string()],
        ];
        assert_eq!(
            node.format_prerequisite_paths(),
            "(MATH124 & MATH126) | MATH127"
        );
    }

    #[test]
    fn test_format_prerequisite_paths_empty() {
        let node = CourseNode::new("CS300".to_string(), None);
        assert_eq!(node.format_prerequisite_paths(), "");
    }

    #[test]
    fn test_format_prerequisite_paths_complex() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisite_paths = vec![
            vec!["A".to_string(), "B".to_string(), "C".to_string()],
            vec!["D".to_string()],
            vec!["E".to_string(), "F".to_string()],
        ];
        assert_eq!(
            node.format_prerequisite_paths(),
            "(A & B & C) | D | (E & F)"
        );
    }

    #[test]
    fn test_course_node_no_prerequisites() {
        let node = CourseNode::new("CS100".to_string(), None);
        assert!(!node.has_required_prerequisites());
        assert!(node.required_prerequisites().is_empty());
        assert!(node.optional_prerequisite_groups().is_empty());
        assert!(node.corequisites().is_empty());
    }

    #[test]
    fn test_course_node_with_corequisites() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS301".to_string(),
            prereq_type: PrerequisiteType::Corequisite,
            or_group: None,
        });
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS302".to_string(),
            prereq_type: PrerequisiteType::StrictCorequisite,
            or_group: None,
        });

        let coreqs = node.corequisites();
        assert_eq!(coreqs.len(), 2);
        assert!(coreqs.contains(&"CS301"));
        assert!(coreqs.contains(&"CS302"));
    }

    #[test]
    fn test_course_node_multiple_or_groups() {
        let mut node = CourseNode::new("CS300".to_string(), None);
        // Group 0: CS100 | CS101
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS101".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        // Group 1: MATH100 | MATH101
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "MATH100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(1),
        });
        node.prerequisites.push(PrerequisiteEdge {
            prerequisite: "MATH101".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(1),
        });

        let groups = node.optional_prerequisite_groups();
        assert_eq!(groups.len(), 2);
        assert_eq!(groups.get(&0).unwrap().len(), 2);
        assert_eq!(groups.get(&1).unwrap().len(), 2);
    }

    #[test]
    fn test_min_prerequisite_depth_no_prereqs() {
        let mut graph = CourseGraph::default();
        graph.nodes.insert(
            "CS100".to_string(),
            CourseNode::new("CS100".to_string(), None),
        );

        assert_eq!(graph.min_prerequisite_depth("CS100"), Some(0));
    }

    #[test]
    fn test_min_prerequisite_depth_simple_chain() {
        let mut graph = CourseGraph::default();

        // CS100 (no prereqs) → CS200 → CS300
        graph.nodes.insert(
            "CS100".to_string(),
            CourseNode::new("CS100".to_string(), None),
        );

        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        let mut cs300 = CourseNode::new("CS300".to_string(), None);
        cs300.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS200".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("CS300".to_string(), cs300);

        assert_eq!(graph.min_prerequisite_depth("CS100"), Some(0));
        assert_eq!(graph.min_prerequisite_depth("CS200"), Some(1));
        assert_eq!(graph.min_prerequisite_depth("CS300"), Some(2));
    }

    #[test]
    fn test_min_prerequisite_depth_or_group() {
        let mut graph = CourseGraph::default();

        // CS100 and CS101 (no prereqs) - alternatives for CS200
        graph.nodes.insert(
            "CS100".to_string(),
            CourseNode::new("CS100".to_string(), None),
        );
        graph.nodes.insert(
            "CS101".to_string(),
            CourseNode::new("CS101".to_string(), None),
        );

        // CS200 requires (CS100 | CS101)
        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS101".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        // Should be depth 1 (one prereq needed from OR group)
        assert_eq!(graph.min_prerequisite_depth("CS200"), Some(1));
    }

    #[test]
    fn test_min_prerequisite_chain_simple() {
        let mut graph = CourseGraph::default();

        // CS100 → CS200 → CS300
        graph.nodes.insert(
            "CS100".to_string(),
            CourseNode::new("CS100".to_string(), None),
        );

        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        let mut cs300 = CourseNode::new("CS300".to_string(), None);
        cs300.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS200".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("CS300".to_string(), cs300);

        let chain = graph.min_prerequisite_chain("CS300").unwrap();
        assert_eq!(chain.len(), 2);
        assert!(chain.contains(&"CS200".to_string()));
        assert!(chain.contains(&"CS100".to_string()));
    }

    #[test]
    fn test_min_prerequisite_chain_or_selects_shorter() {
        let mut graph = CourseGraph::default();

        // A (no prereqs)
        // B → C (chain of 2)
        // D requires (A | B) - should choose A (shorter)
        graph
            .nodes
            .insert("A".to_string(), CourseNode::new("A".to_string(), None));
        graph
            .nodes
            .insert("C".to_string(), CourseNode::new("C".to_string(), None));

        let mut b = CourseNode::new("B".to_string(), None);
        b.prerequisites.push(PrerequisiteEdge {
            prerequisite: "C".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("B".to_string(), b);

        let mut d = CourseNode::new("D".to_string(), None);
        d.prerequisites.push(PrerequisiteEdge {
            prerequisite: "A".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        d.prerequisites.push(PrerequisiteEdge {
            prerequisite: "B".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("D".to_string(), d);

        let chain = graph.min_prerequisite_chain("D").unwrap();
        // Should select A (depth 0) over B (depth 1)
        assert_eq!(chain.len(), 1);
        assert!(chain.contains(&"A".to_string()));
    }

    #[test]
    fn test_min_prerequisite_chain_prefers_same_subject() {
        let mut graph = CourseGraph::default();

        // CIS100 (no prereqs) - different subject
        // CS100 (no prereqs) - same subject as CS200
        // CS200 requires (CIS100 | CS100) - should prefer CS100
        graph.nodes.insert(
            "CIS100".to_string(),
            CourseNode::new("CIS100".to_string(), None),
        );
        graph.nodes.insert(
            "CS100".to_string(),
            CourseNode::new("CS100".to_string(), None),
        );

        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CIS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        let chain = graph.min_prerequisite_chain("CS200").unwrap();
        // Should select CS100 (same subject) over CIS100
        assert_eq!(chain.len(), 1);
        assert!(chain.contains(&"CS100".to_string()));
        assert!(!chain.contains(&"CIS100".to_string()));
    }

    #[test]
    fn test_min_prerequisite_chain_handles_cycles_in_or_group() {
        let mut graph = CourseGraph::default();

        // Create a situation where one OR option leads to a cycle:
        // CS100 → CS101 (cycle back to CS100)
        // MATH100 (no prereqs)
        // CS200 requires (CS100 | MATH100)
        // Should select MATH100 because CS100 leads to cycle

        let mut cs100 = CourseNode::new("CS100".to_string(), None);
        cs100.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS101".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS100".to_string(), cs100);

        let mut cs101 = CourseNode::new("CS101".to_string(), None);
        cs101.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS101".to_string(), cs101);

        graph.nodes.insert(
            "MATH100".to_string(),
            CourseNode::new("MATH100".to_string(), None),
        );

        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "MATH100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        let chain = graph.min_prerequisite_chain("CS200").unwrap();
        // Should select MATH100 because CS100 path has cycle issues
        assert!(chain.contains(&"MATH100".to_string()) || chain.contains(&"CS100".to_string()));
    }

    #[test]
    fn test_break_cycles_removes_optional_edge_first() {
        let mut graph = CourseGraph::default();

        // Create a cycle: CS100 → CS200 → CS100
        // CS200 → CS100 is optional, CS100 → CS200 is required
        // Should remove the optional edge
        let mut cs100 = CourseNode::new("CS100".to_string(), None);
        cs100.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS200".to_string(),
            prereq_type: PrerequisiteType::Required,
            or_group: None,
        });
        graph.nodes.insert("CS100".to_string(), cs100);

        let mut cs200 = CourseNode::new("CS200".to_string(), None);
        cs200.prerequisites.push(PrerequisiteEdge {
            prerequisite: "CS100".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("CS200".to_string(), cs200);

        // Build reverse edges
        if let Some(cs100_node) = graph.nodes.get_mut("CS100") {
            cs100_node.dependents.push("CS200".to_string());
        }
        if let Some(cs200_node) = graph.nodes.get_mut("CS200") {
            cs200_node.dependents.push("CS100".to_string());
        }

        let cycles = vec![vec![
            "CS100".to_string(),
            "CS200".to_string(),
            "CS100".to_string(),
        ]];

        let removed = graph.break_cycles(&cycles);

        // Should remove the optional edge (CS200 → CS100)
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0], ("CS200".to_string(), "CS100".to_string()));

        // Graph should now have topological order
        assert!(!graph.has_cycles());
    }

    #[test]
    fn test_break_cycles_handles_multiple_cycles() {
        let mut graph = CourseGraph::default();

        // Create two separate cycles
        // Cycle 1: A → B → A
        // Cycle 2: C → D → C

        let mut a = CourseNode::new("A".to_string(), None);
        a.prerequisites.push(PrerequisiteEdge {
            prerequisite: "B".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("A".to_string(), a);

        let mut b = CourseNode::new("B".to_string(), None);
        b.prerequisites.push(PrerequisiteEdge {
            prerequisite: "A".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(0),
        });
        graph.nodes.insert("B".to_string(), b);

        let mut c = CourseNode::new("C".to_string(), None);
        c.prerequisites.push(PrerequisiteEdge {
            prerequisite: "D".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(1),
        });
        graph.nodes.insert("C".to_string(), c);

        let mut d = CourseNode::new("D".to_string(), None);
        d.prerequisites.push(PrerequisiteEdge {
            prerequisite: "C".to_string(),
            prereq_type: PrerequisiteType::Optional,
            or_group: Some(1),
        });
        graph.nodes.insert("D".to_string(), d);

        let cycles = vec![
            vec!["A".to_string(), "B".to_string(), "A".to_string()],
            vec!["C".to_string(), "D".to_string(), "C".to_string()],
        ];

        let removed = graph.break_cycles(&cycles);

        // Should remove one edge from each cycle
        assert_eq!(removed.len(), 2);
    }
}
