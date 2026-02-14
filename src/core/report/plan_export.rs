//! CSV export for selected degree plans
//!
//! Exports selected plans (shortest, longest, calc-ready, random samples)
//! to CSV format compatible with the existing planner export.
//!
//! Also provides JSONL (JSON Lines) and index CSV formats for aggregating
//! multiple degree analyses.

use crate::core::degree::plan_selector::{PlanCategory, ScoredPlan, SelectedPlans};
use crate::core::models::{Course, Degree, Plan, School};
use crate::core::prerequisite_parser::parse_to_dnf;
use crate::core::statistics::aggregator::{AggregatedDegreeStats, MetricStats, MetricsAggregator};
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

    // Output longest delay chain if available
    if !plan.score.longest_delay_chain.is_empty() {
        let chain_str = plan.score.longest_delay_chain.join(" → ");
        writeln!(file, "Longest Delay Chain,\"{chain_str}\"")?;
    }

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

    // Build course key to row ID mapping (1-indexed for CSV output)
    let course_to_row: std::collections::HashMap<&str, usize> = courses
        .iter()
        .enumerate()
        .map(|(idx, key)| (key.as_str(), idx + 1))
        .collect();

    // Build a set of courses in this plan for filtering prerequisites
    let plan_courses: std::collections::HashSet<&str> =
        plan.variant.courses.iter().map(String::as_str).collect();

    for (idx, course_key) in courses.iter().enumerate() {
        let course = school.get_course(course_key);
        let metrics = plan.course_metrics.get(*course_key);

        // Determine course type and properties based on key pattern
        let course_info = classify_course_key(course_key, course);

        let canonical = course
            .and_then(|c| c.canonical_name.as_deref())
            .unwrap_or("");

        // Filter prerequisites to only include courses that are in this plan
        // and convert to row IDs for CSV output
        let prereqs = filter_prerequisites_for_plan_as_ids(course, &plan_courses, &course_to_row);
        let coreqs = filter_coreqs_for_plan_as_ids(course, &plan_courses, &course_to_row);
        let strict_coreqs =
            filter_strict_coreqs_for_plan_as_ids(course, &plan_courses, &course_to_row);

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

/// Filter prerequisites to only include courses that are in the plan
/// and convert to row IDs for CSV output.
///
/// This uses DNF (Disjunctive Normal Form) parsing to properly handle OR prerequisites.
/// For OR groups, only ONE satisfying option is included (the first one found in the plan).
/// For AND groups, all prerequisites are included.
///
/// This ensures the CSV shows the actual prerequisites used in this specific plan,
/// not all possible prerequisite options from the YAML.
fn filter_prerequisites_for_plan_as_ids(
    course: Option<&Course>,
    plan_courses: &std::collections::HashSet<&str>,
    course_to_row: &std::collections::HashMap<&str, usize>,
) -> String {
    let Some(c) = course else {
        return String::new();
    };

    let raw = match &c.prerequisites_raw {
        Some(r) if !r.is_empty() => r.clone(),
        _ if !c.prerequisites.is_empty() => c.prerequisites.join(" & "),
        _ => return String::new(),
    };

    // Parse to DNF form: Vec<Vec<String>> where outer is OR, inner is AND
    let dnf_paths = parse_to_dnf(&raw);

    if dnf_paths.is_empty() {
        return String::new();
    }

    // Find the first path (AND group) where ALL courses are in the plan
    // This gives us the actual prerequisites used
    let mut selected_prereqs: Vec<&str> = Vec::new();

    for path in &dnf_paths {
        let all_in_plan = path
            .iter()
            .all(|prereq| plan_courses.contains(prereq.as_str()));
        if all_in_plan {
            selected_prereqs = path.iter().map(String::as_str).collect();
            break;
        }
    }

    // If no complete path found, find the path with the most courses in the plan
    // and use only the courses that ARE in the plan from that path
    if selected_prereqs.is_empty() {
        let mut best_path: Vec<&str> = Vec::new();
        let mut best_count = 0;

        for path in &dnf_paths {
            let in_plan: Vec<&str> = path
                .iter()
                .map(String::as_str)
                .filter(|prereq| plan_courses.contains(*prereq))
                .collect();
            if in_plan.len() > best_count {
                best_count = in_plan.len();
                best_path = in_plan;
            }
        }
        selected_prereqs = best_path;
    }

    // Convert to row IDs
    let filtered: Vec<String> = selected_prereqs
        .into_iter()
        .filter_map(|prereq| course_to_row.get(prereq).map(usize::to_string))
        .collect();

    filtered.join(";")
}

/// Filter corequisites to only include courses that are in the plan
/// and convert to row IDs for CSV output.
fn filter_coreqs_for_plan_as_ids(
    course: Option<&Course>,
    plan_courses: &std::collections::HashSet<&str>,
    course_to_row: &std::collections::HashMap<&str, usize>,
) -> String {
    let Some(c) = course else {
        return String::new();
    };

    let filtered: Vec<String> = c
        .corequisites
        .iter()
        .map(String::as_str)
        .filter(|coreq| plan_courses.contains(coreq))
        .filter_map(|coreq| course_to_row.get(coreq).map(usize::to_string))
        .collect();

    filtered.join(";")
}

/// Filter strict corequisites to only include courses that are in the plan
/// and convert to row IDs for CSV output.
fn filter_strict_coreqs_for_plan_as_ids(
    course: Option<&Course>,
    plan_courses: &std::collections::HashSet<&str>,
    course_to_row: &std::collections::HashMap<&str, usize>,
) -> String {
    let Some(c) = course else {
        return String::new();
    };

    let filtered: Vec<String> = c
        .strict_corequisites
        .iter()
        .map(String::as_str)
        .filter(|coreq| plan_courses.contains(coreq))
        .filter_map(|coreq| course_to_row.get(coreq).map(usize::to_string))
        .collect();

    filtered.join(";")
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

/// Degree summary data for JSONL export
///
/// Contains all data needed to regenerate box plots and combine multiple degree analyses.
/// Uses JSON Lines format (one JSON object per line) for easy streaming and concatenation.
#[derive(Debug, Clone)]
pub struct DegreeSummary {
    /// Degree identifier
    pub degree_id: String,
    /// Degree name
    pub degree_name: String,
    /// Degree type (e.g., "BS", "BA")
    pub degree_type: String,
    /// Institution name
    pub institution: String,
    /// CIP code if available
    pub cip_code: Option<String>,
    /// System type (semester/quarter)
    pub system_type: String,
    /// Total number of plans analyzed
    pub plans_analyzed: usize,
    /// Degree-level statistics
    pub stats: AggregatedDegreeStats,
    /// Selected plan summaries
    pub selected_plans: Vec<PlanSummary>,
    /// Timestamp of analysis
    pub timestamp: String,
}

/// Summary of a single selected plan for JSONL export
#[derive(Debug, Clone)]
pub struct PlanSummary {
    /// Plan category name
    pub category: String,
    /// Terms required
    pub terms_required: usize,
    /// Total complexity
    pub total_complexity: usize,
    /// Longest delay factor
    pub longest_delay: usize,
    /// Is calc-ready
    pub is_calc_ready: bool,
}

impl DegreeSummary {
    /// Create a new degree summary from analysis results
    ///
    /// # Arguments
    /// * `school` - School containing course catalog
    /// * `degree` - Degree being analyzed
    /// * `aggregator` - Metrics aggregator with statistics
    /// * `selected` - Selected plans
    #[must_use]
    pub fn from_analysis(
        school: &School,
        degree: &Degree,
        aggregator: &MetricsAggregator,
        selected: &SelectedPlans,
    ) -> Self {
        let selected_plans = selected
            .iter()
            .map(|(cat, plan)| PlanSummary {
                category: cat.display_name().to_string(),
                terms_required: plan.score.terms_required,
                total_complexity: plan.score.total_complexity,
                longest_delay: plan.score.longest_delay,
                is_calc_ready: plan.score.is_calc_ready,
            })
            .collect();

        Self {
            degree_id: degree.degree_id(),
            degree_name: degree.name.clone(),
            degree_type: degree.degree_type.clone(),
            institution: school.name.clone(),
            cip_code: degree.cip_code.clone(),
            system_type: degree.system_type.clone(),
            plans_analyzed: aggregator.plan_count(),
            stats: aggregator.degree_stats(),
            selected_plans,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    /// Convert to JSON string (single line)
    ///
    /// # Errors
    /// Returns an error if serialization fails
    pub fn to_json_line(&self) -> Result<String, Box<dyn Error>> {
        Ok(format!(
            r#"{{"degree_id":"{}","degree_name":"{}","degree_type":"{}","institution":"{}","cip_code":{},"system_type":"{}","plans_analyzed":{},"stats":{},"selected_plans":{},"timestamp":"{}"}}"#,
            escape_json(&self.degree_id),
            escape_json(&self.degree_name),
            escape_json(&self.degree_type),
            escape_json(&self.institution),
            self.cip_code
                .as_ref()
                .map_or_else(|| "null".to_string(), |c| format!("\"{}\"", escape_json(c))),
            escape_json(&self.system_type),
            self.plans_analyzed,
            metric_stats_to_json(&self.stats),
            plans_to_json(&self.selected_plans),
            self.timestamp
        ))
    }
}

/// Convert aggregated degree stats to JSON
fn metric_stats_to_json(stats: &AggregatedDegreeStats) -> String {
    format!(
        r#"{{"plan_count":{},"complexity":{},"delay":{},"credits":{}}}"#,
        stats.plan_count,
        single_metric_to_json(&stats.total_complexity),
        single_metric_to_json(&stats.longest_delay),
        single_metric_to_json(&stats.total_credits)
    )
}

/// Convert a single metric stats to JSON
fn single_metric_to_json(m: &MetricStats) -> String {
    format!(
        r#"{{"min":{},"max":{},"mean":{},"std_dev":{},"median":{},"q1":{},"q3":{}}}"#,
        m.min, m.max, m.mean, m.std_dev, m.median, m.q1, m.q3
    )
}

/// Convert plan summaries to JSON array
fn plans_to_json(plans: &[PlanSummary]) -> String {
    let items: Vec<String> = plans
        .iter()
        .map(|p| {
            format!(
                r#"{{"category":"{}","terms_required":{},"total_complexity":{},"longest_delay":{},"is_calc_ready":{}}}"#,
                escape_json(&p.category),
                p.terms_required,
                p.total_complexity,
                p.longest_delay,
                p.is_calc_ready
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Escape JSON string characters
fn escape_json(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// Export degree summary to JSONL format
///
/// JSONL (JSON Lines) format stores one JSON object per line, making it easy to:
/// - Stream large datasets without loading everything into memory
/// - Concatenate multiple files with simple `cat file1.jsonl file2.jsonl > combined.jsonl`
/// - Process line by line in any language
///
/// The filename includes the degree ID for easy identification:
/// `{degree_id}_summary.jsonl`
///
/// # Arguments
/// * `school` - School containing course catalog
/// * `degree` - Degree being analyzed
/// * `aggregator` - Metrics aggregator with statistics
/// * `selected` - Selected plans
/// * `output_dir` - Directory to write JSONL file
///
/// # Returns
/// Path to the generated JSONL file
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_degree_summary_jsonl(
    school: &School,
    degree: &Degree,
    aggregator: &MetricsAggregator,
    selected: &SelectedPlans,
    output_dir: &Path,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let summary = DegreeSummary::from_analysis(school, degree, aggregator, selected);
    let json_line = summary.to_json_line()?;

    let filename = format!("{}_summary.jsonl", sanitize_filename(&degree.degree_id()));
    let output_path = output_dir.join(&filename);

    // Create directory if needed
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    let mut file = File::create(&output_path)?;
    writeln!(file, "{json_line}")?;

    Ok(output_path)
}

/// Append degree summary to an existing JSONL file
///
/// Useful for building up a combined analysis file across multiple degrees.
///
/// # Arguments
/// * `school` - School containing course catalog
/// * `degree` - Degree being analyzed
/// * `aggregator` - Metrics aggregator with statistics
/// * `selected` - Selected plans
/// * `output_path` - Path to JSONL file to append to
///
/// # Errors
/// Returns an error if file writing fails
pub fn append_degree_summary_jsonl(
    school: &School,
    degree: &Degree,
    aggregator: &MetricsAggregator,
    selected: &SelectedPlans,
    output_path: &Path,
) -> Result<(), Box<dyn Error>> {
    let summary = DegreeSummary::from_analysis(school, degree, aggregator, selected);
    let json_line = summary.to_json_line()?;

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(output_path)?;

    writeln!(file, "{json_line}")?;
    Ok(())
}

/// Index CSV header for multi-degree analysis
const INDEX_CSV_HEADER: &str = "degree_id,degree_name,degree_type,institution,cip_code,system_type,plans_analyzed,complexity_min,complexity_max,complexity_median,complexity_mean,complexity_std_dev,complexity_q1,complexity_q3,delay_min,delay_max,delay_median,delay_mean,delay_std_dev,delay_q1,delay_q3,shortest_terms,shortest_complexity,longest_terms,longest_complexity,timestamp";

/// Export degree summary to index CSV format
///
/// Creates a single-row CSV entry for this degree that can be combined with other
/// degree analyses into a master index. Ideal for spreadsheet analysis.
///
/// # Arguments
/// * `school` - School containing course catalog
/// * `degree` - Degree being analyzed
/// * `aggregator` - Metrics aggregator with statistics
/// * `selected` - Selected plans
/// * `output_dir` - Directory to write index.csv file
///
/// # Returns
/// Path to the generated index.csv file
///
/// # Errors
/// Returns an error if file writing fails
pub fn export_index_csv(
    school: &School,
    degree: &Degree,
    aggregator: &MetricsAggregator,
    selected: &SelectedPlans,
    output_dir: &Path,
) -> Result<std::path::PathBuf, Box<dyn Error>> {
    let output_path = output_dir.join("index.csv");

    // Create directory if needed
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)?;
    }

    // Check if file exists to determine if we need header
    let needs_header = !output_path.exists();

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&output_path)?;

    if needs_header {
        writeln!(file, "{INDEX_CSV_HEADER}")?;
    }

    let row = format_index_csv_row(school, degree, aggregator, selected);
    writeln!(file, "{row}")?;

    Ok(output_path)
}

/// Format a single row for the index CSV
fn format_index_csv_row(
    school: &School,
    degree: &Degree,
    aggregator: &MetricsAggregator,
    selected: &SelectedPlans,
) -> String {
    let stats = aggregator.degree_stats();
    let timestamp = chrono::Utc::now().to_rfc3339();

    // Get shortest and longest plan stats
    let (shortest_terms, shortest_complexity) = selected.shortest.as_ref().map_or((0, 0), |p| {
        (p.score.terms_required, p.score.total_complexity)
    });
    let (longest_terms, longest_complexity) = selected.longest.as_ref().map_or((0, 0), |p| {
        (p.score.terms_required, p.score.total_complexity)
    });

    format!(
        "\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",\"{}\",{},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{},{},{},{},\"{}\"",
        csv_escape(&degree.degree_id()),
        csv_escape(&degree.name),
        csv_escape(&degree.degree_type),
        csv_escape(&school.name),
        degree.cip_code.as_deref().unwrap_or(""),
        csv_escape(&degree.system_type),
        aggregator.plan_count(),
        stats.total_complexity.min,
        stats.total_complexity.max,
        stats.total_complexity.median,
        stats.total_complexity.mean,
        stats.total_complexity.std_dev,
        stats.total_complexity.q1,
        stats.total_complexity.q3,
        stats.longest_delay.min,
        stats.longest_delay.max,
        stats.longest_delay.median,
        stats.longest_delay.mean,
        stats.longest_delay.std_dev,
        stats.longest_delay.q1,
        stats.longest_delay.q3,
        shortest_terms,
        shortest_complexity,
        longest_terms,
        longest_complexity,
        timestamp
    )
}

/// Escape a string for CSV (double quotes inside)
fn csv_escape(s: &str) -> String {
    s.replace('"', "\"\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::degree::plan_selector::PlanScore;
    use crate::core::degree::plan_variant::PlanVariant;
    use crate::core::metrics::CourseMetrics;
    use crate::core::models::Course;
    use crate::core::report::term_scheduler::TermPlan;
    use crate::core::statistics::aggregator::AggregatorConfig;
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
                longest_delay_chain: Vec::new(),
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

    fn create_test_aggregator() -> MetricsAggregator {
        let mut agg = MetricsAggregator::new(AggregatorConfig::default());
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
        agg.add_plan(&metrics, 120.0);
        agg
    }

    #[test]
    fn test_escape_json() {
        assert_eq!(escape_json("hello"), "hello");
        assert_eq!(escape_json("hello\"world"), "hello\\\"world");
        assert_eq!(escape_json("line1\nline2"), "line1\\nline2");
        assert_eq!(escape_json("tab\there"), "tab\\there");
    }

    #[test]
    fn test_csv_escape() {
        assert_eq!(csv_escape("hello"), "hello");
        assert_eq!(csv_escape("hello\"world"), "hello\"\"world");
    }

    #[test]
    fn test_degree_summary_creation() {
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        let summary = DegreeSummary::from_analysis(&school, &degree, &aggregator, &selected);

        assert_eq!(summary.degree_name, "Computer Science");
        assert_eq!(summary.degree_type, "BS");
        assert_eq!(summary.institution, "Test University");
        assert_eq!(summary.plans_analyzed, 1);
        assert!(!summary.timestamp.is_empty());
    }

    #[test]
    fn test_degree_summary_to_json_line() {
        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        let summary = DegreeSummary::from_analysis(&school, &degree, &aggregator, &selected);
        let json_line = summary.to_json_line().unwrap();

        // Verify it's valid JSON
        assert!(json_line.starts_with('{'));
        assert!(json_line.ends_with('}'));
        assert!(json_line.contains("\"degree_id\":"));
        assert!(json_line.contains("\"plans_analyzed\":1"));
        assert!(json_line.contains("\"stats\":"));
    }

    #[test]
    fn test_export_degree_summary_jsonl() {
        let tmp = TempDir::new().unwrap();

        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        let result =
            export_degree_summary_jsonl(&school, &degree, &aggregator, &selected, tmp.path());
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("_summary.jsonl"));

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("\"degree_id\":"));
        assert!(contents.ends_with('\n'));
    }

    #[test]
    fn test_append_degree_summary_jsonl() {
        let tmp = TempDir::new().unwrap();
        let output_path = tmp.path().join("combined.jsonl");

        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        // Append twice
        append_degree_summary_jsonl(&school, &degree, &aggregator, &selected, &output_path)
            .unwrap();
        append_degree_summary_jsonl(&school, &degree, &aggregator, &selected, &output_path)
            .unwrap();

        let contents = fs::read_to_string(&output_path).unwrap();
        assert_eq!(contents.lines().count(), 2);
    }

    #[test]
    fn test_export_index_csv() {
        let tmp = TempDir::new().unwrap();

        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        let result = export_index_csv(&school, &degree, &aggregator, &selected, tmp.path());
        assert!(result.is_ok());

        let path = result.unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().contains("index.csv"));

        let contents = fs::read_to_string(&path).unwrap();
        assert!(contents.contains("degree_id,degree_name")); // Header
        assert!(contents.contains("Computer Science")); // Data
    }

    #[test]
    fn test_export_index_csv_appends() {
        let tmp = TempDir::new().unwrap();

        let school = create_test_school();
        let degree = create_test_degree();
        let aggregator = create_test_aggregator();
        let selected = create_test_selected();

        // Export twice - second should append without header
        export_index_csv(&school, &degree, &aggregator, &selected, tmp.path()).unwrap();
        export_index_csv(&school, &degree, &aggregator, &selected, tmp.path()).unwrap();

        let path = tmp.path().join("index.csv");
        let contents = fs::read_to_string(&path).unwrap();

        // Should have 1 header + 2 data rows
        assert_eq!(contents.lines().count(), 3);
        // Header should only appear once
        assert_eq!(
            contents.matches("degree_id,degree_name").count(),
            1,
            "Header should appear exactly once"
        );
    }
}
