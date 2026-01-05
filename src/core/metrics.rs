//! Complexity and curriculum metrics

use crate::core::models::DAG;
use std::collections::{HashMap, HashSet, VecDeque};

/// Delay factor per course keyed by course code (e.g., "CS2510").
pub type DelayByCourse = HashMap<String, usize>;

/// Blocking factor per course keyed by course code (e.g., "CS2510").
pub type BlockingByCourse = HashMap<String, usize>;

/// Structural complexity per course keyed by course code (e.g., "CS2510").
pub type ComplexityByCourse = HashMap<String, usize>;

/// Centrality per course keyed by course code (e.g., "CS2510").
pub type CentralityByCourse = HashMap<String, usize>;

/// Metrics for a single course
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CourseMetrics {
    /// Delay factor (longest requisite path length in vertices)
    pub delay: usize,
    /// Blocking factor (number of courses blocked)
    pub blocking: usize,
    /// Structural complexity (delay + blocking)
    pub complexity: usize,
    /// Centrality (sum of path lengths through this course)
    pub centrality: usize,
}

/// All metrics for a curriculum, keyed by course code
pub type CurriculumMetrics = HashMap<String, CourseMetrics>;

/// Compute all metrics for every course in the requisite graph.
///
/// # Errors
///
/// Returns an error if the graph contains a cycle.
pub fn compute_all_metrics(dag: &DAG) -> Result<CurriculumMetrics, String> {
    let delay = compute_delay(dag)?;
    let blocking = compute_blocking(dag)?;
    let complexity = compute_complexity(&delay, &blocking)?;
    let centrality = compute_centrality(dag)?;

    let mut metrics = CurriculumMetrics::new();

    for course in &dag.courses {
        let delay_val = delay.get(course).copied().unwrap_or(0);
        let blocking_val = blocking.get(course).copied().unwrap_or(0);
        let complexity_val = complexity.get(course).copied().unwrap_or(0);
        let centrality_val = centrality.get(course).copied().unwrap_or(0);

        metrics.insert(
            course.clone(),
            CourseMetrics {
                delay: delay_val,
                blocking: blocking_val,
                complexity: complexity_val,
                centrality: centrality_val,
            },
        );
    }

    Ok(metrics)
}

/// Compute the delay factor for every course in the requisite graph.
///
/// The delay factor of a course is the length (in vertices) of the longest
/// path in the requisite DAG that contains that course. Both prerequisites
/// and corequisites are treated as edges when forming paths.
///
/// # Errors
///
/// Returns an error if the graph contains a cycle because longest-path
/// computation assumes a DAG.
pub fn compute_delay(dag: &DAG) -> Result<DelayByCourse, String> {
    let outgoing = build_outgoing_edges(dag);
    let indegree = build_indegree_counts(dag);

    let topo_order = topological_order(&dag.courses, &outgoing, &indegree)?;
    let longest_to = longest_paths_to(&topo_order, dag);
    let longest_from = longest_paths_from(&topo_order, &outgoing);

    let delays = dag
        .courses
        .iter()
        .map(|course| {
            let to_len = longest_to.get(course).copied().unwrap_or(0);
            let from_len = longest_from.get(course).copied().unwrap_or(0);
            (course.clone(), to_len + from_len + 1)
        })
        .collect();

    Ok(delays)
}

/// Compute the blocking factor for every course in the requisite graph.
///
/// The blocking factor of a course is the number of other courses in the
/// curriculum that are reachable from it (i.e., courses that have this course
/// somewhere in their prerequisite or corequisite chain). A high blocking factor
/// indicates a gateway course that blocks access to many other courses.
///
/// # Errors
///
/// Returns an error if the graph contains a cycle (though blocking factor
/// computation itself doesn't strictly require acyclicity, we verify it for
/// consistency with other metrics).
pub fn compute_blocking(dag: &DAG) -> Result<BlockingByCourse, String> {
    let outgoing = build_outgoing_edges(dag);
    let indegree = build_indegree_counts(dag);

    // Verify DAG is acyclic
    let _ = topological_order(&dag.courses, &outgoing, &indegree)?;

    let blocking = dag
        .courses
        .iter()
        .map(|course| {
            let reachable_count = count_reachable(course, &outgoing);
            (course.clone(), reachable_count)
        })
        .collect();

    Ok(blocking)
}

/// Compute the structural complexity for every course.
///
/// Structural complexity is defined as the sum of delay factor and blocking
/// factor. It captures both the length of prerequisite chains leading to a
/// course (delay) and the number of courses that depend on it (blocking).
///
/// # Errors
///
/// Returns an error if the input maps have mismatched keys.
pub fn compute_complexity(
    delay: &DelayByCourse,
    blocking: &BlockingByCourse,
) -> Result<ComplexityByCourse, String> {
    let mut complexity = HashMap::new();

    for (course, delay_val) in delay {
        let blocking_val = blocking
            .get(course)
            .ok_or_else(|| format!("Course '{course}' missing from blocking map"))?;

        complexity.insert(course.clone(), delay_val + blocking_val);
    }

    Ok(complexity)
}

/// Compute the centrality for every course in the requisite graph.
///
/// Centrality of a course is the sum of the lengths of all source-to-sink paths
/// that pass through that course. Source and sink vertices have centrality 0.
/// This metric identifies courses that are central to many pathways through
/// the curriculum.
///
/// # Errors
///
/// Returns an error if the graph contains a cycle.
pub fn compute_centrality(dag: &DAG) -> Result<CentralityByCourse, String> {
    let outgoing = build_outgoing_edges(dag);
    let incoming = build_incoming_edges(dag);
    let indegree = build_indegree_counts(dag);

    // Verify DAG is acyclic
    let _ = topological_order(&dag.courses, &outgoing, &indegree)?;

    // Find sources (no incoming edges) and sinks (no outgoing edges)
    let sources: Vec<String> = dag
        .courses
        .iter()
        .filter(|c| incoming.get(*c).is_none_or(Vec::is_empty))
        .cloned()
        .collect();

    let sinks: Vec<String> = dag
        .courses
        .iter()
        .filter(|c| outgoing.get(*c).is_none_or(Vec::is_empty))
        .cloned()
        .collect();

    // Initialize centrality to 0 for all courses
    let mut centrality: HashMap<String, usize> =
        dag.courses.iter().map(|c| (c.clone(), 0)).collect();

    // For each source, enumerate all paths to all sinks
    for source in &sources {
        for sink in &sinks {
            if source != sink {
                enumerate_paths_and_update_centrality(source, sink, &outgoing, &mut centrality);
            }
        }
    }

    Ok(centrality)
}

/// Enumerate all paths from source to sink and update centrality counts.
/// Only intermediate nodes (not source or sink) get centrality updates.
fn enumerate_paths_and_update_centrality(
    source: &str,
    sink: &str,
    outgoing: &HashMap<String, Vec<String>>,
    centrality: &mut HashMap<String, usize>,
) {
    let mut path = Vec::new();
    let mut visited = HashSet::new();

    path.push(source.to_string());
    visited.insert(source.to_string());

    dfs_paths(source, sink, &mut path, &mut visited, outgoing, centrality);
}

/// DFS to find all paths from current node to target, updating centrality.
/// Only intermediate nodes (not source or sink) are included in centrality updates.
fn dfs_paths(
    current: &str,
    target: &str,
    path: &mut Vec<String>,
    visited: &mut HashSet<String>,
    outgoing: &HashMap<String, Vec<String>>,
    centrality: &mut HashMap<String, usize>,
) {
    if current == target {
        // Found a complete path - add its length to centrality of intermediate nodes only
        // (exclude the source at path[0] and sink at path[len-1])
        if path.len() <= 2 {
            // If path has only source and sink, no intermediate nodes
            return;
        }

        let path_length = path.len();
        // Skip first (source) and last (sink) elements
        for course in path.iter().skip(1).take(path.len() - 2) {
            if let Some(count) = centrality.get_mut(course) {
                *count += path_length;
            }
        }
        return;
    }

    if let Some(neighbors) = outgoing.get(current) {
        for neighbor in neighbors {
            if !visited.contains(neighbor) {
                visited.insert(neighbor.clone());
                path.push(neighbor.clone());

                dfs_paths(neighbor, target, path, visited, outgoing, centrality);

                path.pop();
                visited.remove(neighbor);
            }
        }
    }
}

/// Build a map of incoming edges (prerequisites and corequisites) for each course
///
/// # Arguments
/// * `dag` - The directed acyclic graph of course prerequisites
///
/// # Returns
/// A map from each course to its sorted list of prerequisite and corequisite courses
fn build_incoming_edges(dag: &DAG) -> HashMap<String, Vec<String>> {
    let mut incoming = HashMap::new();

    for course in &dag.courses {
        let mut neighbors: HashSet<String> = HashSet::new();

        if let Some(prereqs) = dag.dependencies.get(course) {
            neighbors.extend(prereqs.iter().cloned());
        }

        if let Some(coreqs) = dag.corequisites.get(course) {
            neighbors.extend(coreqs.iter().cloned());
        }

        let mut sorted: Vec<String> = neighbors.into_iter().collect();
        sorted.sort();

        incoming.insert(course.clone(), sorted);
    }

    incoming
}

/// Count the number of courses reachable from a given course via breadth-first search
///
/// # Arguments
/// * `start` - The course key to start from
/// * `outgoing` - Map of outgoing edges from each course
///
/// # Returns
/// The count of reachable courses (excluding the start course itself)
fn count_reachable(start: &str, outgoing: &HashMap<String, Vec<String>>) -> usize {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();

    queue.push_back(start.to_string());
    visited.insert(start.to_string());

    while let Some(course) = queue.pop_front() {
        if let Some(neighbors) = outgoing.get(&course) {
            for neighbor in neighbors {
                if visited.insert(neighbor.clone()) {
                    queue.push_back(neighbor.clone());
                }
            }
        }
    }

    // Subtract 1 because we don't count the starting course itself
    visited.len() - 1
}

/// Build a map of outgoing edges (dependents) for each course
///
/// Creates the reverse graph where edges point from prerequisites to courses that require them.
///
/// # Arguments
/// * `dag` - The directed acyclic graph of course prerequisites
///
/// # Returns
/// A map from each course to its sorted list of dependent courses
fn build_outgoing_edges(dag: &DAG) -> HashMap<String, Vec<String>> {
    let mut outgoing = HashMap::new();

    for course in &dag.courses {
        let mut neighbors: HashSet<String> = HashSet::new();

        if let Some(dependents) = dag.dependents.get(course) {
            neighbors.extend(dependents.iter().cloned());
        }

        if let Some(coreq_dependents) = dag.coreq_dependents.get(course) {
            neighbors.extend(coreq_dependents.iter().cloned());
        }

        let mut sorted: Vec<String> = neighbors.into_iter().collect();
        sorted.sort();

        outgoing.insert(course.clone(), sorted);
    }

    outgoing
}

/// Calculate the in-degree (number of incoming edges) for each course
///
/// The in-degree represents how many prerequisites and corequisites a course has.
///
/// # Arguments
/// * `dag` - The directed acyclic graph of course prerequisites
///
/// # Returns
/// A map from each course to its in-degree count
fn build_indegree_counts(dag: &DAG) -> HashMap<String, usize> {
    let mut indegree = HashMap::new();

    for course in &dag.courses {
        let mut incoming: HashSet<String> = HashSet::new();

        if let Some(prereqs) = dag.dependencies.get(course) {
            incoming.extend(prereqs.iter().cloned());
        }

        if let Some(coreqs) = dag.corequisites.get(course) {
            incoming.extend(coreqs.iter().cloned());
        }

        indegree.insert(course.clone(), incoming.len());
    }

    indegree
}

/// Compute a topological ordering of courses using Kahn's algorithm
///
/// # Arguments
/// * `courses` - List of all course keys
/// * `outgoing` - Map of outgoing edges from each course
/// * `indegree` - Map of in-degree counts for each course
///
/// # Returns
/// A topologically sorted list of courses
///
/// # Errors
/// Returns an error if a cycle is detected in the graph
fn topological_order(
    courses: &[String],
    outgoing: &HashMap<String, Vec<String>>,
    indegree: &HashMap<String, usize>,
) -> Result<Vec<String>, String> {
    let mut indegree_mut = indegree.clone();
    let mut queue: VecDeque<String> = courses
        .iter()
        .filter(|c| indegree_mut.get(*c).copied().unwrap_or(0) == 0)
        .cloned()
        .collect();

    let mut order = Vec::with_capacity(courses.len());

    while let Some(course) = queue.pop_front() {
        order.push(course.clone());

        if let Some(children) = outgoing.get(&course) {
            for child in children {
                let entry = indegree_mut
                    .get_mut(child)
                    .ok_or_else(|| format!("Course '{child}' missing from indegree map"))?;

                if *entry > 0 {
                    *entry -= 1;
                }

                if *entry == 0 {
                    queue.push_back(child.clone());
                }
            }
        }
    }

    if order.len() != courses.len() {
        return Err("Cycle detected in requisite graph; cannot compute delay factors".to_string());
    }

    Ok(order)
}

/// Compute the longest path length from any root to each course
///
/// Uses dynamic programming over the topological order to find the longest
/// incoming path to each course.
///
/// # Arguments
/// * `topo_order` - Topologically sorted list of courses
/// * `dag` - The directed acyclic graph of course prerequisites
///
/// # Returns
/// A map from each course to its longest incoming path length
fn longest_paths_to(topo_order: &[String], dag: &DAG) -> HashMap<String, usize> {
    let mut longest = HashMap::new();

    for course in topo_order {
        let mut best = 0usize;

        if let Some(prereqs) = dag.dependencies.get(course) {
            for parent in prereqs {
                let candidate = longest.get(parent).copied().unwrap_or(0) + 1;
                if candidate > best {
                    best = candidate;
                }
            }
        }

        if let Some(coreqs) = dag.corequisites.get(course) {
            for parent in coreqs {
                let candidate = longest.get(parent).copied().unwrap_or(0) + 1;
                if candidate > best {
                    best = candidate;
                }
            }
        }

        longest.insert(course.clone(), best);
    }

    longest
}

/// Compute the longest path length from each course to any leaf
///
/// Uses dynamic programming over the reverse topological order to find the
/// longest outgoing path from each course.
///
/// # Arguments
/// * `topo_order` - Topologically sorted list of courses
/// * `outgoing` - Map of outgoing edges from each course
///
/// # Returns
/// A map from each course to its longest outgoing path length
fn longest_paths_from(
    topo_order: &[String],
    outgoing: &HashMap<String, Vec<String>>,
) -> HashMap<String, usize> {
    let mut longest = HashMap::new();

    for course in topo_order.iter().rev() {
        let mut best = 0usize;

        if let Some(children) = outgoing.get(course) {
            for child in children {
                let candidate = longest.get(child).copied().unwrap_or(0) + 1;
                if candidate > best {
                    best = candidate;
                }
            }
        }

        longest.insert(course.clone(), best);
    }

    longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::planner::parse_curriculum_csv;

    #[test]
    fn computes_delay_on_simple_dag() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("D".to_string(), "B");
        dag.add_prerequisite("C".to_string(), "A");

        let delays = compute_delay(&dag).expect("delay factors");

        assert_eq!(delays.get("A"), Some(&3));
        assert_eq!(delays.get("B"), Some(&3));
        assert_eq!(delays.get("C"), Some(&2));
        assert_eq!(delays.get("D"), Some(&3));
    }

    #[test]
    fn counts_corequisites_as_edges() {
        let mut dag = DAG::new();
        dag.add_corequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "B");

        let delays = compute_delay(&dag).expect("delay factors");

        assert_eq!(delays.get("A"), Some(&3));
        assert_eq!(delays.get("B"), Some(&3));
        assert_eq!(delays.get("C"), Some(&3));
    }

    #[test]
    fn matches_sample_delay_values() {
        let school = parse_curriculum_csv("samples/correct/Colostate_CSDegree_w_metrics.csv")
            .expect("parse sample curriculum");
        let dag = school.build_dag();
        let delays = compute_delay(&dag).expect("delay factors");

        assert_eq!(delays.get("MATH156"), Some(&3));
        assert_eq!(delays.get("CS165"), Some(&6));
        assert_eq!(delays.get("CO150"), Some(&2));
        assert_eq!(delays.get("CS320"), Some(&4));
    }

    #[test]
    fn computes_blocking_on_simple_dag() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("D".to_string(), "B");
        dag.add_prerequisite("C".to_string(), "A");

        let blocking = compute_blocking(&dag).expect("blocking factors");

        // A blocks B, C, D (3 courses)
        assert_eq!(blocking.get("A"), Some(&3));
        // B blocks D (1 course)
        assert_eq!(blocking.get("B"), Some(&1));
        // C blocks nothing
        assert_eq!(blocking.get("C"), Some(&0));
        // D blocks nothing
        assert_eq!(blocking.get("D"), Some(&0));
    }

    #[test]
    fn blocking_counts_corequisites() {
        let mut dag = DAG::new();
        dag.add_corequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "B");

        let blocking = compute_blocking(&dag).expect("blocking factors");

        // A blocks B and C (via coreq edge to B)
        assert_eq!(blocking.get("A"), Some(&2));
        // B blocks C
        assert_eq!(blocking.get("B"), Some(&1));
        // C blocks nothing
        assert_eq!(blocking.get("C"), Some(&0));
    }

    #[test]
    fn matches_sample_blocking_values() {
        let school = parse_curriculum_csv("samples/correct/Colostate_CSDegree_w_metrics.csv")
            .expect("parse sample curriculum");
        let dag = school.build_dag();
        let blocking = compute_blocking(&dag).expect("blocking factors");

        assert_eq!(blocking.get("MATH156"), Some(&6));
        assert_eq!(blocking.get("CS150B"), Some(&16));
        assert_eq!(blocking.get("CS165"), Some(&11));
        assert_eq!(blocking.get("CS220"), Some(&2));
    }

    #[test]
    fn computes_complexity_from_delay_and_blocking() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "B");

        let delay = compute_delay(&dag).expect("delay");
        let blocking = compute_blocking(&dag).expect("blocking");
        let complexity = compute_complexity(&delay, &blocking).expect("complexity");

        // A: delay=3, blocking=2, complexity=5
        assert_eq!(complexity.get("A"), Some(&5));
        // B: delay=3, blocking=1, complexity=4
        assert_eq!(complexity.get("B"), Some(&4));
        // C: delay=3, blocking=0, complexity=3
        assert_eq!(complexity.get("C"), Some(&3));
    }

    #[test]
    fn matches_sample_complexity_values() {
        let school = parse_curriculum_csv("samples/correct/Colostate_CSDegree_w_metrics.csv")
            .expect("parse sample curriculum");
        let dag = school.build_dag();

        let delay = compute_delay(&dag).expect("delay");
        let blocking = compute_blocking(&dag).expect("blocking");
        let complexity = compute_complexity(&delay, &blocking).expect("complexity");

        // MATH156: delay=3, blocking=6, complexity=9
        assert_eq!(complexity.get("MATH156"), Some(&9));
        // CS150B: delay=6, blocking=16, complexity=22
        assert_eq!(complexity.get("CS150B"), Some(&22));
        // CS165: delay=6, blocking=11, complexity=17
        assert_eq!(complexity.get("CS165"), Some(&17));
        // CS220: delay=3, blocking=2, complexity=5
        assert_eq!(complexity.get("CS220"), Some(&5));
    }

    #[test]
    fn computes_centrality_simple_chain() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "B");

        let centrality = compute_centrality(&dag).expect("centrality");

        // One path A->B->C of length 3
        // A: source, centrality=0
        assert_eq!(centrality.get("A"), Some(&0));
        // B: intermediate node in path A->B->C (length 3)
        assert_eq!(centrality.get("B"), Some(&3));
        // C: sink, centrality=0
        assert_eq!(centrality.get("C"), Some(&0));
    }

    #[test]
    fn computes_centrality_with_fork() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "A");
        dag.add_prerequisite("D".to_string(), "B");

        let centrality = compute_centrality(&dag).expect("centrality");

        // Paths: A->B->D (length 3), A->C (length 2)
        // A: source, centrality=0
        assert_eq!(centrality.get("A"), Some(&0));
        // B: intermediate in path A->B->D (length 3)
        assert_eq!(centrality.get("B"), Some(&3));
        // C: sink (in path A->C), centrality=0
        assert_eq!(centrality.get("C"), Some(&0));
        // D: sink, centrality=0
        assert_eq!(centrality.get("D"), Some(&0));
    }

    #[test]
    fn matches_sample_centrality_values() {
        let school = parse_curriculum_csv("samples/correct/Colostate_CSDegree_w_metrics.csv")
            .expect("parse sample curriculum");
        let dag = school.build_dag();
        let centrality = compute_centrality(&dag).expect("centrality");

        // Sources and sinks should have centrality 0
        assert_eq!(centrality.get("MATH156"), Some(&0));
        assert_eq!(centrality.get("CS150B"), Some(&0));
        assert_eq!(centrality.get("CO150"), Some(&0));

        // Intermediate courses should have non-zero centrality
        assert_eq!(centrality.get("CS164"), Some(&44));
        assert_eq!(centrality.get("CS220"), Some(&12));
    }

    #[test]
    fn compute_all_metrics_combines_all_metrics() {
        let mut dag = DAG::new();
        dag.add_prerequisite("B".to_string(), "A");
        dag.add_prerequisite("C".to_string(), "B");

        let all_metrics = compute_all_metrics(&dag).expect("all metrics");

        // Check A
        let a_metrics = all_metrics.get("A").expect("A metrics");
        assert_eq!(a_metrics.delay, 3);
        assert_eq!(a_metrics.blocking, 2);
        assert_eq!(a_metrics.complexity, 5);
        assert_eq!(a_metrics.centrality, 0);

        // Check B
        let b_metrics = all_metrics.get("B").expect("B metrics");
        assert_eq!(b_metrics.delay, 3);
        assert_eq!(b_metrics.blocking, 1);
        assert_eq!(b_metrics.complexity, 4);
        assert_eq!(b_metrics.centrality, 3);

        // Check C
        let c_metrics = all_metrics.get("C").expect("C metrics");
        assert_eq!(c_metrics.delay, 3);
        assert_eq!(c_metrics.blocking, 0);
        assert_eq!(c_metrics.complexity, 3);
        assert_eq!(c_metrics.centrality, 0);
    }
}
