//! Fidelity guard: parsing a unified degree and serializing it back must not
//! silently drop fields.
//!
//! This exists because it did. Until 2026-09-23 the models had no field for
//! `external_requirement`/`external_credits`/`external_note` (2,082 requirement nodes
//! across 508 of the 1,088 corpus degrees), `conversion_warnings` (12,428 entries),
//! course `grade_minimum` (520 courses), or several degree-level provenance strings.
//! They vanished at parse, so they were already absent from every analysis report and
//! could never reach the database — whose `programs.document` column is nonetheless
//! documented as the lossless source of truth.
//!
//! A key present in the source and absent after a round trip fails here. Values are not
//! compared: `prerequisites` is deliberately re-expressed (structured tree -> boolean
//! string -> structured tree), and serde omits `None`, so an explicit `null` in the
//! source legitimately disappears.

use nu_analytics::core::degree::{parse_degree_json, to_unified_value};
use serde_json::Value;
use std::collections::BTreeSet;

use super::degree_fixtures::{
    ASU, BELLEVUE, BOWDOIN, CALSTATELA, COC, LIBERTY, METRO, NMSU, RIC, SYRACUSE, TULANE, TXSTATE,
    WKU,
};

/// Key paths in a JSON document, with map keys under `courses` / `requirements`
/// collapsed to `*` so per-degree course codes do not swamp the comparison.
///
/// `options` is deliberately *not* collapsed: it is an array, not a map, so collapsing on
/// the key name rewrote each `RequirementOption`'s own fields to `.../options[]/*` and
/// never recorded them — `RequirementOption::id`, which feeds
/// `StoredProgramRequirement::option_id`, was invisible to this guard.
///
/// Returns `(all, carrying_a_value)`. The second set omits paths whose every occurrence
/// is an explicit `null`: serde writes nothing for `None`, so a source `null` vanishing
/// is correct, not a dropped field.
///
/// Tracked during the walk rather than re-resolved afterwards. Re-resolving cannot work:
/// a collapsed path has no pointer into the document, so every explicit `null` under
/// `courses` or `requirements` — most of the document — would be reported as a loss.
fn key_paths(v: &Value) -> (BTreeSet<String>, BTreeSet<String>) {
    fn rec(
        v: &Value,
        path: &str,
        parent: Option<&str>,
        all: &mut BTreeSet<String>,
        valued: &mut BTreeSet<String>,
    ) {
        const COLLAPSE: [&str; 2] = ["courses", "requirements"];
        match v {
            Value::Object(map) => {
                for (k, child) in map {
                    if parent.is_some_and(|p| COLLAPSE.contains(&p)) {
                        let key = format!("{path}/*");
                        rec(child, &key, Some("*"), all, valued);
                    } else {
                        let key = format!("{path}/{k}");
                        all.insert(key.clone());
                        if !child.is_null() {
                            valued.insert(key.clone());
                        }
                        rec(child, &key, Some(k), all, valued);
                    }
                }
            }
            Value::Array(items) => {
                let key = format!("{path}[]");
                for item in items {
                    // An array element is not a map entry, so it is never collapsible;
                    // passing `parent` through is what hid the option fields.
                    rec(item, &key, None, all, valued);
                }
            }
            _ => {}
        }
    }
    let (mut all, mut valued) = (BTreeSet::new(), BTreeSet::new());
    rec(v, "", None, &mut all, &mut valued);
    (all, valued)
}

/// Keys the round trip is allowed to lose, with the reason.
fn is_expected_loss(path: &str) -> bool {
    // The structured prerequisite tree is re-expressed through `prerequisites_raw` and
    // rebuilt on the way out, so its interior shape legitimately differs.
    path.contains("/prerequisites")
}

#[test]
fn parsing_and_reserializing_a_degree_keeps_every_field() {
    let fixtures: [(&str, &str); 13] = [
        ("tulane", TULANE),
        ("coc", COC),
        ("bowdoin", BOWDOIN),
        ("nmsu", NMSU),
        ("liberty", LIBERTY),
        ("ric", RIC),
        ("calstatela", CALSTATELA),
        ("metro", METRO),
        ("wku", WKU),
        ("txstate", TXSTATE),
        ("asu", ASU),
        ("syracuse", SYRACUSE),
        ("bellevue", BELLEVUE),
    ];

    let mut report = Vec::new();
    for (name, body) in fixtures {
        let source: Value = serde_json::from_str(body).expect("fixture is valid JSON");
        let program = parse_degree_json(body).expect("fixture parses as a unified degree");
        let round_tripped = to_unified_value(&program).expect("program re-serializes");

        // A field check cannot see a *node* vanish: with map keys collapsed to `*`, 647
        // of the 653 fixture courses could be dropped individually without changing the
        // key set at all. Parse skipping an entry it does not recognise is the same class
        // of loss this guard exists for, so count them.
        assert_eq!(
            program.courses.len(),
            source["courses"]
                .as_object()
                .map_or(0, serde_json::Map::len),
            "{name}: parse dropped whole course entries"
        );
        assert_eq!(
            program.requirements.len(),
            source["requirements"]
                .as_object()
                .map_or(0, serde_json::Map::len),
            "{name}: parse dropped whole requirement entries"
        );

        // A key counts as lost only if the source held a non-null value for it.
        let (after, _) = key_paths(&round_tripped);
        let (_, source_valued) = key_paths(&source);
        for path in source_valued {
            if after.contains(&path) || is_expected_loss(&path) {
                continue;
            }
            report.push(format!("  {name}: {path}"));
        }
    }
    assert!(
        report.is_empty(),
        "fields present in the source were dropped by parse -> serialize:\n{}",
        report.join("\n")
    );
}

/// The vendored fixtures cover the high-volume fields — `external_*` (41 nodes across 8
/// of them) and `conversion_warnings` (all 13) — but not the rare ones: course-level
/// `grade_minimum`, `source_url_final`, `needs_review`, `gpa_minimum_note` and
/// `corrections_applied` appear 520/15/1/1/1 times in a 1,088-degree corpus and in none
/// of the 13. Note `grade_minimum` exists on both `degree` and a course; only the degree
/// one is in the fixtures, so a by-name search over them looks like coverage and is not.
const RARE_FIELDS: &str = r#"{
  "degree": {
    "name": "Synthetic", "degree_type": "bs", "system_type": "semester",
    "institution": "T", "total_credits": 3,
    "source_url": "https://example.edu/a",
    "source_url_final": "https://example.edu/b",
    "needs_review": true,
    "gpa_minimum": 3.0,
    "gpa_minimum_note": "3.0 across all courses used for the certificate"
  },
  "requirements": {
    "core": { "type": "all", "name": "Core", "courses": ["CS101"] }
  },
  "courses": {
    "CS101": { "name": "Intro", "prefix": "CS", "number": "101",
               "credit_hours": 3, "grade_minimum": "C-" }
  },
  "conversion_warnings": ["cip_code inferred from program name"],
  "corrections_applied": ["restored a truncated selection pool"]
}"#;

#[test]
fn the_rare_provenance_fields_also_survive_a_round_trip() {
    let source: Value = serde_json::from_str(RARE_FIELDS).expect("valid JSON");
    let program = parse_degree_json(RARE_FIELDS).expect("parses as a unified degree");
    let round_tripped = to_unified_value(&program).expect("re-serializes");

    let (after, _) = key_paths(&round_tripped);
    let (_, source_valued) = key_paths(&source);
    let lost: Vec<String> = source_valued
        .into_iter()
        .filter(|p| !after.contains(p) && !is_expected_loss(p))
        .collect();
    assert!(lost.is_empty(), "dropped by parse -> serialize: {lost:?}");

    // Spot-check values, not just key presence: a field can survive as the wrong thing.
    assert_eq!(
        program.courses["CS101"].grade_minimum.as_deref(),
        Some("C-"),
        "course grade_minimum must keep the catalog's wording"
    );
    assert_eq!(program.degree.needs_review, Some(true));
    assert_eq!(
        program.degree.source_url_final.as_deref(),
        Some("https://example.edu/b")
    );
    assert_eq!(program.conversion_warnings.len(), 1);
    assert_eq!(program.corrections_applied.len(), 1);
}
