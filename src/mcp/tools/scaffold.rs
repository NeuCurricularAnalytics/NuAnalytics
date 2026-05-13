//! Degree YAML scaffolder.
//!
//! Provides the `scaffold_degree_yaml` MCP tool that pulls institution name
//! and CIP title from the IPEDS-backed database and emits a minimal degree
//! YAML skeleton ready for the caller to fill in.
//!
//! Minimal-header mode only: writes the `degree:` block plus empty
//! `requirements:` and `courses:` maps. No requirement scaffolding heuristics
//! — the caller adds those.

use std::sync::Arc;

use crate::core::database::models::{CipCode, Institution};
use crate::core::database::{tables, DbClient, QueryFilters};
use crate::mcp::tools::shared::{parse_first, to_json_pretty};
use rmcp::schemars;
use serde::{Deserialize, Serialize};

// ============================================================================
// Request / Response types
// ============================================================================

/// Request parameters for `scaffold_degree_yaml`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ScaffoldDegreeYamlRequest {
    /// IPEDS Unit ID of the institution.
    #[schemars(description = "IPEDS UNITID of the institution (e.g. 167358)")]
    #[serde(
        default,
        deserialize_with = "crate::mcp::tools::shared::deserialize_opt_i32"
    )]
    pub unitid: Option<i32>,

    /// CIP code in dot notation (e.g. `"11.0101"`).
    #[schemars(description = "CIP code in dot notation (e.g. \"11.0101\")")]
    pub cip_code: Option<String>,

    /// Catalog year string (e.g. `"2024-2025"`). Used for the slug + the
    /// `catalog_year` field in the YAML; falls back to `"TBD"` when omitted.
    #[schemars(description = "Catalog year (e.g. \"2024-2025\"). Optional.")]
    pub catalog_year: Option<String>,

    /// Calendar system to inject as `system_type`. Defaults to `"semester"`
    /// because the IPEDS schema currently exposed by this tree does not carry
    /// a `calsys` column, and the caller knows their institution.
    #[schemars(description = "Calendar system: \"semester\" (default) or \"quarter\".")]
    pub system_type: Option<String>,
}

/// Source-data echo so the caller can confirm the IPEDS lookup landed.
#[derive(Debug, Serialize)]
pub struct ScaffoldSource {
    /// Institution name pulled from IPEDS.
    pub institution_name: String,
    /// CIP title pulled from the `cip_codes` table.
    pub cip_title: String,
    /// IPEDS Unit ID (echo).
    pub unitid: i32,
    /// CIP code (echo).
    pub cip_code: String,
    /// Calendar system used in the emitted YAML.
    pub system_type: String,
}

/// Response for `scaffold_degree_yaml`.
#[derive(Debug, Serialize)]
pub struct ScaffoldDegreeYamlResponse {
    /// True when both lookups succeeded and the YAML was generated.
    pub success: bool,
    /// Error message when `success` is false.
    pub error: Option<String>,
    /// Auto-generated slug (the YAML's `degree.id`).
    pub degree_id: Option<String>,
    /// The minimal YAML body, ready for the caller to extend.
    pub yaml_content: Option<String>,
    /// Echoed inputs + the resolved IPEDS / CIP titles.
    pub source: Option<ScaffoldSource>,
    /// Follow-up hints for the model.
    pub suggestions: Vec<String>,
}

// ============================================================================
// Execution
// ============================================================================

/// Execute `scaffold_degree_yaml` and return the structured response.
pub async fn execute(
    client: &Arc<DbClient>,
    req: ScaffoldDegreeYamlRequest,
) -> ScaffoldDegreeYamlResponse {
    let Some(unitid) = req.unitid else {
        return ScaffoldDegreeYamlResponse::error("Missing required field `unitid`.");
    };
    let Some(cip_code) = req.cip_code.as_deref() else {
        return ScaffoldDegreeYamlResponse::error("Missing required field `cip_code`.");
    };
    let catalog_year = req.catalog_year.as_deref();
    let system_type = req.system_type.as_deref().unwrap_or("semester");

    let institution = match fetch_institution(client, unitid).await {
        Ok(Some(i)) => i,
        Ok(None) => {
            return ScaffoldDegreeYamlResponse::error(format!(
                "No institution found for UNITID {unitid}. Use search_institutions to find a valid ID."
            ));
        }
        Err(e) => return ScaffoldDegreeYamlResponse::error(e),
    };

    let cip = match fetch_cip_code(client, cip_code).await {
        Ok(Some(c)) => c,
        Ok(None) => {
            return ScaffoldDegreeYamlResponse::error(format!(
                "No CIP entry found for {cip_code:?}. Use search_cip_codes to find a valid code."
            ));
        }
        Err(e) => return ScaffoldDegreeYamlResponse::error(e),
    };

    let inst_slug = institution_slug(&institution.name);
    let cip_slug = cip_family_slug(&cip.cip_code);
    let year_slug = year_slug(catalog_year);
    let base_slug = format!("{inst_slug}-{cip_slug}-bscs-{year_slug}");
    let degree_id = ensure_unique_slug(client, &base_slug).await;

    let yaml_content = render_yaml(
        &degree_id,
        &institution.name,
        &cip.title,
        catalog_year,
        system_type,
        &cip.cip_code,
    );

    let suggestions = vec![
        "Fill in `major_subjects` with the subject prefixes used in this program (e.g. [\"CS\"]).".to_string(),
        "Call `get_degree_schema(section=\"quickstart\")` for a worked example of requirements + courses.".to_string(),
        "When the YAML is ready, run `validate_degree` to catch structural errors before analysis.".to_string(),
    ];

    ScaffoldDegreeYamlResponse {
        success: true,
        error: None,
        degree_id: Some(degree_id),
        yaml_content: Some(yaml_content),
        source: Some(ScaffoldSource {
            institution_name: institution.name,
            cip_title: cip.title,
            unitid,
            cip_code: cip.cip_code,
            system_type: system_type.to_string(),
        }),
        suggestions,
    }
}

/// Execute and serialize as JSON.
pub async fn execute_json(client: &Arc<DbClient>, req: ScaffoldDegreeYamlRequest) -> String {
    let response = execute(client, req).await;
    to_json_pretty(&response)
}

// ============================================================================
// Helpers
// ============================================================================

impl ScaffoldDegreeYamlResponse {
    fn error(msg: impl Into<String>) -> Self {
        Self {
            success: false,
            error: Some(msg.into()),
            degree_id: None,
            yaml_content: None,
            source: None,
            suggestions: Vec::new(),
        }
    }
}

async fn fetch_institution(
    client: &Arc<DbClient>,
    unitid: i32,
) -> Result<Option<Institution>, String> {
    let filters = QueryFilters::new().eq("unitid", Some(unitid));
    let value = client
        .select(tables::INSTITUTIONS, "*", &filters, Some(1))
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_first::<Institution>(&value))
}

async fn fetch_cip_code(client: &Arc<DbClient>, cip_code: &str) -> Result<Option<CipCode>, String> {
    let filters = QueryFilters::new().eq("cip_code", Some(cip_code));
    let value = client
        .select(tables::CIP_CODES, "cip_code,title", &filters, Some(1))
        .await
        .map_err(|e| e.to_string())?;
    Ok(parse_first::<CipCode>(&value))
}

/// Append `-2`, `-3`, … until `base` doesn't collide with a stored degree.
/// Best-effort: probes up to 9 suffixes; falls back to the base slug if
/// every probe fails (the caller can rename the YAML manually).
async fn ensure_unique_slug(client: &Arc<DbClient>, base: &str) -> String {
    if !slug_is_taken(client, base).await {
        return base.to_string();
    }
    for n in 2..=9 {
        let candidate = format!("{base}-{n}");
        if !slug_is_taken(client, &candidate).await {
            return candidate;
        }
    }
    base.to_string()
}

async fn slug_is_taken(client: &Arc<DbClient>, slug: &str) -> bool {
    let filters = QueryFilters::new().eq("degree_id", Some(slug));
    let Ok(value) = client
        .select(tables::DEGREES, "degree_id", &filters, Some(1))
        .await
    else {
        return false;
    };
    value.as_array().is_some_and(|arr| !arr.is_empty())
}

/// Lowercase initialism of capitalized words in `name`, dropping common
/// stop-words. Falls back to the first 4 lowercase chars when no initials
/// can be extracted (e.g. all-lowercase names).
fn institution_slug(name: &str) -> String {
    const STOP_WORDS: &[&str] = &["of", "at", "the", "and", "for", "in", "on", "to", "a", "an"];

    let initials: String = name
        .split_whitespace()
        .filter(|word| {
            !STOP_WORDS.contains(&word.to_lowercase().as_str())
                && word.chars().next().is_some_and(char::is_uppercase)
        })
        .filter_map(|word| word.chars().next())
        .map(|c| c.to_ascii_lowercase())
        .collect();

    if initials.len() >= 2 {
        return initials;
    }

    // Fall back to the first 4 lowercase alphabetic chars from `name`.
    name.chars()
        .filter(char::is_ascii_alphabetic)
        .take(4)
        .collect::<String>()
        .to_lowercase()
}

/// Map a CIP code to a short family slug. Hits a small built-in table for
/// the common cases; otherwise returns the CIP code with dots stripped.
fn cip_family_slug(cip_code: &str) -> String {
    // Sorted longest-prefix-first so "30.70" wins over "30.".
    const FAMILY_TABLE: &[(&str, &str)] = &[
        ("30.70", "dsci"),
        ("30.71", "dsci"),
        ("11.", "cs"),
        ("14.", "eng"),
        ("27.", "math"),
        ("52.", "bus"),
        ("13.", "edu"),
        ("26.", "bio"),
        ("40.", "phys"),
        ("42.", "psy"),
        ("45.", "ssci"),
    ];
    for (prefix, slug) in FAMILY_TABLE {
        if cip_code.starts_with(prefix) {
            return (*slug).to_string();
        }
    }
    cip_code.replace('.', "")
}

/// Derive a year segment from `catalog_year`. Accepts `"2024-2025"`
/// (returns `"2024"`), `"2024"` (returns as-is), or `None` → `"TBD"`.
fn year_slug(catalog_year: Option<&str>) -> String {
    catalog_year.map_or_else(
        || "TBD".to_string(),
        |y| {
            y.split('-')
                .next()
                .filter(|s| !s.is_empty())
                .unwrap_or("TBD")
                .to_string()
        },
    )
}

/// Build the minimal YAML body as a hand-rolled string template.
///
/// We deliberately don't pipe through `serde_yaml` here because the output
/// embeds caller-facing `# TODO: …` comments (placeholder hints for
/// `total_credits`, `major_subjects`) and matches the formatting style of
/// the existing sample YAMLs — both of which a serializer would erase.
/// Every interpolated field is either an internal constant or sourced from
/// IPEDS/CIP lookups, so there's no untrusted input to escape; `{:?}`
/// formatting handles the inline string quoting.
fn render_yaml(
    degree_id: &str,
    institution: &str,
    program: &str,
    catalog_year: Option<&str>,
    system_type: &str,
    cip_code: &str,
) -> String {
    let catalog = catalog_year.unwrap_or("TBD");
    format!(
        "degree:\n  \
         id: {degree_id}\n  \
         institution: {institution:?}\n  \
         program: {program:?}\n  \
         catalog_year: {catalog:?}\n  \
         cip_code: {cip_code:?}\n  \
         system_type: {system_type:?}\n  \
         total_credits: 120          # TODO: verify against the program catalog\n  \
         gpa_minimum: 2.0\n  \
         major_subjects: []           # TODO: fill in subject prefixes used by this major\n  \
         allow_double_counting: false\n\
         \n\
         # Requirements describe how courses combine into degree groups\n\
         # (intro sequence, electives, gen-eds…). See get_degree_schema(section=\"quickstart\")\n\
         # for the supported types: `all`, `select`, `one_of`.\n\
         requirements: {{}}\n\
         \n\
         # Course catalog. Each entry needs title, prefix, number, credits;\n\
         # add `prerequisites_raw` for prerequisite expressions.\n\
         courses: {{}}\n"
    )
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_institution_slug_uses_capitalized_initials() {
        assert_eq!(institution_slug("Colorado State University"), "csu");
        assert_eq!(institution_slug("Northeastern University"), "nu");
        assert_eq!(institution_slug("University of Hawaii at Manoa"), "uhm");
    }

    #[test]
    fn test_institution_slug_drops_stop_words() {
        assert_eq!(institution_slug("School of the Arts"), "sa");
    }

    #[test]
    fn test_institution_slug_fallback_when_no_capitals() {
        // Lowercase name → first 4 alphabetic chars in source order (spaces
        // and other non-alpha skipped); the limit hits inside the first word.
        assert_eq!(institution_slug("some little college"), "some");
    }

    #[test]
    fn test_institution_slug_falls_back_when_every_word_is_a_stop_word() {
        // "The And Of" — every capitalized word is filtered out, leaving the
        // initialism empty. The function must fall back to the first-4-alpha
        // rule rather than emit a zero-length slug.
        assert_eq!(institution_slug("The And Of"), "thea");
    }

    #[test]
    fn test_cip_family_slug_uses_table_for_common_families() {
        assert_eq!(cip_family_slug("11.0101"), "cs");
        assert_eq!(cip_family_slug("11.0701"), "cs");
        assert_eq!(cip_family_slug("14.0901"), "eng");
        assert_eq!(cip_family_slug("27.0101"), "math");
        // 30.70 wins over the (absent) 30. catch-all.
        assert_eq!(cip_family_slug("30.7001"), "dsci");
    }

    #[test]
    fn test_cip_family_slug_falls_back_to_stripped_code() {
        // 99.x isn't in the table → dotless code.
        assert_eq!(cip_family_slug("99.0101"), "990101");
    }

    #[test]
    fn test_year_slug_extracts_first_year_from_range() {
        assert_eq!(year_slug(Some("2024-2025")), "2024");
        assert_eq!(year_slug(Some("2024")), "2024");
        assert_eq!(year_slug(None), "TBD");
        // Edge case: empty string after split → fall back.
        assert_eq!(year_slug(Some("-2025")), "TBD");
    }

    #[test]
    fn test_render_yaml_emits_tbd_when_catalog_year_is_none() {
        // The scaffold must produce a syntactically valid YAML body even
        // when the caller omits the catalog year. The `catalog_year` field
        // should fall back to the literal "TBD" placeholder.
        let yaml = render_yaml(
            "csu-cs-bscs-TBD",
            "Colorado State University",
            "Computer and Information Sciences, General.",
            None,
            "semester",
            "11.0101",
        );
        assert!(
            yaml.contains("catalog_year: \"TBD\""),
            "render_yaml must emit catalog_year: \"TBD\" when catalog_year is None"
        );
    }

    #[test]
    fn test_render_yaml_includes_required_header_fields() {
        let yaml = render_yaml(
            "csu-cs-bscs-2024",
            "Colorado State University",
            "Computer and Information Sciences, General.",
            Some("2024-2025"),
            "semester",
            "11.0101",
        );
        assert!(yaml.contains("id: csu-cs-bscs-2024"));
        assert!(yaml.contains("Colorado State University"));
        assert!(yaml.contains("Computer and Information Sciences"));
        assert!(yaml.contains("catalog_year: \"2024-2025\""));
        assert!(yaml.contains("cip_code: \"11.0101\""));
        assert!(yaml.contains("system_type: \"semester\""));
        assert!(yaml.contains("requirements: {}"));
        assert!(yaml.contains("courses: {}"));
    }

    #[test]
    fn test_error_response_factory_zeros_payload_fields() {
        let r = ScaffoldDegreeYamlResponse::error("nope");
        assert!(!r.success);
        assert_eq!(r.error.as_deref(), Some("nope"));
        assert!(r.degree_id.is_none());
        assert!(r.yaml_content.is_none());
        assert!(r.source.is_none());
        assert!(r.suggestions.is_empty());
    }
}
