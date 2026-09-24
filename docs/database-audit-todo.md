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
metrics. The only such entry point is `build_artifacts`, which is gated behind the `mcp`
feature; the CLI's `run_analyze` / `run_analyze_from_db` write files and print, returning
nothing.

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

---

## What this audit did *not* check

- **Completions for 2022–2024.** Only the 2025 source files are available locally, so the
  earlier years were checked for row counts only, never column values.
- **`institution_completion_totals` (77,422 rows).** Not compared against a recomputation
  from the source file.
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
