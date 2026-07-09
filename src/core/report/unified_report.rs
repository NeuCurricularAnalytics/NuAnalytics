//! Unified metrics-rich degree report (JSON).
//!
//! Produces a single self-contained JSON document carrying **the whole degree
//! structure plus the numbers**: the unified program (degree metadata + tags,
//! requirements, courses with structured prerequisites), per-course metric
//! statistics, degree-level metric statistics, and the sampling metadata
//! (`variations_run`, `sample_type`). This is what the ai-landscape
//! whole-degree visualization (and a future DB load) consume.

use std::error::Error;
use std::path::{Path, PathBuf};

use serde::Serialize;
use serde_json::{json, Value};

use crate::core::degree::json_parser::to_unified_value;
use crate::core::models::DegreeProgram;
use crate::core::report::plan_export::sanitize_filename;
use crate::core::statistics::aggregator::{MetricStats, MetricsAggregator};
use crate::core::statistics::DescriptiveStats;

/// One term of a selected plan's schedule (mirrors the MCP `analyze_degree`
/// term shape so consumers can reuse the same rendering).
#[derive(Serialize)]
struct TermSchedule {
    /// 1-based term number.
    term: usize,
    /// Course ids scheduled in this term.
    courses: Vec<String>,
    /// Total credits this term.
    credits: f32,
}

/// A selected sample plan with its course schedule, surfaced in the report so a
/// consumer can see exactly which courses each exemplar (shortest/longest/…)
/// contains — especially the shortest path.
#[derive(Serialize)]
struct SelectedPlanReport {
    /// Plan category ("shortest", "longest", …).
    category: String,
    /// Terms required to complete this plan.
    terms_required: usize,
    /// Total structural complexity.
    total_complexity: usize,
    /// Longest delay factor.
    longest_delay: usize,
    /// Whether the plan is calc-ready.
    is_calc_ready: bool,
    /// Total credits across the plan.
    credits: f32,
    /// Number of courses in the plan.
    course_count: usize,
    /// Longest prerequisite (delay) chain — the critical path.
    critical_path: Vec<String>,
    /// Term-by-term schedule (empty terms omitted).
    schedule: Vec<TermSchedule>,
}

/// Build the unified report `Value` for a single analyzed degree.
///
/// `sample_type` is the human-readable sampling strategy (e.g. "shuffled").
///
/// # Errors
/// Returns an error if the program cannot be serialized.
pub fn build_degree_report(
    program: &DegreeProgram,
    aggregator: &MetricsAggregator,
    selected: &crate::core::degree::SelectedPlans,
    sample_type: &str,
) -> Result<Value, Box<dyn Error>> {
    // Start from the unified program (degree, requirements, courses w/ prereqs).
    let mut value = to_unified_value(program).map_err(|e| Box::<dyn Error>::from(e.to_string()))?;

    // Attach per-course metric statistics, keyed by course id.
    if let Some(courses) = value.get_mut("courses").and_then(Value::as_object_mut) {
        for (key, course_value) in courses.iter_mut() {
            if let (Some(stats), Some(obj)) =
                (aggregator.course_stats(key), course_value.as_object_mut())
            {
                obj.insert("metrics".to_string(), serde_json::to_value(&stats)?);
            }
        }
    }

    // Degree-level metrics + sampling metadata.
    let degree_stats = aggregator.degree_stats();
    let selected_plans: Vec<SelectedPlanReport> = selected
        .iter()
        .map(|(category, plan)| SelectedPlanReport {
            category: category.display_name().to_string(),
            terms_required: plan.score.terms_required,
            total_complexity: plan.score.total_complexity,
            longest_delay: plan.score.longest_delay,
            is_calc_ready: plan.score.is_calc_ready,
            credits: plan.variant.total_credits,
            course_count: plan.variant.courses.len(),
            critical_path: plan.score.longest_delay_chain.clone(),
            schedule: plan
                .schedule
                .terms
                .iter()
                .filter(|t| !t.courses.is_empty())
                .map(|t| TermSchedule {
                    term: t.number,
                    courses: t.courses.clone(),
                    credits: t.total_credits,
                })
                .collect(),
        })
        .collect();

    if let Some(obj) = value.as_object_mut() {
        obj.insert(
            "analysis".to_string(),
            json!({
                "variations_run": aggregator.plan_count(),
                "sample_type": sample_type,
                "metrics": {
                    "complexity": degree_stats.total_complexity,
                    "delay": degree_stats.longest_delay,
                    "credits": degree_stats.total_credits,
                }
            }),
        );
        obj.insert(
            "selected_plans".to_string(),
            serde_json::to_value(&selected_plans)?,
        );
    }

    Ok(value)
}

/// Write the unified report JSON to `<out_dir>/<degree_id>_report.json`.
///
/// # Errors
/// Returns an error if the report cannot be built or written.
pub fn export_degree_report_json(
    program: &DegreeProgram,
    aggregator: &MetricsAggregator,
    selected: &crate::core::degree::SelectedPlans,
    sample_type: &str,
    out_dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let value = build_degree_report(program, aggregator, selected, sample_type)?;
    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(format!(
        "{}_report.json",
        sanitize_filename(&program.degree.degree_id())
    ));
    std::fs::write(&path, report_value_to_pretty(&value)?)?;
    Ok(path)
}

/// Serialize a built report `Value` to pretty JSON with `degree` first, then the
/// analysis summary, requirements, and selected plans, with the large `courses`
/// block last. Nested objects keep `serde_json`'s deterministic sorted key order.
fn report_value_to_pretty(value: &Value) -> Result<String, serde_json::Error> {
    #[derive(Serialize)]
    struct Ordered<'a> {
        degree: &'a Value,
        analysis: &'a Value,
        requirements: &'a Value,
        selected_plans: &'a Value,
        courses: &'a Value,
    }
    let null = Value::Null;
    let ordered = Ordered {
        degree: value.get("degree").unwrap_or(&null),
        analysis: value.get("analysis").unwrap_or(&null),
        requirements: value.get("requirements").unwrap_or(&null),
        selected_plans: value.get("selected_plans").unwrap_or(&null),
        courses: value.get("courses").unwrap_or(&null),
    };
    serde_json::to_string_pretty(&ordered)
}

/// Degree-level rollup for one program, used to build a school-level report.
#[derive(Debug, Clone)]
pub struct ProgramRollup {
    /// Degree identifier.
    pub id: String,
    /// Degree/program name.
    pub name: String,
    /// Program-level tags (e.g. `["ai"]`).
    pub tags: Option<Vec<String>>,
    /// Number of plan variations analyzed.
    pub variations_run: usize,
    /// Degree-level total-complexity statistics.
    pub complexity: MetricStats,
    /// Degree-level longest-delay statistics.
    pub delay: MetricStats,
    /// Degree-level total-credits statistics.
    pub credits: MetricStats,
}

impl ProgramRollup {
    /// Build a rollup from an analyzed program + aggregator.
    #[must_use]
    pub fn from_analysis(program: &DegreeProgram, aggregator: &MetricsAggregator) -> Self {
        let stats = aggregator.degree_stats();
        Self {
            id: program.degree.degree_id(),
            name: program.degree.name.clone(),
            tags: program.degree.tags.clone(),
            variations_run: aggregator.plan_count(),
            complexity: stats.total_complexity,
            delay: stats.longest_delay,
            credits: stats.total_credits,
        }
    }
}

/// Write a school-level report aggregating the degree-level metrics of every
/// program in the school. School metrics summarize each program's median
/// values across the school.
///
/// # Errors
/// Returns an error if the file cannot be written.
pub fn export_school_report_json(
    school: &str,
    programs: &[ProgramRollup],
    out_dir: &Path,
) -> Result<PathBuf, Box<dyn Error>> {
    let program_values: Vec<Value> = programs
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "name": p.name,
                "tags": p.tags,
                "variations_run": p.variations_run,
                "metrics": {
                    "complexity": p.complexity,
                    "delay": p.delay,
                    "credits": p.credits,
                }
            })
        })
        .collect();

    let complexity_medians: Vec<f64> = programs.iter().map(|p| p.complexity.median).collect();
    let delay_medians: Vec<f64> = programs.iter().map(|p| p.delay.median).collect();
    let credit_medians: Vec<f64> = programs.iter().map(|p| p.credits.median).collect();

    let report = json!({
        "school": school,
        "program_count": programs.len(),
        "school_metrics": {
            "complexity": aggregate(&complexity_medians),
            "delay": aggregate(&delay_medians),
            "credits": aggregate(&credit_medians),
        },
        "programs": program_values,
    });

    std::fs::create_dir_all(out_dir)?;
    let path = out_dir.join(format!("{}_school_report.json", sanitize_filename(school)));
    std::fs::write(&path, serde_json::to_string_pretty(&report)?)?;
    Ok(path)
}

/// Summary statistics over a set of values (the per-program medians).
///
/// Returns `Null` for an empty set; otherwise the full quartile / mean /
/// standard-deviation block from [`DescriptiveStats`] (shared, correct
/// percentile median).
fn aggregate(values: &[f64]) -> Value {
    if values.is_empty() {
        return Value::Null;
    }
    let s = DescriptiveStats::from_values(values);
    json!({
        "count": s.count,
        "mean": s.mean,
        "median": s.median,
        "std_dev": s.std_dev,
        "min": s.min,
        "max": s.max,
        "q1": s.q1,
        "q3": s.q3,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::metrics::CourseMetrics;
    use crate::core::models::Degree;
    use std::collections::HashMap;

    fn f(v: &Value, key: &str) -> f64 {
        v.get(key).and_then(Value::as_f64).unwrap()
    }

    #[test]
    fn test_aggregate_empty_is_null() {
        assert_eq!(aggregate(&[]), Value::Null);
    }

    #[test]
    fn test_aggregate_forwards_descriptive_stats() {
        // Population std_dev (/n): values 2,4,4,4,5,5,7,9 -> mean 5, std_dev 2.
        let v = aggregate(&[2.0, 4.0, 4.0, 4.0, 5.0, 5.0, 7.0, 9.0]);
        assert_eq!(v.get("count").and_then(Value::as_u64), Some(8));
        assert!((f(&v, "mean") - 5.0).abs() < 1e-9);
        assert!((f(&v, "std_dev") - 2.0).abs() < 1e-9);
        assert!((f(&v, "min") - 2.0).abs() < 1e-9);
        assert!((f(&v, "max") - 9.0).abs() < 1e-9);
        // Quartiles present (the bug-fix that the old hand-rolled version lacked).
        assert!(v.get("q1").is_some() && v.get("q3").is_some());
    }

    fn aggregator_with_two_plans() -> MetricsAggregator {
        let mut agg = MetricsAggregator::default();
        for c in [10usize, 20] {
            let mut cm = HashMap::new();
            cm.insert(
                "CS1000".to_string(),
                CourseMetrics {
                    complexity: c,
                    centrality: c / 2,
                    delay: c / 5,
                    blocking: c / 3,
                    chain_length: 1,
                },
            );
            agg.add_plan(&cm, 4.0);
        }
        agg
    }

    fn sample_program() -> DegreeProgram {
        let mut degree = Degree::new(
            "Computer Science".to_string(),
            "BS".to_string(),
            None,
            "semester".to_string(),
        );
        degree.id = Some("bs-cs".to_string());
        degree.tags = Some(vec!["ai".to_string()]);
        DegreeProgram {
            degree,
            requirements: HashMap::new(),
            courses: HashMap::new(),
        }
    }

    #[test]
    fn test_program_rollup_from_analysis_copies_identity_and_stats() {
        let rollup = ProgramRollup::from_analysis(&sample_program(), &aggregator_with_two_plans());
        assert_eq!(rollup.id, "bs-cs");
        assert_eq!(rollup.name, "Computer Science");
        assert_eq!(rollup.tags.as_deref(), Some(&["ai".to_string()][..]));
        assert_eq!(rollup.variations_run, 2);
        // per-plan total complexity = 10 then 20 -> mean 15.
        assert!((rollup.complexity.mean - 15.0).abs() < 1e-9);
    }

    #[test]
    fn test_export_school_report_json_writes_sanitized_file_with_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let rollups = vec![ProgramRollup::from_analysis(
            &sample_program(),
            &aggregator_with_two_plans(),
        )];
        let path = export_school_report_json("Khoury College", &rollups, dir.path()).unwrap();

        assert_eq!(
            path.file_name().unwrap().to_str().unwrap(),
            "Khoury_College_school_report.json"
        );
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["school"], "Khoury College");
        assert_eq!(v["program_count"], 1);
        assert!(v["school_metrics"]["complexity"].is_object());
        assert_eq!(v["programs"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn test_export_school_report_json_empty_programs_null_metrics() {
        let dir = tempfile::tempdir().unwrap();
        let path = export_school_report_json("Empty School", &[], dir.path()).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["program_count"], 0);
        assert!(v["school_metrics"]["complexity"].is_null());
        assert!(v["school_metrics"]["delay"].is_null());
        assert!(v["school_metrics"]["credits"].is_null());
    }
}
