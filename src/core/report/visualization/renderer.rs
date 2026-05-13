//! Renderer trait and implementations for curriculum graph visualization.
//!
//! A [`CurriculumGraphRenderer`] converts a [`CurriculumGraphSpec`] to HTML.
//! The trait is designed for swappability: the current implementation is
//! [`VanillaJsRenderer`] (no external dependencies); future implementations
//! could use D3.js, Cytoscape.js, etc.

use std::collections::HashMap;
use std::fmt::Write as FmtWrite;

use serde::Serialize;

use super::curriculum_graph::{CourseNode, CurriculumGraphSpec, EdgeType, GraphEdge};

// ── Embedded JS asset ─────────────────────────────────────────────────────────

const GRAPH_VANILLA_JS: &str = include_str!("../../../assets/graph_vanilla.js");

// ── Trait ─────────────────────────────────────────────────────────────────────

/// A renderer that converts a [`CurriculumGraphSpec`] to HTML.
pub trait CurriculumGraphRenderer {
    /// Render the spec as a **self-contained HTML fragment**.
    ///
    /// The returned string contains:
    /// - A `<style>` block with scoped `.nu-graph` CSS (safe to embed in any page).
    /// - A `<div class="nu-graph curriculum-graph-wrapper">` with term columns,
    ///   course nodes, and an SVG overlay.
    /// - A `<script>` block that registers the graph data with `nuGraphs` and
    ///   triggers the first draw.
    ///
    /// The fragment is safe to embed directly in a larger HTML document.
    #[must_use]
    fn render(&self, spec: &CurriculumGraphSpec) -> String;

    /// Render the spec as a **standalone HTML page**.
    ///
    /// Wraps [`Self::render`] output in a minimal `<!DOCTYPE html>` shell.
    /// Used by the MCP tool so the returned string can be opened directly in
    /// a browser.
    #[must_use]
    fn render_standalone(&self, spec: &CurriculumGraphSpec) -> String {
        let fragment = self.render(spec);
        format!(
            "<!DOCTYPE html>\n\
             <html lang=\"en\">\n\
             <head>\n\
             <meta charset=\"UTF-8\">\n\
             <meta name=\"viewport\" content=\"width=device-width, initial-scale=1.0\">\n\
             <title>Curriculum Graph</title>\n\
             </head>\n\
             <body style=\"margin:1rem;font-family:sans-serif;\">\n\
             {fragment}\n\
             </body>\n\
             </html>"
        )
    }
}

// ── VanillaJsRenderer ─────────────────────────────────────────────────────────

/// Renders a [`CurriculumGraphSpec`] using only vanilla JavaScript — no external
/// libraries required.
///
/// The output embeds the shared `graph_vanilla.js` asset (guarded against
/// double-inclusion) plus a per-graph registration call.  Multiple instances on
/// the same page work independently through the `nuGraphs` registry.
pub struct VanillaJsRenderer;

impl CurriculumGraphRenderer for VanillaJsRenderer {
    fn render(&self, spec: &CurriculumGraphSpec) -> String {
        let id = &spec.graph_id;
        let mut out = String::new();
        render_style(&mut out);
        render_modal_style(&mut out);
        render_legend(&mut out);
        render_graph_html(spec, id, &mut out);
        render_script(spec, id, &mut out, true);
        out
    }
}

impl VanillaJsRenderer {
    /// Render a fragment without inlining the shared `GRAPH_VANILLA_JS` library.
    ///
    /// Useful when a single page embeds many graphs: include the library once,
    /// then emit one ~10 KB fragment per graph instead of one ~30 KB fragment
    /// per graph. The caller is responsible for ensuring `window.nuGraphs` is
    /// defined before the fragment's script runs (e.g. by emitting the full
    /// `render` output for the first graph and `render_without_library` for
    /// the rest).
    #[must_use]
    pub fn render_without_library(&self, spec: &CurriculumGraphSpec) -> String {
        let id = &spec.graph_id;
        let mut out = String::new();
        render_style(&mut out);
        render_modal_style(&mut out);
        render_legend(&mut out);
        render_graph_html(spec, id, &mut out);
        render_script(spec, id, &mut out, false);
        out
    }
}

// ── Render helpers ────────────────────────────────────────────────────────────

/// Emit the `<style>` block with scoped `.nu-graph` CSS.
///
/// All selectors are prefixed with `.nu-graph` so the output is safe to embed
/// inside any larger HTML document without CSS collisions.
fn render_style(out: &mut String) {
    // `{{` and `}}` are literal braces in the format string (CSS uses them for
    // selector blocks), not Rust format placeholders.
    let _ = write!(
        out,
        r"<style>
.nu-graph {{
  --nu-primary:  #2c3e50;
  --nu-secondary:#3498db;
  --nu-success:  #4CAF50;
  --nu-warning:  #FF9800;
  --nu-danger:   #F44336;
  --nu-critical: #9C27B0;
  --nu-term-bg:  #e8e8e8;
  --nu-node:     #37474f;
}}
.nu-graph .curriculum-graph-wrapper {{
  overflow-x: auto; margin: 1rem 0; position: relative;
}}
.nu-graph .curriculum-graph {{
  display: flex; gap: 0; width: 100%; position: relative;
}}
.nu-graph .term-column {{
  display: flex; flex-direction: column; align-items: center;
  flex: 1; min-width: 100px;
  background: var(--nu-term-bg);
  border-radius: 4px; margin: 0 16px; padding: 10px 6px;
}}
.nu-graph .term-header {{
  font-weight: 600; font-size: 0.75rem; color: var(--nu-primary);
  margin-bottom: 10px; padding: 4px 8px;
  background: white; border-radius: 4px; width: 100%; text-align: center;
  box-sizing: border-box;
}}
.nu-graph .term-courses {{
  display: flex; flex-direction: column; gap: 16px; width: 100%;
}}
.nu-graph .course-node {{
  position: relative; background: white; border-radius: 6px;
  padding: 10px 8px 8px; text-align: center;
  border: 2px solid var(--nu-node); cursor: pointer;
  transition: transform .2s, box-shadow .2s, opacity .2s;
  margin-top: 10px;
}}
.nu-graph .course-node:hover {{
  transform: translateY(-2px); box-shadow: 0 4px 8px rgba(0,0,0,.15);
}}
.nu-graph .course-node.faded {{ opacity: .25; }}
.nu-graph .course-node.highlighted {{
  border-color: var(--nu-secondary); border-width: 3px;
  box-shadow: 0 0 8px rgba(52,152,219,.5);
}}
.nu-graph .course-node.critical-highlight {{
  border-color: var(--nu-critical); border-width: 3px;
  box-shadow: 0 0 8px rgba(156,39,176,.5);
}}
.nu-graph .course-node .complexity-badge {{
  position: absolute; top: -12px; left: 50%; transform: translateX(-50%);
  width: 22px; height: 22px; border-radius: 50%;
  display: flex; align-items: center; justify-content: center;
  font-size: .65rem; font-weight: bold; color: white;
  border: 2px solid white; box-shadow: 0 1px 3px rgba(0,0,0,.2);
}}
.nu-graph .complexity-low  {{ background-color: var(--nu-success);  }}
.nu-graph .complexity-medium {{ background-color: var(--nu-warning); }}
.nu-graph .complexity-high {{ background-color: var(--nu-danger);   }}
.nu-graph .course-node .course-id {{
  font-weight: 600; font-size: .75rem; color: var(--nu-primary); margin-top: 6px;
}}
.nu-graph .course-node .course-name {{
  font-size: .65rem; color: #666; line-height: 1.2; display: -webkit-box;
  -webkit-line-clamp: 2; -webkit-box-orient: vertical; overflow: hidden; text-overflow: ellipsis;
}}
.nu-graph .connections-svg {{
  position: absolute; top: 0; left: 0;
  pointer-events: none; width: 100%; height: 100%; min-height: 600px;
}}
.nu-graph .prereq-line {{
  stroke: #666; stroke-width: 1.5; fill: none;
  transition: stroke .2s, stroke-width .2s, opacity .2s;
}}
.nu-graph .coreq-line {{
  stroke: var(--nu-secondary); stroke-width: 1.5; stroke-dasharray: 4,3; fill: none;
  transition: stroke .2s, stroke-width .2s, opacity .2s;
}}
.nu-graph .prereq-line.faded, .nu-graph .coreq-line.faded {{ opacity: .15; }}
.nu-graph .prereq-line.highlighted {{ stroke: var(--nu-secondary); stroke-width: 2.5; }}
.nu-graph .coreq-line.highlighted  {{ stroke: var(--nu-secondary); stroke-width: 2.5; }}
.nu-graph .prereq-line.critical, .nu-graph .coreq-line.critical {{
  stroke: var(--nu-critical); stroke-width: 3;
}}
.nu-graph .graph-legend {{
  display: flex; gap: 1.5rem; margin: .75rem 0; flex-wrap: wrap; font-size: .8rem;
}}
.nu-graph .legend-item {{ display: flex; align-items: center; gap: .4rem; }}
.nu-graph .legend-color {{ width: 16px; height: 16px; border-radius: 50%; }}
.nu-graph .legend-line  {{ width: 30px; height: 2px; }}
.nu-graph .legend-line.solid    {{ background: #666; }}
.nu-graph .legend-line.dashed   {{ background: repeating-linear-gradient(90deg,var(--nu-secondary),var(--nu-secondary) 4px,transparent 4px,transparent 7px); }}
.nu-graph .legend-line.critical {{ background: var(--nu-critical); height: 3px; }}
</style>
"
    );
}

/// Emit a separate `<style>` block for the course-detail modal.
///
/// The modal is appended to `<body>`, not inside `.nu-graph`, so its selectors
/// are *not* descendants of `.nu-graph`.  All class names are prefixed with
/// `.nu-graph-modal-` to keep them safely namespaced from host page styles.
fn render_modal_style(out: &mut String) {
    // `{{` and `}}` are literal CSS selector braces, not Rust placeholders.
    let _ = write!(
        out,
        r"<style>
.nu-graph-modal-backdrop {{
  position: fixed; inset: 0; background: rgba(0,0,0,0.45);
  display: flex; align-items: center; justify-content: center;
  z-index: 9999; animation: nu-graph-modal-fade .15s ease-out;
}}
.nu-graph-modal {{
  background: white; border-radius: 8px; padding: 1.25rem 1.5rem;
  min-width: 320px; max-width: 92vw; max-height: 86vh; overflow: auto;
  box-shadow: 0 10px 30px rgba(0,0,0,0.25); position: relative;
  font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
  color: #2c3e50;
}}
.nu-graph-modal-close {{
  position: absolute; top: .5rem; right: .6rem;
  background: transparent; border: none; cursor: pointer;
  font-size: 1.4rem; line-height: 1; color: #666; padding: .2rem .4rem;
}}
.nu-graph-modal-close:hover {{ color: #2c3e50; }}
.nu-graph-modal-title {{
  margin: 0 1.5rem .25rem 0; font-size: 1.1rem; font-weight: 600;
}}
.nu-graph-modal-subtitle {{
  margin: 0 0 .9rem 0; font-size: .85rem; color: #666;
}}
.nu-graph-modal-table {{
  width: 100%; border-collapse: collapse; font-size: .9rem;
}}
.nu-graph-modal-table th, .nu-graph-modal-table td {{
  padding: .35rem .6rem; text-align: right; border-bottom: 1px solid #e8e8e8;
}}
.nu-graph-modal-table th:first-child, .nu-graph-modal-table td:first-child {{
  text-align: left; font-weight: 600;
}}
.nu-graph-modal-table thead th {{
  background: #f4f6f8; font-size: .75rem; text-transform: uppercase;
  letter-spacing: .03em; color: #555;
}}
@keyframes nu-graph-modal-fade {{
  from {{ opacity: 0; }}
  to   {{ opacity: 1; }}
}}
</style>
"
    );
}

/// Emit the legend bar (complexity colour key + edge type key).
fn render_legend(out: &mut String) {
    let _ = write!(
        out,
        r#"<div class="nu-graph">
<div class="graph-legend">
  <div class="legend-item"><div class="legend-color complexity-low"></div><span>Low (1&#x2013;5)</span></div>
  <div class="legend-item"><div class="legend-color complexity-medium"></div><span>Medium (6&#x2013;15)</span></div>
  <div class="legend-item"><div class="legend-color complexity-high"></div><span>High (16+)</span></div>
  <div class="legend-item"><div class="legend-line solid"></div><span>Prerequisite</span></div>
  <div class="legend-item"><div class="legend-line dashed"></div><span>Corequisite</span></div>
  <div class="legend-item"><div class="legend-line critical"></div><span>Critical path</span></div>
</div>
"#
    );
}

/// Emit term columns, course nodes, SVG overlay, and closing wrappers.
fn render_graph_html(spec: &CurriculumGraphSpec, id: &str, out: &mut String) {
    // Build a course-id → node lookup once. The inner loop previously did
    // `spec.nodes.iter().find(...)` for every term × course pairing, which
    // was O(terms × courses²) — pathological for 300+ course curricula.
    let node_by_id: HashMap<&str, &CourseNode> =
        spec.nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let _ = writeln!(out, "<div class=\"curriculum-graph-wrapper\">");
    let _ = writeln!(out, "<div class=\"curriculum-graph\" id=\"graph-{id}\">");

    for term in &spec.terms {
        if term.course_ids.is_empty() {
            continue;
        }
        let _ = writeln!(out, "<div class=\"term-column\">");
        let _ = writeln!(
            out,
            "<div class=\"term-header\">Semester {}</div>",
            term.number
        );
        let _ = writeln!(out, "<div class=\"term-courses\">");

        for course_id in &term.course_ids {
            let node = node_by_id.get(course_id.as_str()).copied();
            let complexity = node.map_or(0, |n| n.complexity);
            let name = node.map_or("", |n| n.name.as_str());
            let cls = complexity_class(complexity);

            let _ = writeln!(
                out,
                "<div class=\"course-node\" data-course-id=\"{course_id}\" data-graph-id=\"{id}\">"
            );
            let _ = writeln!(
                out,
                "<span class=\"complexity-badge {cls}\">{complexity}</span>"
            );
            let _ = writeln!(out, "<div class=\"course-id\">{course_id}</div>");
            let _ = writeln!(
                out,
                "<div class=\"course-name\">{}</div>",
                escape_html(name)
            );
            let _ = writeln!(out, "</div>");
        }

        let _ = writeln!(out, "</div>"); // term-courses
        let _ = writeln!(out, "</div>"); // term-column
    }

    let _ = writeln!(out, "</div>"); // curriculum-graph
    let _ = writeln!(out, "<svg class=\"connections-svg\" id=\"svg-{id}\"></svg>");
    let _ = writeln!(out, "</div>"); // curriculum-graph-wrapper
    let _ = writeln!(out, "</div>"); // nu-graph
}

/// JS-payload shape for one edge.  The renderer flattens [`EdgeType`] to a
/// boolean `dashes` field because the JS draws prerequisite vs corequisite
/// edges as solid vs dashed lines.
#[derive(Serialize)]
struct EdgeJs<'a> {
    from: &'a str,
    to: &'a str,
    dashes: bool,
}

impl<'a> From<&'a GraphEdge> for EdgeJs<'a> {
    fn from(e: &'a GraphEdge) -> Self {
        Self {
            from: &e.from,
            to: &e.to,
            dashes: matches!(e.edge_type, EdgeType::Corequisite),
        }
    }
}

/// JS-payload shape for one course node.  Differs from [`CourseNode`] in two
/// ways: field names are camelCase (JS convention) and `term` /
/// `on_critical_path` are dropped because the modal does not display them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct NodeJs<'a> {
    id: &'a str,
    name: &'a str,
    credits: f32,
    complexity: usize,
    delay: usize,
    blocking: usize,
    median_complexity: Option<f32>,
    median_delay: Option<f32>,
    median_blocking: Option<f32>,
}

impl<'a> From<&'a CourseNode> for NodeJs<'a> {
    fn from(n: &'a CourseNode) -> Self {
        Self {
            id: &n.id,
            name: &n.name,
            credits: n.credits,
            complexity: n.complexity,
            delay: n.delay,
            blocking: n.blocking,
            median_complexity: n.median_complexity,
            median_delay: n.median_delay,
            median_blocking: n.median_blocking,
        }
    }
}

/// Emit the `<script>` block that registers graph data and triggers the first draw.
///
/// The shared `GRAPH_VANILLA_JS` asset is included inline but guards itself with
/// `if (!window.nuGraphs)` so it is safe to emit multiple times on one page.
fn render_script(spec: &CurriculumGraphSpec, id: &str, out: &mut String, include_library: bool) {
    // serde_json::to_string never fails for these fully-owned, finite types.
    let edges_js: Vec<EdgeJs<'_>> = spec.edges.iter().map(EdgeJs::from).collect();
    let nodes_js: Vec<NodeJs<'_>> = spec.nodes.iter().map(NodeJs::from).collect();
    let edges_json = serde_json::to_string(&edges_js).unwrap_or_else(|_| "[]".to_string());
    let nodes_json = serde_json::to_string(&nodes_js).unwrap_or_else(|_| "[]".to_string());
    let critical_json =
        serde_json::to_string(&spec.critical_path_ids).unwrap_or_else(|_| "[]".to_string());
    let id_json = serde_json::to_string(id).unwrap_or_else(|_| "\"\"".to_string());

    let library = if include_library {
        GRAPH_VANILLA_JS
    } else {
        ""
    };
    let _ = write!(
        out,
        "<script>\n\
         {library}\n\
         nuGraphs.register({id_json}, {{ edges: {edges_json}, criticalPath: {critical_json}, nodes: {nodes_json} }});\n\
         if (document.readyState !== 'loading') {{\n\
           nuGraphs.draw({id_json});\n\
           nuGraphs.attachHoverHandlers();\n\
           nuGraphs.attachClickHandlers();\n\
         }}\n\
         </script>\n",
    );
}

// ── Other helpers ─────────────────────────────────────────────────────────────

/// Map a complexity score to its CSS badge class name.
const fn complexity_class(c: usize) -> &'static str {
    match c {
        0..=5 => "complexity-low",
        6..=15 => "complexity-medium",
        _ => "complexity-high",
    }
}

/// Escape HTML special characters for safe embedding in element content.
pub(crate) fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::core::report::visualization::curriculum_graph::{
        CourseNode, CurriculumGraphSpec, GraphEdge, TermGroup,
    };

    fn minimal_spec() -> CurriculumGraphSpec {
        CurriculumGraphSpec {
            graph_id: "test".to_string(),
            nodes: vec![
                CourseNode {
                    id: "CS101".to_string(),
                    name: "Intro to CS".to_string(),
                    credits: 4.0,
                    complexity: 3,
                    delay: 1,
                    blocking: 1,
                    on_critical_path: true,
                    term: 1,
                    median_complexity: None,
                    median_delay: None,
                    median_blocking: None,
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
        }
    }

    #[test]
    fn test_render_contains_graph_id() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("graph-test"), "expected graph-test DOM id");
        assert!(html.contains("svg-test"), "expected svg-test DOM id");
    }

    #[test]
    fn test_render_contains_course_nodes() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("data-course-id=\"CS101\""));
        assert!(html.contains("data-course-id=\"CS201\""));
        assert!(html.contains("data-graph-id=\"test\""));
    }

    #[test]
    fn test_render_contains_nu_graphs_register() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("nuGraphs.register"));
        assert!(html.contains("\"CS101\""));
        assert!(html.contains("\"CS201\""));
    }

    #[test]
    fn test_render_contains_nu_graph_class() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("class=\"nu-graph\"") || html.contains("class=\"nu-graph "));
    }

    #[test]
    fn test_render_standalone_is_full_page() {
        let html = VanillaJsRenderer.render_standalone(&minimal_spec());
        assert!(html.starts_with("<!DOCTYPE html>"));
        assert!(html.contains("<body"));
        assert!(html.contains("nuGraphs.register"));
    }

    #[test]
    fn test_render_without_library_drops_shared_prelude() {
        // Direct renderer-level coverage for the size-saving variant: the
        // per-graph register call and structural HTML must remain, but the
        // shared GRAPH_VANILLA_JS prelude (window.nuGraphs = …) must NOT.
        let with_lib = VanillaJsRenderer.render(&minimal_spec());
        let without_lib = VanillaJsRenderer.render_without_library(&minimal_spec());

        assert!(
            !without_lib.contains("window.nuGraphs ="),
            "render_without_library must drop the shared library prelude"
        );
        assert!(
            with_lib.contains("window.nuGraphs ="),
            "control: render() should still inline the library"
        );
        assert!(without_lib.contains("nuGraphs.register"));
        assert!(without_lib.contains(".nu-graph"));
        assert!(without_lib.contains("data-graph-id=\"test\""));
        assert!(
            without_lib.len() < with_lib.len(),
            "library-less variant must be smaller ({} >= {})",
            without_lib.len(),
            with_lib.len()
        );
    }

    #[test]
    fn test_complexity_classes() {
        assert_eq!(complexity_class(0), "complexity-low");
        assert_eq!(complexity_class(5), "complexity-low");
        assert_eq!(complexity_class(6), "complexity-medium");
        assert_eq!(complexity_class(15), "complexity-medium");
        assert_eq!(complexity_class(16), "complexity-high");
    }

    #[test]
    fn test_escape_html_special_chars() {
        assert_eq!(escape_html("<script>"), "&lt;script&gt;");
        assert_eq!(escape_html("a&b"), "a&amp;b");
        assert_eq!(escape_html("\"quoted\""), "&quot;quoted&quot;");
        assert_eq!(escape_html("plain"), "plain");
    }

    #[test]
    fn test_render_encodes_both_edge_types() {
        use super::super::curriculum_graph::{EdgeType, GraphEdge};
        let mut spec = minimal_spec();
        spec.edges.push(GraphEdge {
            from: "CS101".to_string(),
            to: "CS101L".to_string(),
            edge_type: EdgeType::Corequisite,
        });
        let html = VanillaJsRenderer.render(&spec);
        assert!(html.contains("\"from\":\"CS101\""));
        assert!(html.contains("\"to\":\"CS201\""));
        assert!(
            html.contains("\"dashes\":false"),
            "prerequisite edge should serialize as dashes:false"
        );
        assert!(
            html.contains("\"dashes\":true"),
            "corequisite edge should serialize as dashes:true"
        );
    }

    #[test]
    fn test_render_encodes_edge_as_dashes_false_for_prereq() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        // The spec has one Prerequisite edge; dashes must be false
        assert!(html.contains("\"dashes\":false"));
        assert!(!html.contains("\"dashes\":true"));
    }

    #[test]
    fn test_render_critical_path_serializes_as_array() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        // critical_path_ids = ["CS101", "CS201"] should serialize verbatim.
        assert!(html.contains("criticalPath: [\"CS101\",\"CS201\"]"));
    }

    #[test]
    fn test_render_includes_node_data_in_register_call() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        // The register call must carry per-node detail used by the modal.
        assert!(
            html.contains("nodes:"),
            "register call should include nodes array"
        );
        assert!(html.contains("\"complexity\":3"));
        assert!(html.contains("\"complexity\":8"));
        assert!(html.contains("\"delay\":1"));
        assert!(html.contains("\"delay\":2"));
        assert!(html.contains("\"blocking\":1"));
    }

    #[test]
    fn test_render_emits_null_for_missing_medians() {
        // minimal_spec() builds nodes with all median_* = None
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("\"medianComplexity\":null"));
        assert!(html.contains("\"medianDelay\":null"));
        assert!(html.contains("\"medianBlocking\":null"));
    }

    #[test]
    fn test_render_emits_numbers_for_present_medians() {
        let mut spec = minimal_spec();
        spec.nodes[0].median_complexity = Some(5.5);
        spec.nodes[0].median_delay = Some(2.0);
        spec.nodes[0].median_blocking = Some(0.5);
        let html = VanillaJsRenderer.render(&spec);
        // serde_json serialises f32 with a fractional part, so 2.0 → "2.0".
        assert!(html.contains("\"medianComplexity\":5.5"));
        assert!(html.contains("\"medianDelay\":2.0"));
        assert!(html.contains("\"medianBlocking\":0.5"));
    }

    #[test]
    fn test_render_includes_modal_css_classes() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains(".nu-graph-modal-backdrop"));
        assert!(html.contains(".nu-graph-modal-close"));
        assert!(html.contains(".nu-graph-modal-table"));
    }

    #[test]
    fn test_render_wires_up_click_handlers() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        assert!(html.contains("attachClickHandlers"));
    }

    /// Long names previously got byte-sliced to 19 chars; the full name now
    /// flows into the HTML and CSS line-clamp handles overflow at render time.
    #[test]
    fn test_render_emits_full_course_name_without_truncation() {
        let mut spec = minimal_spec();
        let long_name = "Calculus for Physical Scientists I";
        spec.nodes[0].name = long_name.to_string();
        let html = VanillaJsRenderer.render(&spec);
        assert!(
            html.contains(long_name),
            "rendered HTML should carry the full course name, got: {html}"
        );
        // The old 19-byte truncation produced "Calculus for Physic" — guard
        // against regression by asserting the truncated form is NOT present
        // as a standalone course-name body.
        assert!(
            !html.contains(">Calculus for Physic<"),
            "course name must not be byte-truncated to 19 chars"
        );
    }

    /// The previous `&name[..19]` byte slice panicked when byte 19 fell
    /// inside a multi-byte UTF-8 codepoint. Removing the slice eliminates
    /// the latent panic; this test pins down the new behavior.
    #[test]
    fn test_render_does_not_panic_on_non_ascii_course_name() {
        let mut spec = minimal_spec();
        // Build a name where the boundary at byte 19 lands mid-codepoint:
        // ASCII chars get 1 byte, "é" gets 2, "🎓" gets 4. Crafting the
        // string so byte 19 sits inside a multi-byte char.
        spec.nodes[0].name = "Introducción a la algorítmica 🎓".to_string();
        // Should not panic — and the full name should appear in the HTML.
        let html = VanillaJsRenderer.render(&spec);
        assert!(html.contains("Introducción"));
    }

    /// Guard against regressions of the O(terms × courses²) hot loop fix.
    /// Release-only: debug builds are far slower at HashMap construction +
    /// String growth, and the goal is to catch a real perf regression, not
    /// quirks of unoptimised builds.
    #[cfg(not(debug_assertions))]
    #[test]
    fn test_render_500_course_curriculum_under_5s_release() {
        use std::time::Instant;

        let course_count: usize = 500;
        let term_count: usize = 8;
        let courses_per_term = course_count.div_ceil(term_count);

        let mut nodes = Vec::with_capacity(course_count);
        let mut terms = Vec::with_capacity(term_count);
        for term_idx in 0..term_count {
            let start = term_idx * courses_per_term;
            let end = ((term_idx + 1) * courses_per_term).min(course_count);
            let mut course_ids = Vec::with_capacity(end - start);
            for i in start..end {
                let id = format!("CS{i:04}");
                course_ids.push(id.clone());
                nodes.push(CourseNode {
                    id,
                    name: format!("Synthetic Course {i}"),
                    credits: 4.0,
                    complexity: (i % 30) + 1,
                    delay: (i % 8) + 1,
                    blocking: i % 5,
                    on_critical_path: i % 17 == 0,
                    term: term_idx + 1,
                    median_complexity: None,
                    median_delay: None,
                    median_blocking: None,
                });
            }
            terms.push(TermGroup {
                number: term_idx + 1,
                course_ids,
            });
        }

        let spec = CurriculumGraphSpec {
            graph_id: "bench".to_string(),
            nodes,
            edges: vec![],
            terms,
            critical_path_ids: vec![],
        };

        let start = Instant::now();
        let html = VanillaJsRenderer.render_standalone(&spec);
        let elapsed = start.elapsed();

        assert!(!html.is_empty());
        assert!(
            elapsed.as_secs() < 5,
            "render_standalone on 500-course spec took {elapsed:?}; \
             threshold is 5 s. The hot loop in render_graph_html may \
             have regressed to O(n²) lookup."
        );
    }
}
