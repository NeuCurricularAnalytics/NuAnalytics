//! Mermaid diagram generator for curriculum graphs
//!
//! Generates Mermaid flowchart syntax that can be embedded in Markdown files
//! and rendered by GitHub, GitLab, and other Markdown viewers.

use crate::core::metrics::CurriculumMetrics;
use crate::core::models::{School, DAG};
use crate::core::report::term_scheduler::TermPlan;
use std::fmt::Write;

/// Generator for Mermaid diagram syntax
pub struct MermaidGenerator;

impl MermaidGenerator {
    /// Generate a Mermaid flowchart from a DAG
    ///
    /// Creates a left-to-right flowchart showing prerequisite relationships.
    /// Each node displays the course name and complexity metric.
    #[must_use]
    pub fn generate_dag(dag: &DAG, school: &School, metrics: &CurriculumMetrics) -> String {
        let mut output = String::from("```mermaid\nflowchart LR\n");

        // Define nodes with their complexity values
        for course_key in &dag.courses {
            let label = Self::get_node_label(course_key, school, metrics);
            let safe_id = Self::sanitize_id(course_key);
            let _ = writeln!(output, "    {safe_id}[\"{label}\"]");
        }

        output.push('\n');

        // Add prerequisite edges
        for (course, prereqs) in &dag.dependencies {
            let course_id = Self::sanitize_id(course);
            for prereq in prereqs {
                let prereq_id = Self::sanitize_id(prereq);
                let _ = writeln!(output, "    {prereq_id} --> {course_id}");
            }
        }

        // Add corequisite edges (dashed)
        for (course, coreqs) in &dag.corequisites {
            let course_id = Self::sanitize_id(course);
            for coreq in coreqs {
                let coreq_id = Self::sanitize_id(coreq);
                let _ = writeln!(output, "    {coreq_id} -.-> {course_id}");
            }
        }

        output.push_str("```\n");
        output
    }

    /// Generate a term-organized diagram showing courses grouped by term
    ///
    /// Creates a flowchart with subgraphs for each term, showing course
    /// placement and prerequisite/corequisite relationships.
    #[must_use]
    pub fn generate_term_diagram(
        term_plan: &TermPlan,
        dag: &DAG,
        school: &School,
        metrics: &CurriculumMetrics,
    ) -> String {
        let mut output = String::from("```mermaid\nflowchart LR\n");
        let term_label = term_plan.term_label();

        // Create subgraphs for each term
        for term in &term_plan.terms {
            if term.courses.is_empty() {
                continue;
            }

            let subgraph_id = format!("term{}", term.number);
            let subgraph_label = format!("{term_label} {}", term.number);
            let _ = writeln!(output, "    subgraph {subgraph_id}[\"{subgraph_label}\"]");

            for course_key in &term.courses {
                let label = Self::get_node_label(course_key, school, metrics);
                let safe_id = Self::sanitize_id(course_key);
                let _ = writeln!(output, "        {safe_id}[\"{label}\"]");
            }

            output.push_str("    end\n\n");
        }

        // Add prerequisite edges between terms
        let all_scheduled: std::collections::HashSet<_> = term_plan
            .terms
            .iter()
            .flat_map(|t| t.courses.iter())
            .collect();

        for (course, prereqs) in &dag.dependencies {
            if !all_scheduled.contains(course) {
                continue;
            }
            let course_id = Self::sanitize_id(course);
            for prereq in prereqs {
                if !all_scheduled.contains(prereq) {
                    continue;
                }
                let prereq_id = Self::sanitize_id(prereq);
                let _ = writeln!(output, "    {prereq_id} --> {course_id}");
            }
        }

        // Add corequisite edges (dashed)
        for (course, coreqs) in &dag.corequisites {
            if !all_scheduled.contains(course) {
                continue;
            }
            let course_id = Self::sanitize_id(course);
            for coreq in coreqs {
                if !all_scheduled.contains(coreq) {
                    continue;
                }
                let coreq_id = Self::sanitize_id(coreq);
                let _ = writeln!(output, "    {coreq_id} -.-> {course_id}");
            }
        }

        output.push_str("```\n");
        output
    }

    /// Get a display label for a course node
    fn get_node_label(course_key: &str, school: &School, metrics: &CurriculumMetrics) -> String {
        let course_name = school.get_course(course_key).map_or_else(
            || course_key.to_string(),
            |c| {
                // Truncate long names
                if c.name.len() > 20 {
                    format!("{}...", &c.name[..17])
                } else {
                    c.name.clone()
                }
            },
        );

        let complexity = metrics.get(course_key).map_or(0, |m| m.complexity);

        format!("{course_key}<br/>{course_name}<br/>C:{complexity}")
    }

    /// Sanitize a course key for use as a Mermaid node ID
    fn sanitize_id(key: &str) -> String {
        key.chars()
            .map(|c| if c.is_alphanumeric() { c } else { '_' })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metrics::CourseMetrics;
    use crate::core::models::Course;

    #[test]
    fn test_mermaid_generation() {
        let mut school = School::new("Test".to_string());
        school.add_course(Course::new(
            "Intro".to_string(),
            "CS".to_string(),
            "101".to_string(),
            3.0,
        ));
        school.add_course(Course::new(
            "Data Structures".to_string(),
            "CS".to_string(),
            "201".to_string(),
            3.0,
        ));

        let mut dag = DAG::new();
        dag.add_course("CS101".to_string());
        dag.add_course("CS201".to_string());
        dag.add_prerequisite("CS201".to_string(), "CS101");

        let mut metrics = CurriculumMetrics::new();
        metrics.insert(
            "CS101".to_string(),
            CourseMetrics {
                delay: 1,
                blocking: 1,
                complexity: 2,
                centrality: 1,
            },
        );
        metrics.insert(
            "CS201".to_string(),
            CourseMetrics {
                delay: 2,
                blocking: 0,
                complexity: 2,
                centrality: 1,
            },
        );

        let diagram = MermaidGenerator::generate_dag(&dag, &school, &metrics);

        assert!(diagram.contains("```mermaid"));
        assert!(diagram.contains("flowchart LR"));
        assert!(diagram.contains("CS101"));
        assert!(diagram.contains("CS201"));
        assert!(diagram.contains("-->"));
    }

    #[test]
    fn test_sanitize_id() {
        assert_eq!(MermaidGenerator::sanitize_id("CS 101"), "CS_101");
        assert_eq!(MermaidGenerator::sanitize_id("MATH-1341"), "MATH_1341");
    }
}
