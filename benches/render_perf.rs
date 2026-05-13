//! Renderer perf benchmark.
//!
//! Builds a synthetic 500-course / 8-term / 1200-edge `CurriculumGraphSpec`
//! and times the three rendering paths. Used to guard against regressions
//! in the renderer hot loop — the previous O(terms × courses²) lookup turned
//! 4-minute render times into <10 s after switching to a `HashMap`.

// Synthetic fixture data is small-integer arithmetic only; the precision
// concerns clippy flags don't apply, and bench files generate no public docs.
#![allow(clippy::cast_precision_loss, missing_docs)]

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use nu_analytics::core::report::visualization::{
    CourseNode, CurriculumGraphRenderer, CurriculumGraphSpec, EdgeType, GraphEdge, TermGroup,
    VanillaJsRenderer,
};

/// Build a synthetic spec with `course_count` nodes spread across
/// `term_count` terms. Each node has a single prerequisite edge to a node
/// from the prior term — yielding roughly `course_count - courses_per_term`
/// prerequisite edges plus a sprinkling of corequisites (every 5th node).
fn synthetic_spec(course_count: usize, term_count: usize) -> CurriculumGraphSpec {
    let courses_per_term = course_count.div_ceil(term_count);
    let mut nodes = Vec::with_capacity(course_count);
    let mut terms = Vec::with_capacity(term_count);
    let mut edges = Vec::new();

    for term_idx in 0..term_count {
        let start = term_idx * courses_per_term;
        let end = ((term_idx + 1) * courses_per_term).min(course_count);
        let mut course_ids = Vec::with_capacity(end - start);

        for i in start..end {
            let id = format!("CS{i:04}");
            course_ids.push(id.clone());
            nodes.push(CourseNode {
                id: id.clone(),
                name: format!("Synthetic Course {i}"),
                credits: 4.0,
                complexity: (i % 30) + 1,
                delay: (i % 8) + 1,
                blocking: i % 5,
                on_critical_path: i % 17 == 0,
                term: term_idx + 1,
                median_complexity: Some(((i % 30) + 1) as f32),
                median_delay: Some(((i % 8) + 1) as f32),
                median_blocking: Some((i % 5) as f32),
            });

            // Prereq edge from the previous term, when one exists.
            if term_idx > 0 {
                let from_idx = ((term_idx - 1) * courses_per_term) + (i % courses_per_term);
                if from_idx < start {
                    edges.push(GraphEdge {
                        from: format!("CS{from_idx:04}"),
                        to: id.clone(),
                        edge_type: EdgeType::Prerequisite,
                    });
                }
            }
            // Sprinkle in corequisites every 5th node.
            if i > 0 && i % 5 == 0 {
                edges.push(GraphEdge {
                    from: format!("CS{:04}", i - 1),
                    to: id,
                    edge_type: EdgeType::Corequisite,
                });
            }
        }

        terms.push(TermGroup {
            number: term_idx + 1,
            course_ids,
        });
    }

    let critical_path_ids = nodes
        .iter()
        .filter(|n| n.on_critical_path)
        .map(|n| n.id.clone())
        .collect();

    CurriculumGraphSpec {
        graph_id: "bench".to_string(),
        nodes,
        edges,
        terms,
        critical_path_ids,
    }
}

fn bench_render_500_courses(c: &mut Criterion) {
    let spec = synthetic_spec(500, 8);
    let renderer = VanillaJsRenderer;

    let mut group = c.benchmark_group("render_500_courses_8_terms");
    // Each render allocates ~150 KB; sample size of 10 keeps the bench short
    // while still giving criterion enough data for a confidence interval.
    group.sample_size(10);

    group.bench_function("standalone", |b| {
        b.iter(|| black_box(renderer.render_standalone(black_box(&spec))));
    });
    group.bench_function("fragment", |b| {
        b.iter(|| black_box(renderer.render(black_box(&spec))));
    });
    group.bench_function("fragment_no_library", |b| {
        b.iter(|| black_box(renderer.render_without_library(black_box(&spec))));
    });
    group.finish();
}

criterion_group!(benches, bench_render_500_courses);
criterion_main!(benches);
