//! Visualization generation for curriculum graphs.
//!
//! Provides:
//! - [`CurriculumGraphSpec`] — canonical serializable data format (the
//!   "intermediate format" between the analysis pipeline and the renderer).
//! - [`CurriculumGraphRenderer`] trait + [`VanillaJsRenderer`] — swappable
//!   renderers that convert a spec to self-contained HTML.
//! - [`MermaidGenerator`] — Mermaid flowchart syntax for Markdown reports.

pub mod curriculum_graph;
pub mod mermaid;
pub mod renderer;

pub use curriculum_graph::{
    spec_from_components, spec_from_report_context, spec_from_scored_plan, CourseNode,
    CurriculumGraphSpec, EdgeType, GraphEdge, TermGroup,
};
pub use mermaid::MermaidGenerator;
pub use renderer::{CurriculumGraphRenderer, VanillaJsRenderer};
