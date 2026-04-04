//! Database error types

use std::fmt;

/// Errors that can occur during database operations
#[derive(Debug)]
pub enum DatabaseError {
    /// Database is not configured (missing endpoint or token)
    NotConfigured,
    /// Database is disabled in configuration
    Disabled,
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
                "Database not configured. Set `endpoint` and `token` in [database] config."
            ),
            Self::Disabled => write!(
                f,
                "Database is disabled. Set `enabled = true` in [database] config."
            ),
            Self::ConnectionError(msg) => write!(f, "Database connection error: {msg}"),
            Self::QueryError(msg) => write!(f, "Database query error: {msg}"),
            Self::ParseError(msg) => write!(f, "Data parse error: {msg}"),
            Self::IngestError(msg) => write!(f, "Data ingest error: {msg}"),
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
}
