//! Rust integration test modules

pub mod cli_degree;
pub mod course_graph;
pub mod course_syntax;
pub mod cross_listing;
/// Guards that parsing a degree and serializing it back drops no field. Uses the
/// vendored fixtures, so it carries their `mcp` gate.
#[cfg(feature = "mcp")]
pub mod degree_fidelity;
/// Fixtures and the analysis helper the target-course modules share. Needs `mcp`,
/// which is where the analysis pipeline lives.
#[cfg(feature = "mcp")]
pub mod degree_fixtures;
pub mod degree_yaml;
pub mod end_to_end;
pub mod logger;
pub mod metrics_comparison;
pub mod plan_generation;
pub mod planner;
#[cfg(feature = "mcp")]
pub mod report_tool;
pub mod smoke;
pub mod statistics;
#[cfg(feature = "mcp")]
pub mod target_course_population;
#[cfg(feature = "mcp")]
pub mod target_course_selected_plans;
pub mod validation;
