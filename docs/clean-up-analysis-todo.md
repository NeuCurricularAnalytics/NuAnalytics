# Analysis-pipeline clean-up — plan

Status: **not started.** This is a plan, not a record of work done.

Written after a `/check-rs` pass over the whole crate raised "the analysis pipeline exists
twice". Before planning a merge, the obvious objection was checked: *aren't these two
different kinds of analysis — one whole-degree, one a single curricular map?* The answer
is below, with the measurements behind it.

---

## 1. The question: are these the same level of analysis?

**Partly. There are three analysis paths, and only two of them are the same level.**

| path | input | enumerates plans? | output |
|---|---|---|---|
| `nuanalytics planner <curriculum.csv>` | one curricular map | **no** | CSV metrics + report |
| `nuanalytics degree analyze <degree.yaml\|json>` | a degree program | **yes** | report JSON, plan CSVs, HTML |
| MCP tool `analyze_degree` | a degree program | **yes** | JSON tool response |

`planner` (`src/cli/commands/planner.rs`, `src/core/planner/`) is genuinely a different
level: `parse_curriculum_csv` → `School` → `build_dag` → `compute_all_metrics`. One map in,
metrics out. It contains no `PlanGenerator` and no `RequirementResolver` — verified by
grep. **It is out of scope for this work and should not be merged into anything.**

The other two are the same level. Both build a `PlanGenerator` from
`program.requirements` + `program.courses` and enumerate many candidate maps:

- `degree.rs:2369 enumerate_and_analyze_plans` → `PlanGenerator::new(&ctx.program.requirements, &ctx.program.courses, …)`
- `analyze.rs:563 build_artifacts` → the same call

So the distinction worth preserving is **degree-vs-map** (`planner` vs the other two), not
**CLI-vs-MCP**. What differs between the CLI and MCP paths is the *output layer*, not the
level of analysis:

- the CLI emits a degree **report artifact** — `{degree, courses, requirements, selected_plans, analysis}` plus per-plan CSVs and HTML
- the MCP tool emits a flat **metrics response** — `{complexity, longest_delay, avg_chain_length, plans_analyzed, population_size, notes, recommended_max_plans, …}`

Those two output shapes are both legitimate and should both survive. Only the pipeline
underneath them should be shared.

## 2. Why this matters: they disagree today

Same file, same cap, both reporting `plans_analyzed: 200`:

    nuanalytics degree analyze samples/degrees/csu-cs-bscs-general.yaml --max-plans 200
    analyze::execute_json(csu_yaml, Some(200), …)

| metric | CLI | MCP |
|---|---|---|
| complexity median | 220.5 | **261.5** |
| complexity range | 170.0 – 424.0 | **177.0 – 567.0** |
| Shortest Path credits | 120.0 | **119.0** |
| Longest Path credits | 123.0 | **125.0** |
| Longest Path terms | 8 | **11** |

A 19% difference in median complexity for the same degree. The MCP numbers are the ones
served to AI agents.

## 3. Root cause of the divergence

`build_plan_dag` (MCP, `analyze.rs:1407`) adds an edge for **every** in-plan option of an
OR-group:

```rust
for (_group, options) in or_groups {
    for opt in options.iter().filter(|o| plan_set.contains(**o)) {
        dag.add_prerequisite(key.clone(), opt);
    }
}
```

`build_dag_for_plan` (CLI, `degree.rs:3327`) selects **exactly one** option per group —
preferring an `--include-courses` course, else the option most needed by other courses.

The CLI is semantically right: an OR-group means one prerequisite was satisfied, so one
edge belongs in that plan's DAG. The MCP version treats `A | B` as `A & B` whenever both
land in the plan, adding spurious edges that inflate complexity, delay and centrality —
consistent with the direction and size of the gap above.

**This is a correctness bug in its own right and does not need the refactor.** Fix it
first (step 1), independently, so the merge is not also a behaviour change.

## 4. Divergence inventory

Ten helper pairs, with which side is believed correct. Each needs confirming as it is
merged — the merged version must take the union of behaviours, not one side wholesale.

| concern | CLI (`degree.rs`) | MCP (`analyze.rs`) | difference | keep |
|---|---|---|---|---|
| per-plan DAG | `build_dag_for_plan:3267` | `build_plan_dag:1373` | one edge per OR-group vs every in-plan option | **CLI** |
| build `School` | `build_school_from_program:2783` | `build_school:1260` | MCP drops `typically_offered`, `gen_ed_attributes` | **CLI** |
| degree-level DAG | `build_dag_from_graph:2866` | `build_dag:1297` | CLI omits corequisites; MCP includes them | **MCP** |
| equivalence map | `build_equivalence_map:1332` | `build_equivalences:1317` | CLI also scans `req.from.courses` + nested options | **CLI** |
| expand prereqs | `expand_courses_with_prerequisites:2911` | `expand_with_prereqs:1342` | MCP lacks `exclude_from_prereqs` + redundancy removal | **CLI** |
| prereq tokenizer | `parse_prerequisites_from_raw:2834` | `parse_prereqs:1288` | different grade-letter handling; both duplicate `core::prerequisite_parser::extract_all_courses:377` | **neither** |
| placeholder credits | `placeholder_credits:3247` | `placeholder_credits:1507` | MCP also treats `…SM` as small | **MCP** |
| elective filler | `generate_elective_placeholders:3224` | `gen_elective_placeholders:1516` | different ids and thresholds | see note |
| equivalent-in-plan | `find_equivalent_in_plan_set:3387` | `find_equivalent_in_plan:1420` | identical | either |
| expanded variant | `create_expanded_variant:3135` | `build_expanded_variant:1435` | identical | either |
| plan loop | `process_plan_variants:2439` | `run_plan_analysis:799` | MCP has deadline + seed + target-course; CLI has progress + exclude set | **union** |

Notes:
- The elective-filler naming mismatch was already fixed in the placeholder consolidation
  (`is_placeholder_course` is now single-sourced in `plan_generator`), but the two
  *generators* still differ in id scheme and remainder threshold. Consolidating them is
  step 4.
- `core::prerequisite_parser::extract_all_courses` already exists and is tested; both
  copies should be deleted in favour of it, and `build_school` should read the
  already-populated `course.prerequisites` rather than re-parsing `prerequisites_raw`.

## 5. Step-by-step plan

Each step ends with `/check-rs` and a green `cargo test` under `--no-default-features`,
`--features database`, and `--all-features`.

### Step 1 — Fix the OR-group DAG bug in the MCP path *(no refactor)*
Make `build_plan_dag` select one option per OR-group, matching `build_dag_for_plan`.
- **Gate:** re-run the section-2 comparison; CLI and MCP complexity should converge.
- **Expect published metrics to change.** Record before/after for all three sample degrees
  in the commit message.
- Add a test asserting an OR-group contributes exactly one edge to a plan DAG.

### Step 2 — Pin current behaviour before moving anything
Add a characterisation test that runs both paths on the same degree and asserts they agree
on complexity, longest delay, plan count, and the selected-plan credit totals. This is the
safety net for steps 3–6: if the merge changes a number, this fails and names it.
- Put it in `tests/rs/` and feature-gate on `mcp`, as `degree_fixtures` already is.
- Assert only figures measured stable (see the reproducibility caveat in step 7).

### Step 3 — Create `src/core/analysis/`, not feature-gated
Move, unchanged, from `analyze.rs`:
`AnalysisArtifacts` (make `pub`), `build_artifacts`, `AnalysisCtx`, `run_plan_analysis`,
`build_target_course_stats`, `build_target_term_stats`, `default_seed_for_yaml`, and the
graph helpers (`build_school`, `build_dag`, `build_equivalences`, `expand_with_prereqs`,
`build_plan_dag`, `find_equivalent_in_plan`, `build_expanded_variant`,
`placeholder_credits`, `gen_elective_placeholders`).

Leave in `mcp/tools/analyze.rs`: `AnalyzeDegreeRequest`, the `*Json` DTOs,
`AnalysisResponse`, `execute`/`execute_json`, `build_response`, `build_response_notes`,
`build_analysis_followups`, `finalize_followups`, `parse_error_response`,
`recommend_max_plans`/`next_max_plans`/`complexity_cv`, `build_per_course_metrics`.
`mcp::cache::cached_artifacts` stays in `mcp` — the `Arc` cache is a session concern; only
the `AnalysisArtifacts` type needs to be public.

- **Gate:** step 2's test still passes; no behaviour change intended in this step.
- **Bonus:** this removes the reason `--target-course` and `--metrics-out` are gated on the
  `mcp` feature (`degree.rs emit_target_course_stats`), and lets the four `#[cfg(feature =
  "mcp")]` test modules in `tests/rs/mod.rs` drop their gate.

### Step 4 — Fold the CLI's copies into core, taking the union
Delete the ten CLI helpers, routing `analyze_program` through `core::analysis`. Per the
step-4 column above, carry over the CLI-only behaviours the MCP copy lacks
(`exclude_from_prereqs`, the include-set OR preference, `typically_offered` /
`gen_ed_attributes`, the `req.from.courses` + nested-options equivalence scan,
`sampling_strategy` and `ignore_duplicates` from `Config`) and the MCP-only ones the CLI
lacks (deadline, seed, target-course stats, `is_full_population` / `population_size`).

Expect roughly 600 lines to leave `degree.rs`. Keep both output layers:
`generate_analysis_outputs` (CLI report + CSVs) and `build_response` (MCP JSON).

- **Gate:** step 2's test passes and now compares two callers of one pipeline.

### Step 5 — One elective-placeholder owner
`core::degree::placeholder`: the naming scheme, the generator (honouring
`PlanGeneratorConfig::default_elective_credits`, which only
`plan_generator::add_elective_placeholders` currently respects), the credit fallback
(routed through `term_scheduler::course_credits_with_fallback`, which already has named
constants), and `is_placeholder_course` (already single-sourced — move it here).
Promote `"ELEC"`, `"_prerequisites"` and `"_elective_placeholders"` to constants there.

### Step 6 — Delete the prereq-tokenizer copies
Route `build_school` at `core::prerequisite_parser::extract_all_courses`, and have it read
`course.prerequisites` (already populated by `resolve_prerequisites` at parse time)
instead of re-parsing `prerequisites_raw`. Also fix `yaml_parser.rs:121`, which inlines a
fourth variant.

### Step 7 — Tighten the reproducibility tests
Enumeration became reproducible once `build_artifacts` began passing its seed to
`PlanGeneratorConfig.random_seed`. A residual remains: `term_distribution` still varies at
roughly 1 run in 30, because prerequisite-chain option selection
(`course_graph::select_best_prerequisite_option_with_exclusions:1432`) breaks ties with a
stable sort over an `options` list whose order is hash-derived upstream. Pin that ordering,
then tighten `analyze::tests::test_build_artifacts_is_reproducible_for_identical_inputs`
to assert `term_distribution` too, and widen the
`target_course_population` baselines beyond `earliest_term`.

## 6. Out of scope

- **`planner` / `src/core/planner/`** — different level of analysis (section 1). Leave it.
- **Splitting the large files** (`degree.rs` 3.9k, `analyze.rs` 2.7k, `completions.rs`
  2.4k). Step 4 removes ~600 lines from `degree.rs` on its own; re-assess afterwards.
- **`src/mcp/tools/completions.rs`** — the IPEDS demographics engine has the same
  trapped-in-`mcp` shape and deserves the same treatment, but independently and later.

## 7. Decisions needed before starting

1. **Step 1 changes published metrics.** Is that acceptable now, or should the OR-group fix
   ship with a migration note / version bump? Nothing downstream is known to pin these
   numbers, but the corpus in `WebScrappedCombinedDataMetrics` was analysed with the
   current code.
2. **Which side wins where the table above says "confirm".** The `build_dag_from_graph`
   corequisite difference in particular: the CLI omits corequisites from the degree-level
   DAG and the MCP path includes them. One of those is wrong and it is not obvious which.
3. **Does `core::analysis` want to be `core::degree::analysis`?** `core::degree` already
   holds the generator, selector and validator, so the pipeline arguably belongs beside
   them rather than in a new top-level module.
