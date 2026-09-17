//! How a target course relates to the plans `selected_plans` surfaces.
//!
//! Companion to `target_course_population`, which covers the aggregate
//! `target_course_stats` figures. The boundary between the two is easy to conflate:
//! the statistics summarise every enumerated plan, while `selected_plans` exposes only a
//! handful (shortest, longest, and a few reservoir samples).
//!
//! Which plans reach `selected_plans` depends on reservoir sampling, so no test here
//! asserts an exact term count, credit total, or which plans contain a given course.
//! The assertions are inequalities and per-plan invariants that hold for any selection.

use super::degree_fixtures::{analyze_target, ASU, CALSTATELA, MAX_PLANS};
use nu_analytics::core::degree::PlanCategory;
use nu_analytics::mcp::tools::analyze::PlanSummaryJson;
use std::collections::HashMap;

/// The term a course occupies in a plan summary, if it is scheduled at all.
fn scheduled_term(plan: &PlanSummaryJson, course: &str) -> Option<usize> {
    plan.schedule
        .iter()
        .find(|term| term.courses.iter().any(|c| c == course))
        .map(|term| term.term)
}

/// Each course in a plan mapped to the term it is scheduled in.
///
/// Panics if a course is scheduled twice: a plan that places one course in two terms is
/// malformed, and reporting it here names both terms.
fn placements<'p>(plan: &'p PlanSummaryJson, category: &str) -> HashMap<&'p str, usize> {
    let mut placements: HashMap<&str, usize> = HashMap::new();
    for term in &plan.schedule {
        for course in &term.courses {
            if let Some(previous) = placements.insert(course.as_str(), term.term) {
                panic!(
                    "{category}: {course} is scheduled in both term {previous} and {}",
                    term.term
                );
            }
        }
    }
    placements
}

#[test]
fn target_course_stats_cover_plans_beyond_the_selected_ones() {
    // ASU's DAT402 is an elective option, so it appears in many more enumerated plans
    // than `selected_plans` exposes. That gap is the contract worth pinning: reading a
    // term number off `selected_plans` instead of `target_course_stats` would
    // under-report a course the degree does offer.
    let response = analyze_target("ASU", ASU, MAX_PLANS, "DAT402");
    let stats = response
        .target_course_stats
        .as_ref()
        .expect("ASU: requesting a target course populates target_course_stats");
    assert!(
        stats.error.is_none(),
        "ASU: DAT402 should be reachable in the BS CS, got error {:?}",
        stats.error
    );
    assert!(
        stats.all_plans.plans_containing > 0,
        "ASU: DAT402 must be reported in at least one enumerated plan"
    );
    assert!(
        !response.selected_plans.is_empty(),
        "ASU: an analysis of {MAX_PLANS} plans must select at least one"
    );
    assert!(
        response.plans_analyzed > response.selected_plans.len(),
        "ASU: {} plans were analyzed but {} selected — the statistics would add nothing \
         over reading selected_plans directly",
        response.plans_analyzed,
        response.selected_plans.len()
    );
}

#[test]
fn selected_plan_schedules_are_well_formed() {
    let response = analyze_target("CalStateLA", CALSTATELA, 10, "CS4963");
    assert!(
        !response.selected_plans.is_empty(),
        "CalStateLA: an analysis must select at least one plan"
    );
    let categories: Vec<&str> = response
        .selected_plans
        .iter()
        .map(|p| p.category.as_str())
        .collect();
    assert!(
        categories.contains(&PlanCategory::Shortest.display_name()),
        "CalStateLA: a {} plan is always among the selected plans, got {categories:?}",
        PlanCategory::Shortest.display_name()
    );

    for plan in &response.selected_plans {
        let category = plan.category.as_str();
        assert!(
            !plan.schedule.is_empty(),
            "CalStateLA {category}: schedule must not be empty"
        );
        assert_eq!(
            plan.terms,
            plan.schedule.len(),
            "CalStateLA {category}: reported term count must match the scheduled terms"
        );
        assert!(
            plan.credits > 0.0,
            "CalStateLA {category}: plan must carry a positive credit total, got {}",
            plan.credits
        );
        // Panics naming both terms if any course is placed twice.
        let placed = placements(plan, category);
        assert!(
            !placed.is_empty(),
            "CalStateLA {category}: a non-empty schedule must place at least one course"
        );
    }
}

#[test]
fn population_minimum_is_no_later_than_a_term_the_course_occupies() {
    let response = analyze_target("CalStateLA", CALSTATELA, 10, "CS4963");
    let stats = response
        .target_course_stats
        .as_ref()
        .expect("CalStateLA: requesting a target course populates target_course_stats");
    // Asserted before unwrapping earliest_term: if CS4963 ever stops being reachable in
    // this fixture, the cause should be named rather than surfacing as a missing value.
    assert!(
        stats.error.is_none(),
        "CalStateLA: CS4963 should be reachable in the BS CS, got error {:?}",
        stats.error
    );
    let earliest = stats
        .all_plans
        .earliest_term
        .expect("CalStateLA: earliest_term is set when the course is found");

    let mut occupied = 0;
    for plan in &response.selected_plans {
        if let Some(term) = scheduled_term(plan, "CS4963") {
            occupied += 1;
            assert!(
                earliest <= term,
                "CalStateLA {}: all_plans.earliest_term {earliest} is later than term {term}, \
                 where CS4963 is actually scheduled",
                plan.category
            );
        }
    }
    assert!(
        occupied > 0,
        "CS4963 is scheduled in none of the selected plans, so this test compared nothing; \
         selected categories were {:?}",
        response
            .selected_plans
            .iter()
            .map(|p| p.category.as_str())
            .collect::<Vec<_>>()
    );
}
