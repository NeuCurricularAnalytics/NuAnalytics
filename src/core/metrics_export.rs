//! Export metrics to various formats

use super::metrics::CurriculumMetrics;
use super::models::DAG;
use crate::core::models::{Plan, School};
use std::collections::HashMap;
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
    pub fn from_metrics(plan: &Plan, school: &School, metrics: &CurriculumMetrics) -> Self {
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

        // Compute the longest path through the curriculum
        let dag = school.build_dag();
        let longest_delay_path = compute_longest_path(&dag, metrics);

        Self {
            total_complexity,
            highest_centrality,
            highest_centrality_course,
            longest_delay,
            longest_delay_course,
            longest_delay_path,
        }
    }
}

/// Compute the longest path through the curriculum DAG
fn compute_longest_path(dag: &DAG, metrics: &CurriculumMetrics) -> Vec<String> {
    // Build a map of course -> (delay_value, predecessor)
    let mut delay_info: HashMap<String, (usize, Option<String>)> = HashMap::new();

    // Initialize all courses with delay 0 and no predecessor
    for course in &dag.courses {
        if let Some(m) = metrics.get(course) {
            delay_info.insert(course.clone(), (m.delay, None));
        }
    }

    // Find the course with maximum delay
    let max_delay_course = delay_info
        .iter()
        .max_by_key(|(_, (delay, _))| delay)
        .map(|(course, _)| course.clone());

    max_delay_course.map_or_else(Vec::new, |end_course| {
        // Trace back the path using prerequisites
        let mut path = vec![end_course.clone()];
        let mut current = end_course;

        // Trace back through prerequisites
        while let Some(prereqs) = dag.get_prerequisites(&current) {
            if prereqs.is_empty() {
                break;
            }

            // Find the prerequisite with the highest delay
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
    })
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
        let summary = CurriculumSummary::from_metrics(plan, school, metrics);
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

    // Try to find the degree to get degree type
    let degree_info = school
        .degrees
        .iter()
        .find(|d| d.id() == plan.degree_id)
        .map_or_else(
            || ("BS".to_string(), String::new()),
            |d| (d.degree_type.clone(), d.cip_code.clone()),
        );

    let institution = plan.institution.as_deref().unwrap_or(&school.name);

    // Write header section with summary statistics - one item per row
    // Row 1: Curriculum name
    writeln!(file, "Curriculum,{}", plan.name)?;

    // Row 2: Institution
    writeln!(file, "Institution,{institution}")?;

    // Row 3: Degree Type
    writeln!(file, "Degree Type,\"{}\"", degree_info.0)?;

    // Row 4: System Type (hardcoded as semester for now)
    writeln!(file, "System Type,semester")?;

    // Row 5: CIP code
    writeln!(file, "CIP,\"{}\"", degree_info.1)?;

    // Row 6: Total Structural Complexity
    writeln!(
        file,
        "Total Structural Complexity,{}",
        summary.total_complexity
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
    let mut course_id = 1;
    for course_key in &plan.courses {
        if let Some(course) = school.get_course(course_key) {
            let metrics_data = metrics.get(course_key);

            let prereqs = course.prerequisites.join(";");
            let coreqs = course.corequisites.join(";");

            let (complexity, blocking, delay, centrality) = metrics_data
                .map_or((0, 0, 0, 0), |m| {
                    (m.complexity, m.blocking, m.delay, m.centrality)
                });

            writeln!(
                file,
                "{},{},\"{}\",\"{}\",\"{}\",\"{}\",\"\",{},\"{}\",\"{}\",{},{},{},{}",
                course_id,
                course.name,
                course.prefix,
                course.number,
                prereqs,
                coreqs,
                course.credit_hours,
                institution,
                course.canonical_name.as_deref().unwrap_or(""),
                complexity,
                blocking,
                delay,
                centrality
            )?;

            course_id += 1;
        }
    }

    Ok(())
}

/// Convenience function to export metrics using the default CSV exporter
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_metrics_csv<P: AsRef<Path>>(
    school: &School,
    plan: &Plan,
    metrics: &CurriculumMetrics,
    output_path: P,
) -> Result<(), Box<dyn Error>> {
    CsvExporter.export(school, plan, metrics, output_path.as_ref())
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
        export_metrics_csv(&school, &plan, &metrics_data, output_path).expect("export metrics");

        let contents = fs::read_to_string(output_path).expect("read file");
        assert!(contents.contains("Curriculum"));
        assert!(contents.contains("Course ID,Course Name"));
        assert!(contents.contains("Complexity,Blocking,Delay,Centrality"));
        assert!(contents.contains("Structural Complexity"));
        assert!(contents.contains("Longest Delay"));
        assert!(contents.contains("Highest Centrality Course"));

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
}
