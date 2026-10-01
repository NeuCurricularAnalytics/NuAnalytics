# Analysis-pipeline clean-up — plan

Status (2026-09-30): **the merge is done — one pipeline, `core::degree::analysis`, which
`degree analyze` and every MCP analysis tool call.** Steps 1–4 and 7 are done; what is left
is in steps 5 and 6 (small, independent) and the out-of-scope notes in section 6.

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

- `degree.rs enumerate_and_analyze_plans` → `PlanGenerator::new(&ctx.program.requirements, &ctx.program.courses, …)`
- `analyze.rs build_artifacts` → the same call

(Both now call `core::degree::analysis::analyze`, which makes that call once.)

So the distinction worth preserving is **degree-vs-map** (`planner` vs the other two), not
**CLI-vs-MCP**. What differs between the CLI and MCP paths is the *output layer*, not the
level of analysis:

- the CLI emits a degree **report artifact** — `{degree, courses, requirements, selected_plans, analysis}` plus per-plan CSVs and HTML
- the MCP tool emits a flat **metrics response** — `{complexity, longest_delay, avg_chain_length, plans_analyzed, population_size, notes, recommended_max_plans, …}`

Those two output shapes are both legitimate and should both survive. Only the pipeline
underneath them should be shared.

## 2. Why this mattered: they disagreed

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

**Now:** on all 13 fixtures and 4 samples at `max_plans` 200, the MCP and the CLI agree on
every metric's five-number summary, the plan count, the seed and the selected plans term
by term (17/17). `tests/rs/one_pipeline.rs` asserts it exactly on three sampled fixtures.

## 3. Root cause of the divergence — *fixed, kept for the shape of the problem*

The MCP per-plan DAG added an edge for **every** in-plan option of an OR-group; the CLI
selected exactly one. `A | B` became `A & B` whenever both landed in the plan, inflating
complexity, delay and centrality — the direction and size of the gap in section 2.

Both copies are gone (step 1). The pattern is the point: two functions answering the same
question, neither wrong-looking on its own, disagreeing only in aggregate. Sections 4
onwards are the remaining instances of that shape.

## 4. Divergence inventory — resolved

Where the two copies differed in anything that feeds a metric, the CLI's behaviour was
kept, because the stored corpus was produced by it. So the CLI's output did not move —
byte-identical before and after on every fixture, sample and the full 1,088-degree
conversion corpus — and fresh MCP figures converged on the stored ones.

| concern | outcome |
|---|---|
| per-plan DAG | step 1 — `core::degree::plan_dag` |
| default seed | `default_seed_for_program` (canonical JSON). The MCP's hashed the raw text, so re-indenting a file changed which plans it sampled |
| `School` | `core::report::inputs::build_school_from_program`; the MCP's dropped `typically_offered` and `gen_ed_attributes` |
| degree-level DAG | **deleted** — neither copy was read. `PlanSelector::new` ignored its `_dag` and `DegreeReportContext.dag` was never read, so decision 2 below was moot |
| equivalence map | `build_equivalence_map`; the MCP's saw only top-level `courses` |
| expand prerequisites | the CLI's exclusion set, shallow-first expansion and redundancy pruning, now in core |
| prereq tokenizer | the MCP's copy went with its `build_school`; see step 6 for the rest |
| placeholder credits, elective filler | already shared (`core::degree::placeholder`) |
| expanded variant | one copy, in core |
| plan loop | the union: the CLI's progress (as `AnalysisEvent`s) and exclusions, the MCP's deadline and target-course capture |

What moved, measured on the 17 inputs at 200 plans: MCP complexity means by up to 2.5%
(CSU 234.29 → 240.25), delay means by up to 0.16; the CLI not at all.

**Found and deliberately not changed:** both school builders drop `strict_corequisites`,
which `TermScheduler` reads, so the scheduler never enforces a degree's strict
corequisites. 73 of the 1,088 corpus degrees declare some. Honouring them moves term
counts and selected plans for those degrees, so it is its own change, with the corpus
regenerated — not part of a merge whose gate is "the CLI does not move".

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

### Steps 2–4 — Characterise, move into core, fold the CLI in — **DONE 2026-09-30**

`src/core/degree/analysis.rs` (decision 3: beside the generator, selector and
validator), not feature-gated. `analyze(program, &AnalysisConfig, on_event) ->
DegreeAnalysis`; nothing in it prints. The CLI renders its `AnalysisEvent`s when
verbose; the MCP ignores them. Each surface keeps its own defaults (MCP: 500 plans,
three samples, a 180 s deadline; CLI: `Config`) and its own output layer
(`generate_analysis_outputs`; `build_response`). `mcp::cache` caches
`Arc<DegreeAnalysis>`, and `get_course_detail` reads its graph instead of rebuilding it.

- **Proof, CLI:** every fixture, sample and corpus conversion byte-identical to the
  pre-merge binary, selected plans included, and verbose stderr and metrics files
  identical with `--include` on all 13 fixtures (the exclusion path the golden capture
  does not exercise).
- **Proof, MCP:** equals the CLI on 17/17 inputs (section 2); `tests/rs/one_pipeline.rs`.
- **`--target-course` and `--metrics-out` no longer need `mcp`**, and use the CLI's
  configuration. `--metrics-out` now writes the CLI's report JSON with
  `analysis.target_course_stats`, and `--from-db` honours `--target-course`. A
  target-course query always runs in-process: a pooled run (`-j`, several files)
  silently ignored it before, because workers were never passed the flag and their
  stdout goes nowhere.
- `degree_fixtures`, `degree_fidelity` and `target_course_population` dropped their
  `mcp` gate; `target_course_selected_plans` keeps it, as it tests the MCP response.

### Step 5 — One elective-placeholder owner — *partly done*
Done: `ELECTIVE_PREFIX` (`core::degree::placeholder`) and the `PREREQUISITES_KEY` /
`ELECTIVE_PLACEHOLDERS_KEY` requirement-choice keys (`core::degree::plan_variant`) replace
the inline literals. Left: honour `PlanGeneratorConfig::default_elective_credits`, which
only `plan_generator::add_elective_placeholders` respects, and route the credit fallback
through `term_scheduler::course_credits_with_fallback`. Move `is_placeholder_course` to
`placeholder` while there.

### Step 6 — Delete the prereq-tokenizer copies — *MCP copy gone*
Left: route `build_school_from_program` at `core::prerequisite_parser::extract_all_courses`,
reading `course.prerequisites` (already populated by `resolve_prerequisites` at parse
time) instead of re-parsing `prerequisites_raw`, and fold in the fourth variant in
`yaml_parser.rs`. This changes `School.prerequisites`, which the scheduler reads, so gate
it on the CLI output like the merge was.

### Step 7 — Tighten the reproducibility tests
**The ordering fix is done (2026-09-30).** Fresh analysis was not reproducible across
processes: on a 60-degree corpus sample, 21 degrees reported different named plans run to
run and 55 different random samples; CSU's CLI aggregates moved about one run in 20.
Five hash-order sources, found with a cross-process probe:

- `PlanIterator::sample_indices` collected the seeded sample from a `HashSet` (right set,
  varying order) — now sorted;
- OR-groups resolved in `HashMap` order in `collect_min_chain_from_edges_with_exclusions`
  and in the structured chains (`OrGroupsMap`) — now `BTreeMap`;
- `detect_cycles` started its search in hash order, so a tied cycle lost a different edge
  (CSU's CS152 ↔ CS163) — roots now sorted;
- `compute_longest_delay_chain` broke a critical-path tie by hash order — now by course key;
- the CLI never seeded the plan selector — now seeded like enumeration.

After: 0 of 60 vary in anything, and all 17 fixtures/samples are identical across 5
processes on both paths. Aggregates never moved; stored runs are untouched.
`test_build_artifacts_is_reproducible_for_identical_inputs` now asserts the whole
`target_course_stats`, `term_distribution` included. The `target_course_population`
baselines stay at `earliest_term`: runs are reproducible on one toolchain now, but the
default seed still comes from `DefaultHasher`, which std does not promise to keep stable
across toolchains, so the other figures could still move on a compiler upgrade.

## 6. Out of scope

- **`planner` / `src/core/planner/`** — different level of analysis (section 1). Leave it.
- **Splitting the large files** (`completions.rs` 2.4k). The merge took `degree.rs` from
  3.8k to 2.9k lines and `analyze.rs` from 2.5k to 1.7k.
- ~~**`src/mcp/tools/completions.rs`** — the IPEDS demographics engine has the same
  trapped-in-`mcp` shape.~~ **Done 2026-09-24.** It, `institutions`, `cip_codes`, `lookup`
  and most of `degrees` now live in `src/core/query/`, with the shared JSON helpers in
  `src/core/json.rs`. `compare_degrees`' fresh metrics call the `analyze_degree` tool,
  which now runs the shared pipeline.
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

## 7. Decisions

1. ~~**Step 1 changes published metrics.**~~ **Settled 2026-09-24: regenerate.** The
   full-corpus numbers are under step 1 — 72% of degrees unchanged, 2.8% moving ≥5%. Two
   CLI-visible outputs moved: the OR-group tie-break and rendered `graph_spec` edge order.
   The regenerated corpus is staged; re-import is item 1 of `database-audit-todo.md`.
2. ~~**Which side wins where the table above says "confirm".**~~ **Moot 2026-09-30.** The
   corequisite difference was in the degree-level DAG, which nothing read; it is deleted.
3. ~~**Does `core::analysis` want to be `core::degree::analysis`?**~~ **Settled
   2026-09-30:** `core::degree::analysis`, beside the generator, selector and validator.
