//! Where a target course lands across a degree's enumerated plan population.
//!
//! `analyze_degree` is asked for a specific course in a specific degree; the answer is
//! checked against a recorded baseline plus the invariants `target_course_stats` is
//! documented to hold (`TargetCourseStats`, `src/mcp/tools/analyze.rs`).
//!
//! Companion module: `target_course_selected_plans` covers the boundary between these
//! population figures and the handful of plans `selected_plans` surfaces.
//!
//! # Why only `earliest_term` is baselined
//!
//! Plan enumeration is seeded (`build_artifacts` passes its seed to the generator), so a
//! run is reproducible — `analyze::tests::test_build_artifacts_is_reproducible_for_identical_inputs`
//! pins that directly, cache-free. But with `random_seed: None` the seed is derived via
//! `DefaultHasher`, whose output std does not guarantee stable across toolchains. A
//! different seed enumerates a different sample of the plan space, which moves
//! `plans_containing`, `avg_term` and `term_distribution`.
//!
//! `earliest_term` survives that: the generator always enumerates the extreme plans
//! first, so the minimum is observed whatever the seed. It is therefore the one figure
//! baselined below; the other three are checked for internal consistency instead.
//!
//! These baselines characterise current behaviour on these fixtures — they are not an
//! independent oracle. See `tests/assets/degrees/Readme.md`.

use super::degree_fixtures::{
    bundled_sample, target_stats, ASU, BELLEVUE, BOWDOIN, CALSTATELA, COC, LIBERTY, MAX_PLANS,
    METRO, NMSU, RIC, SYRACUSE, TULANE, TXSTATE, WKU,
};
use nu_analytics::mcp::tools::analyze::{TargetCourseStats, TargetTermStats};
use std::sync::LazyLock;

/// A course code none of these degrees uses, for the not-found path.
const ABSENT_COURSE: &str = "ZZZ9999";

/// One probe: which degree, which course, and the earliest term it is placed in.
struct Case {
    /// Short institution label, used in assertion messages.
    label: &'static str,
    /// Vendored degree JSON.
    degree_json: &'static str,
    /// Course to locate.
    course: &'static str,
    /// Recorded `all_plans.earliest_term`. See the module docs for what this is, and is
    /// not, evidence of.
    earliest_term: usize,
}

// Baselines re-recorded 2026-09-23 when the OR-group DAG defect was fixed: the MCP path
// had been adding an edge for *every* in-plan option of an OR-group, so courses carried
// prerequisites the degree never required. Three of twenty moved — Bowdoin CSCI3465 6->4,
// Liberty CSCN354 9->6, Liberty CSIS316 2->6.
//
// The last one moved *later*, which looks wrong for a change that only removes
// constraints. It is not: `term_scheduler` balances credits across terms (~15 a
// semester) rather than scheduling each course as early as its prerequisites allow. Once
// other courses are freed to move earlier they consume capacity, and CSIS316 is packed
// into a later term. Term placement is a packing, so it is not monotonic in the
// prerequisite set.
const CASES: &[Case] = &[
    Case {
        label: "Tulane",
        degree_json: TULANE,
        course: "CMPS2200",
        earliest_term: 3,
    },
    Case {
        label: "Tulane",
        degree_json: TULANE,
        course: "CMPS3340",
        earliest_term: 2,
    },
    Case {
        label: "CoC",
        degree_json: COC,
        course: "CSCI218",
        earliest_term: 6,
    },
    Case {
        label: "CoC",
        degree_json: COC,
        course: "CSCI495",
        earliest_term: 5,
    },
    Case {
        label: "Bowdoin",
        degree_json: BOWDOIN,
        course: "CSCI2101",
        earliest_term: 2,
    },
    Case {
        label: "Bowdoin",
        degree_json: BOWDOIN,
        course: "CSCI3465",
        earliest_term: 4,
    },
    Case {
        label: "NMSU",
        degree_json: NMSU,
        course: "CSCI2220",
        earliest_term: 3,
    },
    Case {
        label: "NMSU",
        degree_json: NMSU,
        course: "CSCI4270",
        earliest_term: 6,
    },
    Case {
        label: "Liberty",
        degree_json: LIBERTY,
        course: "CSIS316",
        earliest_term: 6,
    },
    Case {
        label: "Liberty",
        degree_json: LIBERTY,
        course: "CSCN354",
        earliest_term: 6,
    },
    Case {
        label: "RIC",
        degree_json: RIC,
        course: "DATA245",
        earliest_term: 3,
    },
    Case {
        label: "RIC",
        degree_json: RIC,
        course: "CSCI446",
        earliest_term: 5,
    },
    Case {
        label: "CalStateLA",
        degree_json: CALSTATELA,
        course: "CS4963",
        earliest_term: 7,
    },
    Case {
        label: "Metro",
        degree_json: METRO,
        course: "CS4050",
        earliest_term: 5,
    },
    Case {
        label: "WKU",
        degree_json: WKU,
        course: "STAT402",
        earliest_term: 6,
    },
    Case {
        label: "TxState",
        degree_json: TXSTATE,
        course: "CS4398",
        earliest_term: 7,
    },
    Case {
        label: "ASU",
        degree_json: ASU,
        course: "MAT266",
        earliest_term: 2,
    },
    Case {
        label: "ASU",
        degree_json: ASU,
        course: "DAT402",
        earliest_term: 7,
    },
    Case {
        label: "Syracuse",
        degree_json: SYRACUSE,
        course: "MAT397",
        earliest_term: 3,
    },
    Case {
        label: "Bellevue",
        degree_json: BELLEVUE,
        course: "AI240",
        earliest_term: 3,
    },
];

/// Every case analyzed once, shared across the tests below.
///
/// Each entry enumerates up to `MAX_PLANS` plans. The library's own artifact cache holds
/// only `ARTIFACT_CACHE_CAPACITY` (4) entries against these 20+ distinct keys, so without
/// this every test would pay full price again.
static PROBES: LazyLock<Vec<TargetCourseStats>> = LazyLock::new(|| {
    CASES
        .iter()
        .map(|c| target_stats(c.label, c.degree_json, MAX_PLANS, c.course))
        .collect()
});

fn cases() -> impl Iterator<Item = (&'static Case, &'static TargetCourseStats)> {
    CASES.iter().zip(PROBES.iter())
}

/// Mean of a term distribution, weighted by how many plans landed on each term.
fn weighted_mean(stats: &TargetTermStats) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    let total: f64 = stats
        .term_distribution
        .iter()
        .map(|(term, count)| (term * count) as f64)
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let count = stats.plans_containing as f64;
    total / count
}

#[test]
fn every_target_course_is_reported_as_found() {
    for (case, stats) in cases() {
        let who = format!("{} {}", case.label, case.course);
        assert!(
            stats.error.is_none(),
            "{who}: course should be reachable in this degree, got error {:?}",
            stats.error
        );
        assert_eq!(
            stats.course_id, case.course,
            "{who}: the requested course id must be echoed back"
        );
        // Guards the response shape rather than the analysis: `earliest_term` is
        // `skip_serializing_if`, so a found course must not serialise it away.
        assert!(
            stats.all_plans.plans_containing > 0,
            "{who}: no error reported, so at least one plan must contain the course"
        );
        assert!(
            stats.all_plans.earliest_term.is_some(),
            "{who}: {} plans contain the course, so earliest_term must be set",
            stats.all_plans.plans_containing
        );
    }
}

#[test]
fn earliest_term_matches_recorded_baseline() {
    // Checked before the comparison: `filter_map`ping a missing earliest_term away would
    // let a regression that made every course unfindable report zero drift.
    let missing: Vec<&str> = cases()
        .filter(|(_, stats)| stats.all_plans.earliest_term.is_none())
        .map(|(case, _)| case.course)
        .collect();
    assert!(
        missing.is_empty(),
        "no earliest_term recorded for {missing:?}, so the baseline comparison below \
         would pass without checking anything"
    );

    let drifted: Vec<String> = cases()
        .filter(|(case, stats)| stats.all_plans.earliest_term != Some(case.earliest_term))
        .map(|(case, stats)| {
            format!(
                "  {} {}: baseline {}, got {:?}",
                case.label, case.course, case.earliest_term, stats.all_plans.earliest_term
            )
        })
        .collect();
    assert!(
        drifted.is_empty(),
        "earliest_term drifted from the recorded baseline in {} of {} cases:\n{}\n\n\
         If the change was intended, re-record the baselines and say why in the commit.",
        drifted.len(),
        CASES.len(),
        drifted.join("\n")
    );
}

#[test]
fn term_distribution_agrees_with_its_summary_stats() {
    for (case, probe) in cases() {
        let who = format!("{} {}", case.label, case.course);
        let stats = &probe.all_plans;
        let counted: usize = stats.term_distribution.values().sum();
        assert_eq!(
            counted, stats.plans_containing,
            "{who}: term_distribution accounts for {counted} plans but plans_containing is {}",
            stats.plans_containing
        );

        let lowest = stats
            .term_distribution
            .keys()
            .next()
            .copied()
            .unwrap_or_else(|| {
                panic!(
                    "{who}: term_distribution is empty but plans_containing is {}",
                    stats.plans_containing
                )
            });
        assert_eq!(
            stats.earliest_term,
            Some(lowest),
            "{who}: earliest_term must be the lowest term in term_distribution"
        );

        // Equality with the distribution-weighted mean, not just membership in the
        // min..=max band, which any mean satisfies by construction.
        let avg = stats.avg_term.unwrap_or_else(|| {
            panic!("{who}: avg_term must be set alongside earliest_term");
        });
        let expected = weighted_mean(stats);
        assert!(
            (avg - expected).abs() < 1e-9,
            "{who}: avg_term {avg} does not match the distribution-weighted mean {expected}"
        );
    }
}

#[test]
fn calc_ready_plans_is_a_subset_of_all_plans() {
    // Holds trivially for the vendored fixtures — see
    // `calc_ready_plans_are_populated_when_the_degree_uses_recognised_calculus_codes`
    // for why, and for the case that exercises a non-empty slice.
    for (case, probe) in cases() {
        let who = format!("{} {}", case.label, case.course);
        let (all, calc) = (&probe.all_plans, &probe.calc_ready_plans);
        assert!(
            calc.plans_containing <= all.plans_containing,
            "{who}: calc_ready_plans ({}) cannot exceed all_plans ({})",
            calc.plans_containing,
            all.plans_containing
        );
        for term in calc.term_distribution.keys() {
            assert!(
                all.term_distribution.contains_key(term),
                "{who}: calc_ready_plans reports term {term}, which is absent from all_plans"
            );
        }
        if let (Some(calc_earliest), Some(all_earliest)) = (calc.earliest_term, all.earliest_term) {
            assert!(
                calc_earliest >= all_earliest,
                "{who}: calc-ready earliest_term {calc_earliest} precedes the all-plans \
                 minimum {all_earliest}"
            );
        }
        assert_eq!(
            calc.earliest_term.is_some(),
            calc.plans_containing > 0,
            "{who}: calc_ready_plans earliest_term and plans_containing disagree on whether \
             the slice is empty"
        );
    }
}

#[test]
fn calc_ready_plans_are_populated_when_the_degree_uses_recognised_calculus_codes() {
    // None of the vendored fixtures can exercise a non-empty calc-ready slice:
    // `PlanSelectorConfig::default().calculus_courses` lists only Northeastern and
    // Colorado State course codes, and it is matched against course *ids*. The bundled
    // CSU sample uses those codes, so it is the input that reaches this path.
    let csu = bundled_sample("csu-cs-bscs-general.yaml");
    let stats = target_stats("CSU", &csu, 50, "CS320");
    assert!(
        stats.error.is_none(),
        "CS320 is required in the CSU BS CS: {:?}",
        stats.error
    );
    // CS320 and MATH160/161 are all required, so every enumerated plan contains them and
    // the calc-ready slice is the entire population.
    assert_eq!(
        stats.all_plans.plans_containing, 50,
        "CS320 is required, so every enumerated plan should contain it"
    );
    assert_eq!(stats.all_plans.earliest_term, Some(4));
    assert_eq!(
        stats.calc_ready_plans.plans_containing, stats.all_plans.plans_containing,
        "every CSU plan includes MATH160/161, so the calc-ready slice is the whole population"
    );
    assert_eq!(stats.calc_ready_plans.earliest_term, Some(4));
    assert_eq!(
        stats.calc_ready_plans.term_distribution, stats.all_plans.term_distribution,
        "an all-calc-ready population must report the same distribution in both slices"
    );
}

#[test]
fn calc_ready_plans_is_empty_when_no_course_id_matches_the_calculus_list() {
    // The contrast to the CSU case: the course is found, but nothing qualifies.
    let uhm = bundled_sample("uhm-ics-bscs-general.yaml");
    let stats = target_stats("UHM", &uhm, 30, "ICS311");
    assert!(
        stats.error.is_none(),
        "ICS311 is required in the UHM BS CS: {:?}",
        stats.error
    );
    assert_eq!(stats.all_plans.plans_containing, 30);
    // 4 -> 3 when the OR-group DAG defect was fixed: ICS311 had been carrying every
    // in-plan option of an OR-group as a prerequisite instead of one.
    assert_eq!(stats.all_plans.earliest_term, Some(3));
    assert_eq!(
        stats.calc_ready_plans.plans_containing, 0,
        "UHM numbers calculus MATH241/242, which the default calculus_courses list omits"
    );
    assert!(stats.calc_ready_plans.earliest_term.is_none());
    assert!(stats.calc_ready_plans.term_distribution.is_empty());
}

#[test]
fn absent_target_course_is_reported_as_an_error() {
    let stats = target_stats("Syracuse", SYRACUSE, MAX_PLANS, ABSENT_COURSE);
    let message = stats
        .error
        .as_deref()
        .expect("a course present in no plan must report an error");
    assert!(
        message.contains(ABSENT_COURSE),
        "the error should name the course that was not found, got: {message}"
    );
    assert_eq!(
        stats.all_plans.plans_containing, 0,
        "the not-found path must report an empty all_plans slice"
    );
    assert!(
        stats.all_plans.earliest_term.is_none(),
        "the not-found path must not report an earliest_term"
    );
    assert!(
        stats.all_plans.term_distribution.is_empty(),
        "the not-found path must report an empty term_distribution"
    );
}
