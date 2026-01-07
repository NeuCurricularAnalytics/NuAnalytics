//! CLI command handlers and utilities for `NuAnalytics`.
//!
//! This module provides handlers for CLI subcommands and shared utilities.
//!
//! ## Command Handlers
//! - [`config`] - Configuration management
//! - [`planner`] - Curriculum planning and CSV export
//!
//! ## Utilities
//! - [`report`] - Report generation utilities (used by multiple commands)

pub mod config;
pub mod planner;
pub mod report;
