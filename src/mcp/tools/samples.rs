//! Sample-degree discovery tool.
//!
//! Provides the `list_sample_degrees` MCP tool that lists the bundled sample
//! degree YAMLs and (optionally) returns their content. The YAMLs are
//! embedded at compile time via `include_str!` so the tool keeps working
//! inside an installed binary — the `samples/` directory is not packaged
//! when consumers `cargo install` this crate.

use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Embedded sample bundle
// ============================================================================

/// CSU Fort Collins BS Computer Science (general track).
const SAMPLE_CSU: &str = include_str!("../../../samples/degrees/csu-cs-bscs-general.yaml");

/// Northeastern Khoury College BS Computer Science (Boston campus).
const SAMPLE_NEU: &str = include_str!("../../../samples/degrees/neu-khoury-bscs-boston.yaml");

/// University of Hawaii at Manoa BS Information & Computer Sciences.
const SAMPLE_UHM: &str = include_str!("../../../samples/degrees/uhm-ics-bscs-general.yaml");

/// Compile-time metadata for each bundled sample.
///
/// `key` is the model-facing handle (callers pass it to subsequent tools to
/// fetch the body). `institution` / `program` / `summary` are short strings
/// the model can pick from without having to parse YAML.
struct SampleMeta {
    key: &'static str,
    institution: &'static str,
    program: &'static str,
    catalog_year: &'static str,
    total_credits: u32,
    summary: &'static str,
    yaml: &'static str,
}

const SAMPLES: &[SampleMeta] = &[
    SampleMeta {
        key: "csu",
        institution: "Colorado State University - Fort Collins",
        program: "B.S. Computer Science (General)",
        catalog_year: "2024-2025",
        total_credits: 120,
        summary:
            "R1 public; CS major with calculus track + tech-elective pool; 65 modelled courses.",
        yaml: SAMPLE_CSU,
    },
    SampleMeta {
        key: "neu-khoury",
        institution: "Northeastern University",
        program: "B.S. Computer Science (Khoury, Boston)",
        catalog_year: "2024-2025",
        total_credits: 134,
        summary: "Private R1; co-op program; quarter-system schedule semantics with one_of tracks.",
        yaml: SAMPLE_NEU,
    },
    SampleMeta {
        // The YAML spells the name with diacritics ("Hawaiʻi", "Mānoa"); the
        // const mirrors that verbatim so model-facing display matches the
        // canonical institution name.
        key: "uhm",
        institution: "University of Hawaiʻi at Mānoa",
        program: "B.S. Information and Computer Sciences (General)",
        catalog_year: "2024-2025",
        total_credits: 120,
        summary: "R2 public; ICS major + math/science cores; smaller modelled corpus.",
        yaml: SAMPLE_UHM,
    },
];

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for `list_sample_degrees`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ListSampleDegreesRequest {
    /// When true, include the full embedded YAML body for each sample in the
    /// response. Default false — the metadata is enough for discovery and
    /// keeps the response compact (~1 KB vs ~50 KB).
    #[schemars(
        description = "Include the embedded YAML body for each sample. Default false; pass true when you want the full content in one call."
    )]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_bool"
    )]
    pub include_yaml: Option<bool>,
}

/// One bundled sample's metadata, optionally with its full YAML body.
#[derive(Debug, Serialize)]
pub struct SampleEntry {
    /// Short identifier the model can pass to subsequent calls.
    pub key: &'static str,
    /// Institution name.
    pub institution: &'static str,
    /// Program name + track / campus where applicable.
    pub program: &'static str,
    /// Catalog year string.
    pub catalog_year: &'static str,
    /// Total credits target from the YAML's `degree.total_credits`.
    pub total_credits: u32,
    /// One-line summary of the curriculum's characteristics.
    pub summary: &'static str,
    /// Full YAML body. Populated only when `include_yaml=true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub yaml_content: Option<&'static str>,
}

/// Response for `list_sample_degrees`.
#[derive(Debug, Serialize)]
pub struct ListSampleDegreesResponse {
    /// Total number of bundled samples.
    pub count: usize,
    /// One entry per bundled sample.
    pub samples: Vec<SampleEntry>,
    /// Follow-up hint for the model.
    pub note: &'static str,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute the `list_sample_degrees` tool.
#[must_use]
pub fn execute(include_yaml: bool) -> ListSampleDegreesResponse {
    let samples: Vec<SampleEntry> = SAMPLES
        .iter()
        .map(|s| SampleEntry {
            key: s.key,
            institution: s.institution,
            program: s.program,
            catalog_year: s.catalog_year,
            total_credits: s.total_credits,
            summary: s.summary,
            yaml_content: include_yaml.then_some(s.yaml),
        })
        .collect();
    ListSampleDegreesResponse {
        count: samples.len(),
        samples,
        note: "Call this tool again with include_yaml=true to receive the full YAML body, then feed yaml_content into validate_degree, audit_degree, analyze_degree, or generate_degree_report.",
    }
}

/// Execute and serialize as JSON.
#[must_use]
pub fn execute_json(include_yaml: bool) -> String {
    let response = execute(include_yaml);
    serde_json::to_string_pretty(&response)
        .unwrap_or_else(|e| format!("{{\"error\": \"Failed to serialize response: {e}\"}}"))
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_list_samples_returns_all_three_entries_without_yaml_by_default() {
        let response = execute(false);
        assert_eq!(response.count, 3);
        assert_eq!(response.samples.len(), 3);
        for entry in &response.samples {
            assert!(entry.yaml_content.is_none());
            assert!(!entry.institution.is_empty());
            assert!(!entry.program.is_empty());
            assert!(entry.total_credits > 0);
        }
        let keys: Vec<&str> = response.samples.iter().map(|s| s.key).collect();
        assert_eq!(keys, vec!["csu", "neu-khoury", "uhm"]);
    }

    #[test]
    fn test_list_samples_populates_yaml_when_include_yaml_true() {
        let response = execute(true);
        for entry in &response.samples {
            let body = entry
                .yaml_content
                .expect("include_yaml=true must populate yaml_content");
            // Sanity: the embedded YAML must contain the institution name + a
            // `degree:` block. Catches misconfigured include_str! paths at
            // test time rather than at runtime.
            assert!(
                body.contains("degree:"),
                "sample {} missing degree: block",
                entry.key
            );
            assert!(
                body.contains(entry.institution)
                    || body.contains(entry.institution.split(" - ").next().unwrap()),
                "sample {} body should reference its institution name",
                entry.key
            );
        }
    }

    #[test]
    fn test_execute_json_serializes_with_expected_keys() {
        let json = execute_json(false);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["count"].as_u64(), Some(3));
        assert!(parsed["samples"].is_array());
        assert!(parsed["note"].is_string());
    }
}
