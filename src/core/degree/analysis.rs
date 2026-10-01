//! Degree analysis: enumerate a degree's plans, expand each, aggregate their metrics.
//!
//! One pipeline, which `degree analyze` and the MCP analysis tools both call; each keeps
//! only its own output layer and defaults.
//!
//! Where the two copies this replaces differed in anything that feeds a metric, the CLI's
//! behaviour is the one kept, because the stored corpus was produced by it: prerequisite
//! expansion avoids alternatives to `--include` courses and prunes redundant
//! prerequisites; the seed is taken from the canonical degree; courses and equivalences
//! come from `core::report::inputs`. The MCP-only features — a wall-clock limit and
//! target-course statistics — are options here, off unless asked for.
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
use crate::core::report::term_scheduler::{SchedulerConfig, TermScheduler};
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
    let mut graph_result = CourseGraph::from_degree_program(&program);
    if !graph_result.cycles.is_empty() {
        let removed = graph_result.graph.break_cycles(&graph_result.cycles);
        on_event(AnalysisEvent::CyclesBroken {
            cycles: graph_result.cycles.len(),
            removed: &removed,
        });
        graph_result.cycles.clear();
    }
    let graph = graph_result.graph;

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
        let mut fill: Option<crate::core::degree::fill_electives::FillResize> = None;
        if let Some(target) = ctx.gen_config.target_credits {
            #[allow(clippy::cast_precision_loss)] // target credits < 1000
            if let Some(resize) = crate::core::degree::fill_electives::shrink_fill_blocks(
                &expanded_courses,
                &variant.requirement_choices,
                &fill_ids,
                target as f32,
                |c| {
                    ctx.school
                        .get_course(c)
                        .map_or_else(|| placeholder_credits(c), |co| co.credit_hours)
                },
                |c| ctx.school.get_course(c).is_some(),
                |c| c.starts_with(ELECTIVE_PREFIX),
            ) {
                expanded_courses.clone_from(&resize.courses);
                fill = Some(resize);
            }
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
        if let Some(target) = target_course {
            if expanded_variant.courses.iter().any(|c| c == target) {
                let scheduler =
                    TermScheduler::new(ctx.school, &plan_dag, SchedulerConfig::default());
                let schedule = scheduler.schedule(&expanded_variant.courses);
                let term = schedule
                    .terms
                    .iter()
                    .find(|t| t.courses.iter().any(|c| c == target))
                    .map(|t| t.number);
                if let Some(t) = term {
                    outcome.target_all_terms.push(t);
                    if selector.is_calc_ready_plan(&variant) {
                        outcome.target_calc_ready_terms.push(t);
                    }
                }
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

/// Credits of a placeholder course, from its name. See `core::degree::placeholder`.
fn placeholder_credits(course_key: &str) -> f32 {
    crate::core::degree::placeholder::placeholder_credits(course_key)
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

        // Calculate the total prerequisite chain length for each option
        let mut option_chains: Vec<(String, HashSet<String>)> = Vec::new();

        for edge in &edges {
            let chain = collect_all_prereqs(&edge.prerequisite, graph, &mut HashSet::new());
            option_chains.push((edge.prerequisite.clone(), chain));
        }

        // Find the option with the shortest total chain
        if let Some((shortest_prereq, shortest_chain)) =
            option_chains.iter().min_by_key(|(_, chain)| chain.len())
        {
            // Exclude courses from other chains that aren't in the shortest chain
            for (prereq, chain) in &option_chains {
                if prereq != shortest_prereq {
                    for course in chain {
                        if !shortest_chain.contains(course) {
                            excluded.insert(course.clone());
                        }
                    }
                    // Also exclude the top-level alternative prereq itself
                    if !shortest_chain.contains(prereq) {
                        excluded.insert(prereq.clone());
                    }
                }
            }
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
    let mut redundant = Vec::new();

    // Build a map of which courses ACTUALLY depend on which prerequisites
    // Only count a prerequisite as "used" if no other option in its OR-group is in the plan
    let mut prereq_usage: HashMap<String, Vec<String>> = HashMap::new();

    for course_key in courses {
        if let Some(node) = graph.get(course_key) {
            // Group prerequisites by OR-group
            let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
            let mut required_prereqs: Vec<&str> = Vec::new();

            for edge in &node.prerequisites {
                if edge.prereq_type == crate::core::models::course_graph::PrerequisiteType::Required
                {
                    if courses.contains(&edge.prerequisite) {
                        required_prereqs.push(&edge.prerequisite);
                    }
                } else if let Some(group) = edge.or_group {
                    or_groups.entry(group).or_default().push(&edge.prerequisite);
                }
            }

            // Required prereqs are always used
            for prereq in required_prereqs {
                prereq_usage
                    .entry(prereq.to_string())
                    .or_default()
                    .push(course_key.clone());
            }

            // For OR-groups, only mark as "used" if this is the ONLY option in the plan
            for (_group, options) in or_groups {
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| courses.contains(opt))
                    .copied()
                    .collect();

                if in_plan.len() == 1 {
                    // Only one option satisfies this - it's truly needed
                    prereq_usage
                        .entry(in_plan[0].to_string())
                        .or_default()
                        .push(course_key.clone());
                }
                // If multiple options are in plan, we'll handle redundancy below
            }
        }
    }

    // Check for courses that are redundant because an equivalent is in the plan
    for course in courses {
        if let Some(equivs) = equivalences.get(course) {
            for equiv in equivs {
                if equiv != course && courses.contains(equiv) {
                    let usages = prereq_usage.get(course);
                    if usages.is_none_or(std::vec::Vec::is_empty) {
                        let equiv_satisfies_same = prereq_usage
                            .get(equiv)
                            .is_some_and(|equiv_usages| !equiv_usages.is_empty());

                        if equiv_satisfies_same {
                            redundant.push(course.clone());
                        }
                    }
                }
            }
        }
    }

    // For each course, check its OR-groups for redundant prerequisites
    for course_key in courses {
        if let Some(node) = graph.get(course_key) {
            // Group prerequisites by OR-group
            let mut or_groups: HashMap<usize, Vec<&str>> = HashMap::new();
            for edge in &node.prerequisites {
                if let Some(group) = edge.or_group {
                    if edge.prereq_type
                        == crate::core::models::course_graph::PrerequisiteType::Optional
                    {
                        or_groups.entry(group).or_default().push(&edge.prerequisite);
                    }
                }
            }

            // For each OR-group, check if we have multiple options in the plan
            for (_group, options) in or_groups {
                let in_plan: Vec<&str> = options
                    .iter()
                    .filter(|&&opt| courses.contains(opt))
                    .copied()
                    .collect();

                if in_plan.len() > 1 {
                    // Multiple options satisfy this OR-group - find redundant ones
                    // A course is redundant if another course in this OR-group
                    // is actually NEEDED by other courses (has real dependents)
                    for &option in &in_plan {
                        let option_usage = prereq_usage.get(option).map_or(0, Vec::len);

                        // Check if another option has MORE dependents (is more useful)
                        let better_exists = in_plan.iter().any(|&other| {
                            if other == option {
                                return false;
                            }
                            let other_usage = prereq_usage.get(other).map_or(0, Vec::len);
                            other_usage > option_usage
                        });

                        // If this option has no unique dependents and a better option exists
                        if option_usage == 0 && better_exists {
                            redundant.push(option.to_string());
                        }
                    }
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

    // Calculate actual credits from non-elective courses
    let non_elective_credits: f32 = expanded_courses
        .iter()
        .filter(|c| !c.starts_with(ELECTIVE_PREFIX))
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |course| course.credit_hours)
        })
        .sum();

    // Adjust electives if we have a target
    #[allow(clippy::option_if_let_else)] // More readable with if-let here
    #[allow(clippy::cast_precision_loss)] // Safe: target credits < 1000
    let final_courses = if let Some(target) = target_credits {
        let target_f32 = target as f32;
        if non_elective_credits >= target_f32 {
            // Already at or over target - remove all electives
            new_choices.remove(ELECTIVE_PLACEHOLDERS_KEY);
            expanded_courses
                .iter()
                .filter(|c| !c.starts_with(ELECTIVE_PREFIX))
                .cloned()
                .collect()
        } else {
            // Need some electives - calculate exactly how many
            let elective_credits_needed = target_f32 - non_elective_credits;
            let new_electives =
                crate::core::degree::placeholder::elective_placeholders(elective_credits_needed);

            // Replace elective placeholders with exact amount needed
            if new_electives.is_empty() {
                new_choices.remove(ELECTIVE_PLACEHOLDERS_KEY);
            } else {
                new_choices.insert(ELECTIVE_PLACEHOLDERS_KEY.to_string(), new_electives.clone());
            }

            // Build final course list with new electives
            let mut courses: Vec<String> = expanded_courses
                .iter()
                .filter(|c| !c.starts_with(ELECTIVE_PREFIX))
                .cloned()
                .collect();
            courses.extend(new_electives);
            courses.sort();
            courses
        }
    } else {
        expanded_courses.to_vec()
    };

    // Calculate final total credits
    let total_credits: f32 = final_courses
        .iter()
        .map(|c| {
            school
                .get_course(c)
                .map_or_else(|| placeholder_credits(c), |course| course.credit_hours)
        })
        .sum();

    PlanVariant::from_parts(final_courses, new_choices, total_credits)
}

#[cfg(test)]
mod tests {
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
