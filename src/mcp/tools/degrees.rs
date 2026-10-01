//! `compare_degrees`: degrees side by side, with fresh or stored metrics.
//!
//! Each entry is any degree source — a stored program, a sample, a cache handle, inline, a
//! file — which the server loads as it loads the source of every other degree tool. This
//! module turns the loaded degrees into the comparison: identity from the degree itself,
//! then metrics — read from a stored program's newest stored run, or computed afresh for
//! any other source, unless `metrics` says otherwise. The degree text is not echoed back:
//! the caller already has it, and three whole documents would dwarf the comparison.

use rmcp::schemars;
use serde::Deserialize;

use crate::core::degree::parse_degree_auto;
use crate::mcp::tools::shared::DegreeSourceArgs;

/// Where a comparison's metrics come from. Omitted, each entry follows its source: a
/// stored program's stored run, a fresh enumeration for anything else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CompareMetrics {
    /// Enumerate each degree's plans now, stored programs included. Works for any source.
    Fresh,
    /// The newest stored run of `variant`. Reproducible and cheap; stored programs only.
    Stored,
    /// Identity only.
    None,
}

/// One degree to compare.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareSource {
    /// Name to show this degree under.
    #[schemars(description = "Label for this degree in the result (optional)")]
    pub label: Option<String>,
    /// Where the degree comes from: exactly one of `degree`, `content`, `path`.
    #[serde(flatten)]
    pub source: DegreeSourceArgs,
}

/// Request parameters for `compare_degrees`.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct CompareDegreesRequest {
    /// The degrees to compare.
    #[schemars(
        description = "The degrees to compare: each {label?, degree | content | path}, e.g. [{\"degree\": \"sample:csu\"}, {\"degree\": \"<program_key>\"}]"
    )]
    pub sources: Vec<CompareSource>,
    /// Where the metrics come from.
    #[schemars(
        description = "Omitted: each stored program's newest stored run, and a fresh enumeration for any other source. \"fresh\" enumerates every degree now, stored programs included; \"stored\" reads stored runs only (stored programs); \"none\" gives identity only."
    )]
    #[serde(default)]
    pub metrics: Option<CompareMetrics>,
    /// Stored metrics: which variant's run.
    #[schemars(
        description = "Stored metrics only: the run variant, \"full\" (default) or \"trimmed\""
    )]
    pub variant: Option<String>,
    /// Fresh metrics: cap on plans per degree.
    #[schemars(
        description = "Fresh metrics only: maximum plans to enumerate per degree. With a stored program among the sources it needs metrics=\"fresh\"."
    )]
    #[serde(default, deserialize_with = "crate::core::json::deserialize_opt_usize")]
    pub max_plans: Option<usize>,
}

/// A degree the server has loaded, ready to compare.
#[derive(Debug)]
pub struct LoadedDegree {
    /// The caller's label, if any.
    pub label: Option<String>,
    /// The degree's text.
    pub text: String,
    /// The stored program it is, when it is one.
    pub program_key: Option<String>,
    /// Where it came from, as the other tools report it.
    pub source: serde_json::Value,
}

/// Metrics computed afresh: the analysis pipeline's headline figures.
#[must_use]
pub fn fresh_metrics(yaml: &str, max_plans: Option<usize>) -> serde_json::Value {
    let response = crate::mcp::tools::analyze::execute(
        yaml,
        &crate::mcp::tools::analyze::AnalyzeOptions {
            max_plans,
            ..Default::default()
        },
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

impl CompareMetrics {
    /// The name the request and the response use.
    const fn name(self) -> &'static str {
        match self {
            Self::Fresh => "fresh",
            Self::Stored => "stored",
            Self::None => "none",
        }
    }
}

/// The comparison, as JSON.
///
/// `mode` omitted reads each stored program's stored run and enumerates every other source
/// afresh; each entry's `metrics_from` says which. `stored` returns a stored program's
/// newest run, or why there is none; it is called only for degrees that are stored
/// programs. One entry's missing metrics are reported in that entry, not as the call's
/// failure.
pub fn compare_json(
    degrees: Vec<LoadedDegree>,
    mode: Option<CompareMetrics>,
    max_plans: Option<usize>,
    stored: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> String {
    let entries: Vec<serde_json::Value> = degrees
        .into_iter()
        .map(|d| {
            let mut entry = identity(&d);
            let this = mode.unwrap_or_else(|| {
                if d.program_key.is_some() {
                    CompareMetrics::Stored
                } else {
                    CompareMetrics::Fresh
                }
            });
            match this {
                CompareMetrics::None => {}
                CompareMetrics::Fresh => entry["metrics"] = fresh_metrics(&d.text, max_plans),
                CompareMetrics::Stored => entry["metrics"] = stored_metrics(&d, stored),
            }
            if this != CompareMetrics::None {
                entry["metrics_from"] = this.name().into();
            }
            entry
        })
        .collect();
    crate::core::json::to_json_pretty(&serde_json::json!({
        "count": entries.len(),
        "metrics": mode.map_or("by_source", CompareMetrics::name),
        "degrees": entries,
    }))
}

/// A stored program's newest run, or `{"error": why}` when it has none.
fn stored_metrics(
    d: &LoadedDegree,
    stored: &dyn Fn(&str) -> Result<serde_json::Value, String>,
) -> serde_json::Value {
    let Some(key) = d.program_key.as_deref() else {
        return serde_json::json!({
            "error": "stored metrics need a stored program; use metrics=\"fresh\" for this one",
        });
    };
    stored(key).unwrap_or_else(|why| serde_json::json!({ "error": why }))
}

/// Who a degree is: label, source, and what it says about itself.
fn identity(d: &LoadedDegree) -> serde_json::Value {
    let parsed = parse_degree_auto(&d.text).ok().map(|(p, _)| p);
    let degree = parsed.as_ref().map(|p| &p.degree);
    serde_json::json!({
        "label": d.label.clone().or_else(|| degree.map(|g| g.name.clone())),
        "source": d.source,
        "program_key": d.program_key,
        "name": degree.map(|g| g.name.clone()),
        "institution": degree.and_then(|g| g.institution.clone()),
        "total_credits": degree.and_then(|g| g.total_credits),
        "course_count": parsed.as_ref().map(|p| p.courses.len()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fresh_metrics_returns_metrics_for_valid_yaml() {
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
        let value = fresh_metrics(yaml, Some(10));
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
    fn test_fresh_metrics_surfaces_parse_error_for_invalid_yaml() {
        // Failure mode the field report cared about: if a single bad YAML
        // shows up in compare_degrees, return its parse error inline so the
        // good degrees still come back with metrics.
        let value = fresh_metrics("not: valid: yaml: {{", None);
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

    fn loaded(text: &str, program_key: Option<&str>) -> LoadedDegree {
        LoadedDegree {
            label: None,
            text: text.to_string(),
            program_key: program_key.map(str::to_string),
            source: serde_json::json!({ "kind": "inline" }),
        }
    }

    #[test]
    fn a_stored_program_without_a_run_says_why_without_failing_the_call() {
        let yaml = include_str!("../../../samples/degrees/uhm-ics-bscs-general.yaml");
        let stored = |key: &str| -> Result<serde_json::Value, String> {
            Err(format!("no stored `full` run for {key}"))
        };
        let out: serde_json::Value = serde_json::from_str(&compare_json(
            vec![loaded(yaml, Some("prog:none"))],
            Some(CompareMetrics::Stored),
            None,
            &stored,
        ))
        .unwrap();
        assert_eq!(
            out["degrees"][0]["metrics"]["error"],
            "no stored `full` run for prog:none"
        );
        assert!(
            out.get("error").is_none(),
            "one entry's missing run must not become the call's failure"
        );
    }

    #[test]
    fn a_comparison_names_each_degree_and_takes_stored_metrics_only_for_stored_ones() {
        let yaml = include_str!("../../../samples/degrees/uhm-ics-bscs-general.yaml");
        let stored = |key: &str| Ok(serde_json::json!({ "run_key": format!("run-{key}") }));
        let out: serde_json::Value = serde_json::from_str(&compare_json(
            vec![loaded(yaml, Some("prog:1")), loaded(yaml, None)],
            Some(CompareMetrics::Stored),
            None,
            &stored,
        ))
        .expect("json");
        assert_eq!(out["count"], 2);
        assert_eq!(out["metrics"], "stored");
        assert_eq!(out["degrees"][0]["metrics"]["run_key"], "run-prog:1");
        assert!(out["degrees"][1]["metrics"]["error"]
            .as_str()
            .unwrap()
            .contains("fresh"));
        assert!(
            out["degrees"][0]["name"].is_string(),
            "identity comes from the degree"
        );
        assert!(
            out["degrees"][0].get("text").is_none(),
            "the degree is not echoed back"
        );
    }

    /// Omitted, `metrics` follows each source: the stored program is read from its stored
    /// run, never enumerated, and the inline degree is enumerated.
    #[test]
    fn metrics_omitted_reads_stored_programs_and_enumerates_the_rest() {
        let tiny = "degree: {id: t, institution: T, program: T, total_credits: 3, gpa_minimum: 2.0}\n\
                    requirements:\n  core: {name: Core, type: all, category: major, courses: [CS101]}\n\
                    courses:\n  CS101: {title: Intro, prefix: CS, number: \"101\", credits: 3}\n";
        let stored = |key: &str| Ok(serde_json::json!({ "run_key": format!("run-{key}") }));
        let out: serde_json::Value = serde_json::from_str(&compare_json(
            vec![loaded(tiny, Some("prog:1")), loaded(tiny, None)],
            None,
            None,
            &stored,
        ))
        .expect("json");
        assert_eq!(out["metrics"], "by_source");
        assert_eq!(out["degrees"][0]["metrics_from"], "stored");
        assert_eq!(out["degrees"][0]["metrics"]["run_key"], "run-prog:1");
        assert_eq!(out["degrees"][1]["metrics_from"], "fresh");
        assert_eq!(out["degrees"][1]["metrics"]["plans_analyzed"], 1);
    }

    #[test]
    fn metrics_none_gives_identity_only() {
        let yaml = include_str!("../../../samples/degrees/uhm-ics-bscs-general.yaml");
        let never = |_: &str| -> Result<serde_json::Value, String> {
            panic!("stored lookup must not run for metrics=none")
        };
        let out: serde_json::Value = serde_json::from_str(&compare_json(
            vec![loaded(yaml, None)],
            Some(CompareMetrics::None),
            None,
            &never,
        ))
        .expect("json");
        assert!(out["degrees"][0].get("metrics").is_none());
    }
}
