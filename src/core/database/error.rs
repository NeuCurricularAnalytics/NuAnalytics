//! Database error types

use std::fmt;

/// Errors that can occur during database operations
#[derive(Debug)]
pub enum DatabaseError {
    /// Database is not configured (missing `endpoint` or `anon_key`)
    NotConfigured,
    /// Database is disabled in configuration
    Disabled,
    /// No valid user session — both reads and writes require a logged-in
    /// user (`nuanalytics db login`). Carries an explanatory detail (e.g.
    /// "auth file missing", "refresh token rejected") for diagnostics.
    NotAuthenticated(String),
    /// Failed to connect to the database
    ConnectionError(String),
    /// Query execution failed
    QueryError(String),
    /// Failed to parse response data
    ParseError(String),
    /// Ingest operation failed
    IngestError(String),
    /// A write hit a natural-key row that row-level security hides from this user.
    ///
    /// Carries the table and the backend's own message. This is *not* the `42501` an RLS
    /// refusal would suggest: the `merge-duplicates` upsert cannot see the other user's
    /// row to UPDATE it, so it falls through to an INSERT and trips the natural-key
    /// unique constraint instead. "duplicate key" alone gives no hint that ownership is
    /// involved, which is why this variant exists.
    RowOwnedByAnother {
        /// Table the conflicting row lives in.
        table: String,
        /// The backend's verbatim message.
        detail: String,
    },
}

impl fmt::Display for DatabaseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotConfigured => write!(
                f,
                "Database not configured. Set `endpoint` and `anon_key` in [database] config."
            ),
            Self::Disabled => write!(
                f,
                "Database is disabled. Set `enabled = true` in [database] config."
            ),
            Self::NotAuthenticated(detail) => write!(f, "Not signed in ({detail})."),
            Self::ConnectionError(msg) => write!(f, "Database connection error: {msg}"),
            Self::QueryError(msg) => write!(f, "Database query error: {msg}"),
            Self::ParseError(msg) => write!(f, "Data parse error: {msg}"),
            Self::IngestError(msg) => write!(f, "Data ingest error: {msg}"),
            Self::RowOwnedByAnother { table, detail } => write!(
                f,
                "A row in `{table}` with this natural key already exists and is not \
                 writable by you — most likely created by another user ({detail})"
            ),
        }
    }
}

impl DatabaseError {
    /// What the user should do next, one line per step.
    ///
    /// Branches on the variant because the fixes are different and not interchangeable:
    /// a not-configured install has no backend to log in to, and an unreachable backend
    /// is not a login problem. `endpoint` is named so the message says *which* backend it
    /// is talking about — with cloud and self-hosted both supported, that is the first
    /// question in any support exchange.
    ///
    /// Shared by the CLI (`db status`) and the MCP server so the two cannot drift.
    #[must_use]
    pub fn next_steps(&self, endpoint: &str) -> Vec<String> {
        let backend = crate::core::config::endpoint_label(endpoint);
        match self {
            Self::NotConfigured => vec![
                "no backend is configured. Set one:".to_string(),
                "  nuanalytics config set database.endpoint <url>".to_string(),
                "  nuanalytics config set database.anon_key <key>".to_string(),
                "`config set` writes to the home config; a project-local nuanalytics.toml takes precedence over it."
                    .to_string(),
            ],
            Self::Disabled => vec![
                "the database is disabled in configuration. Enable it:".to_string(),
                "  nuanalytics config set database.enabled true".to_string(),
            ],
            // The detail is not echoed: callers print the error itself before these
            // steps, so repeating it made one failure state the same path twice.
            Self::NotAuthenticated(_) => vec![
                "no valid session.".to_string(),
                format!("  nuanalytics db login      # authenticates against {backend}"),
            ],
            Self::ConnectionError(_) => vec![
                format!("could not reach {backend}."),
                "Check the endpoint is correct and the backend is running; this is not a login problem."
                    .to_string(),
            ],
            Self::QueryError(_) => vec![format!(
                "{backend} answered, but the request failed. The message above is the backend's own."
            )],
            // Not folded in with QueryError: a parse failure is *this client's* error, and
            // for a serialisation failure the backend was never contacted at all. Saying
            // "the backend answered" would assert something that did not happen.
            Self::ParseError(_) => vec![format!(
                "the payload could not be parsed. The message above is this client's parse \
                 error, not {backend}'s."
            )],
            Self::IngestError(_) => {
                vec!["the write failed; the message above is the backend's own.".to_string()]
            }
            // No "run this to fix it" step, because there is none that is safe to
            // suggest: the row belongs to someone else and overwriting it is exactly
            // what the ownership policy exists to stop.
            Self::RowOwnedByAnother { table, .. } => vec![
                format!("the `{table}` row is owned by another user on {backend}."),
                "Reads are unaffected — you can still SELECT it. To store your own version, \
                 import under a different program key, or ask the owner to re-import."
                    .to_string(),
            ],
        }
    }

    /// Short machine-readable tag for the failure class, for JSON consumers.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::NotConfigured => "not_configured",
            Self::Disabled => "disabled",
            Self::NotAuthenticated(_) => "not_authenticated",
            Self::ConnectionError(_) => "unreachable",
            Self::QueryError(_) => "query_failed",
            Self::ParseError(_) => "parse_failed",
            Self::IngestError(_) => "ingest_failed",
            Self::RowOwnedByAnother { .. } => "owned_by_another_user",
        }
    }
}

impl std::error::Error for DatabaseError {}

/// Convenience alias for database results
pub type DatabaseResult<T> = Result<T, DatabaseError>;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_display_not_configured() {
        let msg = DatabaseError::NotConfigured.to_string();
        assert!(msg.contains("endpoint"));
    }

    #[test]
    fn test_display_disabled() {
        let msg = DatabaseError::Disabled.to_string();
        assert!(msg.contains("enabled"));
    }

    #[test]
    fn test_display_connection_error_includes_detail() {
        let msg = DatabaseError::ConnectionError("timeout".to_string()).to_string();
        assert!(msg.contains("timeout"));
    }

    #[test]
    fn test_display_query_error_includes_detail() {
        let msg = DatabaseError::QueryError("42501".to_string()).to_string();
        assert!(msg.contains("42501"));
    }

    #[test]
    fn test_display_ingest_error_includes_detail() {
        let msg = DatabaseError::IngestError("No CSV".to_string()).to_string();
        assert!(msg.contains("No CSV"));
    }

    #[test]
    fn test_display_not_authenticated_states_the_fact_without_the_remedy() {
        // Display carries what happened; `next_steps` carries what to do. It used to do
        // both, so a single failure printed `db login` up to three times.
        let msg = DatabaseError::NotAuthenticated("auth file missing".to_string()).to_string();
        assert!(msg.contains("Not signed in"));
        assert!(msg.contains("auth file missing"));
        assert!(
            !msg.contains("db login"),
            "the remedy belongs to next_steps: {msg}"
        );
    }

    /// Compile-time fence for the table below. Adding a `DatabaseError` variant fails to
    /// build here, which is the prompt to add it to `cases`. The array length cannot do
    /// that job: it sat at 7 while an eighth variant shipped unasserted.
    #[allow(dead_code)]
    fn every_variant_is_listed(e: &DatabaseError) {
        match e {
            DatabaseError::NotConfigured
            | DatabaseError::Disabled
            | DatabaseError::NotAuthenticated(_)
            | DatabaseError::ConnectionError(_)
            | DatabaseError::QueryError(_)
            | DatabaseError::ParseError(_)
            | DatabaseError::IngestError(_)
            | DatabaseError::RowOwnedByAnother { .. } => {}
        }
    }

    #[test]
    fn next_steps_and_kind_cover_every_variant() {
        // `next_steps` is the single source of remediation for both the CLI and the MCP
        // server, so every variant's wording is user-facing. Four were previously
        // asserted nowhere.
        let cases: [(DatabaseError, &str, &str); 8] = [
            (
                DatabaseError::NotConfigured,
                "not_configured",
                "config set database.endpoint",
            ),
            (
                DatabaseError::Disabled,
                "disabled",
                "config set database.enabled true",
            ),
            (
                DatabaseError::NotAuthenticated("auth file missing".to_string()),
                "not_authenticated",
                "db login",
            ),
            (
                DatabaseError::ConnectionError("refused".to_string()),
                "unreachable",
                "not a login problem",
            ),
            (
                DatabaseError::QueryError("42501".to_string()),
                "query_failed",
                "answered, but the request failed",
            ),
            (
                DatabaseError::ParseError("eof".to_string()),
                "parse_failed",
                "could not be parsed",
            ),
            (
                DatabaseError::IngestError("no csv".to_string()),
                "ingest_failed",
                "the write failed",
            ),
            (
                DatabaseError::RowOwnedByAnother {
                    table: "programs".to_string(),
                    detail: "duplicate key".to_string(),
                },
                "owned_by_another_user",
                "owned by another user",
            ),
        ];
        for (error, kind, needle) in cases {
            assert_eq!(error.kind(), kind, "kind for {error:?}");
            let steps = error.next_steps("https://nu.example.com").join("\n");
            assert!(steps.contains(needle), "{kind} steps: {steps}");
        }
    }

    #[test]
    fn the_ownership_error_names_the_table_and_suggests_nothing_destructive() {
        // There is no safe remediation to offer: the row belongs to someone else and
        // overwriting it is precisely what the ownership policy exists to prevent. A
        // future edit that helpfully suggests `--force` would undo that.
        let err = DatabaseError::RowOwnedByAnother {
            table: "program_requirements".to_string(),
            detail: "duplicate key value violates unique constraint".to_string(),
        };
        assert!(
            err.to_string().contains("program_requirements"),
            "must name the table: {err}"
        );
        let steps = err.next_steps("https://nu.example.com").join(" ");
        assert!(
            !steps.contains("--force") && !steps.contains("delete"),
            "must not suggest overwriting another user's row: {steps}"
        );
    }

    #[test]
    fn next_steps_names_the_backend_or_says_none_is_configured() {
        for error in [
            DatabaseError::NotAuthenticated("x".to_string()),
            DatabaseError::ConnectionError("x".to_string()),
            DatabaseError::QueryError("x".to_string()),
            DatabaseError::ParseError("x".to_string()),
            DatabaseError::RowOwnedByAnother {
                table: "programs".to_string(),
                detail: "x".to_string(),
            },
        ] {
            let named = error.next_steps("https://nu.example.com").join(" ");
            assert!(
                named.contains("https://nu.example.com"),
                "{error:?} must name the backend: {named}"
            );
            let blank = error.next_steps("").join(" ");
            assert!(
                blank.contains("(no endpoint configured)"),
                "{error:?} must state a blank endpoint rather than leave a gap: {blank}"
            );
        }
    }

    #[test]
    fn an_install_with_no_backend_is_never_told_to_log_in() {
        // There is nothing to log in to; this was the original reported defect.
        for error in [DatabaseError::NotConfigured, DatabaseError::Disabled] {
            let steps = error.next_steps("").join(" ");
            assert!(!steps.contains("db login"), "{error:?}: {steps}");
        }
    }

    #[test]
    fn a_parse_failure_is_not_attributed_to_the_backend() {
        // The payload is this client's serde error, and for a serialisation failure the
        // backend was never contacted — claiming "the backend answered" would assert
        // something that did not happen.
        let steps = DatabaseError::ParseError("expected value".to_string())
            .next_steps("https://nu.example.com")
            .join(" ");
        assert!(
            steps.contains("this client's parse error"),
            "must attribute the error to the client: {steps}"
        );
        assert!(
            !steps.contains("answered, but the request failed"),
            "must not claim the backend answered: {steps}"
        );
    }

    #[test]
    fn remediation_is_stated_once_per_failure() {
        // Display says what happened; next_steps says what to do. `db login` used to
        // appear in the detail, in Display, and in next_steps — three times for one
        // failure.
        let error = DatabaseError::NotAuthenticated("auth file disappeared at /x".to_string());
        let shown = error.to_string();
        assert!(
            !shown.contains("db login"),
            "Display must state the fact only, not the remedy: {shown}"
        );
        let steps = error.next_steps("https://nu.example.com").join("\n");
        assert_eq!(
            steps.matches("db login").count(),
            1,
            "the remedy must appear exactly once: {steps}"
        );
        assert!(
            !steps.contains("/x"),
            "next_steps must not echo the detail the caller already printed: {steps}"
        );
    }
}
