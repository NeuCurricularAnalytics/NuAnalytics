//! CLI command handlers and utilities for `NuAnalytics`.
//!
//! This module provides handlers for CLI subcommands and shared utilities.
//!
//! ## Command Handlers
//! - [`config`] - Configuration management
//! - [`degree`] - Degree program validation
//! - [`planner`] - Curriculum planning and CSV export
//! - [`mcp`] - MCP server for AI model integration (feature-gated)
//!
//! ## Utilities
//! - [`report`] - Report generation utilities (used by multiple commands)

pub mod config;
pub mod degree;
pub mod planner;
pub mod report;

#[cfg(feature = "mcp")]
pub mod mcp;
