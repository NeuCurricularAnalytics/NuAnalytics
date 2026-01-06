//! Report format implementations
//!
//! Provides exporters for different report formats: Markdown, HTML, and PDF.

pub mod html;
pub mod markdown;

pub use html::HtmlReporter;
pub use markdown::MarkdownReporter;

use std::fmt;
use std::str::FromStr;

/// Supported report formats
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportFormat {
    /// Markdown format with Mermaid diagrams
    Markdown,
    /// HTML format with interactive vis.js graphs
    Html,
    /// PDF format (generated from HTML)
    Pdf,
}

impl ReportFormat {
    /// Get the file extension for this format
    #[must_use]
    pub const fn extension(&self) -> &'static str {
        match self {
            Self::Markdown => "md",
            Self::Html => "html",
            Self::Pdf => "pdf",
        }
    }
}

impl FromStr for ReportFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "md" | "markdown" => Ok(Self::Markdown),
            "html" | "htm" => Ok(Self::Html),
            "pdf" => Ok(Self::Pdf),
            _ => Err(format!("Unknown report format: {s}")),
        }
    }
}

impl fmt::Display for ReportFormat {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Markdown => write!(f, "markdown"),
            Self::Html => write!(f, "html"),
            Self::Pdf => write!(f, "pdf"),
        }
    }
}
