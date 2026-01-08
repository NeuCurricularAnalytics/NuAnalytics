//! Export metrics to various formats

use super::metrics::CurriculumMetrics;
use super::models::DAG;
use crate::core::models::{Course, Degree, Plan, School};
use std::error::Error;
use std::path::Path;

/// Trait for exporting curriculum metrics in different formats
pub trait MetricsExporter {
    /// Export metrics for a curriculum plan
    ///
    /// # Errors
    /// Returns an error if export fails
    fn export(
        &self,
        school: &School,
        plan: &Plan,
        metrics: &CurriculumMetrics,
        output_path: &Path,
    ) -> Result<(), Box<dyn Error>>;
}

/// Summary statistics for a curriculum
#[derive(Debug, Clone)]
pub struct CurriculumSummary {
    /// Total structural complexity (sum of all course complexities)
    pub total_complexity: usize,
    /// Highest centrality value
    pub highest_centrality: usize,
    /// Course with highest centrality
    pub highest_centrality_course: String,
    /// Longest delay value
    pub longest_delay: usize,
    /// Course with longest delay
    pub longest_delay_course: String,
    /// Path of courses that make up the longest delay
    pub longest_delay_path: Vec<String>,
}

impl CurriculumSummary {
    /// Compute summary statistics from curriculum metrics
    #[must_use]
    pub fn from_metrics(plan: &Plan, _school: &School, metrics: &CurriculumMetrics) -> Self {
        let mut total_complexity = 0;
        let mut highest_centrality = 0;
        let mut highest_centrality_course = String::new();
        let mut longest_delay = 0;
        let mut longest_delay_course = String::new();

        for course_key in &plan.courses {
            if let Some(m) = metrics.get(course_key) {
                total_complexity += m.complexity;

                if m.centrality > highest_centrality {
                    highest_centrality = m.centrality;
                    highest_centrality_course.clone_from(course_key);
                }

                if m.delay > longest_delay {
                    longest_delay = m.delay;
                    longest_delay_course.clone_from(course_key);
                }
            }
        }

        Self {
            total_complexity,
            highest_centrality,
            highest_centrality_course,
            longest_delay,
            longest_delay_course,
            longest_delay_path: Vec::new(), // Will be computed separately when DAG is available
        }
    }

    /// Set the longest delay path from a precomputed DAG
    #[must_use]
    pub fn with_delay_path(mut self, dag: &DAG, metrics: &CurriculumMetrics) -> Self {
        self.longest_delay_path = compute_longest_path(dag, metrics);
        self
    }
}

/// Compute the longest path through the curriculum DAG by tracing back prerequisites
///
/// Finds the course with the maximum delay value, then traces back through its
/// prerequisites by following the chain of courses with the highest delay values.
/// This represents the critical path through the curriculum.
///
/// Corequisites are included in each step of the path using `+` notation.
/// For example: `(CS1800+CS1802)->(CS2500+CS2501)->CS3500`
///
/// # Arguments
/// * `dag` - The directed acyclic graph of course prerequisites
/// * `metrics` - Computed metrics for all courses
///
/// # Returns
/// A vector of course keys representing the path from start to end.
/// Each element may contain multiple courses joined by `+` for corequisites.
fn compute_longest_path(dag: &DAG, metrics: &CurriculumMetrics) -> Vec<String> {
    // Find all courses with the maximum delay value
    let max_delay = metrics.values().map(|m| m.delay).max().unwrap_or(0);

    if max_delay == 0 {
        return Vec::new();
    }

    // Among courses with max delay, find the one that's furthest down the dependency chain
    // (i.e., has the most prerequisites to trace back through)
    let max_delay_courses: Vec<_> = metrics
        .iter()
        .filter(|(_, m)| m.delay == max_delay)
        .map(|(course, _)| course)
        .collect();

    let mut longest_path = Vec::new();

    // Try each course with max delay and find which gives the longest traceback path
    for &end_course in &max_delay_courses {
        let path = trace_prerequisites(end_course, dag, metrics);
        if path.len() > longest_path.len() {
            longest_path = path;
        }
    }

    // Now expand each step to include corequisites
    expand_path_with_corequisites(&longest_path, dag)
}

/// Expand a path to include corequisites for each course
///
/// Takes a simple path like `[A, B, C]` and expands it to include corequisites,
/// resulting in something like `[(A+A_coreq), (B+B_coreq), C]` where courses
/// with corequisites are grouped together.
fn expand_path_with_corequisites(path: &[String], dag: &DAG) -> Vec<String> {
    let mut expanded = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

    for course in path {
        if seen.contains(course) {
            continue; // Skip if already included as a corequisite
        }

        // Get corequisites for this course (both regular and strict)
        let mut group = vec![course.clone()];
        seen.insert(course.clone());

        if let Some(coreqs) = dag.get_corequisites(course) {
            for coreq in coreqs {
                if !seen.contains(coreq) && dag.courses.contains(coreq) {
                    group.push(coreq.clone());
                    seen.insert(coreq.clone());
                }
            }
        }

        // Also check if this course is listed as a corequisite by another course on the path
        if let Some(coreq_parents) = dag.get_coreq_dependents(course) {
            for parent in coreq_parents {
                if path.contains(parent) && !seen.contains(parent) {
                    // This course is a coreq of another course on the path
                    // They should be grouped together
                    group.push(parent.clone());
                    seen.insert(parent.clone());
                }
            }
        }

        // Sort for consistent output (main course first if possible)
        group.sort();
        // But keep the original course first
        if let Some(pos) = group.iter().position(|c| c == course) {
            group.swap(0, pos);
        }

        if group.len() > 1 {
            expanded.push(format!("({})", group.join("+")));
        } else {
            expanded.push(course.clone());
        }
    }

    expanded
}

/// Trace back through prerequisites to build a path.
///
/// Starting from a course, recursively follows the prerequisite chain by selecting
/// the prerequisite with the highest delay value at each step. This creates a
/// "critical path" through the curriculum prerequisites, representing the longest
/// sequence of requirements leading to the start course.
///
/// # Arguments
/// * `start` - The course to start tracing from (typically has highest delay)
/// * `dag` - The DAG containing prerequisite relationships
/// * `metrics` - Course metrics including delay values (used to select best path)
///
/// # Returns
/// A vector of courses from leaf prerequisite to start course (in reverse topological order).
/// The first element is typically a root course (no prerequisites), and the last element
/// is the start course.
fn trace_prerequisites(start: &str, dag: &DAG, metrics: &CurriculumMetrics) -> Vec<String> {
    let mut path = vec![start.to_string()];
    let mut current = start.to_string();

    // Trace back through prerequisites by greedily selecting the prerequisite
    // with the highest delay at each step. This ensures we follow the longest chain.
    while let Some(prereqs) = dag.get_prerequisites(&current) {
        if prereqs.is_empty() {
            break;
        }

        // Find the prerequisite with the highest delay value
        let best_prereq = prereqs
            .iter()
            .max_by_key(|p| metrics.get(*p).map_or(0, |m| m.delay));

        if let Some(prereq) = best_prereq {
            path.push(prereq.clone());
            current = prereq.clone();
        } else {
            break;
        }
    }

    // Reverse to get the path from start to end
    path.reverse();
    path
}

/// CSV exporter for curriculum metrics
pub struct CsvExporter;

impl MetricsExporter for CsvExporter {
    fn export(
        &self,
        school: &School,
        plan: &Plan,
        metrics: &CurriculumMetrics,
        output_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        let dag = school.build_dag();
        let summary =
            CurriculumSummary::from_metrics(plan, school, metrics).with_delay_path(&dag, metrics);
        export_metrics_csv_with_summary(school, plan, metrics, &summary, output_path)
    }
}

/// Export curriculum metrics to CSV format with summary statistics
///
/// # Arguments
/// * `school` - The school with courses and degrees
/// * `plan` - The plan to export metrics for
/// * `metrics` - The computed metrics for all courses
/// * `summary` - Summary statistics
/// * `output_path` - Path to write the CSV file to
///
/// # Errors
/// Returns an error if file writing fails
#[allow(clippy::too_many_lines)]
pub fn export_metrics_csv_with_summary(
    school: &School,
    plan: &Plan,
    metrics: &CurriculumMetrics,
    summary: &CurriculumSummary,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    use std::fs::File;
    use std::io::Write;

    let mut file = File::create(output_path)?;

    // Try to find the degree to get degree type and system type
    let degree = school.degrees.iter().find(|d| d.id() == plan.degree_id);

    let degree_type = degree.map_or_else(|| "BS".to_string(), |d| d.degree_type.clone());
    let cip_code = degree.map_or_else(String::new, |d| d.cip_code.clone());
    let system_type = degree.map_or_else(|| "semester".to_string(), |d| d.system_type.clone());
    let scale_factor = degree.map_or(1.0, Degree::complexity_scale_factor);

    let institution = plan.institution.as_deref().unwrap_or(&school.name);

    // Pre-compute scaled complexity for each course to get accurate total
    // (scaling each course individually and rounding matches reference tool behavior)
    let mut courses_by_csv_id: Vec<(String, String, &Course)> = plan
        .courses
        .iter()
        .filter_map(|storage_key| {
            school.get_course(storage_key).map(|course| {
                (
                    course.csv_id.clone().unwrap_or_else(|| "0".to_string()),
                    storage_key.clone(),
                    course,
                )
            })
        })
        .collect();

    // Sort by CSV ID (numerically if possible)
    courses_by_csv_id.sort_by(|a, b| {
        let a_num = a.0.parse::<usize>().unwrap_or(0);
        let b_num = b.0.parse::<usize>().unwrap_or(0);
        a_num.cmp(&b_num)
    });

    // Compute scaled complexity for each course, then sum for total
    // This matches the reference tool which rounds per-course before summing
    #[allow(clippy::cast_precision_loss)]
    let scaled_total_complexity: f64 = courses_by_csv_id
        .iter()
        .map(|(_, storage_key, _)| {
            let complexity = metrics.get(storage_key).map_or(0, |m| m.complexity);
            // Round to 1 decimal place per course (matches reference tool)
            ((complexity as f64 * scale_factor) * 10.0).round() / 10.0
        })
        .sum();

    // Write header section with summary statistics - one item per row
    // Row 1: Curriculum name
    writeln!(file, "Curriculum,{}", plan.name)?;

    // Row 2: Institution
    writeln!(file, "Institution,{institution}")?;

    // Row 3: Degree Type
    writeln!(file, "Degree Type,\"{degree_type}\"")?;

    // Row 4: System Type
    writeln!(file, "System Type,{system_type}")?;

    // Row 5: CIP code
    writeln!(file, "CIP,\"{cip_code}\"")?;

    // Row 6: Total Structural Complexity (sum of scaled per-course values)
    writeln!(
        file,
        "Total Structural Complexity,{scaled_total_complexity:.1}"
    )?;

    // Row 7: Longest Delay with path
    write!(file, "Longest Delay,{}", summary.longest_delay)?;
    if !summary.longest_delay_path.is_empty() {
        write!(file, ",{}", summary.longest_delay_path.join("->"))?;
    }
    writeln!(file)?;

    // Row 8: Highest Centrality Course
    writeln!(
        file,
        "Highest Centrality Course,\"{}\",{}",
        summary.highest_centrality_course, summary.highest_centrality
    )?;

    // Write courses section
    writeln!(file, "Courses")?;
    writeln!(
        file,
        "Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name,Complexity,Blocking,Delay,Centrality"
    )?;

    // Write course data
    logger::debug!("Exporting {} courses from plan", plan.courses.len());

    for (csv_id, storage_key, course) in &courses_by_csv_id {
        logger::debug!(
            "Exporting course {} ({}) - storage key: {}",
            csv_id,
            course.name,
            storage_key
        );
        let metrics_data = metrics.get(storage_key);

        // Convert stored keys back to CSV IDs for export
        let prereqs = format_course_keys_as_csv(course.prerequisites.iter(), school);
        let coreqs = format_course_keys_as_csv(course.corequisites.iter(), school);
        let strict_coreqs = format_course_keys_as_csv(course.strict_corequisites.iter(), school);

        let (complexity, blocking, delay, centrality) = metrics_data.map_or((0, 0, 0, 0), |m| {
            (m.complexity, m.blocking, m.delay, m.centrality)
        });

        // Scale complexity for quarter systems
        #[allow(clippy::cast_precision_loss)]
        let scaled_complexity = (complexity as f64) * scale_factor;

        writeln!(
            file,
            "{},{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\",{:.1},{},{},{}",
            csv_id,
            course.name,
            course.prefix,
            course.number,
            prereqs,
            coreqs,
            strict_coreqs,
            course.credit_hours,
            institution,
            course.canonical_name.as_deref().unwrap_or(""),
            scaled_complexity,
            blocking,
            delay,
            centrality
        )?;
    }

    Ok(())
}

/// Convenience function to export metrics using the default CSV exporter
///
/// Returns the computed summary statistics for further use
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_metrics_csv<P: AsRef<Path>>(
    school: &School,
    plan: &Plan,
    metrics: &CurriculumMetrics,
    output_path: P,
) -> Result<CurriculumSummary, Box<dyn Error>> {
    let dag = school.build_dag();
    let summary =
        CurriculumSummary::from_metrics(plan, school, metrics).with_delay_path(&dag, metrics);
    export_metrics_csv_with_summary(school, plan, metrics, &summary, output_path.as_ref())?;
    Ok(summary)
}

/// Format course storage keys as CSV with semicolons.
///
/// # Arguments
/// * `keys` - Iterator over course storage keys (references)
/// * `school` - School containing course lookup table
///
/// # Returns
/// Semicolon-separated string of CSV IDs or storage keys
fn format_course_keys_as_csv<'a>(
    keys: impl Iterator<Item = &'a String>,
    school: &School,
) -> String {
    keys.map(|k| {
        school
            .get_course(k)
            .and_then(|c| c.csv_id.clone())
            .unwrap_or_else(|| k.clone())
    })
    .collect::<Vec<_>>()
    .join(";")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metrics;
    use crate::core::planner::parse_curriculum_csv;
    use std::fs;

    #[test]
    fn exports_metrics_csv() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let plan = school.plans.first().expect("has at least one plan").clone();
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let output_path = "/tmp/test_metrics_export.csv";
        let summary =
            export_metrics_csv(&school, &plan, &metrics_data, output_path).expect("export metrics");

        let contents = fs::read_to_string(output_path).expect("read file");
        assert!(contents.contains("Curriculum"));
        assert!(contents.contains("Course ID,Course Name"));
        assert!(contents.contains("Complexity,Blocking,Delay,Centrality"));
        assert!(contents.contains("Structural Complexity"));
        assert!(contents.contains("Longest Delay"));
        assert!(contents.contains("Highest Centrality Course"));

        // Verify summary was computed
        assert!(summary.total_complexity > 0);
        assert!(summary.longest_delay > 0);

        fs::remove_file(output_path).ok();
    }

    #[test]
    fn computes_curriculum_summary() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let plan = school.plans.first().expect("has at least one plan").clone();
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let summary = CurriculumSummary::from_metrics(&plan, &school, &metrics_data);

        assert!(summary.total_complexity > 0);
        assert!(summary.longest_delay > 0);
        assert!(!summary.highest_centrality_course.is_empty());
        assert!(!summary.longest_delay_course.is_empty());
    }

    #[test]
    fn csv_exporter_trait_works() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let plan = school.plans.first().expect("has at least one plan").clone();
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let output_path = "/tmp/test_exporter_trait.csv";
        let exporter = CsvExporter;
        exporter
            .export(
                &school,
                &plan,
                &metrics_data,
                std::path::Path::new(output_path),
            )
            .expect("export metrics");

        assert!(std::path::Path::new(output_path).exists());
        fs::remove_file(output_path).ok();
    }

    #[test]
    fn computes_longest_delay_path() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let path = compute_longest_path(&dag, &metrics_data);

        // Should have at least one course in the path
        assert!(!path.is_empty(), "Longest path should not be empty");

        // Path should be ordered from prerequisite to dependent
        if path.len() > 1 {
            for i in 0..path.len() - 1 {
                let current = &path[i];
                let next = &path[i + 1];

                // Verify that current is a prerequisite of next
                let prereqs = dag.get_prerequisites(next);
                assert!(
                    prereqs.is_some_and(|deps| deps.contains(current)),
                    "Course {current} should be a prerequisite of {next}"
                );
            }
        }
    }

    #[test]
    fn summary_with_delay_path_includes_path() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let plan = school.plans.first().expect("has at least one plan").clone();
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let summary = CurriculumSummary::from_metrics(&plan, &school, &metrics_data)
            .with_delay_path(&dag, &metrics_data);

        // Path should be populated
        assert!(
            !summary.longest_delay_path.is_empty(),
            "Delay path should be computed"
        );

        // Path should start and end with actual courses
        for course in &summary.longest_delay_path {
            assert!(
                dag.contains_course(course),
                "Path should only contain valid courses"
            );
        }
    }

    #[test]
    fn csv_contains_delay_path() {
        let school =
            parse_curriculum_csv("samples/plans/Colostate_CSDegree.csv").expect("parse curriculum");
        let plan = school.plans.first().expect("has at least one plan").clone();
        let dag = school.build_dag();
        let metrics_data = metrics::compute_all_metrics(&dag).expect("compute metrics");

        let output_path = "/tmp/test_delay_path.csv";
        export_metrics_csv(&school, &plan, &metrics_data, output_path).expect("export metrics");

        let contents = fs::read_to_string(output_path).expect("read file");

        // Check that the CSV contains the path separator
        assert!(
            contents.contains("->"),
            "CSV should contain delay path with -> separator"
        );

        // Find the "Longest Delay" line
        let delay_line = contents
            .lines()
            .find(|line| line.starts_with("Longest Delay"))
            .expect("Should have Longest Delay line");

        // Should have at least 3 fields: label, value, and path
        assert!(
            delay_line.split(',').count() >= 3,
            "Longest Delay line should include the path"
        );

        fs::remove_file(output_path).ok();
    }
}
