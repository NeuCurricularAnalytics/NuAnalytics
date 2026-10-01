//! Rust integration test modules

pub mod cli_degree;
pub mod course_graph;
pub mod course_syntax;
pub mod cross_listing;
/// Guards that parsing a degree and serializing it back drops no field.
pub mod degree_fidelity;
/// Fixtures and the analysis helpers the target-course modules share.
pub mod degree_fixtures;
pub mod degree_yaml;
pub mod end_to_end;
pub mod group_choice;
pub mod logger;
pub mod metrics_comparison;
/// The MCP and the CLI configuration give identical results; needs `mcp` for the tool.
#[cfg(feature = "mcp")]
pub mod one_pipeline;
pub mod plan_generation;
pub mod planner;
#[cfg(feature = "mcp")]
pub mod report_tool;
pub mod smoke;
pub mod statistics;
pub mod target_course_population;
#[cfg(feature = "mcp")]
pub mod target_course_selected_plans;
pub mod validation;
