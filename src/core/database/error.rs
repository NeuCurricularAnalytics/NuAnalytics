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
            Self::NotAuthenticated(detail) => write!(
                f,
                "Not signed in ({detail}). Run `nuanalytics db login` first."
            ),
            Self::ConnectionError(msg) => write!(f, "Database connection error: {msg}"),
            Self::QueryError(msg) => write!(f, "Database query error: {msg}"),
            Self::ParseError(msg) => write!(f, "Data parse error: {msg}"),
            Self::IngestError(msg) => write!(f, "Data ingest error: {msg}"),
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
        let backend = if endpoint.is_empty() {
            "(no endpoint configured)"
        } else {
            endpoint
        };
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
            Self::NotAuthenticated(detail) => vec![
                format!("no valid session ({detail})."),
                format!("  nuanalytics db login      # authenticates against {backend}"),
            ],
            Self::ConnectionError(_) => vec![
                format!("could not reach {backend}."),
                "Check the endpoint is correct and the backend is running; this is not a login problem."
                    .to_string(),
            ],
            Self::QueryError(_) | Self::ParseError(_) => vec![format!(
                "{backend} answered, but the request failed. The message above is the backend's own."
            )],
            Self::IngestError(_) => {
                vec!["the write failed; the message above is the backend's own.".to_string()]
            }
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
    fn test_display_not_authenticated_prompts_login() {
        let msg = DatabaseError::NotAuthenticated("auth file missing".to_string()).to_string();
        assert!(msg.contains("Not signed in"));
        assert!(msg.contains("auth file missing"));
        assert!(msg.contains("db login"));
    }
}
