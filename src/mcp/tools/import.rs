//! `import_degree` MCP tool.
//!
//! Wraps the tested import core ([`crate::core::database::import::execute_import`])
//! so an AI client can load a degree-first analysis report (or a plain unified
//! degree) JSON into the normalized program tables (`programs`, `courses`,
//! `program_courses`, `program_requirements`) plus, when the input carries an
//! `analysis` block, one analysis run.
//!
//! The tool resolves the report text (inline `json_content` or a `json_path`
//! file on the server's filesystem), builds [`ImportOptions`], calls
//! `execute_import`, and maps the [`ImportOutcome`] into a structured JSON
//! response. `dry_run` previews the write counts without touching the DB; an
//! ambiguous institution name returns the candidate `(unitid, name)` pairs so
//! the agent can re-call with an explicit `unitid`.

use std::sync::Arc;

use crate::core::database::import::{execute_import, ImportOptions, ImportOutcome, ImportResult};
use crate::core::database::DbClient;
use crate::mcp::tools::shared::{self, error_json};
use rmcp::schemars;
use serde::Deserialize;

// ============================================================================
// Request type
// ============================================================================

/// Request parameters for the `import_degree` tool.
///
/// Provide exactly one report source: `json_content` (inline string) or
/// `json_path` (a file the MCP server reads from its filesystem). The remaining
/// fields mirror the import core's [`ImportOptions`] overrides.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ImportDegreeRequest {
    /// Inline degree-first report (or unified degree) JSON. Mutually exclusive
    /// with `json_path`; exactly one is required.
    #[schemars(
        description = "Inline degree-first report (or unified degree) JSON. Mutually exclusive with json_path; provide exactly one."
    )]
    pub json_content: Option<String>,

    /// Filesystem path to a report JSON file the server will read. Mutually
    /// exclusive with `json_content`; exactly one is required.
    #[schemars(
        description = "Path to a report JSON file on the MCP server's filesystem. Mutually exclusive with json_content; provide exactly one."
    )]
    pub json_path: Option<String>,

    /// Analysis-run variant label (default `"full"`). `full` writes the program
    /// projection; a non-full variant only attaches an analysis run.
    #[schemars(
        description = "Analysis-run variant label (default \"full\"). \"full\" writes the program projection; a non-full variant only attaches an analysis run."
    )]
    pub variant: Option<String>,

    /// Override the resolved institution IPEDS unit id. Set this to
    /// disambiguate when a previous call returned `institution_ambiguous`.
    #[schemars(
        description = "Override the resolved IPEDS UNITID. Set this to disambiguate after an institution_ambiguous result."
    )]
    #[serde(default, deserialize_with = "shared::deserialize_opt_i32")]
    pub unitid: Option<i32>,

    /// Override the institution name used for resolution / display.
    #[schemars(description = "Override the institution name used for resolution/display.")]
    pub institution: Option<String>,

    /// Override the CIP code (part of the natural program key).
    #[schemars(description = "Override the CIP code (part of the natural program key).")]
    pub cip: Option<String>,

    /// Override the catalog year (part of the program identity).
    #[schemars(description = "Override the catalog year (part of the program identity).")]
    pub catalog: Option<String>,

    /// Override the degree id.
    #[schemars(description = "Override the degree id.")]
    pub degree_id: Option<String>,

    /// Overwrite a verified program / skip the confirmation gate.
    #[schemars(description = "Overwrite a verified program / skip the confirmation gate.")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub force: Option<bool>,

    /// Replace an existing (unverified) program.
    #[schemars(description = "Replace an existing (unverified) program.")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub replace: Option<bool>,

    /// Skip the program entirely if it already exists.
    #[schemars(description = "Skip the program entirely if it already exists.")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub skip_existing: Option<bool>,

    /// Build the plan and report counts but write nothing.
    #[schemars(description = "Preview the import (report counts) without writing anything.")]
    #[serde(default, deserialize_with = "shared::deserialize_opt_bool")]
    pub dry_run: Option<bool>,
}

// ============================================================================
// Execute
// ============================================================================

/// Execute `import_degree` and return JSON.
///
/// Resolves the report text from `json_content` / `json_path` (exactly one
/// required), builds [`ImportOptions`], runs the import core, and maps the
/// [`ImportOutcome`] to a structured JSON string. Transport/parse failures
/// surface via [`error_json`].
pub async fn execute_json(client: &Arc<DbClient>, req: ImportDegreeRequest) -> String {
    let text = match resolve_report_text(req.json_content, req.json_path) {
        Ok(t) => t,
        Err(e) => return e,
    };

    let opts = ImportOptions {
        variant: req.variant.unwrap_or_else(|| "full".to_string()),
        unitid: req.unitid,
        institution: req.institution,
        cip_code: req.cip,
        catalog_year: req.catalog,
        degree_id: req.degree_id,
        force: req.force.unwrap_or(false),
        replace: req.replace.unwrap_or(false),
        skip_existing: req.skip_existing.unwrap_or(false),
        dry_run: req.dry_run.unwrap_or(false),
    };

    match execute_import(client, &text, &opts).await {
        Ok(outcome) => outcome_to_json(&outcome).to_string(),
        Err(e) => error_json(e),
    }
}

/// Resolve the report text from exactly one of `json_content` / `json_path`.
///
/// Returns a JSON error string (ready to surface) when neither or both are
/// provided, or when a `json_path` file cannot be read.
fn resolve_report_text(
    json_content: Option<String>,
    json_path: Option<String>,
) -> Result<String, String> {
    match (json_content, json_path) {
        (Some(_), Some(_)) => Err(error_json(
            "Provide exactly one of: json_content or json_path (not both)",
        )),
        (None, None) => Err(error_json(
            "Must provide exactly one of: json_content or json_path",
        )),
        (Some(c), None) => Ok(c),
        (None, Some(p)) => shared::read_yaml_file(&p),
    }
}

/// Map an [`ImportOutcome`] to the structured JSON response.
///
/// `ImportOutcome` is not `Serialize`, so the object is built explicitly with
/// `serde_json::json!`. The `result` field carries a stable lowercase tag; the
/// `NeedsConfirmation` / `InstitutionAmbiguous` / `Rejected` variants attach
/// their extra payload (`reason` / `institution_candidates` / `errors`).
fn outcome_to_json(outcome: &ImportOutcome) -> serde_json::Value {
    let mut value = serde_json::json!({
        "result": result_tag(&outcome.result),
        "program_key": outcome.program_key,
        "resolved_unitid": outcome.resolved_unitid,
        "institution": outcome.institution,
        "variant": outcome.variant,
        "variations_run": outcome.variations_run,
        "sample_type": outcome.sample_type,
        "courses_written": outcome.courses_written,
        "requirements_written": outcome.requirements_written,
        "run_written": outcome.run_written,
        "plans_written": outcome.plans_written,
        "course_metrics_written": outcome.course_metrics_written,
        "conversion_warnings": outcome.conversion_warnings,
        "messages": outcome.messages,
    });

    // `value` is built from a json! object literal, so it is always an object.
    if let Some(map) = value.as_object_mut() {
        match &outcome.result {
            ImportResult::NeedsConfirmation(reason) => {
                map.insert("reason".to_string(), serde_json::json!(reason));
            }
            ImportResult::InstitutionAmbiguous(candidates) => {
                let cands: Vec<serde_json::Value> = candidates
                    .iter()
                    .map(|(unitid, name)| serde_json::json!({ "unitid": unitid, "name": name }))
                    .collect();
                map.insert(
                    "institution_candidates".to_string(),
                    serde_json::Value::Array(cands),
                );
            }
            ImportResult::Rejected(errors) => {
                map.insert("errors".to_string(), serde_json::json!(errors));
            }
            ImportResult::Created | ImportResult::Updated | ImportResult::Skipped => {}
        }
    }

    value
}

/// Stable lowercase tag for an [`ImportResult`] variant.
const fn result_tag(result: &ImportResult) -> &'static str {
    match result {
        ImportResult::Created => "created",
        ImportResult::Updated => "updated",
        ImportResult::Skipped => "skipped",
        ImportResult::NeedsConfirmation(_) => "needs_confirmation",
        ImportResult::InstitutionAmbiguous(_) => "institution_ambiguous",
        ImportResult::Rejected(_) => "rejected",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a default `ImportOutcome` with the given result for response-shape
    /// tests. Avoids calling `execute_import`, which needs a live DB.
    fn outcome_with(result: ImportResult) -> ImportOutcome {
        ImportOutcome {
            program_key: "prog:167358|11.0701|2024-2025|BS".to_string(),
            result,
            resolved_unitid: Some(167_358),
            variant: "full".to_string(),
            institution: Some("Test University".to_string()),
            variations_run: Some(137),
            sample_type: Some("shuffled".to_string()),
            courses_written: 5,
            requirements_written: 4,
            run_written: true,
            plans_written: 1,
            course_metrics_written: 0,
            conversion_warnings: vec![],
            messages: vec!["ok".to_string()],
        }
    }

    #[test]
    fn request_deserializes_with_lenient_scalars() {
        // Mirror the lenient deserializers used elsewhere: stringified unitid
        // and string booleans must coerce cleanly.
        let req: ImportDegreeRequest = serde_json::from_value(serde_json::json!({
            "json_content": "{}",
            "unitid": "167358",
            "dry_run": "true",
            "force": false,
        }))
        .expect("request must deserialize");
        assert_eq!(req.unitid, Some(167_358));
        assert_eq!(req.dry_run, Some(true));
        assert_eq!(req.force, Some(false));
        assert_eq!(req.json_content.as_deref(), Some("{}"));
    }

    #[test]
    fn request_defaults_optional_fields_to_none() {
        let req: ImportDegreeRequest =
            serde_json::from_value(serde_json::json!({ "json_path": "/tmp/r.json" }))
                .expect("request must deserialize");
        assert_eq!(req.json_path.as_deref(), Some("/tmp/r.json"));
        assert!(req.json_content.is_none());
        assert!(req.variant.is_none());
        assert!(req.unitid.is_none());
        assert!(req.dry_run.is_none());
    }

    #[test]
    fn resolve_report_text_requires_exactly_one_source() {
        assert!(resolve_report_text(None, None)
            .unwrap_err()
            .contains("Must provide exactly one"));
        assert!(
            resolve_report_text(Some("{}".to_string()), Some("/tmp/r.json".to_string()))
                .unwrap_err()
                .contains("not both")
        );
        assert_eq!(
            resolve_report_text(Some("{\"a\":1}".to_string()), None).unwrap(),
            "{\"a\":1}"
        );
    }

    #[test]
    fn updated_outcome_carries_counts_no_extra_payload() {
        let value = outcome_to_json(&outcome_with(ImportResult::Updated));
        assert_eq!(value["result"], "updated");
        assert_eq!(value["courses_written"], 5);
        assert!(value.get("reason").is_none());
        assert!(value.get("institution_candidates").is_none());
        assert!(value.get("errors").is_none());
    }

    #[test]
    fn skipped_outcome_carries_counts_no_extra_payload() {
        let value = outcome_to_json(&outcome_with(ImportResult::Skipped));
        assert_eq!(value["result"], "skipped");
        assert_eq!(value["courses_written"], 5);
        assert!(value.get("reason").is_none());
        assert!(value.get("institution_candidates").is_none());
        assert!(value.get("errors").is_none());
    }

    #[test]
    fn ambiguous_outcome_maps_to_institution_candidates() {
        let outcome = outcome_with(ImportResult::InstitutionAmbiguous(vec![
            (167_358, "Northeastern University".to_string()),
            (166_027, "Harvard University".to_string()),
        ]));
        let value = outcome_to_json(&outcome);
        assert_eq!(value["result"], "institution_ambiguous");
        let cands = value["institution_candidates"]
            .as_array()
            .expect("candidates array");
        assert_eq!(cands.len(), 2);
        assert_eq!(cands[0]["unitid"], 167_358);
        assert_eq!(cands[0]["name"], "Northeastern University");
        assert_eq!(cands[1]["unitid"], 166_027);
        // No reason/errors keys on this variant.
        assert!(value.get("reason").is_none());
        assert!(value.get("errors").is_none());
    }

    #[test]
    fn needs_confirmation_outcome_carries_reason() {
        let outcome = outcome_with(ImportResult::NeedsConfirmation(
            "verified program; pass --force to overwrite".to_string(),
        ));
        let value = outcome_to_json(&outcome);
        assert_eq!(value["result"], "needs_confirmation");
        assert_eq!(
            value["reason"],
            "verified program; pass --force to overwrite"
        );
        assert!(value.get("institution_candidates").is_none());
    }

    #[test]
    fn rejected_outcome_carries_errors() {
        let outcome = outcome_with(ImportResult::Rejected(vec![
            "bad thing".to_string(),
            "other bad thing".to_string(),
        ]));
        let value = outcome_to_json(&outcome);
        assert_eq!(value["result"], "rejected");
        let errors = value["errors"].as_array().expect("errors array");
        assert_eq!(errors.len(), 2);
        assert_eq!(errors[0], "bad thing");
    }

    #[test]
    fn created_outcome_carries_counts_and_no_extra_payload() {
        let outcome = outcome_with(ImportResult::Created);
        let value = outcome_to_json(&outcome);
        assert_eq!(value["result"], "created");
        assert_eq!(value["program_key"], "prog:167358|11.0701|2024-2025|BS");
        assert_eq!(value["resolved_unitid"], 167_358);
        assert_eq!(value["courses_written"], 5);
        assert_eq!(value["requirements_written"], 4);
        assert_eq!(value["run_written"], true);
        assert_eq!(value["plans_written"], 1);
        assert_eq!(value["variations_run"], 137);
        assert!(value.get("reason").is_none());
        assert!(value.get("institution_candidates").is_none());
        assert!(value.get("errors").is_none());
    }
}
