//! Integration tests for the `generate_degree_report` MCP tool.
//!
//! Drives the real CSU sample through the tool's `execute_json` entry point
//! and asserts the resulting HTML matches the structure the browser-facing
//! pieces depend on (tab strip + per-plan panels + collapsible sections).

use nu_analytics::mcp::tools::report;

const CSU_SAMPLE: &str = "samples/degrees/csu-cs-bscs-general.yaml";

fn read_csu() -> String {
    std::fs::read_to_string(CSU_SAMPLE)
        .unwrap_or_else(|e| panic!("failed to read {CSU_SAMPLE}: {e}"))
}

#[test]
fn report_tool_returns_inline_html_for_csu_sample() {
    let yaml = read_csu();
    let json = report::execute_json(&yaml, Some(200), None, None, None, None, None, None);
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();

    assert_eq!(parsed["success"].as_bool(), Some(true));
    let html = parsed["html_content"]
        .as_str()
        .expect("inline mode must return html_content");

    assert!(html.starts_with("<!DOCTYPE html>"));
    // Tab strip + at least the three named selected plans.
    assert!(html.contains("class=\"tab-strip\""));
    assert!(html.contains(">Shortest Path</button>") || html.contains(">Shortest Path<"));
    assert!(html.contains(">Longest Path<"));
    // Collapsible report sections wrapped in <details class="report-section">.
    assert!(html.contains("details class=\"report-section\""));
    // Library bundled exactly once (render_without_library reuse for tab 2+).
    assert_eq!(
        html.matches("window.nuGraphs =").count(),
        1,
        "GRAPH_VANILLA_JS library must be inlined exactly once"
    );
}

#[test]
fn report_tool_writes_companion_files_to_output_dir() {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let dir = std::env::temp_dir().join(format!(
        "nuanalytics-report-csu-{}-{nanos}",
        std::process::id()
    ));
    let dir_str = dir.to_string_lossy().into_owned();

    let yaml = read_csu();
    let json = report::execute_json(
        &yaml,
        Some(200),
        None,
        Some(&dir_str),
        None, // write_plan_csvs default: true in disk mode
        None, // write_jsonl_summary default: true in disk mode
        None, // write_index_csv default: true in disk mode
        None, // return_html_inline default: false in disk mode
    );
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["success"].as_bool(), Some(true));
    assert!(parsed["html_content"].is_null());

    let html_path = parsed["report_html_path"]
        .as_str()
        .expect("disk mode must populate report_html_path");
    assert!(std::path::Path::new(html_path).exists(), "{html_path}");

    let csvs = parsed["plan_csv_paths"]
        .as_array()
        .expect("plan_csv_paths must be an array");
    assert!(!csvs.is_empty(), "expected at least one per-plan CSV");
    for csv in csvs {
        let path = csv.as_str().unwrap();
        assert!(std::path::Path::new(path).exists(), "{path}");
    }

    let jsonl_path = parsed["jsonl_summary_path"]
        .as_str()
        .expect("disk mode must populate jsonl_summary_path");
    assert!(std::path::Path::new(jsonl_path).exists());

    let index_path = parsed["index_csv_path"]
        .as_str()
        .expect("disk mode must populate index_csv_path");
    assert!(std::path::Path::new(index_path).exists());

    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn caching_yaml_and_referencing_via_degree_id_round_trips() {
    // P0 lookup-chain integration test: stash a YAML via the cache tool, then
    // confirm the layered resolver in run_yaml_tool can fetch it back when an
    // analyze-degree-style consumer passes the handle as `degree_id`.
    use nu_analytics::mcp::cache::{YamlCache, YAML_CACHE};
    use nu_analytics::mcp::tools::cache;

    let yaml = read_csu();
    let response_json = cache::execute_json(yaml.clone());
    let response: serde_json::Value = serde_json::from_str(&response_json).unwrap();
    let handle = response["handle"]
        .as_str()
        .expect("handle string")
        .to_owned();
    assert!(handle.starts_with("cache:"));

    // The handle must be retrievable from the same process-wide cache.
    let body = YAML_CACHE
        .lock()
        .expect("yaml cache mutex poisoned")
        .get(&handle)
        .expect("cache hit");
    assert_eq!(&*body, &yaml);

    // Re-caching the same body returns the same handle (idempotent).
    let again_json = cache::execute_json(yaml.clone());
    let again: serde_json::Value = serde_json::from_str(&again_json).unwrap();
    assert_eq!(again["handle"], handle);

    // Handle format matches YamlCache::handle_for exactly so the caller can
    // pre-compute handles without round-tripping the tool.
    assert_eq!(YamlCache::handle_for(&yaml), handle);
}

#[test]
fn bundled_sample_key_is_accepted_as_degree_id() {
    // P0 sample-key resolution: list_sample_degrees advertises three short
    // keys (csu, neu-khoury, uhm); each must resolve via the layered lookup
    // so analyze-style tools can address them without going through the DB.
    let key = nu_analytics::mcp::tools::samples::yaml_for_key("csu")
        .expect("csu sample key must resolve to embedded YAML");
    assert!(key.contains("Colorado State University"));
    assert!(nu_analytics::mcp::tools::samples::yaml_for_key("neu-khoury").is_some());
    assert!(nu_analytics::mcp::tools::samples::yaml_for_key("uhm").is_some());
    assert!(nu_analytics::mcp::tools::samples::yaml_for_key("nope").is_none());
}

#[test]
fn report_tool_honours_include_courses_constraint() {
    let yaml = read_csu();
    // Force every generated plan to include CS370 (CSU's OS course). The HTML
    // for the Shortest Path tab should then mention CS370 somewhere in its
    // term-schedule cards.
    let json = report::execute_json(
        &yaml,
        Some(50),
        Some(&["CS370".to_string()]),
        None,
        None,
        None,
        None,
        None,
    );
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["success"].as_bool(), Some(true));
    let html = parsed["html_content"].as_str().unwrap();
    assert!(
        html.contains("CS370"),
        "include_courses=CS370 must force the course into every plan"
    );
}
