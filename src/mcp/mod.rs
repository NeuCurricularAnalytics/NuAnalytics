//! MCP (Model Context Protocol) server for `NuAnalytics`
//!
//! This module provides an MCP server that exposes `NuAnalytics` to a model: author,
//! validate, analyze and render degrees; read the stored programs and their analysis; and
//! query IPEDS completion data, through typed tools or read-only SQL.
//!
//! # Architecture
//!
//! ```text
//! src/mcp/
//! ├── mod.rs              # This file - module exports
//! ├── server.rs           # The tools, their router, instructions, source resolution
//! ├── envelope.rs         # A failed call's JSON → a protocol error with one envelope
//! ├── cache.rs            # Cached degree bodies (`cache:` handles) and analysis runs
//! ├── schema_content.rs   # Sections of the degree-format reference
//! └── tools/              # One module per degree tool, plus shared argument types
//! ```
//!
//! The layering rule: a handler in `server.rs` parses its arguments and calls one engine.
//! The database engines live in [`crate::core::query`], shared with the CLI; SQL lives in
//! its catalog and reaches the database only through `DbClient`. `core` must stay free
//! of `crate::mcp` — the CI matrix builds `--features database` alone to enforce it.
//! `compare_degrees` lives in `tools/degrees.rs` because fresh metrics need the analysis,
//! which is still in this module.
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
//! // Async: the database config, and whether to serve the tools that write.
//! mcp::run_server(&config.database, false).await?;
//!
//! // Sync wrapper
//! mcp::run(&config.database, false)?;
//! ```

pub mod cache;
pub mod envelope;
pub mod schema_content;
pub mod server;
pub mod tools;

// Re-export main entry points
pub use server::{run, tool_list};
