//! Visualization generation for curriculum graphs
//!
//! Provides generators for Mermaid diagrams (for Markdown) and data structures
//! for JavaScript-based visualizations (vis.js/Cytoscape.js for HTML).

pub mod mermaid;

pub use mermaid::MermaidGenerator;
