//! Curriculum graph visualization tool
//!
//! Provides the `get_curriculum_visualization` MCP tool.
//!
//! **Usage flow:**
//! 1. Call `analyze_degree(yaml)` — the response includes a `graph_spec` field
//!    on each entry in `selected_plans`.
//! 2. Serialize `selected_plans[N].graph_spec` to a JSON string.
//! 3. Pass that JSON string as `graph_spec_json` to this tool.
//! 4. The tool returns a self-contained HTML page that can be opened in a browser.

use crate::core::report::visualization::{
    renderer::escape_html, CurriculumGraphRenderer, CurriculumGraphSpec, VanillaJsRenderer,
};
use rmcp::schemars;
use serde::Deserialize;

// ── Request type ──────────────────────────────────────────────────────────────

/// Output shape selector for the `get_curriculum_visualization` tool.
#[derive(Debug, Default, Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VisualizationFormat {
    /// Full `<!DOCTYPE html>…</html>` page that opens directly in a browser.
    #[default]
    Standalone,
    /// Self-contained fragment (`<style>` + `<div>` + `<script>`) suitable for
    /// embedding inside another HTML document. Includes the shared library
    /// inline so the fragment is self-sufficient.
    Fragment,
    /// Same as `Fragment`, but omits the shared `GRAPH_VANILLA_JS` library.
    /// Use when embedding multiple graphs on one page: emit one `Fragment`
    /// (or include the library once via another mechanism), then use this
    /// variant for subsequent graphs to drop ~20 KB per fragment.
    FragmentNoLibrary,
}

/// Request parameters for the `get_curriculum_visualization` tool.
#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct GetCurriculumVisualizationRequest {
    /// Serialized `CurriculumGraphSpec` JSON obtained from `analyze_degree`.
    ///
    /// Copy the value of `selected_plans[N].graph_spec` from an `analyze_degree`
    /// response, serialize it to a JSON string, and pass it here.
    #[schemars(
        description = "Serialized CurriculumGraphSpec JSON from analyze_degree's selected_plans[N].graph_spec"
    )]
    pub graph_spec_json: String,

    /// Output shape: `"standalone"` (default) returns a full HTML page;
    /// `"fragment"` returns a self-contained snippet (style + div + script)
    /// safe to embed inside another document's `<body>`;
    /// `"fragment-no-library"` is the same as `"fragment"` but omits the
    /// shared library — use this for the 2nd+ graph on a page to save ~20 KB
    /// per fragment.
    #[serde(default)]
    #[schemars(
        description = "Output shape: \"standalone\" (default, full HTML page), \"fragment\" (embeddable snippet incl. shared JS library), or \"fragment-no-library\" (smaller snippet for 2nd+ graph on a page)"
    )]
    pub format: VisualizationFormat,
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// Render a [`CurriculumGraphSpec`] as HTML in the requested shape.
///
/// On parse failure returns a minimal HTML error page so the response is
/// always valid HTML (never a raw error string).
#[must_use]
pub fn execute_html(graph_spec_json: &str, format: VisualizationFormat) -> String {
    match serde_json::from_str::<CurriculumGraphSpec>(graph_spec_json) {
        Ok(spec) => match format {
            VisualizationFormat::Standalone => VanillaJsRenderer.render_standalone(&spec),
            VisualizationFormat::Fragment => VanillaJsRenderer.render(&spec),
            VisualizationFormat::FragmentNoLibrary => {
                VanillaJsRenderer.render_without_library(&spec)
            }
        },
        Err(e) => error_html(&format!("Invalid graph_spec JSON: {e}")),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Render a minimal HTML error page so the tool always returns valid HTML.
fn error_html(msg: &str) -> String {
    format!(
        "<!DOCTYPE html>\
         <html lang=\"en\"><head><meta charset=\"UTF-8\">\
         <title>Visualization Error</title></head>\
         <body style=\"font-family:sans-serif;padding:2rem;\">\
         <h2 style=\"color:#c53030\">Visualization Error</h2>\
         <p>{}</p>\
         <p>Make sure you passed the <code>graph_spec</code> field from an \
         <code>analyze_degree</code> response.</p>\
         </body></html>",
        escape_html(msg)
    )
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::report::visualization::curriculum_graph::{
        CourseNode, CurriculumGraphSpec, EdgeType, GraphEdge, TermGroup,
    };

    fn sample_spec() -> CurriculumGraphSpec {
        CurriculumGraphSpec {
            graph_id: "test".to_string(),
            nodes: vec![CourseNode {
                id: "CS101".to_string(),
                name: "Intro".to_string(),
                credits: 4.0,
                complexity: 2,
                delay: 0,
                blocking: 0,
                on_critical_path: false,
                term: 1,
                median_complexity: None,
                median_delay: None,
                median_blocking: None,
            }],
            edges: vec![],
            terms: vec![TermGroup {
                number: 1,
                course_ids: vec!["CS101".to_string()],
            }],
            critical_path_ids: vec![],
        }
    }

    #[test]
    fn test_execute_html_valid_spec() {
        let spec = sample_spec();
        let json = serde_json::to_string(&spec).unwrap();
        let html = execute_html(&json, VisualizationFormat::Standalone);
        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "should be full HTML page"
        );
        assert!(html.contains("nuGraphs.register"), "should include JS");
        assert!(html.contains("CS101"), "should include course");
    }

    #[test]
    fn test_execute_html_invalid_json() {
        let html = execute_html("not-valid-json", VisualizationFormat::Standalone);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Visualization Error"));
    }

    #[test]
    fn test_execute_html_roundtrip() {
        let spec = sample_spec();
        let json = serde_json::to_string(&spec).unwrap();
        let html = execute_html(&json, VisualizationFormat::Standalone);
        assert!(html.contains("graph-test"));
        assert!(html.contains("svg-test"));
    }

    #[test]
    fn test_execute_html_fragment_omits_doctype() {
        let spec = sample_spec();
        let json = serde_json::to_string(&spec).unwrap();
        let html = execute_html(&json, VisualizationFormat::Fragment);
        assert!(
            !html.starts_with("<!DOCTYPE html>"),
            "fragment should not include DOCTYPE"
        );
        assert!(
            !html.contains("<html"),
            "fragment should not include <html>"
        );
        assert!(
            !html.contains("<body"),
            "fragment should not include <body>"
        );
        assert!(
            html.contains("nu-graph"),
            "fragment should include graph CSS"
        );
        assert!(
            html.contains("nuGraphs.register"),
            "fragment should include JS"
        );
        assert!(html.contains("CS101"), "fragment should include course");
    }

    #[test]
    fn test_visualization_format_defaults_to_standalone() {
        let req: GetCurriculumVisualizationRequest =
            serde_json::from_str(r#"{"graph_spec_json":"{}"}"#).unwrap();
        assert!(matches!(req.format, VisualizationFormat::Standalone));
    }

    #[test]
    fn test_visualization_format_deserializes_fragment() {
        let req: GetCurriculumVisualizationRequest =
            serde_json::from_str(r#"{"graph_spec_json":"{}","format":"fragment"}"#).unwrap();
        assert!(matches!(req.format, VisualizationFormat::Fragment));
    }

    #[test]
    fn test_visualization_format_deserializes_fragment_no_library() {
        let req: GetCurriculumVisualizationRequest =
            serde_json::from_str(r#"{"graph_spec_json":"{}","format":"fragment-no-library"}"#)
                .unwrap();
        assert!(matches!(req.format, VisualizationFormat::FragmentNoLibrary));
    }

    #[test]
    fn test_fragment_no_library_omits_shared_library() {
        // FragmentNoLibrary keeps the per-graph register call but drops the
        // shared GRAPH_VANILLA_JS prelude — the size delta is the whole point.
        let spec = sample_spec();
        let json = serde_json::to_string(&spec).unwrap();
        let with_lib = execute_html(&json, VisualizationFormat::Fragment);
        let no_lib = execute_html(&json, VisualizationFormat::FragmentNoLibrary);

        assert!(no_lib.contains("nuGraphs.register"));
        assert!(no_lib.contains("CS101"));
        assert!(
            no_lib.len() < with_lib.len(),
            "FragmentNoLibrary should be smaller than Fragment ({} >= {})",
            no_lib.len(),
            with_lib.len()
        );
        // The library exposes a `nuGraphs` namespace via `window.nuGraphs = ...`;
        // FragmentNoLibrary must NOT contain that initialization.
        assert!(
            !no_lib.contains("window.nuGraphs ="),
            "library prelude (window.nuGraphs = ...) must not appear in FragmentNoLibrary"
        );
        assert!(
            with_lib.contains("window.nuGraphs ="),
            "library prelude must appear in Fragment"
        );
    }

    #[test]
    fn test_execute_html_with_edges() {
        let spec = CurriculumGraphSpec {
            graph_id: "edgetest".to_string(),
            nodes: vec![
                CourseNode {
                    id: "CS101".to_string(),
                    name: "Intro".to_string(),
                    credits: 4.0,
                    complexity: 2,
                    delay: 1,
                    blocking: 1,
                    on_critical_path: true,
                    term: 1,
                    median_complexity: Some(3.0),
                    median_delay: Some(1.5),
                    median_blocking: Some(1.0),
                },
                CourseNode {
                    id: "CS201".to_string(),
                    name: "Data Structures".to_string(),
                    credits: 4.0,
                    complexity: 8,
                    delay: 2,
                    blocking: 0,
                    on_critical_path: true,
                    term: 2,
                    median_complexity: None,
                    median_delay: None,
                    median_blocking: None,
                },
            ],
            edges: vec![GraphEdge {
                from: "CS101".to_string(),
                to: "CS201".to_string(),
                edge_type: EdgeType::Prerequisite,
            }],
            terms: vec![
                TermGroup {
                    number: 1,
                    course_ids: vec!["CS101".to_string()],
                },
                TermGroup {
                    number: 2,
                    course_ids: vec!["CS201".to_string()],
                },
            ],
            critical_path_ids: vec!["CS101".to_string(), "CS201".to_string()],
        };
        let json = serde_json::to_string(&spec).unwrap();
        let html = execute_html(&json, VisualizationFormat::Standalone);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("\"from\":\"CS101\""));
        assert!(html.contains("\"to\":\"CS201\""));
        assert!(html.contains("\"dashes\":false")); // prerequisite edge
    }
}
