//! Decide which analysis runs to drop when keeping a bounded history.
//!
//! Runs accumulate: every re-analysis appends rather than replacing, which is what makes
//! it possible to compare a metric across analyzer versions. That is worth keeping, but
//! not forever, so this works out what "keep the newest N" means for a given set of runs.
//!
//! The selection is a pure function over run metadata so it can be tested without a
//! backend, and so a `--dry-run` reports exactly what a real run would delete rather than
//! an approximation of it.

use std::collections::BTreeMap;

/// The identity of one stored run, as far as pruning is concerned.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct RunRef {
    /// Surrogate key of the run.
    pub run_key: String,
    /// Program the run belongs to.
    pub program_key: String,
    /// `full`, `trimmed`, or another transform label.
    pub variant: String,
    /// When it was written, ISO 8601. Runs without one sort oldest.
    pub created_at: Option<String>,
    /// Crate version that produced it.
    pub analyzer_version: Option<String>,
}

/// What a prune would remove, and what it would leave.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PrunePlan {
    /// Runs to delete.
    pub doomed: Vec<RunRef>,
    /// Runs that survive.
    pub kept: usize,
    /// How many distinct program+variant groups were considered.
    pub groups: usize,
}

impl PrunePlan {
    /// Whether the plan would delete anything.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.doomed.is_empty()
    }

    /// The run keys to delete.
    #[must_use]
    pub fn doomed_keys(&self) -> Vec<&str> {
        self.doomed.iter().map(|r| r.run_key.as_str()).collect()
    }
}

/// Keep the `keep` newest runs per program **and variant**, mark the rest for deletion.
///
/// Grouped by variant as well as program because `full` and `trimmed` are different
/// analyses of the same degree, not competing versions of one. Grouping by program alone
/// would let a burst of `full` re-runs evict every `trimmed` run a degree had.
///
/// Ordering is newest-first by `created_at`, with the run key as a tiebreaker so two runs
/// written in the same clock tick still order deterministically — otherwise a dry run and
/// the real run could disagree about which survives.
#[must_use]
pub fn plan_keep_newest(runs: &[RunRef], keep: usize) -> PrunePlan {
    let mut groups: BTreeMap<(&str, &str), Vec<&RunRef>> = BTreeMap::new();
    for run in runs {
        groups
            .entry((run.program_key.as_str(), run.variant.as_str()))
            .or_default()
            .push(run);
    }

    let mut plan = PrunePlan {
        groups: groups.len(),
        ..PrunePlan::default()
    };
    for members in groups.values_mut() {
        // Newest first. `None` sorts oldest, which is what an unstamped legacy row is.
        members.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| b.run_key.cmp(&a.run_key))
        });
        plan.kept += members.len().min(keep);
        plan.doomed
            .extend(members.iter().skip(keep).map(|r| (*r).clone()));
    }
    plan
}

/// Mark every run produced by a given analyzer version for deletion.
///
/// Unlike [`plan_keep_newest`] this can empty a program's history entirely — dropping a
/// version is a deliberate act ("that release computed complexity wrongly"), not a
/// retention policy, so it is not softened by a floor.
#[must_use]
pub fn plan_by_analyzer_version(runs: &[RunRef], version: &str) -> PrunePlan {
    let doomed: Vec<RunRef> = runs
        .iter()
        .filter(|r| r.analyzer_version.as_deref() == Some(version))
        .cloned()
        .collect();
    PrunePlan {
        kept: runs.len() - doomed.len(),
        groups: runs.len(),
        doomed,
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn run(key: &str, program: &str, variant: &str, at: Option<&str>) -> RunRef {
        RunRef {
            run_key: key.to_string(),
            program_key: program.to_string(),
            variant: variant.to_string(),
            created_at: at.map(str::to_string),
            analyzer_version: Some("0.5.4".to_string()),
        }
    }

    #[test]
    fn keeps_the_newest_and_drops_the_rest() {
        let runs = vec![
            run("a", "p1", "full", Some("2026-01-01")),
            run("b", "p1", "full", Some("2026-03-01")),
            run("c", "p1", "full", Some("2026-02-01")),
        ];
        let plan = plan_keep_newest(&runs, 2);
        assert_eq!(plan.kept, 2);
        assert_eq!(plan.doomed_keys(), ["a"], "the oldest goes");
    }

    #[test]
    fn full_and_trimmed_are_separate_histories() {
        // They are different analyses of one degree, not competing versions of it.
        // Grouping by program alone would let three `full` re-runs evict the only
        // `trimmed` run the degree has.
        let runs = vec![
            run("f1", "p1", "full", Some("2026-01-01")),
            run("f2", "p1", "full", Some("2026-02-01")),
            run("f3", "p1", "full", Some("2026-03-01")),
            run("t1", "p1", "trimmed", Some("2026-01-01")),
        ];
        let plan = plan_keep_newest(&runs, 1);
        let doomed = plan.doomed_keys();
        assert!(
            !doomed.contains(&"t1"),
            "the only trimmed run must survive: {doomed:?}"
        );
        assert_eq!(plan.groups, 2);
        assert_eq!(plan.kept, 2, "newest full plus the trimmed one");
    }

    #[test]
    fn programs_do_not_evict_each_other() {
        let runs = vec![
            run("a", "p1", "full", Some("2026-01-01")),
            run("b", "p2", "full", Some("2026-02-01")),
            run("c", "p3", "full", Some("2026-03-01")),
        ];
        assert!(plan_keep_newest(&runs, 1).is_empty());
    }

    #[test]
    fn keeping_more_than_exist_deletes_nothing() {
        let runs = vec![run("a", "p1", "full", Some("2026-01-01"))];
        let plan = plan_keep_newest(&runs, 5);
        assert!(plan.is_empty());
        assert_eq!(plan.kept, 1);
    }

    #[test]
    fn keep_zero_deletes_everything() {
        // Not a special case in the code, so pinned here: `--keep 0` is a full clear of
        // the run history and must not silently behave like `--keep 1`.
        let runs = vec![
            run("a", "p1", "full", Some("2026-01-01")),
            run("b", "p2", "full", Some("2026-02-01")),
        ];
        let plan = plan_keep_newest(&runs, 0);
        assert_eq!(plan.doomed.len(), 2);
        assert_eq!(plan.kept, 0);
    }

    #[test]
    fn a_run_with_no_timestamp_sorts_oldest() {
        // Legacy rows predate `created_at`. Treating them as newest would evict real
        // history in favour of a row we know least about.
        let runs = vec![
            run("legacy", "p1", "full", None),
            run("recent", "p1", "full", Some("2026-03-01")),
        ];
        let plan = plan_keep_newest(&runs, 1);
        assert_eq!(plan.doomed_keys(), ["legacy"]);
    }

    #[test]
    fn ties_break_deterministically() {
        // Two runs in the same clock tick must order the same way every call, or a
        // dry run and the real run disagree about which one survives.
        let runs = vec![
            run("aaa", "p1", "full", Some("2026-01-01T00:00:00Z")),
            run("bbb", "p1", "full", Some("2026-01-01T00:00:00Z")),
        ];
        let first = plan_keep_newest(&runs, 1);
        let second = plan_keep_newest(&runs, 1);
        assert_eq!(first, second);
        assert_eq!(first.doomed_keys(), ["aaa"]);
    }

    #[test]
    fn dropping_an_analyzer_version_ignores_the_retention_floor() {
        // "That release computed complexity wrongly" is a correctness decision, not a
        // retention policy, so it may empty a program's history.
        let mut old = run("a", "p1", "full", Some("2026-01-01"));
        old.analyzer_version = Some("0.5.3".to_string());
        let runs = vec![old, run("b", "p1", "full", Some("2026-02-01"))];

        let plan = plan_by_analyzer_version(&runs, "0.5.3");
        assert_eq!(plan.doomed_keys(), ["a"]);
        assert_eq!(plan.kept, 1);

        let none = plan_by_analyzer_version(&runs, "9.9.9");
        assert!(none.is_empty(), "an unknown version deletes nothing");
    }
}
