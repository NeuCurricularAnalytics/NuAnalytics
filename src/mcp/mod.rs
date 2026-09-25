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
//! │   ├── mod.rs          # Tool exports (+ re-exports of the core query engines)
//! │   ├── schema.rs       # get_degree_schema tool
//! │   └── validate.rs     # validate_degree tool
//! └── schema_content.rs   # Static schema documentation
//! ```
//!
//! The database **query** engines are not here. `institutions`, `cip_codes`, `lookup`,
//! `completions` and most of `degrees` live in [`crate::core::query`] so the CLI can call
//! the same code without the `mcp` feature; `tools/mod.rs` re-exports them under their
//! old names, so the `#[tool]` handlers in `server.rs` are unaffected. `core` must stay
//! free of `crate::mcp` — the CI matrix builds `--features database` alone to enforce it.
//! The one exception is `compare_degrees`' analysis hook, which `tools/degrees.rs`
//! injects because the analysis pipeline is still MCP-gated.
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

pub mod cache;
pub mod schema_content;
pub mod server;
pub mod tools;

// Re-export main entry points
pub use server::run;
