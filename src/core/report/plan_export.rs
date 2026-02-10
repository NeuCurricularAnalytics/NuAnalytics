//! CSV export for selected degree plans
//!
//! Exports selected plans (shortest, longest, calc-ready, random samples)
//! to CSV format compatible with the existing planner export.

use crate::core::degree::plan_selector::{PlanCategory, ScoredPlan, SelectedPlans};
use crate::core::models::{Course, Degree, Plan, School};
use std::error::Error;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;

/// Configuration for plan CSV export
#[derive(Debug, Clone)]
pub struct PlanExportConfig {
    /// Base directory for exports
    pub base_dir: String,
    /// Whether to create parent directories
    pub create_dirs: bool,
}

impl Default for PlanExportConfig {
    fn default() -> Self {
        Self {
            base_dir: "metrics/plans".to_string(),
            create_dirs: true,
        }
    }
}

/// Export selected plans to CSV files
///
/// Creates directory structure: `{base_dir}/{degree_id}/{plan_category}.csv`
///
/// # Arguments
/// * `school` - School containing course catalog
/// * `degree` - Degree being analyzed
/// * `selected` - Selected plans to export
/// * `config` - Export configuration
///
/// # Errors
/// Returns an error if directory creation or file writing fails
pub fn export_selected_plans(
    school: &School,
    degree: &Degree,
    selected: &SelectedPlans,
    config: &PlanExportConfig,
) -> Result<Vec<String>, Box<dyn Error>> {
    let degree_dir = format!(
        "{}/{}",
        config.base_dir,
        sanitize_filename(&degree.degree_id())
    );

    if config.create_dirs {
        fs::create_dir_all(&degree_dir)?;
    }

    let mut exported_files = Vec::new();
    let mut sample_index = 0;

    for (category, plan) in selected.iter() {
        let filename = match category {
            PlanCategory::RandomSample => {
                sample_index += 1;
                format!("{}-{}.csv", category.file_name(), sample_index)
            }
            _ => format!("{}.csv", category.file_name()),
        };

        let path = format!("{degree_dir}/{filename}");
        export_plan_csv(school, degree, plan, &category, &path)?;
        exported_files.push(path);
    }

    Ok(exported_files)
}

/// Export a single plan to CSV format
///
/// # Arguments
/// * `school` - School containing course catalog
/// * `degree` - Degree being analyzed
/// * `plan` - Scored plan to export
/// * `category` - Plan category for metadata
/// * `output_path` - Path to write CSV file
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_plan_csv(
    school: &School,
    degree: &Degree,
    plan: &ScoredPlan,
    category: &PlanCategory,
    output_path: &str,
) -> Result<(), Box<dyn Error>> {
    let mut file = File::create(output_path)?;

    let institution = &school.name;
    let degree_type = &degree.degree_type;
    let cip_code = degree.cip_code.as_deref().unwrap_or("");
    let system_type = &degree.system_type;
    let scale_factor = degree.complexity_scale_factor();

    // Header section
    writeln!(
        file,
        "Curriculum,{} - {}",
        degree.name,
        category.display_name()
    )?;
    writeln!(file, "Institution,{institution}")?;
    writeln!(file, "Degree Type,\"{degree_type}\"")?;
    writeln!(file, "System Type,{system_type}")?;
    writeln!(file, "CIP,\"{cip_code}\"")?;

    // Plan-specific summary
    #[allow(clippy::cast_precision_loss)]
    let scaled_complexity = plan.score.total_complexity as f64 * scale_factor;
    writeln!(file, "Total Structural Complexity,{scaled_complexity:.1}")?;
    writeln!(file, "Longest Delay,{}", plan.score.longest_delay)?;
    writeln!(file, "Terms Required,{}", plan.score.terms_required)?;

    // Courses section
    writeln!(file, "Courses")?;
    writeln!(
        file,
        "Course ID,Course Name,Prefix,Number,Prerequisites,Corequisites,Strict-Corequisites,Credit Hours,Institution,Canonical Name,Complexity,Blocking,Delay,Centrality"
    )?;

    // Sort courses by term, then alphabetically
    let mut courses: Vec<_> = plan.variant.courses.iter().collect();
    courses.sort();

    for (idx, course_key) in courses.iter().enumerate() {
        let course = school.get_course(course_key);
        let metrics = plan.course_metrics.get(*course_key);

        // Determine course type and properties based on key pattern
        let course_info = classify_course_key(course_key, course);

        let canonical = course
            .and_then(|c| c.canonical_name.as_deref())
            .unwrap_or("");

        // Use prerequisites_raw if prerequisites vec is empty (YAML source)
        let prereqs = course
            .map(|c| {
                if c.prerequisites.is_empty() {
                    c.prerequisites_raw.clone().unwrap_or_default()
                } else {
                    c.prerequisites.join(";")
                }
            })
            .unwrap_or_default();
        let coreqs = course.map(|c| c.corequisites.join(";")).unwrap_or_default();
        let strict_coreqs = course
            .map(|c| c.strict_corequisites.join(";"))
            .unwrap_or_default();

        let (complexity, blocking, delay, centrality) = metrics.map_or((0, 0, 0, 0), |m| {
            (m.complexity, m.blocking, m.delay, m.centrality)
        });

        #[allow(clippy::cast_precision_loss)]
        let scaled_complexity = complexity as f64 * scale_factor;

        writeln!(
            file,
            "{},{},\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},\"{}\",\"{}\",{:.1},{},{},{}",
            idx + 1,
            course_info.name,
            course_info.prefix,
            course_info.number,
            prereqs,
            coreqs,
            strict_coreqs,
            course_info.credits,
            institution,
            canonical,
            scaled_complexity,
            blocking,
            delay,
            centrality
        )?;
    }

    Ok(())
}

/// Classification info for a course (real or placeholder)
struct CourseInfo {
    name: String,
    prefix: String,
    number: String,
    credits: f32,
}

/// Classify a course key to determine its type and properties
///
/// Handles:
/// - Real courses from the catalog
/// - ELEC### placeholder electives
/// - Requirement placeholder courses (e.g., WRTC01 for `writing_composition`)
fn classify_course_key(course_key: &str, course: Option<&Course>) -> CourseInfo {
    // Check for ELEC### pattern (free electives)
    if let Some(suffix) = course_key.strip_prefix("ELEC") {
        let is_small = suffix.ends_with('S');
        return CourseInfo {
            name: "Free Elective".to_string(),
            prefix: "ELEC".to_string(),
            number: suffix.to_string(),
            credits: if is_small { 2.0 } else { 3.0 },
        };
    }

    // Check if it's a real course from the catalog
    if let Some(c) = course {
        return CourseInfo {
            name: c.name.clone(),
            prefix: c.prefix.clone(),
            number: c.number.clone(),
            credits: c.credit_hours,
        };
    }

    // Check for requirement placeholder pattern (uppercase letters followed by digits)
    // e.g., WRTC01, AUCC01S, etc.
    let prefix: String = course_key
        .chars()
        .take_while(|c| c.is_alphabetic())
        .collect();
    let number: String = course_key
        .chars()
        .skip_while(|c| c.is_alphabetic())
        .collect();

    if !prefix.is_empty() && !number.is_empty() {
        let is_small = number.ends_with('S');
        let name = humanize_placeholder_prefix(&prefix);
        return CourseInfo {
            name,
            prefix,
            number,
            credits: if is_small { 2.0 } else { 3.0 },
        };
    }

    // Unknown course
    CourseInfo {
        name: "Unknown".to_string(),
        prefix: String::new(),
        number: course_key.to_string(),
        credits: 3.0,
    }
}

/// Convert a placeholder prefix back to a human-readable name
///
/// Maps common prefixes to descriptive names
fn humanize_placeholder_prefix(prefix: &str) -> String {
    match prefix {
        "ELEC" | "FE" => "Free Elective".to_string(),
        "WRTC" | "WC" => "Writing/Composition".to_string(),
        "AUCC" | "AC" => "Gen Ed Citizenship".to_string(),
        "AW" => "Advanced Writing".to_string(),
        "NS" => "Natural Science".to_string(),
        "SB" => "Social/Behavioral Science".to_string(),
        "HP" => "Historical Perspectives".to_string(),
        _ => format!("{prefix} Elective"),
    }
}

/// Export a plan as a Plan model for use with existing reporters
///
/// # Arguments
/// * `scored_plan` - The scored plan to convert
/// * `degree` - Degree for the plan
/// * `category` - Plan category for naming
///
/// # Returns
/// A Plan model compatible with existing report infrastructure
#[must_use]
pub fn scored_plan_to_model(
    scored_plan: &ScoredPlan,
    degree: &Degree,
    category: &PlanCategory,
) -> Plan {
    let name = format!("{} - {}", degree.name, category.display_name());
    let mut plan = Plan::new(name, degree.degree_id());

    for course in &scored_plan.variant.courses {
        plan.add_course(course.clone());
    }

    plan
}

/// Sanitize a string for use as a filename
///
/// Replaces characters that are invalid in filenames with underscores.
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' | ' ' => '_',
            _ => c,
        })
        .collect()
}

/// Export summary statistics to a CSV file
///
/// # Arguments
/// * `degree` - Degree being analyzed
/// * `selected` - Selected plans
/// * `output_path` - Path to write summary CSV
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_summary_csv(
    degree: &Degree,
    selected: &SelectedPlans,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let mut file = File::create(output_path)?;

    writeln!(file, "Degree Analysis Summary")?;
    writeln!(file, "Degree,{}", degree.name)?;
    writeln!(file, "Degree ID,{}", degree.degree_id())?;
    writeln!(file, "Total Plans Analyzed,{}", selected.total_plans_seen)?;
    writeln!(file)?;

    writeln!(file, "Plan Category,Terms,Complexity,Longest Delay")?;

    for (category, plan) in selected.iter() {
        writeln!(
            file,
            "{},{},{},{}",
            category.display_name(),
            plan.score.terms_required,
            plan.score.total_complexity,
            plan.score.longest_delay
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::degree::plan_selector::PlanScore;
    use crate::core::degree::plan_variant::PlanVariant;
    use crate::core::metrics::CourseMetrics;
    use crate::core::models::Course;
    use crate::core::report::term_scheduler::TermPlan;
    use std::collections::HashMap;
    use tempfile::TempDir;

    fn create_test_school() -> School {
        let mut school = School::new("Test University".to_string());
        let course = Course::new(
            "Intro to CS".to_string(),
            "CS".to_string(),
            "1000".to_string(),
            3.0,
        );
        school.add_course(course);
        school
    }

    fn create_test_degree() -> Degree {
        Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            Some("11.0701".to_string()),
            "semester".to_string(),
        )
    }

    fn create_test_plan() -> ScoredPlan {
        let mut metrics = HashMap::new();
        metrics.insert(
            "CS1000".to_string(),
            CourseMetrics {
                complexity: 10,
                centrality: 5,
                delay: 3,
                blocking: 2,
            },
        );

        ScoredPlan {
            variant: PlanVariant::from_parts(vec!["CS1000".to_string()], HashMap::new(), 3.0),
            score: PlanScore {
                terms_required: 8,
                total_complexity: 100,
                longest_delay: 5,
                is_calc_ready: false,
            },
            schedule: TermPlan::new(8, false, 15.0),
            course_metrics: metrics,
        }
    }

    fn create_test_selected() -> SelectedPlans {
        SelectedPlans {
            shortest: Some(create_test_plan()),
            longest: None,
            calc_ready_shortest: None,
            random_samples: vec![],
            total_plans_seen: 10,
        }
    }

    #[test]
    fn test_sanitize_filename() {
        assert_eq!(sanitize_filename("CS BS 2024"), "CS_BS_2024");
        assert_eq!(sanitize_filename("a/b\\c:d"), "a_b_c_d");
    }

    #[test]
    fn test_export_plan_csv() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.csv");

        let school = create_test_school();
        let degree = create_test_degree();
        let plan = create_test_plan();

        let result = export_plan_csv(
            &school,
            &degree,
            &plan,
            &PlanCategory::Shortest,
            path.to_str().unwrap(),
        );
        assert!(result.is_ok());

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Curriculum"));
        assert!(contents.contains("Shortest Path"));
        assert!(contents.contains("Total Structural Complexity"));
    }

    #[test]
    fn test_export_selected_plans() {
        let tmp = TempDir::new().unwrap();
        let config = PlanExportConfig {
            base_dir: tmp.path().to_str().unwrap().to_string(),
            create_dirs: true,
        };

        let school = create_test_school();
        let degree = create_test_degree();
        let selected = create_test_selected();

        let result = export_selected_plans(&school, &degree, &selected, &config);
        assert!(result.is_ok());

        let files = result.unwrap();
        assert_eq!(files.len(), 1); // Only shortest plan
        assert!(Path::new(&files[0]).exists());
    }

    #[test]
    fn test_scored_plan_to_model() {
        let degree = create_test_degree();
        let scored = create_test_plan();

        let plan = scored_plan_to_model(&scored, &degree, &PlanCategory::Shortest);

        assert!(plan.name.contains("Shortest Path"));
        assert!(plan.courses.contains(&"CS1000".to_string()));
    }

    #[test]
    fn test_export_summary_csv() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("summary.csv");

        let degree = create_test_degree();
        let selected = create_test_selected();

        let result = export_summary_csv(&degree, &selected, &path);
        assert!(result.is_ok());

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("Degree Analysis Summary"));
        assert!(contents.contains("Total Plans Analyzed"));
    }

    #[test]
    fn test_default_config() {
        let config = PlanExportConfig::default();
        assert!(config.base_dir.contains("metrics/plans"));
        assert!(config.create_dirs);
    }
}
