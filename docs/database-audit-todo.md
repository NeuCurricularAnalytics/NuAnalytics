# Database audit — what is left

The audit that produced this document ran 2026-09-18 → 2026-09-24 and is finished: the
IPEDS data was checked against its source files, the defects it found were fixed, and the
corpus was loaded. Findings F1–F10 and Steps 1–6, 8 and 9 are done and have been removed;
`git log` has them. What follows is only what has not been done, plus the facts needed to
do it.

**State of the backend as of 2026-09-24**

| | |
|---|---|
| `db doctor` | 8/8 — 20 tables, 2,173 `cip_codes`, no row cap |
| `db validate --year 2025 HD2025.zip` | 5,985 rows, 11/11 columns, zero mismatches |
| `db validate --year 2025 C2025_A.zip` | 313,566 = 313,566 rows, 21/21 columns |
| IPEDS | 6,515 institutions, 1,225,788 completions, 4 years (2022–2025) |
| corpus | 1,088 programs, 2,176 analysis runs, 49,434 program-courses, 13,585 requirements |

---

## 1. ~~Re-import the corpus~~ — **DONE 2026-09-24**

Schema applied to the live instance (the missing columns plus a revert of the
ownership-scoped `degrees` policy), then 2,176 reports imported with `--replace`:
3 created, 2,171 updated, 2 skipped, **0 errors**. The two "skipped" were
document-unchanged only; their runs and children were still written.

Verified against the source corpus — every figure matches exactly:

| | stored | source |
|---|---|---|
| programs | 1,088 | 1,088 |
| `external_requirement = true` | 2,082 across 508 programs | 2,082 / 508 |
| `external_credits` non-null | 2,081 | 2,081 |
| `program_courses.grade_minimum` | 520 | 520 |
| `analysis_course_metrics.chain_length_mean` | 156,397, **zero null** | — |

Orphan check after cleanup: zero orphaned courses, requirements, runs or metrics.

**A real data defect surfaced and was fixed.** The import produced 1,091 programs against
a 1,088-degree corpus. Three Miami University (Oxford, OH) degrees carried
`unitid: 135726` — *University of Miami*, Coral Gables FL — while their `source_url` was
`bulletin.miamioh.edu`, and a fourth degree in the same corpus already used the correct
`204024`. The corpus files were wrong, not the importer; a domain-vs-unitid sweep over all
1,088 degrees found this as the only genuine conflict (8 other multi-domain unitids are
catalog hosts: kuali.co, smartcatalogiq, coursedog, enrole, archive.org).

Fixed at source — `unitid` and `institution` corrected in all six files (3 degrees × both
trees) to match their correct sibling — then re-analysed, re-imported, and the three stale
rows under the wrong key deleted with their children. Miami Oxford is now 4 programs under
204024; University of Miami keeps its 3 under 135726.

**Note on run history:** runs append rather than replace, so `analysis_runs` is now 4,353
(2,176 previous + 2,177 new). `db prune` exists for this; nothing has needed it yet.

## 2. ~~Tidy the corpus repo~~ — **DONE 2026-09-24** *(one decision left for you)*

Answered in the corpus repo's `README.md`, which also had a stale Layout section (it
listed a `json_corrected/` that no longer exists and omitted `v2/`, `metrics/`, `samples/`,
`Presentations/`, `ResearchQuestions/`):

- **`json_corrected_old/` (1,106 files, 26 MB) — superseded, safe to delete.** The
  pre-`unitid` generation. Not a strict subset: 65 filenames appear only there and no
  shared file is byte-identical — but every one of the 65 has a counterpart in the current
  corpus once naming drift is allowed for. No degree is lost. Git-tracked, so removal is
  recoverable. **Not deleted — that is yours to run.**
- **`_audit/` (1,513 files, 43 MB) — keep.** `GAP_REPORT.md` cites it five times for its
  evidence and `full_degree/README.md` points at `_audit/rebuild_agent_prompt.md` as the
  rebuild discipline. Deleting it would leave both citing missing files.

A "Generations" section now records the two analysis generations that exist. `v2/` was
overwritten with the imported run on 2026-09-24, so the corpus repo and the database are
the same generation — verified on both sides (2,082 external nodes / 508 degrees, 520
course grade minimums).

Still open: confirm the partial per-course metrics are the expected consequence of plan
sampling rather than a gap — cheapest via one degree where a named uncovered course is
checked against the plans actually generated.

## 3. `db remetric` — deliberately not built

An in-place metric backfill needs something that analyses a degree and returns its
metrics. Since 2026-09-30 that exists without the `mcp` feature:
`core::degree::analysis::analyze` returns a `DegreeAnalysis`, and it is the pipeline that
produced the stored corpus. So the entry point is no longer the blocker; the verification
problem below still is.

The original blocker — that the MCP pipeline treated an OR-group as an AND — **was fixed
on 2026-09-23**, so the two pipelines now build the same per-plan DAG. What still blocks
it is narrower: the stored corpus was produced before that fix, so a `remetric` that
recomputes and verifies would find its numbers disagreeing with every stored run and
correctly refuse. Item 1 above closes that gap.

Making it "work" by dropping the verification step is the wrong trade — verification is
the one thing stopping it silently overwriting a historical record with numbers from a
different pipeline.

Re-import remains the supported path, and uses the pipeline that produced the existing
numbers, so results stay comparable:

| | re-import | remetric |
|---|---|---|
| corpus (files on disk) | analyse + import | blocked |
| database-only programs | `degree analyze --from-db` | blocked |
| history | appends a run | would patch in place |

## 4. Stored runs against the current analyzer — measured, fixed and re-imported 2026-10-01

61 programs (every 18th by `program_key`), each re-analysed with the current analyzer at its
stored settings: 10,000-plan cap, shuffled, duplicates skipped, the stored seed. All 1,088
latest full runs share those settings (0.5.4, imported 2026-09-29).

| | programs |
|---|---|
| seed derived from the degree = stored seed | 61 / 61 |
| plan count identical | 61 / 61 |
| every figure bit-identical | 49 / 61 |
| means identical, quartiles differ | 6 / 61 |
| means differ | 6 / 61 |

**The stored runs were right.** The analyzer of the day, run on the corpus files the runs
came from (`full_degree/v2/degree/`), reproduced the stored means of all 12 that moved, and
so did the analyzer that produced them (`3f10dbc`, rebuilt from git).

- **Quartiles only:** the quantile reservoir was unseeded until 2026-09-30, so a stored
  quartile for a population over 10,000 is one random draw — five runs of `3f10dbc` gave
  four or five different quartiles and one mean. The current analyzer gives one value.
  Neither is wrong; a re-import pins them.
- **Means:** the *stored documents* did not analyse as the files did.
  `programs.document` carries prerequisites as a structured tree, which reads back with
  precedence-only parentheses (`A & B | C`), and `parse_to_edges` read `|` as binding
  tighter than `&`. 79 stored documents (140 courses) parsed differently from their
  files, so they were misread whenever re-analysed — `degree analyze --from-db`,
  `fresh=true`, `compare_degrees(metrics="fresh")`, or a degree exported from the
  database. BYU moved 21% (complexity 135.9 → 164.8). Reading a stored run was never
  affected.

**Fixed 2026-10-01**, together with OR-of-AND branch resolution
(`clean-up-analysis-todo.md` section 6). Re-measured over all 1,088:

| stored document vs the corpus file it came from | programs |
|---|---|
| prerequisites parse differently | 79 → **0** |
| analysis identical (figures, plan count, selected plans) | **1,075** |
| analysis differs, for an older reason below | 13 |

The 13 are representational differences the parser fix does not touch; the old and new
analyzers agree on every one of them:

- **12 — the seed.** The default seed hashes the canonical JSON, and a corpus string with
  redundant nesting (Oklahoma's `(CS2413 & CS2813) & MATH3333`) keeps a nested `and` that
  the stored tree flattened. Same prerequisites, different seed. Six reach the same plans
  and agree to 15 significant digits, apart from the Random Sample; six are sampled at the
  cap and draw a different sample.
- **1 — Northeastern AI.** Its file gives 67 courses `prerequisites: ""`; the stored
  document omits the field. The resolver orders a choice pool "no prerequisites first" by
  `prerequisites_raw.is_some()` (`requirement_resolver.rs:512`), which counts the empty
  string as a prerequisite, so the enumeration order and a capped sample differ. It is the
  only corpus file with a blank prerequisite string.

Both matter only when a stored program is re-run fresh, and fixing either would move
stored figures, so neither is fixed yet.

**Re-imported 2026-10-01.** Both trees of the corpus (`full_degree/v2/`,
`trimmed_degree/v2/`) were re-analysed with the same documents, parameters and seeds and
imported as 1,088 `full` and 1,088 `trimmed` runs. Every program's newest run of each
variant now equals its report in the corpus repo, 2,176 / 2,176. Against the 2026-09-28
runs, 95 full and 85 trimmed programs' means moved: 91 of the full ones are the OR-of-AND
fix, and the rest predate it. The corpus repo's README ("Generations") has the breakdown and
the archive of the previous `v2/`. Runs append, so `analysis_runs` keeps the earlier
generations; `db prune` removes them if the history is not wanted.

**Two Northeastern degrees lost plans — found and fixed 2026-10-01.** The BA in CS (Boston)
analysed 9,364 plans where the 09-28 build gave 9,978, and the BS concentration 9,099 where
it gave 9,985. The pipeline merge's equivalence builder reads nested options to any depth,
so it newly saw their `{CS2800, CS4820}` slot; since CS4820 requires CS2800, a plan taking
CS4820 resolved that prerequisite to CS4820 itself, drew a self-loop and was discarded. The
same defect had long cost Miami's BS 17% of its plans and Duke 4.4%. Fixed, and the four
degrees re-imported; `clean-up-analysis-todo.md` section 6 has the detail.

---

## What this audit did *not* check

- ~~**Completions for 2022–2024.**~~ **Checked 2026-10-01** against files downloaded from
  NCES: every year's row count and all 21 count columns, summed over every row, equal the
  file. 2022 and 2023 hold the revised releases (every row the revision changed carries the
  revised value). The check found one defect: the 2022 header pads `CNRALW` with spaces,
  so every 2022 row stored NULL nonresident-alien women. Headers are now trimmed and 2022
  re-imported. `db validate` passed throughout, because it parses with the importer's own
  code; the per-column sums, computed independently, are what caught it. HD files older
  than 2025 cannot be validated against `institutions`, which holds only the newest
  directory.
- ~~**`institution_completion_totals` (77,422 rows).**~~ **Fixed 2026-10-01.** Recomputed
  against `completions` on 2026-09-29, every row was exactly **twice** the real total:
  ingest summed the CIP 99 grand-total row beside the detail rows it totals. Ingest has
  left CIP 99 out since (`counts_toward_institution_totals`), and all four years were
  re-ingested on 2026-10-01: every row now equals its detail rows on all 21 columns, with
  no row missing or left over (19,281–19,447 per year). The demographics queries never read
  it, but `query_sql` users do, so it is kept and correct.
- **The `C2025_B` and `C2025_C` files.** Not imported and not examined.
- **Policy state on the live database.** The policies were read from `schema.sql`, not
  from `pg_policies` — there is no SQL path from this client to confirm the live database
  matches the file.

## Settled, recorded here because the reasoning is easy to re-litigate

- **Access model — writes stay open to any authenticated member.** Ownership policies
  (`created_by = auth.uid()`) were implemented and reverted: they stop
  `db import --replace` overwriting a row another member created, and that has to keep
  working. `created_by UUID DEFAULT auth.uid()` remains on `programs`, `degrees`,
  `program_courses` and `program_requirements` as **attribution only** — no policy reads
  it. Drop the column too if you would rather not carry an unused one. If enforcement is
  ever turned on, note that existing rows are all `created_by IS NULL` and a re-import
  will not claim them: the upsert never sends the column.
- **IPEDS stays writable by members.** Making it read-only with a separate importer role
  was considered and dropped.
- **The `completions` table is complete.** All CIP codes, both major numbers, ~313,000
  rows per year — not the CS-only subset four documents used to claim.
