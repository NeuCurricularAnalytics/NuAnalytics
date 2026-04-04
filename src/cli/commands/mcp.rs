//! MCP (Model Context Protocol) server CLI command
//!
//! This module provides the CLI entry point for the MCP server.
//! The actual server implementation is in `nu_analytics::mcp`.
//!
//! # Usage
//!
//! ```sh
//! nuanalytics mcp
//! ```

use nu_analytics::config::DatabaseConfig;

/// Run the MCP server
///
/// Passes database configuration through to the server for optional database-backed tools.
///
/// # Errors
///
/// Returns an error string if the server fails to start or run.
pub fn run(db_config: &DatabaseConfig) -> Result<(), String> {
    nu_analytics::mcp::run(db_config)
}
