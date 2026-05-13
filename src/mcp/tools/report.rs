//! Degree HTML report generation tool.
//!
//! Provides the `generate_degree_report` MCP tool that produces the same
//! artifacts as the CLI `degree --analyze` command: the HTML report and
//! optional per-plan CSVs, JSONL summary, and index CSV. The pipeline is
//! shared with `analyze_degree` through
//! `crate::mcp::tools::analyze::build_artifacts` (a `pub(crate)` helper);
//! this tool then feeds the resulting artifacts into [`DegreeReportGenerator`].
//!
//! Two output modes:
//!
//! - **Inline** (no `output_dir`): the HTML is returned in `html_content`.
//! - **Disk** (`output_dir` set): files are written to that directory and
//!   their absolute paths are returned. `html_content` is omitted unless the
//!   caller forces `return_html_inline=true`. CSV / JSONL / index outputs
//!   default to ON in disk mode and OFF otherwise.

use std::path::PathBuf;

use crate::core::report::degree_report::{DegreeReportContext, DegreeReportGenerator};
use crate::core::report::plan_export::{
    export_degree_summary_jsonl, export_index_csv, export_selected_plans, PlanExportConfig,
};
use crate::mcp::tools::analyze::AnalysisArtifacts;
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for the `generate_degree_report` tool.
///
/// Provide exactly one YAML source — `yaml_content`, `yaml_path`, or
/// `degree_id` — together with the same analysis knobs `analyze_degree`
/// accepts. Set `output_dir` to write artifacts to disk; otherwise the
/// rendered HTML is returned inline.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GenerateDegreeReportRequest {
    /// Inline YAML content. Mutually exclusive with `yaml_path` / `degree_id`.
    #[schemars(description = "Complete degree program YAML content (inline)")]
    pub yaml_content: Option<String>,

    /// Path to a YAML file the MCP server will read.
    #[schemars(
        description = "Path to a YAML file on the MCP server's filesystem. Mutually exclusive with yaml_content/degree_id."
    )]
    pub yaml_path: Option<String>,

    /// Stored degree id (database lookup).
    #[schemars(
        description = "Stored degree ID (DB lookup). Requires the database feature; mutually exclusive with yaml_content/yaml_path."
    )]
    pub degree_id: Option<String>,

    /// Maximum number of plans to generate (default 500). Forwarded directly
    /// to the analysis pipeline; higher values give more accurate per-course
    /// statistics in the report but slow generation.
    #[schemars(description = "Maximum plans to generate (default 500)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_usize"
    )]
    pub max_plans: Option<usize>,

    /// Comma-separated course codes that must appear in every generated plan
    /// (e.g. `"CS150B,MATH156,CS414"`). Lets the caller constrain the report
    /// to a specific track or pre-decided course set.
    #[schemars(
        description = "Comma-separated course codes that every generated plan must include (e.g. \"CS150B,MATH156,CS414\")."
    )]
    pub include_courses: Option<String>,

    /// Output directory for the HTML report and optional companion files.
    /// When omitted, no files are written — the HTML is returned inline.
    #[schemars(
        description = "Filesystem directory to write the HTML report (and optional CSV/JSONL/index files) into. When omitted, the HTML is returned inline."
    )]
    pub output_dir: Option<String>,

    /// Whether to also export per-plan CSVs (one per `selected_plans` entry).
    /// Defaults to true when `output_dir` is set; ignored otherwise.
    #[schemars(
        description = "Also write per-plan CSV files alongside the HTML report (default true when output_dir is set, ignored otherwise)."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub write_plan_csvs: Option<bool>,

    /// Whether to write the degree's one-line JSONL summary into `output_dir`.
    /// Defaults to true when `output_dir` is set; ignored otherwise.
    #[schemars(
        description = "Also write the degree's one-line JSONL summary (default true when output_dir is set, ignored otherwise)."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub write_jsonl_summary: Option<bool>,

    /// Whether to append/refresh an `index.csv` row into `output_dir`.
    /// Defaults to true when `output_dir` is set; ignored otherwise.
    #[schemars(
        description = "Also write the cross-degree index.csv row (default true when output_dir is set, ignored otherwise)."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub write_index_csv: Option<bool>,

    /// Force the HTML to be returned inline even when `output_dir` is set.
    /// Useful when a caller wants both a saved copy and the body in-response.
    /// Defaults to true when `output_dir` is unset and false otherwise.
    #[schemars(
        description = "Return the rendered HTML in the response body. Default: true when output_dir is unset, false otherwise."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub return_html_inline: Option<bool>,
}

/// Response for `generate_degree_report`.
#[derive(Debug, Serialize)]
pub struct GenerateDegreeReportResponse {
    /// Whether the report was rendered successfully.
    pub success: bool,
    /// Error message when `success` is false.
    pub error: Option<String>,

    /// Degree identifier (slug used for file names and the index row).
    pub degree_id: Option<String>,
    /// Human-readable degree name.
    pub degree_name: Option<String>,
    /// Institution name from the parsed program.
    pub institution: Option<String>,

    /// Number of plans actually processed.
    pub plans_analyzed: usize,
    /// Upper bound on distinct plans for this YAML.
    pub population_size: usize,
    /// True when every distinct plan was analyzed.
    pub is_full_population: bool,
    /// Size of `selected_plans` after curation (typically 5–6).
    pub selected_plans_count: usize,

    /// Length of the rendered HTML in bytes (always populated; useful when
    /// callers skip `html_content`).
    pub html_bytes: usize,
    /// Full HTML report body. Present when `return_html_inline` is true.
    pub html_content: Option<String>,

    /// Absolute path to the directory where artifacts were written
    /// (`output_dir` echoed back). `None` in inline-only mode.
    pub output_dir: Option<String>,
    /// Absolute path to the rendered HTML file when written to disk.
    pub report_html_path: Option<String>,
    /// Per-plan CSV file paths (one entry per selected plan when written).
    pub plan_csv_paths: Vec<String>,
    /// Path to the JSONL summary file when written.
    pub jsonl_summary_path: Option<String>,
    /// Path to the index.csv file when written.
    pub index_csv_path: Option<String>,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `generate_degree_report` tool.
///
/// The argument count crosses clippy's default ceiling because every option
/// the CLI exposes (`max_plans`, `include_courses`, disk-mode toggles, the
/// inline-html override) must be reachable from the MCP handler. Grouping
/// them into a struct would just move the same data without reducing the
/// caller's burden.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    output_dir: Option<&str>,
    write_plan_csvs: Option<bool>,
    write_jsonl_summary: Option<bool>,
    write_index_csv: Option<bool>,
    return_html_inline: Option<bool>,
) -> GenerateDegreeReportResponse {
    let artifacts = match crate::mcp::cache::cached_artifacts(
        yaml_content,
        max_plans,
        include_courses,
        None,
        None,
    ) {
        Ok(a) => a,
        Err(e) => return error_response(&e),
    };

    let html = match render_html(&artifacts) {
        Ok(s) => s,
        Err(e) => return error_response(&format!("Failed to render report: {e}")),
    };

    let write_disk = output_dir.is_some();
    // Disk mode defaults: every companion file ON; inline HTML OFF (caller
    // opts back in). Inline mode defaults: companions ignored, HTML returned.
    let inline_html = return_html_inline.unwrap_or(!write_disk);
    let write_csvs = write_plan_csvs.unwrap_or(write_disk);
    let write_jsonl = write_jsonl_summary.unwrap_or(write_disk);
    let write_index = write_index_csv.unwrap_or(write_disk);

    let mut paths = WrittenPaths::default();
    if let Some(dir) = output_dir {
        if let Err(e) = write_artifacts_to_disk(
            dir,
            &html,
            &artifacts,
            write_csvs,
            write_jsonl,
            write_index,
            &mut paths,
        ) {
            return error_response(&format!("Failed to write artifacts: {e}"));
        }
    }

    let html_bytes = html.len();
    GenerateDegreeReportResponse {
        success: true,
        error: None,
        degree_id: Some(artifacts.program.degree.degree_id()),
        degree_name: Some(artifacts.program.degree.name.clone()),
        institution: artifacts.program.degree.institution.clone(),
        plans_analyzed: artifacts.plans_processed,
        population_size: artifacts.population_size(),
        is_full_population: artifacts.is_full_population(),
        selected_plans_count: artifacts.selected.total_count(),
        html_bytes,
        html_content: if inline_html { Some(html) } else { None },
        output_dir: output_dir.map(str::to_string),
        report_html_path: paths.report_html,
        plan_csv_paths: paths.plan_csvs,
        jsonl_summary_path: paths.jsonl_summary,
        index_csv_path: paths.index_csv,
    }
}

/// Execute and serialize as JSON.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn execute_json(
    yaml_content: &str,
    max_plans: Option<usize>,
    include_courses: Option<&[String]>,
    output_dir: Option<&str>,
    write_plan_csvs: Option<bool>,
    write_jsonl_summary: Option<bool>,
    write_index_csv: Option<bool>,
    return_html_inline: Option<bool>,
) -> String {
    let response = execute(
        yaml_content,
        max_plans,
        include_courses,
        output_dir,
        write_plan_csvs,
        write_jsonl_summary,
        write_index_csv,
        return_html_inline,
    );
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Helpers
// ============================================================================

#[derive(Default)]
struct WrittenPaths {
    report_html: Option<String>,
    plan_csvs: Vec<String>,
    jsonl_summary: Option<String>,
    index_csv: Option<String>,
}

fn render_html(artifacts: &AnalysisArtifacts) -> Result<String, Box<dyn std::error::Error>> {
    let ctx = DegreeReportContext::new(
        &artifacts.school,
        &artifacts.program.degree,
        &artifacts.aggregator,
        &artifacts.selected,
        &artifacts.dag,
        &artifacts.equivalences,
    );
    DegreeReportGenerator::new().render(&ctx)
}

/// Write the HTML + optional companion artifacts into `output_dir`. The
/// directory is created if it does not already exist.
fn write_artifacts_to_disk(
    output_dir: &str,
    html: &str,
    artifacts: &AnalysisArtifacts,
    write_csvs: bool,
    write_jsonl: bool,
    write_index: bool,
    out: &mut WrittenPaths,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = PathBuf::from(output_dir);
    std::fs::create_dir_all(&dir)?;

    let html_filename = format!("{}-analysis.html", artifacts.program.degree.degree_id());
    let html_path = dir.join(&html_filename);
    std::fs::write(&html_path, html)?;
    out.report_html = Some(html_path.to_string_lossy().into_owned());

    if write_csvs {
        let plans_dir = dir.join("plans");
        let cfg = PlanExportConfig {
            base_dir: plans_dir.to_string_lossy().into_owned(),
            create_dirs: true,
        };
        let csvs = export_selected_plans(
            &artifacts.school,
            &artifacts.program.degree,
            &artifacts.selected,
            &cfg,
        )?;
        out.plan_csvs = csvs;
    }

    if write_jsonl {
        let path = export_degree_summary_jsonl(
            &artifacts.school,
            &artifacts.program.degree,
            &artifacts.aggregator,
            &artifacts.selected,
            &dir,
        )?;
        out.jsonl_summary = Some(path.to_string_lossy().into_owned());
    }

    if write_index {
        let path = export_index_csv(
            &artifacts.school,
            &artifacts.program.degree,
            &artifacts.aggregator,
            &artifacts.selected,
            &dir,
        )?;
        out.index_csv = Some(path.to_string_lossy().into_owned());
    }

    Ok(())
}

fn error_response(error: &str) -> GenerateDegreeReportResponse {
    GenerateDegreeReportResponse {
        success: false,
        error: Some(error.to_string()),
        degree_id: None,
        degree_name: None,
        institution: None,
        plans_analyzed: 0,
        population_size: 0,
        is_full_population: false,
        selected_plans_count: 0,
        html_bytes: 0,
        html_content: None,
        output_dir: None,
        report_html_path: None,
        plan_csv_paths: Vec::new(),
        jsonl_summary_path: None,
        index_csv_path: None,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_YAML: &str = r#"
degree:
  id: test-degree
  institution: Test University
  program: Test Program
  total_credits: 16
  gpa_minimum: 2.0
  major_subjects: ["CS"]

requirements:
  intro:
    name: Introduction
    type: all
    category: major
    courses:
      - CS101
      - CS201

courses:
  CS101:
    title: Intro to CS
    prefix: CS
    number: "101"
    credits: 4
  CS201:
    title: Data Structures
    prefix: CS
    number: "201"
    credits: 4
    prerequisites_raw: "CS101"
"#;

    #[test]
    fn test_inline_mode_returns_html_body() {
        let response = execute(TEST_YAML, Some(10), None, None, None, None, None, None);
        assert!(response.success, "error: {:?}", response.error);
        let html = response
            .html_content
            .as_deref()
            .expect("inline mode must return html_content");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Test Program"));
        assert!(response.html_bytes == html.len());
        assert!(response.report_html_path.is_none());
        assert!(response.plan_csv_paths.is_empty());
        assert_eq!(response.degree_id.as_deref(), Some("test-degree"));
    }

    #[test]
    fn test_parse_error_response_carries_error_message() {
        let response = execute(
            "not: valid: yaml: {{",
            Some(10),
            None,
            None,
            None,
            None,
            None,
            None,
        );
        assert!(!response.success);
        assert!(response.error.is_some());
        assert!(response.html_content.is_none());
        assert_eq!(response.html_bytes, 0);
    }

    #[test]
    fn test_output_dir_writes_files_and_omits_inline_html_by_default() {
        // Use a unique temp dir so concurrent test runs don't collide.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir =
            std::env::temp_dir().join(format!("nuanalytics-report-{}-{nanos}", std::process::id()));
        let dir_str = dir.to_string_lossy().into_owned();

        let response = execute(
            TEST_YAML,
            Some(10),
            None,
            Some(&dir_str),
            None,
            None,
            None,
            None,
        );
        assert!(response.success, "error: {:?}", response.error);
        // Disk-mode default: HTML written, not echoed back inline.
        assert!(response.html_content.is_none());
        let html_path = response
            .report_html_path
            .as_deref()
            .expect("disk mode must write the HTML report");
        assert!(std::path::Path::new(html_path).exists(), "{html_path}");
        // Companions default to ON in disk mode.
        assert!(!response.plan_csv_paths.is_empty());
        for csv in &response.plan_csv_paths {
            assert!(std::path::Path::new(csv).exists(), "{csv}");
        }
        assert!(response.jsonl_summary_path.is_some());
        assert!(response.index_csv_path.is_some());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_output_dir_with_return_html_inline_keeps_both() {
        // Caller opts back into the inline HTML even though disk mode is on.
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = std::env::temp_dir().join(format!(
            "nuanalytics-report-both-{}-{nanos}",
            std::process::id()
        ));
        let dir_str = dir.to_string_lossy().into_owned();

        let response = execute(
            TEST_YAML,
            Some(10),
            None,
            Some(&dir_str),
            Some(false),
            Some(false),
            Some(false),
            Some(true),
        );
        assert!(response.success);
        assert!(
            response.html_content.is_some(),
            "return_html_inline=true must keep the body"
        );
        assert!(response.report_html_path.is_some());
        assert!(response.plan_csv_paths.is_empty());
        assert!(response.jsonl_summary_path.is_none());
        assert!(response.index_csv_path.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn test_execute_json_serializes_response_with_expected_keys() {
        let json = execute_json(TEST_YAML, Some(10), None, None, None, None, None, None);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["success"].as_bool(), Some(true));
        assert!(parsed["html_content"].is_string());
        assert!(parsed["html_bytes"].as_u64().is_some());
        assert!(parsed["selected_plans_count"].as_u64().unwrap() > 0);
    }

    /// Build a unique tmp path so concurrent test runs don't collide.
    fn unique_tmp_path(prefix: &str) -> std::path::PathBuf {
        use std::time::{SystemTime, UNIX_EPOCH};
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        std::env::temp_dir().join(format!("{prefix}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn test_output_dir_pointing_at_file_surfaces_write_error() {
        // create_dir_all should reject the file → execute returns an error
        // response rather than panicking or silently dropping work.
        let file_path = unique_tmp_path("nuanalytics-report-is-file");
        std::fs::write(&file_path, "not a directory").expect("create temp file");
        let response = execute(
            TEST_YAML,
            Some(10),
            None,
            Some(file_path.to_str().unwrap()),
            None,
            None,
            None,
            None,
        );
        assert!(!response.success);
        let err = response.error.expect("error must be populated");
        assert!(
            err.contains("Failed to write artifacts"),
            "error message should identify the failed operation, got: {err}"
        );
        assert!(response.html_content.is_none());
        assert!(response.report_html_path.is_none());

        std::fs::remove_file(&file_path).ok();
    }

    #[test]
    fn test_output_dir_with_all_companions_off_writes_only_html_no_inline_body() {
        // The opposite of test_output_dir_with_return_html_inline_keeps_both:
        // companions explicitly disabled, return_html_inline=false. Expect a
        // single HTML file on disk and a response without html_content.
        let dir = unique_tmp_path("nuanalytics-report-html-only");
        let dir_str = dir.to_string_lossy().into_owned();

        let response = execute(
            TEST_YAML,
            Some(10),
            None,
            Some(&dir_str),
            Some(false),
            Some(false),
            Some(false),
            Some(false),
        );
        assert!(response.success, "error: {:?}", response.error);
        assert!(
            response.html_content.is_none(),
            "return_html_inline=false must suppress the response body"
        );
        let html_path = response
            .report_html_path
            .as_deref()
            .expect("HTML must still be written to disk");
        assert!(std::path::Path::new(html_path).exists(), "{html_path}");
        assert!(response.plan_csv_paths.is_empty());
        assert!(response.jsonl_summary_path.is_none());
        assert!(response.index_csv_path.is_none());

        std::fs::remove_dir_all(&dir).ok();
    }
}
