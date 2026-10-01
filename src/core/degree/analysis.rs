//! Degree analysis: enumerate a degree's plans, expand each, aggregate their metrics.
//!
//! One pipeline, which `degree analyze` and the MCP analysis tools both call; each keeps
//! only its own output layer and defaults.
//!
//! The stored corpus was produced by this pipeline, so everything that feeds a metric is
//! load-bearing: prerequisite expansion avoids alternatives to `--include` courses and
//! prunes redundant prerequisites, the seed comes from the canonical degree, and courses
//! and equivalences come from `core::report::inputs`. Changing any of them moves stored
//! figures. A wall-clock limit and target-course statistics are options, off unless asked
//! for.
//!
//! Nothing here prints. Progress is reported through [`AnalysisEvent`]s, which the CLI
//! renders when verbose and the MCP ignores.

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use serde::Serialize;

use crate::core::degree::placeholder::ELECTIVE_PREFIX;
use crate::core::degree::plan_variant::{ELECTIVE_PLACEHOLDERS_KEY, PREREQUISITES_KEY};
use crate::core::degree::{
    default_seed_for_program, PlanGenerationStats, PlanGenerator, PlanGeneratorConfig,
    PlanSelector, PlanSelectorConfig, PlanVariant, SamplingStrategy, SelectedPlans,
};
use crate::core::metrics::compute_all_metrics;
use crate::core::models::course_graph::{CourseNode, PrerequisiteEdge, PrerequisiteType};
use crate::core::models::{CourseGraph, DegreeProgram, School};
use crate::core::report::inputs::{build_equivalence_map, build_school_from_program};
use crate::core::report::report_stats::ReportStats;
use crate::core::report::term_scheduler::{
    course_credits_with_fallback, SchedulerConfig, TermScheduler,
};
use crate::core::statistics::aggregator::{AggregatorConfig, MetricsAggregator};

/// How an analysis run is bounded and what it computes beyond the aggregate metrics.
///
/// The defaults of each surface — the CLI's from `Config`, the MCP's from its request —
/// are the caller's business; this is only what the pipeline needs.
#[derive(Debug, Clone)]
pub struct AnalysisConfig<'a> {
    /// Stop after this many distinct plans.
    pub max_plans: usize,
    /// Skip a plan whose requirement choices repeat an earlier one.
    pub ignore_duplicates: bool,
    /// Random Sample plans to keep.
    pub sample_count: usize,
    /// Order the plans are visited in.
    pub sampling_strategy: SamplingStrategy,
    /// Courses every plan must include.
    pub include_courses: Vec<String>,
    /// `None` derives the seed from the degree ([`default_seed_for_program`]), so the same
    /// degree always yields the same plans.
    pub random_seed: Option<u64>,
    /// Stop enumerating once this much wall-clock time has passed. `None` runs to
    /// `max_plans` or exhaustion.
    pub time_limit: Option<Duration>,
    /// Record which term this course lands in across the plans.
    pub target_course: Option<&'a str>,
}

impl AnalysisConfig<'_> {
    /// The analysis a configuration's `[degree_analysis]` section describes — what
    /// `degree analyze` runs before its command-line options override it. An unknown
    /// `sampling_strategy` falls back to the default.
    #[must_use]
    pub fn from_config(config: &crate::core::config::DegreeAnalysisConfig) -> Self {
        Self {
            max_plans: config.max_plans,
            ignore_duplicates: config.ignore_duplicates,
            sample_count: config.sample_plan_count,
            sampling_strategy: config.sampling_strategy.parse().unwrap_or_default(),
            include_courses: Vec::new(),
            // Derived from the degree itself, so the same degree always enumerates the
            // same plans — see `default_seed_for_program`.
            random_seed: None,
            time_limit: None,
            target_course: None,
        }
    }
}

/// Progress an analysis reports while it runs.
#[derive(Debug)]
pub enum AnalysisEvent<'a> {
    /// The prerequisite graph had cycles; these edges were removed to break them.
    CyclesBroken {
        /// How many cycles were found.
        cycles: usize,
        /// `(course, prerequisite)` edges removed.
        removed: &'a [(String, String)],
    },
    /// Enumeration is about to start.
    Planning {
        /// What the generator estimates.
        stats: &'a PlanGenerationStats,
        /// The generator's configuration.
        config: &'a PlanGeneratorConfig,
    },
    /// This many plans have been analyzed so far.
    Processed(usize),
    /// A plan was skipped because its metrics could not be computed.
    PlanSkipped(&'a str),
}

/// Term-reach statistics for one slice of plans (all plans, or calc-ready only).
#[derive(Debug, Default, Clone, Serialize)]
pub struct TargetTermStats {
    /// Number of plans that contained the target course.
    pub plans_containing: usize,
    /// Earliest term number the course was scheduled in, across containing plans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub earliest_term: Option<usize>,
    /// Mean scheduled term number across all containing plans.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub avg_term: Option<f64>,
    /// How many plans landed on each term number. Sorted by term for readability.
    pub term_distribution: std::collections::BTreeMap<usize, usize>,
}

/// Earliest-semester statistics for a specific target course.
///
/// `all_plans` covers every plan that contained the course. `calc_ready_plans` is the
/// subset of those where the plan also includes a recognised calculus course — plans
/// where a calc-ready student would reach the target faster.
#[derive(Debug, Clone, Serialize)]
pub struct TargetCourseStats {
    /// The course ID that was looked up.
    pub course_id: String,
    /// Set when the target course did not appear in any generated plan.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Stats across all plans that contained the target course.
    pub all_plans: TargetTermStats,
    /// Stats restricted to plans that also include a calculus course.
    pub calc_ready_plans: TargetTermStats,
}

/// Everything one analysis run produces.
#[derive(Debug)]
pub struct DegreeAnalysis {
    /// The degree analyzed.
    pub program: DegreeProgram,
    /// Its prerequisite graph, cycles broken.
    pub graph: CourseGraph,
    /// Its courses, keyed by document key.
    pub school: School,
    /// Each course's equivalents, from `{A, B}` groups.
    pub equivalences: HashMap<String, HashSet<String>>,
    /// The generator configuration the run used, seed included.
    pub gen_config: PlanGeneratorConfig,
    /// Every analyzed plan's metrics.
    pub aggregator: MetricsAggregator,
    /// The aggregator reduced to what reports read.
    pub report_stats: ReportStats,
    /// Shortest, longest, calc-ready and Random Sample plans.
    pub selected: SelectedPlans,
    /// The generator's estimates; `total_possible` bounds the distinct plans.
    pub stats: PlanGenerationStats,
    /// Plans analyzed.
    pub plans_processed: usize,
    /// The seed the run used.
    pub seed_used: u64,
    /// Whether the time limit stopped enumeration.
    pub time_limit_reached: bool,
    /// Wall-clock time of the enumeration loop, in milliseconds.
    pub time_elapsed_ms: u64,
    /// Where the target course landed, when one was asked for.
    pub target_course_stats: Option<TargetCourseStats>,
}

impl DegreeAnalysis {
    /// The plan cap the run used.
    #[must_use]
    pub const fn max_plans(&self) -> usize {
        self.gen_config.max_plans
    }

    /// True when every distinct plan was analyzed — the cap was not hit, or was hit but
    /// the population did not exceed it.
    #[must_use]
    pub const fn is_full_population(&self) -> bool {
        !(self.plans_processed >= self.max_plans() && self.stats.total_possible > self.max_plans())
    }

    /// The processed count when the run covered everything, otherwise the estimate of
    /// all distinct plans.
    #[must_use]
    pub const fn population_size(&self) -> usize {
        if self.is_full_population() {
            self.plans_processed
        } else {
            self.stats.total_possible
        }
    }
}

/// Analyze `program`: enumerate its plans, expand and measure each, aggregate, select.
#[must_use]
pub fn analyze(
    program: DegreeProgram,
    config: &AnalysisConfig<'_>,
    on_event: &mut dyn FnMut(AnalysisEvent<'_>),
) -> DegreeAnalysis {
    let graph_result = CourseGraph::from_degree_program(&program);
    let cycles = graph_result.cycles.len();
    let (graph, removed) = graph_result.into_acyclic();
    if cycles > 0 {
        on_event(AnalysisEvent::CyclesBroken {
            cycles,
            removed: &removed,
        });
    }

    let equivalences = build_equivalence_map(&program.requirements);
    let exclude_from_prereqs = build_exclude_set(&config.include_courses, &graph);
    let school = build_school_from_program(&program);
    let seed_used = config
        .random_seed
        .unwrap_or_else(|| default_seed_for_program(&program));

    let gen_config = PlanGeneratorConfig {
        random_seed: Some(seed_used),
        max_plans: config.max_plans,
        ignore_duplicates: config.ignore_duplicates,
        sample_count: config.sample_count,
        target_credits: program.degree.total_credits,
        sampling_strategy: config.sampling_strategy.clone(),
        include_courses: config.include_courses.clone(),
        exclude_courses: exclude_from_prereqs.iter().cloned().collect(),
    };
    let generator = PlanGenerator::new(&program.requirements, &program.courses, gen_config.clone());
    let stats = generator.get_stats();
    on_event(AnalysisEvent::Planning {
        stats: &stats,
        config: &gen_config,
    });

    let mut aggregator = MetricsAggregator::new(AggregatorConfig {
        reservoir_size: 1000,
        track_per_course: true,
        exact_mode: stats.total_possible <= 10000,
    });
    let mut selector = PlanSelector::new(
        &school,
        PlanSelectorConfig {
            sample_count: gen_config.sample_count,
            scheduler_config: SchedulerConfig::default(),
            random_seed: Some(seed_used),
            ..Default::default()
        },
    );

    let ctx = LoopContext {
        program: &program,
        graph: &graph,
        school: &school,
        equivalences: &equivalences,
        exclude_from_prereqs: &exclude_from_prereqs,
        gen_config: &gen_config,
    };
    let loop_start = Instant::now();
    let outcome = run_plans(
        &ctx,
        &generator,
        config.time_limit.map(|limit| loop_start + limit),
        config.target_course,
        &mut aggregator,
        &mut selector,
        on_event,
    );
    let time_elapsed_ms = u64::try_from(loop_start.elapsed().as_millis()).unwrap_or(u64::MAX);
    let selected = selector.into_selected_plans();

    let target_course_stats = config.target_course.map(|course_id| {
        build_target_course_stats(
            course_id,
            &outcome.target_all_terms,
            &outcome.target_calc_ready_terms,
            outcome.plans_processed,
        )
    });
    let report_stats = ReportStats::from_aggregator(&aggregator);
    DegreeAnalysis {
        program,
        graph,
        school,
        equivalences,
        gen_config,
        aggregator,
        report_stats,
        selected,
        stats,
        plans_processed: outcome.plans_processed,
        seed_used,
        time_limit_reached: outcome.time_limit_reached,
        time_elapsed_ms,
        target_course_stats,
    }
}

/// What the plan loop reads.
struct LoopContext<'a> {
    program: &'a DegreeProgram,
    graph: &'a CourseGraph,
    school: &'a School,
    equivalences: &'a HashMap<String, HashSet<String>>,
    exclude_from_prereqs: &'a HashSet<String>,
    gen_config: &'a PlanGeneratorConfig,
}

/// What the plan loop found, beyond what it put in the aggregator and selector.
struct LoopOutcome {
    plans_processed: usize,
    time_limit_reached: bool,
    target_all_terms: Vec<usize>,
    target_calc_ready_terms: Vec<usize>,
}

/// Expand, measure, aggregate and select each generated plan.
///
/// Stops at `max_plans`, at `deadline`, or when the generator runs out.
fn run_plans(
    ctx: &LoopContext<'_>,
    generator: &PlanGenerator<'_>,
    deadline: Option<Instant>,
    target_course: Option<&str>,
    aggregator: &mut MetricsAggregator,
    selector: &mut PlanSelector<'_>,
    on_event: &mut dyn FnMut(AnalysisEvent<'_>),
) -> LoopOutcome {
    let mut outcome = LoopOutcome {
        plans_processed: 0,
        time_limit_reached: false,
        target_all_terms: Vec::new(),
        target_calc_ready_terms: Vec::new(),
    };
    let mut seen_fingerprints = HashSet::new();
    // Same set for every plan, so build it once.
    let include_set: HashSet<String> = ctx.gen_config.include_courses.iter().cloned().collect();
    // Requirements marked at conversion as existing only to reach the total. Read from the
    // document's flag, never inferred here — see `core::degree::fill_electives`.
    let fill_ids =
        crate::core::degree::fill_electives::fill_requirement_ids(&ctx.program.requirements);

    for variant in generator.generate() {
        if outcome.plans_processed >= ctx.gen_config.max_plans {
            break;
        }
        // `Instant::now()` is sub-µs on every tier-1 target, so polling each plan is cheap
        // next to the scheduling and metrics that follow.
        if deadline.is_some_and(|d| Instant::now() >= d) {
            outcome.time_limit_reached = true;
            break;
        }

        if ctx.gen_config.ignore_duplicates {
            let fp = variant.fingerprint();
            if seen_fingerprints.contains(&fp) {
                continue;
            }
            seen_fingerprints.insert(fp);
        }

        let mut expanded_courses = expand_courses_with_prerequisites(
            &variant.courses,
            ctx.graph,
            ctx.equivalences,
            ctx.exclude_from_prereqs,
            &include_set,
        );

        // Size fill-to-total blocks to this plan. Done *before* the DAG and metrics so all
        // three — metrics, credits and schedule — describe the same course list; doing it
        // later would leave complexity counting placeholders the plan no longer contains.
        let fill = resize_fill_blocks(ctx, &expanded_courses, &variant, &fill_ids);
        if let Some(resize) = &fill {
            expanded_courses.clone_from(&resize.courses);
        }

        // The final plan first — the `ELEC` filler re-fitted to the expanded course list —
        // and only then its DAG and metrics. The other way round, metrics counted the
        // generator's draft filler: a draft a credit short got an `ELEC` that prerequisite
        // expansion then made unnecessary, so the scheduled plan lacked a placeholder that
        // its complexity still included (Binghamton: 126 credits on target, complexity 331
        // for a plan whose own courses sum to 330).
        let expanded_variant = create_expanded_variant(
            &variant,
            &expanded_courses,
            ctx.school,
            ctx.gen_config.target_credits,
            fill.as_ref(),
        );

        let plan_dag = crate::core::degree::build_plan_dag(
            &expanded_variant.courses,
            ctx.graph,
            ctx.equivalences,
            &include_set,
        );
        let course_metrics = match compute_all_metrics(&plan_dag) {
            Ok(metrics) => metrics,
            Err(e) => {
                on_event(AnalysisEvent::PlanSkipped(&e));
                continue;
            }
        };

        // The target course's actual scheduled term. Scheduled from the expanded course
        // list — the selector's input — so a course without prerequisites is not placed in
        // a term it would not really occupy.
        if let Some(term) =
            target_course.and_then(|c| scheduled_term(ctx.school, &plan_dag, &expanded_variant, c))
        {
            outcome.target_all_terms.push(term);
            if selector.is_calc_ready_plan(&variant) {
                outcome.target_calc_ready_terms.push(term);
            }
        }

        // The variant's total credits include elective placeholders.
        aggregator.add_plan(&course_metrics, f64::from(expanded_variant.total_credits));
        selector.process_plan(&expanded_variant, &course_metrics, &plan_dag);

        outcome.plans_processed += 1;
        on_event(AnalysisEvent::Processed(outcome.plans_processed));
    }
    outcome
}

/// Shrink a plan's fill-to-total blocks so it reaches the degree's credit target and no
/// further, or `None` when there is no target or nothing to shrink.
fn resize_fill_blocks(
    ctx: &LoopContext<'_>,
    courses: &[String],
    variant: &PlanVariant,
    fill_ids: &[String],
) -> Option<crate::core::degree::fill_electives::FillResize> {
    let target = ctx.gen_config.target_credits?;
    #[allow(clippy::cast_precision_loss)] // target credits < 1000
    crate::core::degree::fill_electives::shrink_fill_blocks(
        courses,
        &variant.requirement_choices,
        fill_ids,
        target as f32,
        |c| course_credits_with_fallback(ctx.school, c),
        |c| ctx.school.get_course(c).is_some(),
        |c| c.starts_with(ELECTIVE_PREFIX),
    )
}

/// The term `course` is scheduled in, when the plan contains it.
fn scheduled_term(
    school: &School,
    plan_dag: &crate::core::models::DAG,
    plan: &PlanVariant,
    course: &str,
) -> Option<usize> {
    if !plan.courses.iter().any(|c| c == course) {
        return None;
    }
    TermScheduler::new(school, plan_dag, SchedulerConfig::default())
        .schedule(&plan.courses)
        .terms
        .iter()
        .find(|t| t.courses.iter().any(|c| c == course))
        .map(|t| t.number)
}

/// Summarise where a target course landed across the enumerated plans.
///
/// `all_terms` holds one scheduled term number per plan that contained the course, and
/// `calc_ready_terms` the same for the calc-ready subset. An empty `all_terms` yields
/// zeroed stats either way; the `error` is what distinguishes the cases, and it carries
/// `plans_processed` so a caller can tell "this degree does not offer the course" from
/// "enumeration was capped before reaching it".
fn build_target_course_stats(
    course_id: &str,
    all_terms: &[usize],
    calc_ready_terms: &[usize],
    plans_processed: usize,
) -> TargetCourseStats {
    TargetCourseStats {
        course_id: course_id.to_string(),
        // `calc_ready_terms` is only pushed alongside `all_terms`, so it is necessarily
        // empty whenever `all_terms` is — one emptiness check covers both.
        error: all_terms.is_empty().then(|| {
            format!(
                "Course '{course_id}' did not appear in any of the {plans_processed} generated plans."
            )
        }),
        all_plans: build_target_term_stats(all_terms),
        calc_ready_plans: build_target_term_stats(calc_ready_terms),
    }
}

/// Compute `TargetTermStats` from one scheduled term number per containing plan.
fn build_target_term_stats(terms: &[usize]) -> TargetTermStats {
    if terms.is_empty() {
        return TargetTermStats::default();
    }
    let mut dist = std::collections::BTreeMap::new();
    for &t in terms {
        *dist.entry(t).or_insert(0usize) += 1;
    }
    let earliest = terms.iter().copied().min();
    #[allow(clippy::cast_precision_loss)]
    let avg = terms.iter().sum::<usize>() as f64 / terms.len() as f64;
    TargetTermStats {
        plans_containing: terms.len(),
        earliest_term: earliest,
        avg_term: Some(avg),
        term_distribution: dist,
    }
}

/// Build a set of courses to exclude from prerequisite expansion
///
/// Identifies courses that should be excluded when expanding prerequisites for included courses.
/// This prevents adding conflicting prerequisite paths.
///
/// Excludes:
/// 1. Alternative prerequisite paths: If MATH156 is included with prereqs `(MATH124 & MATH126) | MATH127`,
///    we exclude the longer path (MATH124, MATH125, MATH126, etc.) and prefer the shorter (MATH127).
/// 2. Pathway courses: Courses whose prerequisites REQUIRE excluded courses (all alternatives excluded).
fn build_exclude_set(include_courses: &[String], graph: &CourseGraph) -> HashSet<String> {
    let mut exclude_set = HashSet::new();

    if include_courses.is_empty() {
        return exclude_set;
    }

    let include_set: HashSet<&str> = include_courses.iter().map(String::as_str).collect();

    // Phase 1: For included courses with OR-group prerequisites, exclude the non-preferred paths
    // For example, if MATH156 is included with prereqs `(MATH124 & MATH126) | MATH127`,
    // we exclude MATH124, MATH125, MATH126, MATH117, MATH118 (the longer path).
    for include_course in include_courses {
        if let Some(node) = graph.get(include_course) {
            exclude_set.extend(find_excluded_prereq_paths(node, graph));
        }
    }

    // Phase 2: Expand exclusions to include "pathway" courses
    // These are courses that REQUIRE excluded courses as prerequisites
    // We iterate until no new exclusions are found
    let mut changed = true;
    while changed {
        changed = false;
        let current_excludes: Vec<String> = exclude_set.iter().cloned().collect();

        for (course_key, node) in graph.iter() {
            // Skip if already excluded or included
            if exclude_set.contains(course_key) || include_set.contains(course_key.as_str()) {
                continue;
            }

            // Check if this course REQUIRES any excluded course
            // (i.e., all OR-alternatives for a prereq group are excluded)
            if course_requires_excluded(node, &current_excludes, &include_set, graph) {
                exclude_set.insert(course_key.clone());
                changed = true;
            }
        }
    }

    exclude_set
}

/// Find courses that should be excluded based on included course's prereq OR-groups
///
/// For an included course like MATH156 with prereqs `(MATH124 & MATH126) | MATH127`,
/// we identify the shorter path (MATH127) and exclude courses that are ONLY needed
/// for the longer path (MATH124, MATH125, MATH126, MATH117, MATH118).
fn find_excluded_prereq_paths(node: &CourseNode, graph: &CourseGraph) -> HashSet<String> {
    let mut excluded = HashSet::new();

    // Group prerequisites by their or_group (only care about actual OR-groups, not None)
    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // For each OR-group, identify the shorter path and exclude courses from longer paths
    for (group_id, edges) in or_groups {
        // Skip non-OR-groups (required prereqs)
        if group_id.is_none() || edges.len() <= 1 {
            continue;
        }

        // Each option's whole prerequisite chain.
        let option_chains: Vec<(String, HashSet<String>)> = edges
            .iter()
            .map(|edge| {
                let chain = collect_all_prereqs(&edge.prerequisite, graph, &mut HashSet::new());
                (edge.prerequisite.clone(), chain)
            })
            .collect();
        let Some((shortest_prereq, shortest_chain)) =
            option_chains.iter().min_by_key(|(_, chain)| chain.len())
        else {
            continue;
        };
        // Exclude what only the longer alternatives need, the alternatives included.
        for (prereq, chain) in option_chains.iter().filter(|(p, _)| p != shortest_prereq) {
            excluded.extend(
                chain
                    .iter()
                    .chain(std::iter::once(prereq))
                    .filter(|c| !shortest_chain.contains(*c))
                    .cloned(),
            );
        }
    }

    excluded
}

/// Collect all prerequisites transitively for a course
fn collect_all_prereqs(
    course: &str,
    graph: &CourseGraph,
    visited: &mut HashSet<String>,
) -> HashSet<String> {
    let mut prereqs = HashSet::new();

    if visited.contains(course) {
        return prereqs;
    }
    visited.insert(course.to_string());

    let Some(node) = graph.get(course) else {
        return prereqs;
    };

    for edge in &node.prerequisites {
        prereqs.insert(edge.prerequisite.clone());
        prereqs.extend(collect_all_prereqs(&edge.prerequisite, graph, visited));
    }

    prereqs
}

/// Check if a course requires an excluded course (no valid alternative)
///
/// Returns true if the course has a prerequisite OR-group where ALL options
/// are either excluded or their prerequisites require excluded courses.
fn course_requires_excluded(
    node: &CourseNode,
    exclude_set: &[String],
    include_set: &HashSet<&str>,
    graph: &CourseGraph,
) -> bool {
    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // Check each OR-group
    for (group_id, edges) in or_groups {
        // Handle required prerequisites (not part of any OR-group)
        if group_id.is_none() {
            for edge in &edges {
                if edge.prereq_type == PrerequisiteType::Required
                    && exclude_set.contains(&edge.prerequisite)
                    && !include_set.contains(edge.prerequisite.as_str())
                {
                    return true;
                }
            }
            continue;
        }

        // For OR-groups, check if ALL options are problematic
        // (excluded directly, or their prereqs are exclusively excluded)
        let all_problematic = edges.iter().all(|edge| {
            let prereq = &edge.prerequisite;

            // Directly excluded
            if exclude_set.contains(prereq) && !include_set.contains(prereq.as_str()) {
                return true;
            }

            // Check if this prereq's prerequisites eventually require excluded courses
            prereq_chain_requires_excluded(
                prereq,
                exclude_set,
                include_set,
                graph,
                &mut HashSet::new(),
            )
        });

        if all_problematic && !edges.is_empty() {
            return true;
        }
    }

    false
}

/// Recursively check if a course's prerequisite chain requires excluded courses
///
/// Returns true if ALL prerequisite options for ANY OR-group lead to excluded courses.
/// This handles transitive exclusions where a course's prerequisites eventually
/// require an excluded course with no valid alternatives.
fn prereq_chain_requires_excluded(
    course: &str,
    exclude_set: &[String],
    include_set: &HashSet<&str>,
    graph: &CourseGraph,
    visited: &mut HashSet<String>,
) -> bool {
    // Avoid infinite loops
    if visited.contains(course) {
        return false;
    }
    visited.insert(course.to_string());

    // If included, it's fine
    if include_set.contains(course) {
        return false;
    }

    // If excluded, this path requires excluded courses
    if exclude_set.contains(&course.to_string()) {
        return true;
    }

    // Check the course's prerequisites
    let Some(node) = graph.get(course) else {
        return false;
    };

    let or_groups = group_prereqs_by_or_group(&node.prerequisites);

    // Check if any OR-group has all options leading to excluded courses
    for (_group_id, edges) in or_groups {
        if edges.is_empty() {
            continue;
        }

        let all_require_excluded = edges.iter().all(|edge| {
            prereq_chain_requires_excluded(
                &edge.prerequisite,
                exclude_set,
                include_set,
                graph,
                visited,
            )
        });

        if all_require_excluded {
            return true;
        }
    }

    false
}

/// Group prerequisite edges by their OR-group
///
/// Returns a map where:
/// - `None` key contains required prerequisites (not part of any OR-group)
/// - Numeric keys contain optional prerequisites grouped by their `or_group` ID
fn group_prereqs_by_or_group(
    prerequisites: &[PrerequisiteEdge],
) -> HashMap<Option<usize>, Vec<&PrerequisiteEdge>> {
    let mut or_groups: HashMap<Option<usize>, Vec<&PrerequisiteEdge>> = HashMap::new();
    for edge in prerequisites {
        or_groups.entry(edge.or_group).or_default().push(edge);
    }
    or_groups
}

/// Expand a plan's courses to include all required prerequisites
///
/// For each course in the plan, finds the minimum prerequisite chain and adds
/// any missing prerequisites to the course list. This ensures the plan is
/// complete and can be properly scheduled.
///
/// Uses a two-phase approach:
/// 1. First pass: Sort courses by prerequisite depth (deepest first) so courses
///    that need prerequisites are processed after their potential prereqs are known
/// 2. Second pass: Remove redundant prerequisites where an alternative already exists
///
/// This prevents adding MATH117 for STAT301 when MATH127 (needed by MATH156)
/// would also satisfy STAT301's prerequisite.
///
/// Uses the equivalence map to check if a prerequisite is satisfied by an
/// equivalent course already in the plan.
///
/// Uses the exclude set to avoid adding courses that are alternatives to included
/// courses (e.g., don't add MATH160 if user included MATH156 and they're alternatives).
fn expand_courses_with_prerequisites(
    courses: &[String],
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
    exclude_from_prereqs: &HashSet<String>,
    protected_courses: &HashSet<String>,
) -> Vec<String> {
    // Phase 1: Sort courses by prerequisite depth (deepest chains first)
    // This ensures courses like MATH156 (which needs MATH127) are processed
    // before courses like STAT301 (which can use MATH127 as an alternative)
    let mut sorted_courses: Vec<(String, usize)> = courses
        .iter()
        .map(|c| {
            let depth = graph.min_prerequisite_depth(c).unwrap_or(0);
            (c.clone(), depth)
        })
        .collect();
    sorted_courses.sort_by_key(|(_, depth)| std::cmp::Reverse(*depth));

    let mut expanded: HashSet<String> = courses.iter().cloned().collect();
    let mut to_process: Vec<String> = sorted_courses.into_iter().map(|(c, _)| c).collect();

    while let Some(course_key) = to_process.pop() {
        // Get the minimum prerequisite chain, preferring courses already in the plan
        // and avoiding excluded courses
        if let Some(prereq_chain) = graph.min_prerequisite_chain_with_exclusions(
            &course_key,
            &expanded,
            exclude_from_prereqs,
        ) {
            for prereq in prereq_chain {
                // Skip excluded courses (alternatives to included courses)
                if exclude_from_prereqs.contains(&prereq) {
                    continue;
                }

                // Check if this prerequisite is satisfied by an equivalent course
                let has_equivalent = equivalences
                    .get(&prereq)
                    .is_some_and(|equivs| equivs.iter().any(|e| expanded.contains(e)));

                if !has_equivalent && !expanded.contains(&prereq) {
                    expanded.insert(prereq.clone());
                    to_process.push(prereq); // Process this prereq's chain too
                }
            }
        }
    }

    // Phase 2: Remove redundant prerequisites
    // A prerequisite is redundant if:
    // - It was added as an OR-alternative for some course
    // - Another course in the plan would also satisfy that OR requirement
    // - OR an equivalent course is already in the plan
    // BUT only courses ADDED during expansion (Phase 1) may be pruned. Courses
    // from the original plan are degree requirements (e.g. a `type: all` core
    // course) and must never be dropped here — even when they also happen to be
    // an OR-prerequisite alternative for some elective. Without this guard a
    // required course like CS320 ("Algorithms"), which is also an OR option of
    // an elective's prereq (`CS320 | CS370`), gets deleted whenever the sibling
    // CS370 is present, silently dropping it from generated plans.
    // `protected_courses` additionally pins user --include courses.
    let original_courses: HashSet<&str> = courses.iter().map(String::as_str).collect();
    let expanded_clone = expanded.clone();
    let redundant = find_redundant_prerequisites(&expanded_clone, graph, equivalences);
    for course in redundant {
        if !protected_courses.contains(&course) && !original_courses.contains(course.as_str()) {
            expanded.remove(&course);
        }
    }

    let mut result: Vec<String> = expanded.into_iter().collect();
    result.sort();
    result
}

/// Find prerequisites that are redundant because an alternative already exists
///
/// For each course in the plan, checks if any of its OR-prerequisites could be
/// satisfied by a different course already in the plan. If so, and the current
/// prerequisite is ONLY used for this OR-group (not required elsewhere), it's redundant.
///
/// Also considers equivalent courses: if MATH241 is a prereq but MATH215 (equivalent)
/// is in the plan, MATH241 is redundant.
fn find_redundant_prerequisites(
    courses: &HashSet<String>,
    graph: &CourseGraph,
    equivalences: &HashMap<String, HashSet<String>>,
) -> Vec<String> {
    let usage = prerequisite_usage(courses, graph);
    let mut redundant = redundant_by_equivalence(courses, equivalences, &usage);
    redundant.extend(redundant_or_options(courses, graph, &usage));
    redundant
}

/// Which plan courses actually depend on each prerequisite.
///
/// A required prerequisite in the plan is always used. An OR-group's option counts as used
/// only when it is the group's one option in the plan; with several in the plan, none is
/// counted here, and [`redundant_or_options`] decides between them.
fn prerequisite_usage(
    courses: &HashSet<String>,
    graph: &CourseGraph,
) -> HashMap<String, Vec<String>> {
    let mut usage: HashMap<String, Vec<String>> = HashMap::new();
    for (course_key, node) in courses.iter().filter_map(|c| Some((c, graph.get(c)?))) {
        let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
        for edge in &node.prerequisites {
            if edge.prereq_type == PrerequisiteType::Required {
                if courses.contains(&edge.prerequisite) {
                    usage
                        .entry(edge.prerequisite.clone())
                        .or_default()
                        .push(course_key.clone());
                }
            } else if let Some(group) = edge.or_group {
                or_groups.entry(group).or_default().push(&edge.prerequisite);
            }
        }
        for options in or_groups.into_values() {
            let mut in_plan = options.into_iter().filter(|opt| courses.contains(*opt));
            if let (Some(only), None) = (in_plan.next(), in_plan.next()) {
                usage
                    .entry(only.to_string())
                    .or_default()
                    .push(course_key.clone());
            }
        }
    }
    usage
}

/// Plan courses nothing depends on whose equivalent, also in the plan, is depended on.
fn redundant_by_equivalence(
    courses: &HashSet<String>,
    equivalences: &HashMap<String, HashSet<String>>,
    usage: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let used = |c: &str| usage.get(c).is_some_and(|u| !u.is_empty());
    let mut redundant = Vec::new();
    for (course, equivs) in courses
        .iter()
        .filter_map(|c| Some((c, equivalences.get(c)?)))
    {
        for equiv in equivs {
            if equiv != course && courses.contains(equiv) && !used(course) && used(equiv) {
                redundant.push(course.clone());
            }
        }
    }
    redundant
}

/// Options of an OR-group with several options in the plan that nothing depends on,
/// when another of those options is depended on more.
fn redundant_or_options(
    courses: &HashSet<String>,
    graph: &CourseGraph,
    usage: &HashMap<String, Vec<String>>,
) -> Vec<String> {
    let uses = |c: &str| usage.get(c).map_or(0, Vec::len);
    let mut redundant = Vec::new();
    for node in courses.iter().filter_map(|c| graph.get(c)) {
        let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
        for edge in &node.prerequisites {
            if let (Some(group), PrerequisiteType::Optional) = (edge.or_group, &edge.prereq_type) {
                or_groups.entry(group).or_default().push(&edge.prerequisite);
            }
        }
        for options in or_groups.into_values() {
            let in_plan: Vec<&str> = options
                .into_iter()
                .filter(|opt| courses.contains(*opt))
                .collect();
            if in_plan.len() < 2 {
                continue;
            }
            for &option in &in_plan {
                let better_exists = in_plan
                    .iter()
                    .any(|&other| other != option && uses(other) > uses(option));
                if uses(option) == 0 && better_exists {
                    redundant.push(option.to_string());
                }
            }
        }
    }
    redundant
}

/// Create an expanded plan variant with additional prerequisite courses
///
/// Takes the original variant and creates a new one with the expanded course list,
/// preserving requirement choice metadata. Re-fits the `ELEC` filler so the plan reaches
/// the target credits: exactly for an integral shortfall, overshooting by under one credit
/// for a fractional one, and with no filler at all when real courses already reach it.
///
/// # Arguments
/// * `original` - The original plan variant before prerequisite expansion
/// * `expanded_courses` - All courses including added prerequisites
/// * `school` - School data for credit lookup
/// * `target_credits` - Target total credits for the degree
/// * `fill` - How fill-to-total blocks were resized for this plan, if any; its removed
///   and added placeholders are applied to the requirement choices
fn create_expanded_variant(
    original: &PlanVariant,
    expanded_courses: &[String],
    school: &School,
    target_credits: Option<u32>,
    fill: Option<&crate::core::degree::fill_electives::FillResize>,
) -> PlanVariant {
    let mut new_choices = original.requirement_choices.clone();
    // Placeholders a fill-to-total block gave up for this plan. Dropping them from the
    // choices too keeps the requirement breakdown in step with the courses actually
    // scheduled, rather than listing placeholders the plan no longer contains.
    if let Some(fill) = fill {
        for chosen in new_choices.values_mut() {
            chosen.retain(|c| !fill.removed.contains(c));
        }
        for (id, fresh) in &fill.added {
            new_choices
                .entry(id.clone())
                .or_default()
                .extend(fresh.iter().cloned());
        }
    }

    // Find courses that were added (prerequisites not in original plan)
    let original_set: HashSet<&str> = original.courses.iter().map(String::as_str).collect();
    let added_prereqs: Vec<String> = expanded_courses
        .iter()
        .filter(|c| !original_set.contains(c.as_str()))
        .cloned()
        .collect();

    // Add prerequisites as a special requirement
    if !added_prereqs.is_empty() {
        new_choices.insert(PREREQUISITES_KEY.to_string(), added_prereqs);
    }

    // The generator's `ELEC` filler is re-fitted to the expanded plan, so it is set aside
    // and the real courses' credits decide how much filler the plan still needs.
    let non_electives: Vec<String> = expanded_courses
        .iter()
        .filter(|c| !c.starts_with(ELECTIVE_PREFIX))
        .cloned()
        .collect();
    let final_courses = target_credits.map_or_else(
        || expanded_courses.to_vec(),
        |target| refit_electives(non_electives, school, target, &mut new_choices),
    );

    let total_credits: f32 = final_courses
        .iter()
        .map(|c| course_credits_with_fallback(school, c))
        .sum();

    PlanVariant::from_parts(final_courses, new_choices, total_credits)
}

/// `non_electives` plus exactly the `ELEC` filler that brings them to `target` credits:
/// none when they already reach it, sorted when filler is added. Records the filler under
/// [`ELECTIVE_PLACEHOLDERS_KEY`] in `choices`, or removes the key when there is none.
fn refit_electives(
    non_electives: Vec<String>,
    school: &School,
    target: u32,
    choices: &mut HashMap<String, Vec<String>>,
) -> Vec<String> {
    let credits: f32 = non_electives
        .iter()
        .map(|c| course_credits_with_fallback(school, c))
        .sum();
    #[allow(clippy::cast_precision_loss)] // target credits < 1000
    let target = target as f32;
    if credits >= target {
        choices.remove(ELECTIVE_PLACEHOLDERS_KEY);
        return non_electives;
    }
    let electives = crate::core::degree::placeholder::elective_placeholders(target - credits);
    if electives.is_empty() {
        choices.remove(ELECTIVE_PLACEHOLDERS_KEY);
    } else {
        choices.insert(ELECTIVE_PLACEHOLDERS_KEY.to_string(), electives.clone());
    }
    let mut courses = non_electives;
    courses.extend(electives);
    courses.sort();
    courses
}

#[cfg(test)]
mod tests {
    fn config(max_plans: usize) -> AnalysisConfig<'static> {
        AnalysisConfig {
            max_plans,
            ignore_duplicates: true,
            sample_count: 3,
            sampling_strategy: SamplingStrategy::Shuffled,
            include_courses: Vec::new(),
            random_seed: None,
            time_limit: None,
            target_course: None,
        }
    }

    /// Two courses offering each other as one option of an OR: a cycle to break, and two
    /// plans (`CS152` reached through either option).
    const CYCLE_YAML: &str = "degree: {id: t, institution: T, program: T, total_credits: 6, gpa_minimum: 2.0}\n\
        requirements:\n  core: {name: Core, type: all, category: major, courses: [CS152, CS163]}\n\
        courses:\n  MATH127: {title: M, prefix: MATH, number: \"127\", credits: 3}\n  \
        CS152: {title: A, prefix: CS, number: \"152\", credits: 3, prerequisites_raw: \"CS163 | MATH127\"}\n  \
        CS163: {title: B, prefix: CS, number: \"163\", credits: 3, prerequisites_raw: \"CS152 | MATH127\"}\n";

    /// The events are what the CLI prints when verbose: the broken cycle first, planning
    /// once, then one per plan analyzed — none when a zero time limit stops the run.
    #[test]
    fn test_analyze_reports_its_progress_and_honours_a_time_limit() {
        let (program, _) = crate::core::degree::parse_degree_auto(CYCLE_YAML).unwrap();
        for (limit, expect_plans) in [(None, true), (Some(Duration::ZERO), false)] {
            let (mut removed, mut planning, mut processed) = (0, 0, 0);
            let run = analyze(
                program.clone(),
                &AnalysisConfig {
                    time_limit: limit,
                    ..config(50)
                },
                &mut |event| match event {
                    AnalysisEvent::CyclesBroken { removed: r, .. } => removed += r.len(),
                    AnalysisEvent::Planning { .. } => planning += 1,
                    AnalysisEvent::Processed(n) => processed = n,
                    AnalysisEvent::PlanSkipped(_) => {}
                },
            );
            assert_eq!((removed, planning), (1, 1), "{limit:?}");
            assert_eq!(processed, run.plans_processed, "{limit:?}");
            assert_eq!(run.plans_processed > 0, expect_plans, "{limit:?}");
            assert_eq!(run.time_limit_reached, !expect_plans, "{limit:?}");
        }
    }

    /// A run that reaches its cap is complete only when the population is no larger.
    #[test]
    fn test_is_full_population_at_and_below_the_cap() {
        let (program, _) = crate::core::degree::parse_degree_auto(CYCLE_YAML).unwrap();
        let all = analyze(program.clone(), &config(50), &mut |_| {});
        assert!(all.is_full_population());
        assert_eq!(all.population_size(), all.plans_processed);

        let at_cap = analyze(program, &config(all.plans_processed), &mut |_| {});
        assert_eq!(at_cap.plans_processed, all.plans_processed);
        assert_eq!(
            at_cap.is_full_population(),
            at_cap.stats.total_possible <= at_cap.plans_processed
        );
    }

    use super::*;

    /// Regression: a required course that is ALSO an OR-prerequisite alternative
    /// for another course must survive Phase-2 redundancy pruning in
    /// `expand_courses_with_prerequisites`. This mirrors the CSU bug where CS320
    /// (a `type: all` core course, and the `CS320 | CS370` OR-prereq of an
    /// elective) was silently dropped from generated plans whenever its OR
    /// sibling CS370 was present — only courses ADDED during expansion may be
    /// pruned, never the plan's own required courses.
    #[test]
    fn test_expand_preserves_required_or_alternative() {
        let yaml = r#"
degree:
  id: regress-or-alt
  institution: T
  program: T
  total_credits: 12
  gpa_minimum: 2.0
requirements:
  core:
    name: Core
    type: all
    category: major
    courses: [COR101, COR102]
  upper:
    name: Upper
    type: all
    category: major
    courses: [UPP301, UPP401]
courses:
  COR101: {title: A, prefix: COR, number: "101", credits: 3}
  COR102: {title: B, prefix: COR, number: "102", credits: 3}
  UPP301: {title: X, prefix: UPP, number: "301", credits: 3, prerequisites_raw: "COR101 | COR102"}
  UPP401: {title: Y, prefix: UPP, number: "401", credits: 3, prerequisites_raw: "COR101"}
"#;
        let (program, _) = crate::core::degree::parse_degree_auto(yaml).expect("parse degree");

        let graph = CourseGraph::from_degree_program(&program).graph;
        let equivalences = HashMap::new();
        let exclude = HashSet::new();
        let protected = HashSet::new();
        // The plan as the generator produces it: both required core courses plus
        // the two required upper courses.
        let plan = vec![
            "COR101".to_string(),
            "COR102".to_string(),
            "UPP301".to_string(),
            "UPP401".to_string(),
        ];
        let expanded =
            expand_courses_with_prerequisites(&plan, &graph, &equivalences, &exclude, &protected);

        // COR102 is an OR-alternative of UPP301's `COR101 | COR102` prereq with no
        // other dependents, while COR101 is independently required by UPP401 — so
        // pre-fix COR102 was pruned as "redundant". It is a hard requirement and
        // must survive expansion.
        assert!(
            expanded.contains(&"COR102".to_string()),
            "required type:all course COR102 was dropped as a redundant OR-prereq: {expanded:?}"
        );
        assert!(
            expanded.contains(&"COR101".to_string()),
            "required core course COR101 must survive expansion: {expanded:?}"
        );
    }

    /// One expected summary for a list of scheduled term numbers.
    struct TermStatsCase {
        terms: &'static [usize],
        plans_containing: usize,
        earliest: Option<usize>,
        avg: Option<f64>,
        distribution: &'static [(usize, usize)],
    }

    #[test]
    fn test_build_target_term_stats_summarises_term_numbers() {
        let cases = &[
            // The "no plan contained the course" slice.
            TermStatsCase {
                terms: &[],
                plans_containing: 0,
                earliest: None,
                avg: None,
                distribution: &[],
            },
            // Single observation: earliest and mean are both the only term.
            TermStatsCase {
                terms: &[4],
                plans_containing: 1,
                earliest: Some(4),
                avg: Some(4.0),
                distribution: &[(4, 1)],
            },
            // Term 0 is a real term, not a sentinel for "absent".
            TermStatsCase {
                terms: &[0],
                plans_containing: 1,
                earliest: Some(0),
                avg: Some(0.0),
                distribution: &[(0, 1)],
            },
            // Repeats are counted, not deduplicated.
            TermStatsCase {
                terms: &[3, 3, 5],
                plans_containing: 3,
                earliest: Some(3),
                avg: Some(11.0 / 3.0),
                distribution: &[(3, 2), (5, 1)],
            },
            // Unsorted input: earliest is the minimum, distribution is keyed by term.
            TermStatsCase {
                terms: &[7, 2, 9, 2],
                plans_containing: 4,
                earliest: Some(2),
                avg: Some(5.0),
                distribution: &[(2, 2), (7, 1), (9, 1)],
            },
        ];

        for case in cases {
            let terms = case.terms;
            let got = build_target_term_stats(terms);
            assert_eq!(
                got.plans_containing, case.plans_containing,
                "plans_containing for {terms:?}"
            );
            assert_eq!(
                got.earliest_term, case.earliest,
                "earliest_term for {terms:?}"
            );
            match (got.avg_term, case.avg) {
                (Some(got_avg), Some(want)) => assert!(
                    (got_avg - want).abs() < 1e-9,
                    "avg_term for {terms:?}: got {got_avg}, want {want}"
                ),
                (got_avg, want) => assert_eq!(got_avg, want, "avg_term for {terms:?}"),
            }
            let want: std::collections::BTreeMap<usize, usize> =
                case.distribution.iter().copied().collect();
            assert_eq!(
                got.term_distribution, want,
                "term_distribution for {terms:?}"
            );
            // The distribution must always account for exactly the inputs.
            assert_eq!(
                got.term_distribution.values().sum::<usize>(),
                got.plans_containing,
                "distribution total for {terms:?}"
            );
        }
    }

    #[test]
    fn test_build_target_course_stats_reports_a_found_course() {
        let stats = build_target_course_stats("CS201", &[2, 3, 3], &[3], 10);
        assert_eq!(stats.course_id, "CS201");
        assert!(stats.error.is_none(), "a found course reports no error");
        assert_eq!(stats.all_plans.earliest_term, Some(2));
        assert_eq!(stats.calc_ready_plans.earliest_term, Some(3));
        assert!(
            stats.calc_ready_plans.plans_containing <= stats.all_plans.plans_containing,
            "calc-ready slice cannot exceed all plans"
        );
    }

    #[test]
    fn test_build_target_course_stats_reports_a_missing_course_with_context() {
        let stats = build_target_course_stats("ZZZ999", &[], &[], 42);
        let err = stats
            .error
            .as_deref()
            .expect("missing course reports error");
        assert!(err.contains("ZZZ999"), "error names the course, got: {err}");
        assert!(
            err.contains("42"),
            "error carries plans_processed so the caller can tell 'not offered' from \
             'enumeration capped', got: {err}"
        );
        assert_eq!(stats.all_plans.plans_containing, 0);
        assert!(stats.all_plans.earliest_term.is_none());
        assert!(stats.calc_ready_plans.earliest_term.is_none());
    }
}
