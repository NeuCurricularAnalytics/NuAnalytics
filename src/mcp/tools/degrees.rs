//! The one piece of `compare_degrees` that cannot live in `core`.
//!
//! The query engines moved to [`crate::core::query::degrees`] so the CLI can call them
//! without the `mcp` feature. Only the hook into the analysis pipeline stays here,
//! because that pipeline is still MCP-gated (`docs/clean-up-analysis-todo.md` step 3).
//! Everything else is re-exported so `server.rs` sees no difference.

use std::sync::Arc;

use crate::core::database::DbClient;
use crate::core::query::degrees::CompareDegreesRequest;

pub use crate::core::query::degrees::{
    execute_get_json, execute_search_json, execute_store_json, GetDegreeRequest,
    SearchDegreesRequest, StoreDegreeRequest,
};

/// Run the analyze pipeline on a degree's YAML and pluck the side-by-side
/// fields useful for `compare_degrees`. Errors surface as a `parse_error`
/// payload so a single bad YAML doesn't fail the whole compare call.
fn compute_compare_metrics(yaml: &str, max_plans: Option<usize>) -> serde_json::Value {
    let response = crate::mcp::tools::analyze::execute(
        yaml, max_plans, None, false, None, false, false, None, None, None,
    );
    if !response.success {
        return serde_json::json!({
            "parse_error": response.error,
        });
    }
    serde_json::json!({
        "plans_analyzed": response.plans_analyzed,
        "population_size": response.population_size,
        "is_full_population": response.is_full_population,
        "complexity": response.complexity,
        "longest_delay": response.longest_delay,
        "total_credits": response.total_credits,
    })
}
/// `compare_degrees` with the MCP analysis pipeline wired in as the metrics source.
pub async fn execute_compare_json(client: &Arc<DbClient>, req: CompareDegreesRequest) -> String {
    crate::core::query::degrees::execute_compare_json(client, req, &compute_compare_metrics).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_compare_metrics_returns_metrics_for_valid_yaml() {
        // Minimal valid YAML lets us assert the metrics object carries the
        // analyze fields rather than a parse_error escape hatch.
        let yaml = r#"
degree:
  id: t
  institution: T
  program: T
  total_credits: 8
  gpa_minimum: 2.0

requirements:
  intro:
    name: Intro
    type: all
    category: major
    courses: [CS101, CS201]

courses:
  CS101:
    title: A
    prefix: CS
    number: "101"
    credits: 4
  CS201:
    title: B
    prefix: CS
    number: "201"
    credits: 4
    prerequisites_raw: "CS101"
"#;
        let value = compute_compare_metrics(yaml, Some(10));
        assert!(value.is_object(), "metrics must be a JSON object");
        assert!(value.get("plans_analyzed").is_some());
        assert!(value.get("complexity").is_some());
        assert!(value.get("longest_delay").is_some());
        assert!(value.get("total_credits").is_some());
        assert!(
            value.get("parse_error").is_none(),
            "valid YAML must not surface a parse_error key"
        );
    }

    #[test]
    fn test_compute_compare_metrics_surfaces_parse_error_for_invalid_yaml() {
        // Failure mode the field report cared about: if a single bad YAML
        // shows up in compare_degrees, return its parse error inline so the
        // good degrees still come back with metrics.
        let value = compute_compare_metrics("not: valid: yaml: {{", None);
        assert!(value.is_object());
        assert!(
            value.get("parse_error").is_some(),
            "invalid YAML must surface parse_error"
        );
        assert!(
            value.get("plans_analyzed").is_none(),
            "no analysis fields when parsing fails"
        );
    }
}
