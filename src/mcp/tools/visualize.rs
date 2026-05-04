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
}

// ── Execution ─────────────────────────────────────────────────────────────────

/// Render a [`CurriculumGraphSpec`] as a self-contained HTML page.
///
/// On parse failure returns a minimal HTML error page so the response is
/// always valid HTML (never a raw error string).
#[must_use]
pub fn execute_html(graph_spec_json: &str) -> String {
    match serde_json::from_str::<CurriculumGraphSpec>(graph_spec_json) {
        Ok(spec) => VanillaJsRenderer.render_standalone(&spec),
        Err(e) => error_html(&format!("Invalid graph_spec JSON: {e}")),
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

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
                on_critical_path: false,
                term: 1,
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
        let html = execute_html(&json);
        assert!(
            html.starts_with("<!DOCTYPE html>"),
            "should be full HTML page"
        );
        assert!(html.contains("nuGraphs.register"), "should include JS");
        assert!(html.contains("CS101"), "should include course");
    }

    #[test]
    fn test_execute_html_invalid_json() {
        let html = execute_html("not-valid-json");
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("Visualization Error"));
    }

    #[test]
    fn test_execute_html_roundtrip() {
        let spec = sample_spec();
        let json = serde_json::to_string(&spec).unwrap();
        let html = execute_html(&json);
        assert!(html.contains("graph-test"));
        assert!(html.contains("svg-test"));
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
                    on_critical_path: true,
                    term: 1,
                },
                CourseNode {
                    id: "CS201".to_string(),
                    name: "Data Structures".to_string(),
                    credits: 4.0,
                    complexity: 8,
                    on_critical_path: true,
                    term: 2,
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
        let html = execute_html(&json);
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("\"from\":\"CS101\""));
        assert!(html.contains("\"to\":\"CS201\""));
        assert!(html.contains("\"dashes\":false")); // prerequisite edge
    }
}
