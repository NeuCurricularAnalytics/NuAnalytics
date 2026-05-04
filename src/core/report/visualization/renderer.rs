//! Renderer trait and implementations for curriculum graph visualization.
//!
//! A [`CurriculumGraphRenderer`] converts a [`CurriculumGraphSpec`] to HTML.
//! The trait is designed for swappability: the current implementation is
//! [`VanillaJsRenderer`] (no external dependencies); future implementations
//! could use D3.js, Cytoscape.js, etc.

use std::fmt::Write as FmtWrite;

use super::curriculum_graph::{CurriculumGraphSpec, EdgeType};

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
        render_legend(&mut out);
        render_graph_html(spec, id, &mut out);
        render_script(spec, id, &mut out);
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
  font-size: .65rem; color: #666; line-height: 1.2;
  max-height: 2.4em; overflow: hidden;
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
            let node = spec.nodes.iter().find(|n| &n.id == course_id);
            let complexity = node.map_or(0, |n| n.complexity);
            let name = node.map_or("", |n| n.name.as_str());
            let short_name = if name.len() > 22 { &name[..19] } else { name };
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
                escape_html(short_name)
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

/// Emit the `<script>` block that registers graph data and triggers the first draw.
///
/// The shared `GRAPH_VANILLA_JS` asset is included inline but guards itself with
/// `if (!window.nuGraphs)` so it is safe to emit multiple times on one page.
fn render_script(spec: &CurriculumGraphSpec, id: &str, out: &mut String) {
    let edges_json = edges_to_json(&spec.edges);
    let critical_json = ids_to_json(&spec.critical_path_ids);
    let id_json = js_string(id);

    let _ = write!(
        out,
        "<script>\n\
         {GRAPH_VANILLA_JS}\n\
         nuGraphs.register({id_json}, {{ edges: {edges_json}, criticalPath: {critical_json} }});\n\
         if (document.readyState !== 'loading') {{\n\
           nuGraphs.draw({id_json});\n\
           nuGraphs.attachHoverHandlers();\n\
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

/// Serialize a string as a JS string literal (double-quoted, basic escaping).
fn js_string(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

/// Serialize graph edges to a JSON array.
///
/// Each edge becomes `{"from":"ID","to":"ID","dashes":bool}` where `dashes`
/// is `true` for corequisite edges and `false` for prerequisites.
fn edges_to_json(edges: &[super::curriculum_graph::GraphEdge]) -> String {
    let items: Vec<String> = edges
        .iter()
        .map(|e| {
            let dashes = matches!(e.edge_type, EdgeType::Corequisite);
            format!(
                "{{\"from\":{},\"to\":{},\"dashes\":{}}}",
                js_string(&e.from),
                js_string(&e.to),
                dashes
            )
        })
        .collect();
    format!("[{}]", items.join(","))
}

/// Serialize a list of course IDs to a JSON array of strings.
fn ids_to_json(ids: &[String]) -> String {
    let items: Vec<String> = ids.iter().map(|id| js_string(id)).collect();
    format!("[{}]", items.join(","))
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
    fn test_edges_to_json_prereq_and_coreq() {
        use super::super::curriculum_graph::{EdgeType, GraphEdge};
        let edges = vec![
            GraphEdge {
                from: "CS101".to_string(),
                to: "CS201".to_string(),
                edge_type: EdgeType::Prerequisite,
            },
            GraphEdge {
                from: "CS101".to_string(),
                to: "CS101L".to_string(),
                edge_type: EdgeType::Corequisite,
            },
        ];
        let json = edges_to_json(&edges);
        assert!(json.contains("\"from\":\"CS101\""));
        assert!(json.contains("\"to\":\"CS201\""));
        assert!(json.contains("\"dashes\":false"));
        assert!(json.contains("\"dashes\":true"));
    }

    #[test]
    fn test_ids_to_json_multiple() {
        let ids = vec!["CS101".to_string(), "CS201".to_string()];
        assert_eq!(ids_to_json(&ids), "[\"CS101\",\"CS201\"]");
    }

    #[test]
    fn test_ids_to_json_empty() {
        assert_eq!(ids_to_json(&[]), "[]");
    }

    #[test]
    fn test_render_encodes_edge_as_dashes_false_for_prereq() {
        let html = VanillaJsRenderer.render(&minimal_spec());
        // The spec has one Prerequisite edge; dashes must be false
        assert!(html.contains("\"dashes\":false"));
        assert!(!html.contains("\"dashes\":true"));
    }
}
