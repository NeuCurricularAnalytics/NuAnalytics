//! Directed Acyclic Graph for course prerequisites

use std::collections::HashMap;

/// Represents a directed acyclic graph of course prerequisites
///
/// The DAG uses two association lists:
/// - `dependencies`: maps each course to its prerequisites
/// - `dependents`: maps each course to the courses that depend on it (reverse graph)
///
/// This bidirectional structure enables efficient traversal in both directions.
///
/// # Handling Complex Boolean Prerequisites
/// Currently, the DAG assumes AND semantics (all prerequisites must be satisfied).
/// Real curricula have complex boolean expressions like:
/// - "CS101 OR CS102"
/// - "(CS101 AND MATH156) OR CS200"
/// - "CS101 OR (CS102 AND MATH156)"
///
/// Approaches to handle complex prerequisites:
///
/// ## Option 1: Disjunctive Normal Form (DNF)
/// Store prerequisites as OR of ANDs: `Vec<Vec<String>>`.
/// Example: `[[\"CS101\"], [\"CS102\", \"MATH156\"]]` means CS101 OR (CS102 AND MATH156).
/// - Pros: Standard logical form, reasonably simple
/// - Cons: Some expressions require exponential expansion
///
/// ## Option 2: Expression Tree with Metadata
/// Store prerequisite relationships with a tree structure:
/// ```ignore
/// enum PrereqNode {
///     Course(String),
///     And(Vec<PrereqNode>),
///     Or(Vec<PrereqNode>),
/// }
/// // In DAG: dependencies: HashMap<String, PrereqNode>
/// ```
/// - Pros: Can represent any boolean expression naturally
/// - Cons: More complex to traverse and validate
///
/// ## Option 3: Virtual/Composite Nodes
/// Create synthetic nodes like `\"CS101_OR_CS102\"` or `\"CS101_AND_MATH156\"`.
/// Build satisfaction rules: virtual node is satisfied if any/all children are satisfied.
/// - Pros: Keeps graph structure simple
/// - Cons: Need to track which nodes are virtual and their satisfaction rules
///
/// ## Option 4: Hypergraph with Labeled Edges
/// Extend to hypergraph where edges can connect to multiple nodes with labels:
/// ```ignore
/// struct HyperEdge {
///     to_course: String,
///     from_groups: Vec<(Vec<String>, BoolOp)>,  // (courses, AND/OR)
/// }
/// ```
/// - Pros: Natural representation of complex requirements
/// - Cons: More complex graph algorithms needed
///
/// ## Option 5: Choice Resolution at Build Time (Recommended for Plans)
/// When building a DAG from a plan (not just from courses), require the plan
/// to specify which alternative was chosen, then build a simple DAG.
/// **Important**: The Course struct must still store the full prerequisite expression
/// using one of the above approaches. This option is about how we build the DAG from
/// a plan, not about how we store prerequisites in courses.
/// - Example: Course CS300 requires "(CS101 AND MATH156) OR CS200". The Course stores
///   this full expression. When a plan includes CS300, it also specifies that the student
///   took CS101 and MATH156 (not CS200), so we build the DAG with only those edges.
///
///   This is the recommended approach for plan analysis since plans represent
///   actual course selections where students have already chosen which alternative
///   to take. The boolean logic has been resolved by the student's choices.
/// - Pros: Keeps DAG simple, works for actual plans, preserves full requirement info
/// - Cons: Cannot analyze "what if" scenarios from the DAG alone (need course data)
///
/// ## Option 6: Multi-DAG Expansion
/// Generate all possible DAGs by expanding boolean choices, then analyze each.
/// For "(CS101 AND MATH156) OR CS200", generate two DAGs: one with both CS101 and
/// MATH156, another with CS200.
/// - Pros: Can find optimal paths, analyze all possibilities
/// - Cons: Exponential explosion for complex requirements
#[derive(Debug, Clone)]
pub struct DAG {
    /// Maps course key -> list of prerequisite course keys
    pub dependencies: HashMap<String, Vec<String>>,

    /// Maps course key -> list of courses that depend on it
    pub dependents: HashMap<String, Vec<String>>,

    /// All course keys in the DAG
    pub courses: Vec<String>,
}

impl DAG {
    /// Create a new empty DAG
    #[must_use]
    pub fn new() -> Self {
        Self {
            dependencies: HashMap::new(),
            dependents: HashMap::new(),
            courses: Vec::new(),
        }
    }

    /// Add a course to the DAG
    ///
    /// # Arguments
    /// * `course_key` - The unique course key (e.g., "CS1800")
    pub fn add_course(&mut self, course_key: String) {
        if !self.courses.contains(&course_key) {
            self.courses.push(course_key.clone());
            self.dependencies.entry(course_key.clone()).or_default();
            self.dependents.entry(course_key).or_default();
        }
    }

    /// Add a prerequisite relationship
    ///
    /// # Arguments
    /// * `course_key` - Course that requires the prerequisite
    /// * `prerequisite_key` - Course that must be taken first
    pub fn add_prerequisite(&mut self, course_key: String, prerequisite_key: &str) {
        // Ensure both courses exist in the DAG
        self.add_course(course_key.clone());
        self.add_course(prerequisite_key.to_string());

        // Add to dependencies (course -> prerequisites)
        if let Some(deps) = self.dependencies.get_mut(&course_key) {
            if !deps.contains(&prerequisite_key.to_string()) {
                deps.push(prerequisite_key.to_string());
            }
        }

        // Add to dependents (prerequisite -> courses that depend on it)
        if let Some(deps) = self.dependents.get_mut(prerequisite_key) {
            if !deps.contains(&course_key) {
                deps.push(course_key);
            }
        }
    }

    /// Get all prerequisites for a course
    ///
    /// # Arguments
    /// * `course_key` - The course to query
    ///
    /// # Returns
    /// A reference to the list of prerequisite keys, or None if course not found
    #[must_use]
    pub fn get_prerequisites(&self, course_key: &str) -> Option<&Vec<String>> {
        self.dependencies.get(course_key)
    }

    /// Get all courses that depend on (require) a given course
    ///
    /// # Arguments
    /// * `course_key` - The course to query
    ///
    /// # Returns
    /// A reference to the list of dependent course keys, or None if course not found
    #[must_use]
    pub fn get_dependents(&self, course_key: &str) -> Option<&Vec<String>> {
        self.dependents.get(course_key)
    }

    /// Get the number of courses in the DAG
    #[must_use]
    pub const fn course_count(&self) -> usize {
        self.courses.len()
    }

    /// Check if a course exists in the DAG
    #[must_use]
    pub fn contains_course(&self, course_key: &str) -> bool {
        self.courses.contains(&course_key.to_string())
    }
}

impl Default for DAG {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for DAG {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "Prerequisite DAG ({} courses):", self.courses.len())?;
        writeln!(f)?;

        // Sort courses for consistent output
        let mut sorted_courses = self.courses.clone();
        sorted_courses.sort();

        for course_key in sorted_courses {
            if let Some(deps) = self.dependencies.get(&course_key) {
                if deps.is_empty() {
                    writeln!(f, "  {course_key} → (no prerequisites)")?;
                } else {
                    let deps_str = deps.join(", ");
                    writeln!(f, "  {course_key} → {deps_str}")?;
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dag_creation() {
        let dag = DAG::new();
        assert_eq!(dag.course_count(), 0);
    }

    #[test]
    fn test_add_course() {
        let mut dag = DAG::new();
        dag.add_course("CS1800".to_string());
        assert_eq!(dag.course_count(), 1);
        assert!(dag.contains_course("CS1800"));
    }

    #[test]
    fn test_add_prerequisite() {
        let mut dag = DAG::new();
        dag.add_prerequisite("CS220".to_string(), "CS165");

        assert_eq!(dag.course_count(), 2);
        assert!(dag.contains_course("CS220"));
        assert!(dag.contains_course("CS165"));

        // Verify dependency relationship
        let cs220_deps = dag.get_prerequisites("CS220").unwrap();
        assert!(cs220_deps.contains(&"CS165".to_string()));

        // Verify reverse relationship
        let cs165_dependents = dag.get_dependents("CS165").unwrap();
        assert!(cs165_dependents.contains(&"CS220".to_string()));
    }

    #[test]
    fn test_multiple_prerequisites() {
        let mut dag = DAG::new();
        dag.add_prerequisite("CS320".to_string(), "CS165");
        dag.add_prerequisite("CS320".to_string(), "CS220");

        let cs320_deps = dag.get_prerequisites("CS320").unwrap();
        assert_eq!(cs320_deps.len(), 2);
        assert!(cs320_deps.contains(&"CS165".to_string()));
        assert!(cs320_deps.contains(&"CS220".to_string()));
    }

    #[test]
    fn test_duplicate_prerequisite() {
        let mut dag = DAG::new();
        dag.add_prerequisite("CS220".to_string(), "CS165");
        dag.add_prerequisite("CS220".to_string(), "CS165");

        let cs220_deps = dag.get_prerequisites("CS220").unwrap();
        assert_eq!(cs220_deps.len(), 1); // Should not duplicate
    }

    #[test]
    fn test_dag_display() {
        let mut dag = DAG::new();
        dag.add_prerequisite("CS220".to_string(), "CS165");
        dag.add_prerequisite("CS220".to_string(), "MATH156");
        dag.add_course("CS1800".to_string()); // Course with no prerequisites

        let display = format!("{dag}");
        assert!(display.contains("Prerequisite DAG"));
        assert!(display.contains("CS220"));
        assert!(display.contains("CS165"));
    }
}
