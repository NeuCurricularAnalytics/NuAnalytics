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
#[derive(Debug, Clone)]
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

        // First pass: add all defined courses as nodes
        for (key, course) in &program.courses {
            let node = CourseNode::new(key.clone(), Some(course));
            graph.nodes.insert(key.clone(), node);
        }

        // Second pass: add prerequisite edges and DNF paths
        for (key, course) in &program.courses {
            let (edges, dnf_paths) =
                parse_prerequisites(course, &program.courses, &mut missing_courses);

            // Add edges and DNF paths to the course node
            if let Some(node) = graph.nodes.get_mut(key) {
                node.prerequisites = edges;
                node.prerequisite_paths = dnf_paths;
            }

            // Add corequisites
            for coreq in &course.corequisites {
                if !graph.nodes.contains_key(coreq) {
                    missing_courses.insert(coreq.clone());
                    graph
                        .nodes
                        .insert(coreq.clone(), CourseNode::new(coreq.clone(), None));
                }
                if let Some(node) = graph.nodes.get_mut(key) {
                    node.prerequisites.push(PrerequisiteEdge {
                        prerequisite: coreq.clone(),
                        prereq_type: PrerequisiteType::Corequisite,
                        or_group: None,
                    });
                }
            }

            // Add strict corequisites
            for coreq in &course.strict_corequisites {
                if !graph.nodes.contains_key(coreq) {
                    missing_courses.insert(coreq.clone());
                    graph
                        .nodes
                        .insert(coreq.clone(), CourseNode::new(coreq.clone(), None));
                }
                if let Some(node) = graph.nodes.get_mut(key) {
                    node.prerequisites.push(PrerequisiteEdge {
                        prerequisite: coreq.clone(),
                        prereq_type: PrerequisiteType::StrictCorequisite,
                        or_group: None,
                    });
                }
            }
        }

        // Add missing courses as nodes (prerequisites referenced but not defined)
        for missing in &missing_courses {
            if !graph.nodes.contains_key(missing) {
                graph
                    .nodes
                    .insert(missing.clone(), CourseNode::new(missing.clone(), None));
            }
        }

        // Third pass: build reverse edges (dependents)
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

        // Detect cycles
        let cycles = graph.detect_cycles();

        // Compute topological order if no cycles
        if cycles.is_empty() {
            graph.topo_order = Some(graph.compute_topological_order());
        }

        CourseGraphResult {
            graph,
            cycles,
            missing_courses: missing_courses.into_iter().collect(),
        }
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

        // Count only non-corequisite prerequisites
        let prereq_edges: Vec<_> = node
            .prerequisites
            .iter()
            .filter(|e| {
                e.prereq_type != PrerequisiteType::Corequisite
                    && e.prereq_type != PrerequisiteType::StrictCorequisite
            })
            .collect();

        if prereq_edges.is_empty() {
            return Some(0);
        }

        let mut max_depth = 0;
        for edge in prereq_edges {
            if let Some(prereq_depth) = self.course_depth(&edge.prerequisite) {
                max_depth = max_depth.max(prereq_depth + 1);
            }
        }

        Some(max_depth)
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
                // Skip corequisites - they are expected to be bidirectional
                if edge.prereq_type == PrerequisiteType::Corequisite
                    || edge.prereq_type == PrerequisiteType::StrictCorequisite
                {
                    continue;
                }

                let prereq = &edge.prerequisite;

                if !visited.contains(prereq) {
                    self.dfs_cycles(prereq, visited, rec_stack, on_stack, cycles);
                } else if on_stack.contains(prereq) {
                    // Found a cycle
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
    /// Only considers prerequisite edges (not corequisites)
    fn compute_topological_order(&self) -> Vec<String> {
        let mut in_degree: HashMap<&str, usize> = HashMap::new();
        let mut order = Vec::new();
        let mut queue = VecDeque::new();

        // Initialize in-degrees (only counting non-corequisite edges)
        for (key, node) in &self.nodes {
            let prereq_count = node
                .prerequisites
                .iter()
                .filter(|e| {
                    e.prereq_type != PrerequisiteType::Corequisite
                        && e.prereq_type != PrerequisiteType::StrictCorequisite
                })
                .count();
            in_degree.insert(key.as_str(), prereq_count);
        }

        // Add nodes with zero in-degree to queue
        for (key, &degree) in &in_degree {
            if degree == 0 {
                queue.push_back(*key);
            }
        }

        while let Some(key) = queue.pop_front() {
            order.push(key.to_string());

            if let Some(node) = self.nodes.get(key) {
                for dep in &node.dependents {
                    // Only decrement if this dependent has us as a non-corequisite prereq
                    if let Some(dep_node) = self.nodes.get(dep) {
                        let is_prereq = dep_node.prerequisites.iter().any(|e| {
                            e.prerequisite == key
                                && e.prereq_type != PrerequisiteType::Corequisite
                                && e.prereq_type != PrerequisiteType::StrictCorequisite
                        });
                        if is_prereq {
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
        }

        order
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
}
