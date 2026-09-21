# Database audit — findings and plan

Audit of the live self-hosted backend against the IPEDS source files, plus a review of the
schema, the access model, and the JSON corpus in `../WebScrappedCombinedDataMetrics`.

**Method.** Row counts and column values were compared against `HD2025.csv` and
`C2025_A.csv` (the copies in `~/Downloads`, extracted 2026-09-18). Institutions: all 5,985
HD2025 rows, 11 columns each. Completions: a 5,000-row page of CIP-11 rows for 2025, 21
demographic columns each, ~105,000 value comparisons. Everything marked **[measured]**
below was observed, not inferred from reading code.

Dates are absolute: this audit ran **2026-09-18**.

---

## Findings, worst first

### F1 — `99` is read as a privacy sentinel and nulls legitimate counts **[measured]**

`parse_ipeds_int` (`src/core/database/ipeds/ingest.rs:143-150`) maps `"."`, `"-2"` **and
`"99"`** to `None`. That is right for categorical HD columns — `SECTOR = 99` really does
mean "sector unknown" — and wrong for every count column in the completions file, where
99 is just ninety-nine graduates.

Proof, from `C2025_A.csv`:

    UNITID 110653, CIP 11.0204, award 5, major 1
      CTOTALT = 99     XCTOTALT = 'R'   (R = Reported)
      CTOTALM = 71     CTOTALW  = 28    (71 + 28 = 99)

In the 2025 file, `CTOTALT`/`CTOTALM`/`CTOTALW` hold the value `99` **384 times**, and all
384 carry the imputation flag `R`. Not one is suppressed. Every one of them is `NULL` in
the database today.

Consequence: a silent, one-directional **undercount**. Any institution/CIP/award
combination with exactly 99 completions in a category drops out of sums instead of
contributing. It cannot be noticed from the output — the same class of defect as the
`PGRST_DB_MAX_ROWS` cap, and the reason this audit found it rather than a user.

Scale: 384 in three columns of one year. Across 21 demographic columns and four years,
expect low thousands of nulled counts.

### F2 — Institution attributes are 2022-era **[measured]**

`updated_year` across the 5,985 institutions that exist in HD2025:

| `updated_year` | rows |
|---|---|
| 2022 | 5,784 |
| 2025 | 201 |

Column mismatches against HD2025, over all 5,985:

| Column | Mismatches |
|---|---|
| `carnegie_class` | 3,397 |
| `locale` | 615 |
| `inst_size` | 591 |
| `name` | 222 |
| `sector` | 197 |
| `iclevel` | 172 |
| `city` | 115 |
| `control` | 13 |
| `state` | 9 |
| `hbcu` | 1 |

**Cause: `HD2022` was imported last and overwrote everything.** The upsert uses
`Prefer: resolution=merge-duplicates` on `unitid`, so the most recent *import* wins, not
the most recent *year*. The arithmetic confirms it — the only rows not stamped 2022 are
the 201/32/26 that did not exist when the 2022 file ran.

Consequence: every analysis that joins 2025 completions to institution attributes is using
2022 classifications. `carnegie_class` is wrong for **57%** of institutions.

Not a code bug. But nothing prevents a repeat: `updated_year` is written and never read.

### F3 — Any authenticated user can overwrite or delete the whole shared corpus

`institutions`, `completions` (1.2M rows) and `institution_completion_totals` all carry
`FOR ALL USING (auth.role() = 'authenticated')` (`schema.sql:233-236`). There is no
ownership column, no audit trail, and no soft delete. One member running `db ipeds-import`
against the wrong file, or issuing a DELETE, takes the shared reference data with them.

§5 of `db-migration-todo.md` proposes ownership for the four *program* tables and
explicitly leaves the IPEDS tables "shared by design". That is the right call for reads —
but "shared" currently also means "any member can destroy all of it".

### F4 — The docs describe a `completions` table that does not exist **[measured]**

`docs/database/setup.md` and the `db ipeds-import` help both say the completions file
populates `completions` with **"CS CIP codes only"**, and `CHANGELOG.md` says completions
"now filter to MAJORNUM=1 only". Neither is true. Measured for 2025:

| Cut | Database | `C2025_A.csv` |
|---|---|---|
| all rows | 313,566 | 313,566 |
| `major_num = 1` | 292,851 | 292,851 |
| `major_num = 2` | 20,715 | 20,715 |
| CIP `11.*` | 14,913 | 14,913 |

The import is **complete and faithful** — every row, every CIP, both major numbers. The
data is right and the documentation is wrong, which is the more dangerous direction: it
invites someone to size a query for ~15k rows when the table holds 313k per year, which
is exactly the interaction that makes a `PGRST_DB_MAX_ROWS` cap bite.

### F5 — The entire stored-programs half of the schema is empty **[measured]**

| Table | Rows |
|---|---|
| `degrees` | 0 |
| `programs` | 0 |
| `program_courses` | 0 |
| `program_requirements` | 0 |
| `analysis_runs` | 0 |
| `courses` | 0 |

Meanwhile `../WebScrappedCombinedDataMetrics` holds **1,088 degree builds and 1,088 metrics
reports** per variant (`full_degree/`, `trimmed_degree/`), none of it in the database. Six
of the twenty tables, their indexes and 15 RLS policies exist to serve data that has never
been loaded.

### F6 — The JSON corpus is more consistent than expected **[measured]**

The worry was that a newly added metric had not been backfilled. It has:

- `analysis.metrics` is `('complexity', 'credits', 'delay')` in **all 1,088** files, both
  variants.
- Course-level metrics are `('blocking', 'centrality', 'complexity', 'course_id', 'delay',
  'plan_count')` in **all 1,088** files, both variants.

The metric *set* is therefore uniform across the corpus — but see **F7**: it is uniformly
one metric short, because every file predates `chain_length`. Two further things to
confirm:

- **886 of 1,088** `full_degree` files carry course metrics on only *some* of their
  courses (797 of 1,088 for `trimmed_degree`). Spot checks show the uncovered ones are
  real elective options — `STAT203`, `PSYC324`, `COSC498` — that were never selected into
  a sampled plan, which is the expected consequence of a `max_plans` cap plus reservoir
  sampling. Needs confirming rather than assuming, because the alternative explanation
  (never enumerated at all) looks identical from outside.
- `json_corrected_old/` holds **1,106** files and `_audit/` another ~1,300. Archive or
  delete; right now they are indistinguishable from live inputs to anyone new.

### F7 — The `chain_length` metric exists in the engine and nowhere else **[measured]**

PR #24 (`f2a5f30`, merged **2026-07-09**) added a per-course **`chain_length`** — the
longest *incoming* prerequisite chain including the course, i.e. "how far into the program
must a student get before they can take this course". It is not a shortest-path
computation, but it is the quantity that determines the earliest term a course is
reachable. The same commit added degree-level `avg_chain_length` and `min_chain_length` to
the aggregator.

It is missing from all three places it would need to be:

| Where | State |
|---|---|
| Corpus `*_report.json` | **Absent** — 0 of 1,088 files, both variants |
| Database schema | **No column** — `analysis_course_metrics` has no `chain_length` |
| CLI degree-level report | **Absent** — `analysis.metrics` is complexity/delay/credits only |

**The corpus predates the metric.** Every one of the 1,088 summaries records
`timestamp = 2026-06-09`, a month before the metric landed. The `2026-09-01` file mtimes
are a copy or move, not a re-analysis — which is why the metric set looked uniform in F6.
Uniformly *old* is still uniform.

**A re-run would fix the per-course half but not the degree-level half.** Running today's
binary on `tests/assets/degrees/bowdoin-college-computer-science.unified.json` produces
course metrics `('blocking', 'centrality', 'chain_length', 'complexity', 'course_id',
'delay', 'plan_count')` — seven keys where the corpus has six. But `analysis.metrics` is
still only `('complexity', 'credits', 'delay')`: the degree block in
`unified_report.rs:118-123` is a hand-written `json!` that was never extended, so
`avg_chain_length` and `min_chain_length` are computed and then dropped. They surface only
in the MCP `analyze` output (`src/mcp/tools/analyze.rs:369-371`), and even there one
construction path sets both to `None`.

Per-course `chain_length` rides along automatically because `unified_report.rs:80`
serialises the whole stats struct — which is exactly why nobody noticed the degree-level
fields never joined it.

Not to be confused with the `shortest.csv` plan export ("Shortest Path"), which is a
*selected plan*, not a metric, and does exist.

---

## Plan

Each step ends with `/check-rs` green and the validator from Step 2 re-run.

### Step 1 — Split the IPEDS integer parser *(code, small)*
`parse_ipeds_int` serves two kinds of column and gets one of them wrong. Split it:

- `parse_ipeds_code` — categorical HD columns. Keeps `"."`, `"-2"`, `"99"` as missing.
- `parse_ipeds_count` — completions counts. `"."`, `"-2"` and empty are missing; **`99` is
  a number.**

Check every call site individually rather than swapping the default; the HD columns
genuinely need the old behaviour. Add a test asserting `parse_ipeds_count("99") == Some(99)`
and that the HD path still nulls `SECTOR = 99`.

### Step 2 — `nuanalytics db validate <ipeds-file>` *(code)*
Turn this audit into a command. Given a local HD or C file, compare it against the backend
and report per-column mismatch counts with examples — which is exactly the by-hand work
above, and the only way to know Steps 1 and 3 worked.

Reuse the row-limit lesson: `count=exact` for totals, paged selects for values. Report
"could not check" separately from "mismatched", the way `doctor`'s schema check already
separates missing from unreachable.

**Gate for the whole plan:** `db validate HD2025.csv` and `db validate C2025_A.csv` both
report zero mismatches.

### Step 3 — Re-import, oldest year first *(operational)*
After Step 1 lands, re-import so the corrected parser is applied and the newest HD wins:

    2022 → 2023 → 2024 → 2025,  HD then C for each year

Completions is ~1.2M rows over four years; budget accordingly. Verify with Step 2, and
confirm `updated_year = 2025` for all 5,985 current institutions.

### Step 4 — Stop an older year from overwriting a newer one *(code)*
`updated_year` is written and never read, which is what made F2 possible and invisible.
Before importing an HD file, compare its year against the maximum `updated_year` present;
if it is older, refuse unless `--force`, naming both years. Cheap — one `count_rows`-style
probe — and it turns a silent overwrite into a question.

### Step 5 — Correct the documentation *(docs)*
Fix `setup.md`, the `db ipeds-import` help text and the `CHANGELOG` troubleshooting row to
describe the table that exists: all CIP codes, both major numbers, ~313k rows per year.
State the per-year magnitude explicitly — that number is what makes the row cap matter.

### Step 6 — Decide the access model for shared reference data *(SQL + decision)*
F3 and §5 of `db-migration-todo.md` are the same conversation. Options, not mutually
exclusive:
1. `created_by` + ownership policies on the four program tables — already specified in §5.
2. Make the IPEDS tables read-only to ordinary members, with imports run by a separate
   role. Fits how they are actually used: one person imports, everyone reads.
3. Leave writes open but add an audit column so damage is attributable.

Recommend 1 + 2. Needs a decision before implementation.

### Step 7 — Decide the fate of the stored-programs tables *(decision)*
Either load the corpus with `db import` and make the tables real, or drop them and stop
carrying six tables, their indexes and 15 policies that serve nothing. Loading is the
obvious choice if the research questions need cross-degree SQL; dropping is right if the
JSON corpus is the system of record.

### Step 8 — Carry `chain_length` through to where it is consumed *(code)*
Three separate gaps, smallest first:
1. **Degree-level report.** Extend the hand-written `json!` in `unified_report.rs:118-123`
   to include `avg_chain_length` and `min_chain_length` from the aggregator, which already
   computes both. Without this a corpus re-run still loses them.
2. **Database column.** Add `chain_length_mean` to `analysis_course_metrics` and include
   `chain_length` in its `metrics` JSONB, or the metric is dropped on import. Fold into
   the same `programs-schema.sql` edit as Step 6, since every deployment is a fresh
   install.
3. **The MCP `None` path** (`analyze.rs:767-768`) — establish whether that is deliberate
   or an oversight before relying on the field.

### Step 9 — Re-analyse the corpus *(operational, after Step 8)*
The 1,088 degrees were analysed 2026-06-09 with a binary that had no `chain_length`.
Re-run both variants so the corpus carries it. Do this **after** Step 8.1, or the
degree-level chain statistics will be missing from the new output too.

Expect other numbers to move as well: the run will pick up everything merged since June,
including the plan-generator seeding fix. Record before/after for the three sample degrees,
as the analysis plan's Step 1 already requires.

### Step 10 — Tidy the corpus *(decision + housekeeping)*
Confirm the partial per-course metrics in F6 are the expected consequence of plan sampling
— easiest via one degree where a named uncovered course is checked against the generated
plans. Then decide whether `json_corrected_old/` and `_audit/` are archival or deletable,
and say so in that repo's `README.md`.

---

## What this audit did *not* check

- **Completions for 2022–2024.** Only the 2025 source files are available locally, so the
  earlier years were checked for row counts only, never column values.
- **`institution_completion_totals` (77,413 rows).** Not compared against a
  recomputation from the source file.
- **The `C2025_B` and `C2025_C` files.** Not imported and not examined.
- **Policy state on the live database.** The policies above were read from `schema.sql`,
  not from `pg_policies` — there is no SQL path from this client to confirm the live
  database matches the file.

## Decisions needed before starting

1. **Step 3 re-imports 1.2M rows and will change published figures** wherever a `99` was
   nulled or an institution attribute was stale. Acceptable, or does it want a version
   bump and a note?
2. **Step 6** — which access model.
3. **Step 7** — load the corpus, or drop the tables.
