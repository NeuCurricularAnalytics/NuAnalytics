# Analysis-pipeline clean-up — plan

Status: **step 1 done (2026-09-23); steps 2–7 not started.**

Completed detail has been trimmed — `git log` has it. What is here is what is left,
plus the evidence the remaining steps depend on.

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

## 3. Root cause of the divergence — *fixed, kept for the shape of the problem*

The MCP per-plan DAG added an edge for **every** in-plan option of an OR-group; the CLI
selected exactly one. `A | B` became `A & B` whenever both landed in the plan, inflating
complexity, delay and centrality — the direction and size of the gap in section 2.

Both copies are gone (step 1). The pattern is the point: two functions answering the same
question, neither wrong-looking on its own, disagreeing only in aggregate. Sections 4
onwards are the remaining instances of that shape.

## 4. Divergence inventory

Twelve helper pairs (two now resolved), with which side is believed correct. Each needs confirming as it is
merged — the merged version must take the union of behaviours, not one side wholesale.

| concern | CLI (`degree.rs`) | MCP (`analyze.rs`) | difference | keep |
|---|---|---|---|---|
| ~~per-plan DAG~~ | ~~`build_dag_for_plan`~~ | ~~`build_plan_dag`~~ | ~~one edge per OR-group vs every in-plan option~~ | **done** — both now call `core::degree::plan_dag` |
| default seed | `default_seed_for_document` | `default_seed_for_yaml` | **different seed for the same degree** — Bowdoin derives `10047534808470998596` on the CLI and `13195075234172957162` on MCP, so the two report different samples by default even though they now compute identically | **one derivation** |
| build `School` | `build_school_from_program` | `build_school` | MCP drops `typically_offered`, `gen_ed_attributes` | **CLI** |
| degree-level DAG | `build_dag_from_graph` | `build_dag` | CLI omits corequisites; MCP includes them | **MCP** |
| equivalence map | `build_equivalence_map` | `build_equivalences` | CLI also scans `req.from.courses` + nested options | **CLI** |
| expand prereqs | `expand_courses_with_prerequisites` | `expand_with_prereqs` | MCP lacks `exclude_from_prereqs` + redundancy removal | **CLI** |
| prereq tokenizer | `parse_prerequisites_from_raw` | `parse_prereqs` | different grade-letter handling; both duplicate `core::prerequisite_parser::extract_all_courses:377` | **neither** |
| placeholder credits | `placeholder_credits` | `placeholder_credits` | MCP also treats `…SM` as small | **MCP** |
| elective filler | `generate_elective_placeholders` | `gen_elective_placeholders` | different ids and thresholds | see note |
| ~~equivalent-in-plan~~ | ~~`find_equivalent_in_plan_set`~~ | ~~`find_equivalent_in_plan`~~ | ~~identical~~ | **done** — `core::degree::plan_dag::equivalent_in_plan`, now also used by `curriculum_graph` |
| expanded variant | `create_expanded_variant` | `build_expanded_variant` | identical | either |
| plan loop | `process_plan_variants` | `run_plan_analysis` | MCP has deadline + seed + target-course; CLI has progress + exclude set | **union** |

Notes:
- The elective-filler naming mismatch was already fixed in the placeholder consolidation
  (`is_placeholder_course` is now single-sourced in `plan_generator`), but the two
  *generators* still differ in id scheme and remainder threshold. Consolidating them is
  step 4.
- The `placeholder_credits` row is not theoretical: with the DAG fixed and the seed
  pinned, Bowdoin's median total credits are 32.3834 on the CLI and 31.6166 on MCP, and
  the min/max are exactly one credit apart at both ends (32–33 vs 31–32). It is the only
  metric of the four that still disagrees. Resolve it in step 4 and re-measure.
- `core::prerequisite_parser::extract_all_courses` already exists and is tested; both
  copies should be deleted in favour of it, and `build_school` should read the
  already-populated `course.prerequisites` rather than re-parsing `prerequisites_raw`.

## 5. Step-by-step plan

Each step ends with `/check-rs` and a green `cargo test` under `--no-default-features`,
`--features database`, and `--all-features`.

### Step 1 — Fix the OR-group DAG bug in the MCP path — **DONE 2026-09-23**

Both copies were deleted and replaced with `src/core/degree/plan_dag.rs`, which the CLI
and the MCP server both call. The degree JSON was never at fault and did not change.

Two consequences the remaining steps need to know about:

- **The stored corpus is a generation behind.** The CLI's OR-group tie-break changed as a
  side effect (`.find()` / `max_by_key` with no tiebreak → `.min()` /
  `min_by_key((Reverse(count), name))` — same edge count, different choice on ties).
  Measured over all 1,088 degrees: **781 identical (71.8%), 307 moved**, median change
  among movers −0.52%, and **30 degrees (2.8%) moved ≥5%**; range −29.1% (Dakota State,
  delay 8→5) to +80% (a five-point certificate going to nine). Trimmed variant is the
  same shape. Re-import is tracked in `database-audit-todo.md`.
- **`build_plan_dag` is the shared per-plan DAG.** Steps 3 and 4 should not move or
  duplicate it; it is already where it belongs.

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
`build_expanded_variant`,
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
Delete the ten CLI helpers, routing `analyze_program` through `core::analysis`. Per the `keep` column above, carry over the CLI-only behaviours the MCP copy lacks
(`exclude_from_prereqs`, `typically_offered` /
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
- **The flat edge model cannot represent OR-of-ANDs — but it is a much smaller problem
  than it first looks, and "fix" it carelessly and you move every metric you own.**

  `parse_to_edges` gives a required set plus flat OR-groups, so
  `(A & B & C) | (A & B & D) | (A & C)` collapses to one group `{A, B, C, D}` and
  `build_plan_dag` emits **one** edge where two courses are genuinely required.
  `parse_to_dnf` models it correctly.

  **An earlier version of this note claimed "the drawn graph is right and the metrics DAG
  is wrong". That was wrong on both halves and is retracted.** The drawn graph
  (`curriculum_graph::select_best_prereq_path`) is also a heuristic, and errs the other
  way: it takes the **first** fully-satisfied DNF path rather than the smallest, so with
  `CS312 + MATH215 + STAT121` in the plan it draws three prerequisite edges for BYU's
  CS470 when `CS312 & STAT121` satisfies it with two. When no path is satisfied at all it
  falls back to the "longest partial match" and draws edges for a requirement the plan
  does not meet. Neither model is the reference; they are two different guesses.

  **Measured over 30 degrees × up to 120 plans (66,200 course-instances):**

  | flat vs DNF-minimal, per course-instance | count | share |
  |---|---|---|
  | identical edge set | 65,244 | 98.55% |
  | same size, *different course chosen* | 897 | 1.36% |
  | different size (the real defect) | 59 | **0.09%** |

  Total edges barely move: 63,058 → 63,147 (+0.14%). **The size defect is 0.09% of
  course-instances.** What actually drives metric change is the 1.36% where the two models
  pick a *different member of the same group* — a modelling preference with no right
  answer, and 15× more frequent.

  **That distinction decides whether a fix is safe.** Two candidate replacements, measured
  against current output on the same 30 degrees:

  | model | degrees unchanged | worst move |
  |---|---|---|
  | DNF-minimal, lexicographic tiebreak | 21/30 | UW Tacoma **−20.1%** complexity, delay 10→8 |
  | DNF-minimal, keeping the existing reference preference | **28/30** | +1.0% and +0.4%, both upward |

  The naive version rewrites metrics for nine of thirty degrees, almost entirely for
  reasons unrelated to the defect. The hybrid — take the smallest satisfied conjunction,
  break ties with the same `in_plan_references` preference `select_or_group_option`
  already uses — leaves 28 of 30 untouched and moves the other two *up* slightly, which is
  the expected direction for removing an under-constraint.

  **Recommendation: do not touch this as part of the analysis merge.** It is pre-existing,
  it is in the stored corpus, and step 1 neither caused nor worsened it. If it is ever
  done, do it as the hybrid, in its own commit, with the corpus regenerated and the
  `target_course_population` baselines re-recorded — and keep the size rule and any change
  to the choice rule in separate commits so each effect stays visible.

- **`parse_to_edges` can merge two independent OR-groups into one id.** In the nested-AND
  branch (`src/core/prerequisite_parser.rs`, the `contains_at_level(unwrapped, '&')` arm)
  the recursion's group ids are offset by `or_group_counter` but the counter is never
  advanced past them, so a later group can reuse an id. `((A|B) & C) & (D|E)` puts all four
  of A, B, D, E in group 0; with one edge emitted per group that is one prerequisite where
  two are required. A scan for *this specific shape* found 0 occurrences in 31,601
  expressions — but that scan looked for a group id reappearing after another intervened,
  which missed the OR-of-ANDs case above entirely. Treat the 0 as "this exact signature",
  not "the flat model is fine"; the entry above is the measurement that matters. Fix it when that file is next opened
  (step 6) — advance the counter by the number of groups the recursion produced, not by
  one — and re-run the scan to confirm it stays at zero.

## 7. Decisions needed before starting

1. ~~**Step 1 changes published metrics.**~~ **Settled 2026-09-24: regenerate.** The
   full-corpus numbers are under step 1 — 72% of degrees unchanged, 2.8% moving ≥5%. Two
   CLI-visible outputs moved: the OR-group tie-break and rendered `graph_spec` edge order.
   The regenerated corpus is staged; re-import is item 1 of `database-audit-todo.md`.
2. **Which side wins where the table above says "confirm".** The `build_dag_from_graph`
   corequisite difference in particular: the CLI omits corequisites from the degree-level
   DAG and the MCP path includes them. One of those is wrong and it is not obvious which.
3. **Does `core::analysis` want to be `core::degree::analysis`?** `core::degree` already
   holds the generator, selector and validator, so the pipeline arguably belongs beside
   them rather than in a new top-level module.
