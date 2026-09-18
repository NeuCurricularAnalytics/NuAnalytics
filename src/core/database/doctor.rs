//! Whole-deployment health check behind `nuanalytics db doctor`.
//!
//! The question this answers is "is *my* backend set up correctly", asked by someone who
//! did not build it. Cloud and self-hosted are meant to be interchangeable, and the one
//! real asymmetry is whether the schema and seeds were applied — so the checks walk from
//! configuration outwards to data, and each one names what it actually observed rather
//! than a guess at the cause.
//!
//! The checks are returned as data, not printed, so the ordering and the pass/fail
//! judgement are testable without a terminal.

use super::client::DbClient;
use super::tables;
use crate::core::config::{endpoint_label, DatabaseConfig};

/// Expected `cip_codes` row count for a correctly seeded deployment.
///
/// From `docs/database/cip-seed.sql`. A short count means the seed was truncated, which
/// otherwise shows up much later as CIP lookups silently returning nothing.
pub const EXPECTED_CIP_CODES: u64 = 2173;

/// Outcome of one check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// Observed what a healthy deployment should show.
    Pass(String),
    /// Working, but not as intended — worth acting on, not a hard failure.
    Warn(String),
    /// Broken; the deployment will not work as configured.
    Fail(String),
    /// Not attempted, because something it depends on already failed.
    Skipped(String),
}

impl Outcome {
    /// Single-character marker for terminal output.
    #[must_use]
    pub const fn marker(&self) -> char {
        match self {
            Self::Pass(_) => '✓',
            Self::Warn(_) => '⚠',
            Self::Fail(_) => '✗',
            Self::Skipped(_) => '-',
        }
    }

    /// The observation itself.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Pass(d) | Self::Warn(d) | Self::Fail(d) | Self::Skipped(d) => d,
        }
    }
}

/// One named check and what it found.
#[derive(Debug, Clone)]
pub struct Check {
    /// Short label, stable enough to grep for.
    pub name: &'static str,
    /// What this check observed.
    pub outcome: Outcome,
}

impl Check {
    const fn new(name: &'static str, outcome: Outcome) -> Self {
        Self { name, outcome }
    }
}

/// Every check, in the order they were run.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// Checks in execution order; earlier ones gate later ones.
    pub checks: Vec<Check>,
}

impl Report {
    /// Whether anything failed outright.
    ///
    /// Warnings do not count: a deployment missing only its optional seed data is usable.
    #[must_use]
    pub fn has_failures(&self) -> bool {
        self.checks
            .iter()
            .any(|c| matches!(c.outcome, Outcome::Fail(_)))
    }

    /// Counts of (pass, warn, fail, skipped), for a one-line summary.
    #[must_use]
    pub fn tally(&self) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for check in &self.checks {
            match check.outcome {
                Outcome::Pass(_) => counts.0 += 1,
                Outcome::Warn(_) => counts.1 += 1,
                Outcome::Fail(_) => counts.2 += 1,
                Outcome::Skipped(_) => counts.3 += 1,
            }
        }
        counts
    }

    fn push(&mut self, name: &'static str, outcome: Outcome) {
        self.checks.push(Check::new(name, outcome));
    }
}

/// Run every check against the configured backend.
///
/// Never returns `Err`: a failure to reach the backend *is* the diagnosis, and a caller
/// asking "what is wrong with my deployment" should get a report rather than one error.
pub async fn diagnose(config: &DatabaseConfig) -> Report {
    let mut report = Report::default();

    let config_check = configuration_check(config);
    let config_failed = matches!(config_check.outcome, Outcome::Fail(_));
    report.push(config_check.name, config_check.outcome);
    if config_failed {
        // Deliberately not followed by skipped entries: with no endpoint there is nothing
        // to skip, and a wall of "-" lines would bury the one thing to fix.
        return report;
    }

    if !append_reachability(&mut report, config).await {
        skip_remaining(&mut report, &REACHABILITY_DEPENDENTS, "backend unreachable");
        return report;
    }

    let Some(client) = append_session(&mut report, config).await else {
        skip_remaining(
            &mut report,
            &SESSION_DEPENDENTS,
            "needs a valid session; run `nuanalytics db login`",
        );
        return report;
    };

    match client.ping().await {
        Ok(()) => report.push("authenticated read", Outcome::Pass("succeeded".into())),
        Err(e) => {
            report.push("authenticated read", Outcome::Fail(e.to_string()));
            skip_remaining(&mut report, &READ_DEPENDENTS, "authenticated read failed");
            return report;
        }
    }

    report.push("schema", schema_outcome(&client).await);
    report.push("seed data", cip_seed_outcome(&client).await);
    report
}

/// Checks that cannot run without a reachable backend.
const REACHABILITY_DEPENDENTS: [&str; 5] = [
    "anon-key read",
    "session",
    "authenticated read",
    "schema",
    "seed data",
];
/// Checks that cannot run without a valid session.
const SESSION_DEPENDENTS: [&str; 3] = ["authenticated read", "schema", "seed data"];
/// Checks that cannot run without a working authenticated read.
const READ_DEPENDENTS: [&str; 2] = ["schema", "seed data"];

/// Record `names` as skipped, so the report lists one cause instead of repeating it.
fn skip_remaining(report: &mut Report, names: &[&'static str], reason: &str) {
    for name in names {
        report.push(name, Outcome::Skipped(reason.to_string()));
    }
}

/// Is there enough configuration to check anything at all?
fn configuration_check(config: &DatabaseConfig) -> Check {
    let backend = endpoint_label(&config.endpoint);
    if config.endpoint.is_empty() || config.anon_key.is_empty() {
        return Check::new(
            "configuration",
            Outcome::Fail(format!(
                "endpoint {} and anon key {} — set both before anything else can be checked",
                if config.endpoint.is_empty() {
                    "unset"
                } else {
                    "set"
                },
                if config.anon_key.is_empty() {
                    "unset"
                } else {
                    "set"
                },
            )),
        );
    }
    if !config.enabled {
        return Check::new(
            "configuration",
            Outcome::Fail("database.enabled is false, so no tool will use this backend".into()),
        );
    }
    Check::new(
        "configuration",
        Outcome::Pass(format!("endpoint {backend}, anon key set, enabled")),
    )
}

/// Probe with the anon key alone, before any authenticated call.
///
/// Separates "cannot reach it, or TLS is broken" from "reached it but the session is
/// bad", which the authenticated path conflates. Returns whether to keep going.
async fn append_reachability(report: &mut Report, config: &DatabaseConfig) -> bool {
    let backend = endpoint_label(&config.endpoint);
    let probe = DbClient::new(&config.endpoint, &config.anon_key, "unused".into());
    let status = match probe {
        Ok(client) => client.anon_read_status(tables::CIP_CODES).await,
        Err(e) => Err(e),
    };
    match status {
        Ok(status) => {
            report.push("reachability", Outcome::Pass(format!("{backend} answered")));
            report.push("anon-key read", anon_read_outcome(status));
            true
        }
        Err(e) => {
            report.push(
                "reachability",
                Outcome::Fail(format!("{e}; nothing below could be checked")),
            );
            false
        }
    }
}

/// Build the real client, reporting the session state. `None` means stop.
async fn append_session(report: &mut Report, config: &DatabaseConfig) -> Option<DbClient> {
    match DbClient::from_config(config).await {
        Ok(client) => {
            let detail = client
                .signed_in_email()
                .map_or_else(|| "valid".to_string(), |email| format!("valid, {email}"));
            report.push("session", Outcome::Pass(detail));
            Some(client)
        }
        Err(e) => {
            report.push("session", Outcome::Fail(e.to_string()));
            None
        }
    }
}

/// Check every table the schema defines.
async fn schema_outcome(client: &DbClient) -> Outcome {
    let mut missing = Vec::new();
    let mut unchecked = Vec::new();
    for table in tables::ALL {
        match client.table_exists(table).await {
            Ok(true) => {}
            Ok(false) => missing.push(*table),
            Err(e) => unchecked.push(format!("{table} ({e})")),
        }
    }
    let total = tables::ALL.len();
    if !unchecked.is_empty() {
        return Outcome::Fail(format!(
            "could not check {} of {total} tables: {}",
            unchecked.len(),
            unchecked.join(", ")
        ));
    }
    if missing.is_empty() {
        return Outcome::Pass(format!("all {total} tables present"));
    }
    Outcome::Fail(format!(
        "{} of {total} tables missing: {}. Apply the schema and seed files in the order \
         given in docs/database/setup.md",
        missing.len(),
        missing.join(", ")
    ))
}

/// Judge the status code from an anon-key-only read.
fn anon_read_outcome(status: u16) -> Outcome {
    match status {
        200 => Outcome::Pass(
            "200 — row-level security filters an unauthenticated read rather than rejecting it"
                .into(),
        ),
        401 | 403 => Outcome::Fail(format!(
            "{status} — the anon key alone is refused. Policies should filter to an empty \
             result instead; a rejection breaks the login bootstrap"
        )),
        404 => Outcome::Fail(format!(
            "{status} — the table is absent, so the schema was not applied to this backend"
        )),
        other => Outcome::Warn(format!(
            "{other} — expected 200; check the gateway in front of PostgREST"
        )),
    }
}

/// Compare the `cip_codes` row count against the seed.
async fn cip_seed_outcome(client: &DbClient) -> Outcome {
    match client.count_rows(tables::CIP_CODES).await {
        Ok(EXPECTED_CIP_CODES) => {
            Outcome::Pass(format!("cip_codes has all {EXPECTED_CIP_CODES} rows"))
        }
        Ok(0) => Outcome::Warn(
            "cip_codes is empty — apply docs/database/cip-seed.sql, or CIP lookups will \
             silently return nothing"
                .into(),
        ),
        Ok(count) => Outcome::Warn(format!(
            "cip_codes has {count} rows, expected {EXPECTED_CIP_CODES} — the seed looks \
             truncated"
        )),
        Err(e) => Outcome::Fail(format!("cannot count cip_codes: {e}")),
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anon_read_200_is_the_healthy_case() {
        // RLS should filter an unauthenticated read to nothing, not refuse it.
        let outcome = anon_read_outcome(200);
        assert!(matches!(outcome, Outcome::Pass(_)), "{outcome:?}");
        assert!(outcome.detail().contains("filters"), "{outcome:?}");
    }

    #[test]
    fn anon_read_rejection_is_a_failure_that_explains_the_consequence() {
        for status in [401, 403] {
            let outcome = anon_read_outcome(status);
            assert!(matches!(outcome, Outcome::Fail(_)), "{status}: {outcome:?}");
            assert!(
                outcome.detail().contains("login bootstrap"),
                "must say why a rejection matters, not just that it happened: {outcome:?}"
            );
        }
    }

    #[test]
    fn anon_read_404_is_reported_as_a_missing_schema_not_a_policy_problem() {
        let outcome = anon_read_outcome(404);
        assert!(matches!(outcome, Outcome::Fail(_)));
        assert!(
            outcome.detail().contains("schema was not applied"),
            "404 means the table is absent, which is a different fix: {outcome:?}"
        );
    }

    #[test]
    fn an_unexpected_anon_status_warns_rather_than_asserting_a_cause() {
        let outcome = anon_read_outcome(502);
        assert!(matches!(outcome, Outcome::Warn(_)), "{outcome:?}");
        assert!(
            outcome.detail().contains("gateway"),
            "a 502 points at the proxy, but the check must not claim to know: {outcome:?}"
        );
    }

    #[test]
    fn markers_distinguish_every_outcome() {
        let markers: Vec<char> = [
            Outcome::Pass(String::new()),
            Outcome::Warn(String::new()),
            Outcome::Fail(String::new()),
            Outcome::Skipped(String::new()),
        ]
        .iter()
        .map(Outcome::marker)
        .collect();
        let unique: std::collections::HashSet<char> = markers.iter().copied().collect();
        assert_eq!(unique.len(), markers.len(), "markers must be distinct");
    }

    fn report_of(outcomes: Vec<Outcome>) -> Report {
        Report {
            checks: outcomes
                .into_iter()
                .map(|o| Check::new("check", o))
                .collect(),
        }
    }

    #[test]
    fn only_a_failure_makes_the_report_fail() {
        // A deployment whose optional seed is short is usable; one missing tables is not.
        assert!(!report_of(vec![Outcome::Pass("x".into())]).has_failures());
        assert!(
            !report_of(vec![Outcome::Warn("short seed".into())]).has_failures(),
            "a warning must not fail the command"
        );
        assert!(
            !report_of(vec![Outcome::Skipped("unreachable".into())]).has_failures(),
            "a skipped check is not a failure in itself"
        );
        assert!(report_of(vec![Outcome::Fail("missing tables".into())]).has_failures());
    }

    #[test]
    fn tally_counts_each_outcome_separately() {
        let report = report_of(vec![
            Outcome::Pass("a".into()),
            Outcome::Pass("b".into()),
            Outcome::Warn("c".into()),
            Outcome::Fail("d".into()),
            Outcome::Skipped("e".into()),
            Outcome::Skipped("f".into()),
        ]);
        assert_eq!(report.tally(), (2, 1, 1, 2));
    }

    #[tokio::test]
    async fn an_unconfigured_backend_stops_before_any_network_call() {
        // Every later check would otherwise report a connection failure and bury the
        // actual problem.
        let report = diagnose(&DatabaseConfig::default()).await;
        assert_eq!(report.checks.len(), 1, "{:?}", report.checks);
        assert_eq!(report.checks[0].name, "configuration");
        assert!(report.has_failures());
        assert!(
            report.checks[0].outcome.detail().contains("unset"),
            "{:?}",
            report.checks[0]
        );
    }

    #[tokio::test]
    async fn a_disabled_backend_is_reported_as_configuration_not_unreachable() {
        let config = DatabaseConfig {
            enabled: false,
            endpoint: "https://nu.example.com".to_string(),
            anon_key: "k".to_string(),
            ..DatabaseConfig::default()
        };
        let report = diagnose(&config).await;
        assert_eq!(report.checks.len(), 1);
        assert!(
            report.checks[0]
                .outcome
                .detail()
                .contains("enabled is false"),
            "{:?}",
            report.checks[0]
        );
    }

    #[tokio::test]
    async fn an_unreachable_backend_skips_the_rest_rather_than_repeating_the_error() {
        let config = DatabaseConfig {
            enabled: true,
            // Nothing listening: refused without waiting out a timeout.
            endpoint: "http://127.0.0.1:1".to_string(),
            anon_key: "k".to_string(),
            ..DatabaseConfig::default()
        };
        let report = diagnose(&config).await;
        assert!(report.has_failures());

        let reachability = report
            .checks
            .iter()
            .find(|c| c.name == "reachability")
            .expect("reachability is checked");
        assert!(matches!(reachability.outcome, Outcome::Fail(_)));

        // Everything downstream is skipped, so the report names one cause rather than six.
        for name in ["session", "authenticated read", "schema", "seed data"] {
            let check = report
                .checks
                .iter()
                .find(|c| c.name == name)
                .unwrap_or_else(|| panic!("{name} should still appear in the report"));
            assert!(
                matches!(check.outcome, Outcome::Skipped(_)),
                "{name} should be skipped, got {:?}",
                check.outcome
            );
        }
        let (_, _, fail, skipped) = report.tally();
        assert_eq!(fail, 1, "exactly one thing is actually wrong");
        assert!(skipped >= 4);
    }

    #[test]
    fn every_schema_table_is_checked_and_the_list_has_no_duplicates() {
        // A duplicate would make the "N of 20" counts wrong, and a missing entry would
        // let a half-applied schema pass.
        let unique: std::collections::HashSet<&&str> = tables::ALL.iter().collect();
        assert_eq!(unique.len(), tables::ALL.len(), "duplicate in tables::ALL");
        assert_eq!(
            tables::ALL.len(),
            20,
            "the schema defines 20 tables; update ALL when that changes"
        );
        for named in [
            tables::INSTITUTIONS,
            tables::CIP_CODES,
            tables::PROGRAMS,
            tables::AWARD_LEVELS,
            tables::INSTITUTION_SIZE,
        ] {
            assert!(tables::ALL.contains(&named), "{named} missing from ALL");
        }
    }
}
