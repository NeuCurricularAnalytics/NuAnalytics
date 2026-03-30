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

/// Run the MCP server
///
/// This is a thin wrapper that delegates to the mcp module.
///
/// # Errors
///
/// Returns an error string if the server fails to start or run.
pub fn run() -> Result<(), String> {
    nu_analytics::mcp::run()
}
