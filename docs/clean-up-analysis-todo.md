# Analysis-pipeline clean-up — plan

Status (2026-10-01): **the merge is done — one pipeline, `core::degree::analysis`, which
`degree analyze` and every MCP analysis tool call.** Steps 1–4 and 7 are done; what is left
is in steps 5 and 6 (small, independent) and the out-of-scope notes in section 6. The
corpus was re-imported on 2026-10-01 with everything below except where noted.

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
  same shape. The corpus has been re-imported since (latest 2026-10-01).
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
- **What that proof missed.** It compared corpus *conversions*, not corpus analyses. The
  core equivalence builder that replaced the CLI's (`report::inputs::build_equivalence_map`)
  recurses through every level of nested options; the CLI's read only one level. That
  changes the map for 7 corpus degrees, and the figures of 2: Northeastern's BA and BS
  concentration, whose `{CS2800, CS4820}` slot sits deeper, lost 614 and 886 plans
  because CS4820 then stood in for its own prerequisite. The recursion is kept — the
  deeper groups are real — and the stand-in is fixed (section 6).
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
- **OR-of-ANDs are resolved by branch — DONE 2026-10-01.** Three defects, fixed together
  because each hid the others:

  1. **`parse_to_edges` read `|` as binding tighter than `&`.** It split on `&` first, so
     `CS312 & MATH215 & STAT121 | CS312 & STAT121` meant "CS312, MATH215 and STAT121
     required, plus STAT121 or CS312". Every other reader — `parse_to_ast`,
     `parse_to_dnf`, `PrereqExpr::to_expression_string` — has `&` tighter, and
     `to_expression_string` omits the parentheses precedence implies, so a prerequisite
     stored as a tree (`programs.document`, many corpus files) read back wrong while the
     same prerequisite written with parentheses read right. Measured in
     `database-audit-todo.md` section 4.
  2. **The flat edge model pooled an OR-of-ANDs into one group.**
     `(MATH124 & MATH126) | MATH127` (CSU MATH156) became "any one of the three": a plan
     taking the two-course branch got one edge, and expansion added *one* pooled member —
     the cheapest — leaving plans that satisfied neither branch (CSU's Longest Path had
     MATH156 with MATH126 alone). An earlier sample put the size defect at 0.09% of
     course-instances; it is larger than that once the precedence bug stops masking it.
  3. **A nested AND's OR-group ids were offset but the counter never advanced,** so in
     `X & (A & B | C) & (D | E)` the two groups shared an id. No corpus expression has the
     shape, but the structured form of one would.

  **Now:** `parse_prerequisites` returns the flat edges plus, for each OR-group with a
  multi-course alternative, its branches (the group's DNF), stored as
  `CourseNode::or_branches`. Groups whose alternatives are all single courses — and every
  degree without one — take exactly the old code path. For the others, per plan:

  - the plan DAG draws every course of one branch the plan completes: a forced
    (`--include`) course's branch, else the smallest, else the most referenced
    (`select_or_group_branch`);
  - expansion adds a whole branch: one the plan completes, else the one adding the fewest
    new courses, then same subject, then name — the single-course rule's order, so
    MATH156 gets MATH127 alone, or MATH124 when the plan already has MATH126;
  - redundancy pruning and the `--include` exclusion set reason by branch too.

  **Measured over the whole corpus** (all 1,088 analysed corpus files, 10,000-plan cap,
  stored seeds, old vs new analyzer):

  | | programs |
  |---|---|
  | no multi-course OR-group — full report JSON byte-identical | **958 / 958** |
  | with one | 130 |
  | … report changed | 96 |
  | … complexity mean moved | 91: median +0.38%, 55 up and 36 down, −17.2% to +22.7%; 24 by ≥5% |
  | … delay mean / credits mean moved | 27 / 23 |
  | plan counts changed | 0 |

  Up is a plan now credited every course of the branch it takes (College of Charleston's
  CSCI221 needs a lecture and its lab); down is a course no longer forced by the
  precedence bug (USF's PHYS303 forced CS110). CSU's plans no longer contain an
  OR-of-AND course without a complete branch (2 of 30 before, 0 after).

  **Still flat:** the audit's structured chains (`structured_prerequisite_chain`) and the
  drawn curriculum graph (`curriculum_graph::select_best_prereq_path`, which takes the
  *first* satisfied DNF path rather than the smallest). Neither feeds a metric; aligning
  the drawing with `select_or_group_branch` is the obvious follow-up.

- **A course stood in for its own prerequisite — DONE 2026-10-01.** An equivalence group
  `{X, Y}` lets an in-plan Y satisfy a prerequisite on X. When Y itself requires X
  (Northeastern's `{CS2800, CS4820}`, Duke's `{COMPSCI310, COMPSCI510}`), a plan taking Y
  resolved Y's prerequisite to Y: expansion never added X, the plan DAG drew a self-loop,
  the plan's metrics failed, and the plan was silently discarded. Now the dependent course
  is excluded from its own prerequisite's equivalents — in expansion and in
  `plan_dag::equivalent_in_plan`, which the drawn graph shares — so X is added and the
  plan kept. Whole corpus, 10,000-plan cap: **4 of 1,088 full degrees move, 1,084
  byte-identical** — Miami BS 4,889 → 5,922 plans, Northeastern BS 9,099 → 9,985 and BA
  9,364 → 9,978 (their 09-28 counts), Duke 9,554 → 9,995; complexity means +0.4% to
  +1.0%. 15 corpus degrees have such a group; in the other 11 no plan takes Y without X.

## 7. Decisions

1. ~~**Step 1 changes published metrics.**~~ **Settled 2026-09-24: regenerate.** The
   full-corpus numbers are under step 1 — 72% of degrees unchanged, 2.8% moving ≥5%. Two
   CLI-visible outputs moved: the OR-group tie-break and rendered `graph_spec` edge order.
   The regenerated corpus is staged; re-import is item 1 of `database-audit-todo.md`.
2. ~~**Which side wins where the table above says "confirm".**~~ **Moot 2026-09-30.** The
   corequisite difference was in the degree-level DAG, which nothing read; it is deleted.
3. ~~**Does `core::analysis` want to be `core::degree::analysis`?**~~ **Settled
   2026-09-30:** `core::degree::analysis`, beside the generator, selector and validator.
