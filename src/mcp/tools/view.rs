//! What the tools that present an analysis read, whether it ran now or was stored.
//!
//! A degree pulled from the database means its stored run. Its plans were enumerated when it
//! was imported, and enumerating them again would give a second, different answer to a
//! question the corpus has already answered. So `analyze_degree`, `render_degree_report`,
//! `render_plan_graph` and `get_course_detail` read an [`AnalysisView`], which a fresh
//! [`DegreeAnalysis`] and a [`StoredAnalysis`] both provide, and re-enumerate a stored
//! program only when the caller passes `fresh=true`.
//!
//! A view offers reduced statistics — [`ReportStats`] — and nothing more, because that is
//! all the database keeps: the aggregator's per-plan observations are never stored.

use std::collections::{HashMap, HashSet};

use crate::core::degree::analysis::{DegreeAnalysis, TargetCourseStats};
use crate::core::degree::SelectedPlans;
use crate::core::models::{CourseGraph, School};
use crate::core::query::report_source::{StoredReport, StoredRun};
use crate::core::report::inputs::build_report_inputs;
use crate::core::report::ReportStats;
use crate::core::DegreeProgram;

/// An analysis, fresh or stored, as the presenting tools read it.
#[derive(Clone, Copy)]
pub struct AnalysisView<'a> {
    /// The degree analyzed.
    pub program: &'a DegreeProgram,
    /// Its prerequisite graph, cycles broken.
    pub graph: &'a CourseGraph,
    /// Its courses, keyed by document key.
    pub school: &'a School,
    /// Each course's equivalents.
    pub equivalences: &'a HashMap<String, HashSet<String>>,
    /// Degree- and course-level five-number summaries.
    pub stats: &'a ReportStats,
    /// Shortest, longest, calc-ready and Random Sample plans.
    pub selected: &'a SelectedPlans,
    /// Which kind of run, and what only that kind records.
    pub run: Run<'a>,
}

/// Where a view's analysis came from.
#[derive(Clone, Copy)]
pub enum Run<'a> {
    /// Enumerated by this call (or an earlier one sharing the cache).
    Fresh(&'a DegreeAnalysis),
    /// Read from `analysis_runs` and its child tables.
    Stored(&'a StoredRun),
}

impl<'a> AnalysisView<'a> {
    /// The view of a run enumerated now.
    pub const fn fresh(analysis: &'a DegreeAnalysis) -> Self {
        Self {
            program: &analysis.program,
            graph: &analysis.graph,
            school: &analysis.school,
            equivalences: &analysis.equivalences,
            stats: &analysis.report_stats,
            selected: &analysis.selected,
            run: Run::Fresh(analysis),
        }
    }

    /// The fresh run behind this view, if it is one.
    pub const fn fresh_run(&self) -> Option<&'a DegreeAnalysis> {
        match self.run {
            Run::Fresh(analysis) => Some(analysis),
            Run::Stored(_) => None,
        }
    }

    /// Plans the run analyzed.
    pub const fn plans_analyzed(&self) -> usize {
        match self.run {
            Run::Fresh(analysis) => analysis.plans_processed,
            Run::Stored(_) => self.stats.degree_stats().plan_count,
        }
    }

    /// Whether the run saw every distinct plan.
    ///
    /// A stored run records its cap but not the population's size, so it is known to be
    /// complete only when enumeration ended below the cap; at the cap it is reported as a
    /// sample.
    pub fn is_full_population(&self) -> bool {
        match self.run {
            Run::Fresh(analysis) => analysis.is_full_population(),
            Run::Stored(run) => run
                .max_plans
                .and_then(|cap| usize::try_from(cap).ok())
                .is_some_and(|cap| self.plans_analyzed() < cap),
        }
    }

    /// The number of distinct plans, when known; otherwise the plans analyzed.
    ///
    /// A fresh run estimates the population from the requirement choices. A stored run did
    /// not record that estimate, so a sampled stored run reports the plans it analyzed.
    pub const fn population_size(&self) -> usize {
        match self.run {
            Run::Fresh(analysis) => analysis.population_size(),
            Run::Stored(_) => self.plans_analyzed(),
        }
    }

    /// The seed the run enumerated with, or 0 when a stored run did not record one.
    pub fn seed_used(&self) -> u64 {
        match self.run {
            Run::Fresh(analysis) => analysis.seed_used,
            Run::Stored(run) => run
                .random_seed
                .as_deref()
                .and_then(|s| s.parse().ok())
                .unwrap_or(0),
        }
    }

    /// Whether the wall-clock limit stopped enumeration. Never, for a stored run.
    pub const fn time_limit_reached(&self) -> bool {
        match self.run {
            Run::Fresh(analysis) => analysis.time_limit_reached,
            Run::Stored(_) => false,
        }
    }

    /// Milliseconds this call spent enumerating: 0 for a stored run, which enumerates
    /// nothing.
    pub const fn time_elapsed_ms(&self) -> u64 {
        match self.run {
            Run::Fresh(analysis) => analysis.time_elapsed_ms,
            Run::Stored(_) => 0,
        }
    }

    /// Where the target course landed, when a fresh run was asked for one.
    pub const fn target_course_stats(&self) -> Option<&'a TargetCourseStats> {
        match self.run {
            Run::Fresh(analysis) => analysis.target_course_stats.as_ref(),
            Run::Stored(_) => None,
        }
    }
}

/// A stored run, with the course graph, school and equivalences its view borrows.
///
/// These are derived from the degree as they are for a fresh run — the same builders, so a
/// stored report's page and a fresh one's differ only in the statistics they carry.
pub struct StoredAnalysis {
    report: StoredReport,
    graph: CourseGraph,
    school: School,
    equivalences: HashMap<String, HashSet<String>>,
}

impl StoredAnalysis {
    /// Derive what a view needs from a loaded stored run.
    pub fn new(report: StoredReport) -> Self {
        let mut graph = CourseGraph::from_degree_program(&report.program);
        if !graph.cycles.is_empty() {
            graph.graph.break_cycles(&graph.cycles);
        }
        let (school, equivalences) = build_report_inputs(&report.program);
        Self {
            report,
            graph: graph.graph,
            school,
            equivalences,
        }
    }

    /// The run this was read from.
    pub const fn run(&self) -> &StoredRun {
        &self.report.run
    }

    /// The stored run as the presenting tools read it.
    pub const fn view(&self) -> AnalysisView<'_> {
        AnalysisView {
            program: &self.report.program,
            graph: &self.graph,
            school: &self.school,
            equivalences: &self.equivalences,
            stats: &self.report.stats,
            selected: &self.report.selected,
            run: Run::Stored(&self.report.run),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mcp::tools::{analyze, course_detail, plan_graph, report};

    const TINY: &str = "degree: {id: t, institution: T, program: T, total_credits: 7, gpa_minimum: 2.0}\n\
        requirements:\n  core: {name: Core, type: all, category: major, courses: [CS101, CS201]}\n\
        courses:\n  CS101: {title: Intro, prefix: CS, number: \"101\", credits: 3}\n  \
        CS201: {title: Next, prefix: CS, number: \"201\", credits: 4, prerequisites_raw: \"CS101\"}\n";

    const CAP: usize = 40;

    /// A stored run holding exactly what a fresh run of `text` computed.
    fn stored_copy(fresh: &DegreeAnalysis) -> StoredReport {
        StoredReport {
            program: fresh.program.clone(),
            stats: fresh.report_stats.clone(),
            selected: fresh.selected.clone(),
            run: StoredRun {
                run_key: "run-1".to_string(),
                variant: "full".to_string(),
                created_at: None,
                analyzer_version: Some(env!("CARGO_PKG_VERSION").to_string()),
                variations_run: i64::try_from(fresh.plans_processed).ok(),
                max_plans: i64::try_from(CAP).ok(),
                random_seed: Some(fresh.seed_used.to_string()),
                sampling_strategy: Some("shuffled".to_string()),
            },
        }
    }

    fn json(text: &str) -> serde_json::Value {
        serde_json::from_str(text).expect("tool output is JSON")
    }

    /// A stored run is presented exactly as the run it was stored from: every tool reads
    /// only the view, so given the same statistics and plans they cannot tell the two
    /// apart — except where a stored run records less (its population size) or must not
    /// offer to enumerate again.
    #[test]
    fn a_stored_run_is_presented_as_the_fresh_run_it_came_from() {
        let csu = crate::mcp::tools::samples::yaml_for_key("csu").expect("csu sample");
        for (name, text) in [("sampled", csu), ("fully enumerated", TINY)] {
            let fresh = analyze::build_artifacts(text, Some(CAP), None, None, None, None)
                .expect("analyzes");
            let stored = StoredAnalysis::new(stored_copy(&fresh));
            let (f, s) = (AnalysisView::fresh(&fresh), stored.view());

            let inline = report::ReportOutput {
                output_dir: None,
                write_plan_csvs: None,
                write_jsonl_summary: None,
                write_index_csv: None,
                return_html_inline: Some(true),
                overwrite: false,
            };
            let page = report::present(&s, &inline).html_content;
            assert!(page.is_some(), "{name}: renders");
            assert_eq!(
                page,
                report::present(&f, &inline).html_content,
                "{name}: report"
            );
            assert_eq!(
                page,
                stored_copy(&fresh).render_html().ok(),
                "{name}: the page `db report` renders"
            );

            let (fa, sa) = (
                json(&analyze::view_json(&f, true, false)),
                json(&analyze::view_json(&s, true, false)),
            );
            for key in [
                "complexity",
                "longest_delay",
                "total_credits",
                "avg_chain_length",
                "selected_plans",
                "per_course_metrics",
                "plans_analyzed",
                "is_full_population",
                "was_truncated",
                "seed_used",
                "total_courses",
            ] {
                assert_eq!(fa[key], sa[key], "{name}: analyze_degree {key}");
            }
            assert_eq!(sa["population_size"], sa["plans_analyzed"], "{name}");
            assert!(sa["recommended_max_plans"].is_null(), "{name}");
            assert!(
                sa["tool_followups"]
                    .as_array()
                    .is_some_and(|f| f.iter().all(|f| f["tool"] != "analyze_degree")),
                "{name}: a stored run never suggests enumerating again: {}",
                sa["tool_followups"]
            );
            assert!(sa["notes"][0].as_str().is_some_and(|n| n.contains("run-1")));

            let shortest = plan_graph::PlanGraphOptions {
                plan_category: Some("shortest"),
                ..Default::default()
            };
            assert_eq!(
                plan_graph::present(&s, &shortest).html,
                plan_graph::present(&f, &shortest).html,
                "{name}: plan graph"
            );
            assert_eq!(
                course_detail::present_json("CS201", &s),
                course_detail::present_json("CS201", &f),
                "{name}: course detail"
            );
        }
    }

    /// A stored run records its cap but not its population, so it is complete only when
    /// enumeration ended below the cap.
    #[test]
    fn a_stored_run_is_complete_only_below_its_cap() {
        let fresh = analyze::build_artifacts(TINY, Some(CAP), None, None, None, None).unwrap();
        let mut report = stored_copy(&fresh);
        assert!(StoredAnalysis::new(stored_copy(&fresh))
            .view()
            .is_full_population());
        report.run.max_plans = i64::try_from(fresh.plans_processed).ok();
        assert!(!StoredAnalysis::new(report).view().is_full_population());
        let mut unrecorded = stored_copy(&fresh);
        unrecorded.run.max_plans = None;
        assert!(!StoredAnalysis::new(unrecorded).view().is_full_population());
    }
}
