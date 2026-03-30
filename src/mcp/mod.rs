//! MCP (Model Context Protocol) server for `NuAnalytics`
//!
//! This module provides an MCP server that exposes `NuAnalytics` tools for AI model integration.
//! The server allows AI models to:
//!
//! - Get schema documentation for degree YAML files
//! - Validate degree YAML content and receive structured feedback
//! - (Future) Audit degrees, analyze plans, and more
//!
//! # Architecture
//!
//! ```text
//! src/mcp/
//! ├── mod.rs              # This file - module exports
//! ├── server.rs           # MCP server setup and entry point
//! ├── tools/              # Tool implementations
//! │   ├── mod.rs          # Tool exports
//! │   ├── schema.rs       # get_degree_schema tool
//! │   └── validate.rs     # validate_degree tool
//! └── schema_content.rs   # Static schema documentation
//! ```
//!
//! # Usage
//!
//! The MCP server is typically launched via the CLI:
//!
//! ```sh
//! nuanalytics mcp
//! ```
//!
//! Or programmatically:
//!
//! ```ignore
//! use nu_analytics::mcp;
//!
//! // Async
//! mcp::run_server().await?;
//!
//! // Sync wrapper
//! mcp::run()?;
//! ```

pub mod schema_content;
pub mod server;
pub mod tools;

// Re-export main entry points
pub use server::{run, run_server};
