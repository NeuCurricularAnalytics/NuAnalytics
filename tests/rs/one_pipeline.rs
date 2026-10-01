//! `degree analyze` and the MCP's `analyze_degree` run one pipeline,
//! `core::degree::analysis::analyze`, and differ only in their defaults.
//!
//! Guards against either surface growing a pipeline step of its own: one that builds
//! courses, equivalences, prerequisite expansion or the seed differently makes the same
//! degree give different figures on the two surfaces, and fresh MCP figures disagree with
//! the stored corpus, which the CLI produced. This runs the MCP tool and the core pipeline,
//! configured as the CLI configures it, on real fixtures at equal settings and asserts the
//! results are identical — exactly, not within a tolerance.

use nu_analytics::core::config::Config;
use nu_analytics::core::degree::analysis::{analyze, AnalysisConfig, DegreeAnalysis};
use nu_analytics::core::degree::{parse_degree_auto, SelectedPlans};
use nu_analytics::core::statistics::MetricStats;
use nu_analytics::mcp::tools::analyze::{
    execute, AnalysisResponse, AnalyzeOptions, MetricStatsJson,
};

use super::degree_fixtures::{SYRACUSE, TULANE, TXSTATE};

/// Below each fixture's population (2,000 to 128,000 plans), so the seeded sampling path
/// runs rather than full enumeration.
const MAX_PLANS: usize = 150;

/// The MCP's fixed Random Sample count.
const MCP_SAMPLE_COUNT: usize = 3;

/// The CLI's analysis of `text`: the compiled default configuration, mapped as
/// `degree analyze` maps it, at `MAX_PLANS` and the MCP's sample count.
fn cli_analysis(text: &str) -> DegreeAnalysis {
    let (program, _) = parse_degree_auto(text).expect("fixture parses");
    let config = AnalysisConfig {
        max_plans: MAX_PLANS,
        sample_count: MCP_SAMPLE_COUNT,
        ..AnalysisConfig::from_config(&Config::from_defaults().degree_analysis)
    };
    analyze(program, &config, &mut |_| {})
}

fn mcp_analysis(text: &str) -> AnalysisResponse {
    let response = execute(
        text,
        &AnalyzeOptions {
            max_plans: Some(MAX_PLANS),
            ..AnalyzeOptions::default()
        },
    );
    assert!(response.success, "{:?}", response.error);
    response
}

/// The seven figures reported for a metric, as bit patterns: the comparison is exact.
fn summary_bits(s: &MetricStats) -> [u64; 7] {
    [s.min, s.q1, s.median, s.q3, s.max, s.mean, s.std_dev].map(f64::to_bits)
}

fn summary_bits_json(s: Option<&MetricStatsJson>) -> [u64; 7] {
    let s = s.expect("the MCP reports this metric");
    [s.min, s.q1, s.median, s.q3, s.max, s.mean, s.std_dev].map(f64::to_bits)
}

/// `(complexity, longest delay, critical path, courses)` of each selected plan, in order.
fn selected(plans: &SelectedPlans) -> Vec<(usize, usize, Vec<String>, usize)> {
    plans
        .iter()
        .map(|(_, p)| {
            (
                p.score.total_complexity,
                p.score.longest_delay,
                p.score.longest_delay_chain.clone(),
                p.variant.courses.len(),
            )
        })
        .collect()
}

#[test]
fn the_mcp_and_the_cli_configuration_give_identical_results() {
    // The CLI's defaults are what the stored corpus was run with; if they stop being the
    // MCP's, this comparison no longer describes the two surfaces.
    let defaults = Config::from_defaults().degree_analysis;
    assert!(defaults.ignore_duplicates);
    assert_eq!(defaults.sampling_strategy, "shuffled");

    for (name, text) in [
        ("Syracuse", SYRACUSE),
        ("Texas State", TXSTATE),
        ("Tulane", TULANE),
    ] {
        let cli = cli_analysis(text);
        let mcp = mcp_analysis(text);
        let stats = cli.aggregator.degree_stats();

        assert!(!cli.is_full_population(), "{name}: sampled, not enumerated");
        assert_eq!(mcp.plans_analyzed, cli.plans_processed, "{name}: plans");
        assert_eq!(mcp.seed_used, cli.seed_used, "{name}: seed");
        for (metric, core, tool) in [
            (
                "complexity",
                &stats.total_complexity,
                mcp.complexity.as_ref(),
            ),
            (
                "longest delay",
                &stats.longest_delay,
                mcp.longest_delay.as_ref(),
            ),
            ("credits", &stats.total_credits, mcp.total_credits.as_ref()),
            (
                "average chain length",
                &stats.avg_chain_length,
                mcp.avg_chain_length.as_ref(),
            ),
        ] {
            assert_eq!(
                summary_bits(core),
                summary_bits_json(tool),
                "{name}: {metric}"
            );
        }
        let tool_plans: Vec<_> = mcp
            .selected_plans
            .iter()
            .map(|p| {
                (
                    p.complexity,
                    p.longest_delay,
                    p.critical_path.clone(),
                    p.course_count,
                )
            })
            .collect();
        assert_eq!(
            tool_plans,
            selected(&cli.selected),
            "{name}: selected plans"
        );
    }
}
