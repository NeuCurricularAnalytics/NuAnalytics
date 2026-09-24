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

### Step 1 — Split the IPEDS integer parser *(code)* — **DONE**

`parse_ipeds_int` served two kinds of column and got one wrong. Now two functions:

- `parse_ipeds_code` — categorical HD columns. Only `"."` and empty are missing; every
  code is stored, including `99` and the negatives, because `lookup-seed.sql` labels
  them. (See the decision block below — the first cut of this nulled `99`/`-2`.)
- `parse_ipeds_count` — completions counts. `"."` and empty are missing, **`99` is the
  number ninety-nine**, and **every negative is missing**.

The macros at each call site were renamed `get_code!` / `get_count!`, so the column kind
is now visible where it is used rather than inferred.

**Measured recovery, `C2025_A.csv`:** 566 values across the 21 count columns were being
nulled, totalling **56,034 graduates** — 24 values / 2,376 graduates within CIP 11. About
four times that across the four imported years, pending Step 3.

Two things the review changed beyond the original plan:

- **Rejecting every negative count, not just `-2`.** The first cut kept only `-2` as a
  sentinel while the doc claimed "a count cannot be negative" — and a test had
  incidentally locked in `parse_ipeds_count("-1") == Some(-1)`. `accumulate_demo_totals`
  sums these into the denominator of every representation ratio, so a negative would
  shrink it silently. The rule now matches its documentation.
- **`build_institution` + `HdCols` extracted** from the 110-line `ingest_institutions`,
  mirroring `build_completion`/`DemoCols` on the completions side. Not cosmetic: the HD
  call site had **no test at all**, so switching it to `parse_ipeds_count` compiled and
  the whole suite still passed — re-corrupting `SECTOR`/`LOCALE`/`INSTSIZE` = 99 in the
  opposite direction. That mutation now fails a test.

Also fixed here: two tests that had **encoded the defect** by using `"99"` as their
sentinel example, and the module/function docs claiming the completions ingest filters to
CS CIP codes (it does not — that is F4, fixed in this file at the same time).

**[decided 2026-09-21] Categorical codes are now stored, not nulled.** Option 1 of the
three that were on the table: `parse_ipeds_code` drops only `"."` and empty. `99` and the
negatives are kept, because `lookup-seed.sql` gives each of them a label and nulling them
collapsed "IPEDS told us it is not classified" into "we have no value" while stranding
seed rows nothing could reference.

Checked before changing it: every value in all six categorical columns of `HD2025.csv`
has a matching lookup row, so this cannot create an orphan. A future survey year adding a
code the seeds do not carry would — that is one of the things Step 2's `db validate`
should catch.

**Effect on re-import, measured against `HD2025.csv`:** 2,537 institution fields stop
being `NULL`.

| Column | Rows | Value | Now means |
|---|---|---|---|
| `C21BASIC` | 2,262 | `-2` | Not applicable (not in the Carnegie universe) |
| `INSTSIZE` | 258 | `-2` | Not applicable |
| `SECTOR` | 17 | `99` | Not classified |

The Carnegie figure is the notable one: **38% of institutions** had their classification
nulled when IPEDS had actually told us they are outside the Carnegie universe — a
different fact from "unknown", and one that any analysis grouping by Carnegie class was
silently folding into the missing bucket.

A consequence worth knowing: the two parsers now differ **only** over negatives. Both keep
`99` — as a labelled code on one side, as ninety-nine graduates on the other.

### F8 — The Carnegie column was read from the wrong survey vintage **[measured]** — FIXED

`ingest_institutions` listed the Carnegie candidates as
`("C18BASIC", "C21BASIC", "C15BASIC", "CBASIC")`, and `find_col` returns the **first**
candidate present. HD2022 and HD2023 carry `C21BASIC`, `C18BASIC`, `C15BASIC` *and*
`CCBASIC` simultaneously — so those years imported the **2018** classification, while
HD2024/HD2025 (which carry only `C21BASIC`) imported the 2021 one. Meanwhile `schema.sql`
documents the column as "C21BASIC" and `lookup-seed.sql` titles it "Carnegie
Classification 2021 Basic".

Proven against the live backend, not inferred. Of the 6,256 institutions stamped
`updated_year = 2022`, the two columns disagree for **1,275**, and on every one of those:

| `carnegie_class` agrees with | Rows |
|---|---|
| HD2022 `C18BASIC` | **1,275 / 1,275** |
| HD2022 `C21BASIC` | 0 / 1,275 |

So the live column is 2018-vintage data under a 2021 label, for 20% of institutions.
This also inflated F2's `carnegie_class` mismatch count: part of that 3,397 was the wrong
column, not three years of reclassification.

**Fixed:** candidates reordered newest-first and the dead name corrected —
`("C21BASIC", "C18BASIC", "C15BASIC", "CCBASIC")`. `CBASIC` is not an IPEDS column in any
of HD2022-2025; the pre-2015 name is `CCBASIC`. All four share the `{-2, 1..=33}` code
space that `carnegie_class` seeds, so none can orphan. `CARNEGIE`/`C00CARNEGIE` were
deliberately **not** added as fallbacks: their code space includes 40 and 51-60, which the
lookup table has no rows for.

**The first guard written for this did not work,** and the mistake is worth recording
because it is the same shape as the defect. `hd_carnegie_candidates_are_listed_newest_first`
asserted against a *hand-written copy* of the candidate list, so reverting the production
order to the buggy one failed nothing. Column resolution has since been extracted into
`HdCols::for_hd`, mirroring `DemoCols::for_completions`, and the test now drives that
function with the real HD2022 header set (all four vintages present, `C18BASIC` placed
first so header order cannot be what makes it pass). Reverting the order now fails.

Verified against the real files as well as synthetic headers: `hd2022.csv` has all four
Carnegie columns and resolves to `C21BASIC`; `hd2025.csv` has only `C21BASIC`.

The symmetric gap on the completions side was closed at the same time —
`DemoCols::for_completions` had **no test at all**, so a transposed candidate
(`hispanic_men: col!("CHISPW")`) would have swapped two demographics across 1.2M rows.

Found in the same pass and fixed: `nonresident_alien_men` had a `CNRALT` fallback, but the
`T` suffix is the men+women **total** — latent today because `CNRALM` is present, and it
would have inflated a denominator if it ever fired.

### F9 — MCP tool descriptions gave the wrong Carnegie codes — FIXED

Nine call sites across `src/mcp/` told the model `21=R1-2021, 22=R2-2021`. Under the 2021
Basic classification that `lookup-seed.sql` implements, **21 and 22 are Baccalaureate
Colleges**; R1 and R2 are 15 and 16, and 17 is Doctoral/Professional. A model following
the tool description would have filtered for liberal-arts colleges when asked for R1
institutions, and nothing in the output would have looked wrong.

### F10 — The importer read the *provisional* IPEDS file when a revised one was present **[measured]** — FIXED

`C2022_A.zip` and `C2023_A.zip` each contain **two** CSVs: the provisional release
(`c2022_a.csv`) and the revised one (`c2022_a_rv.csv`). IPEDS publishes the provisional
first and supersedes it months later with corrections, shipping both in the same archive.
`read_file_or_zip` took the first CSV entry it found, which is the provisional.

Measured by diffing the two entries inside each archive:

| Archive | Provisional rows | Revised rows | Only in provisional | Only in revised | `CTOTALT` differs |
|---|---|---|---|---|---|
| `C2022_A.zip` | 300,877 | 301,055 | 51 | 229 | **904** |
| `C2023_A.zip` | 303,292 | 303,460 | 41 | 209 | **681** |

The stored row counts matched the *provisional* files exactly, confirming which was read.
HD files and the 2024/2025 completions archives ship a single CSV and were unaffected.

**Fixed:** `pick_csv_entry` prefers an entry whose stem ends `_rv`, matched
case-insensitively because the archives are inconsistent (`c2022_a_rv.csv` but
`C2023_a_RV.csv`). Guarded by a test that builds archives in both entry orders, so entry
order cannot be what makes it pass.

Third instance of the same shape as F8 and the `CNRALT` fallback: **several plausible
sources present, first match wins, wrong one chosen, row counts unaffected.** Worth
treating as a known hazard class in this importer rather than three separate bugs.

Note this is also why the clear-then-reimport was the right call rather than an upsert
over the top: the revised files *retract* 51 (2022) and 41 (2023) rows, and an upsert
would have left those in place.

### Step 2 — `nuanalytics db validate <ipeds-file>` *(code)* — **DONE**
Turn this audit into a command. Given a local HD or C file, compare it against the backend
and report per-column mismatch counts with examples — which is exactly the by-hand work
above, and the only way to know Steps 1 and 3 worked.

Reuse the row-limit lesson: `count=exact` for totals, paged selects for values. Report
"could not check" separately from "mismatched", the way `doctor`'s schema check already
separates missing from unreachable.

**Gate for the whole plan: MET 2026-09-23.** Built as `src/core/database/validate.rs`;
the CLI takes `db validate --year <YEAR> <FILE>` and accepts the IPEDS `.zip` directly.

    db validate --year 2025 HD2025.zip
      5,985 rows compared, 0 in file but not stored, 11/11 columns match
      (530 stored-not-in-file: institutions accumulate across survey years, not a fault)
    db validate --year 2025 C2025_A.zip
      313,566 rows in file, 313,566 in backend; 21/21 columns match across the sample

Zero mismatches on both, which also confirms Steps 1, 3 and 4 worked.

### Step 3 — Re-import, oldest year first *(operational)* — **DONE 2026-09-21**

All four years cleared and re-imported with the corrected parsers, oldest first so the
newest HD wins. Every count matches what the source files predict:

| | Before | After | Expected from files |
|---|---|---|---|
| institutions | 6,515 | **6,515** | 6,515 |
| completions 2022 | 300,877 | **301,055** | 301,055 |
| completions 2023 | 303,292 | **303,460** | 303,460 |
| completions 2024 | 307,707 | **307,707** | 307,707 |
| completions 2025 | 313,566 | **313,566** | 313,566 |
| completions total | 1,225,442 | **1,225,788** | 1,225,788 |

The +346 is the revised files' net effect (F10): 438 rows added, 92 retracted.

**The six missing source files were fetched before anything was deleted.** Only
`HD2025`/`C2025_A` were on the machine; clearing first would have destroyed ~912,000 rows
of 2022-2024 completions with no way to restore them. `nces.ed.gov/ipeds/datacenter/data/`
serves the rest; the 2025 pair is not at that path, which is presumably why they were
downloaded by hand originally.

**Cleared only the three IPEDS data tables.** `cip_codes`, the seven lookups and
`degree_types` are SQL-seeded and cannot be restored without `psql`, so they were left
alone and verified intact afterwards (2,173 / 12 / 34 / 9).

Verified after:

- `db doctor` — 8 passed, 0 failed.
- `db validate HD2025.zip` — **all 11 columns clean**, 0 rows missing. Was 10 columns and
  5,332 values before.
- `db validate C{2022,2023,2024,2025}_A.zip` — exact row counts and **all 21 columns
  clean** across every sample. The `99` values are stored as numbers now.
- `db validate HD2022.zip` — provenance reports **`C21BASIC` 6,256 of 6,256 rows agree**,
  where before the re-import `C18BASIC` led at 3,835. F8 confirmed fixed from the data
  side, not just the code.

**Independent confirmation, not just the tool agreeing with itself:**

- Every one of the 5,985 institutions in HD2025 matches the stored row on all 9 compared
  columns — **zero** mismatches, checked directly against the CSV rather than through
  `db validate`.
- All 530 institutions absent from HD2025 match the HD file of the year their
  `updated_year` claims — **530 of 530** on name. The table is exactly "HD2025 for
  everything current, last-known record for everything closed".
- Whole-table check of F1, bypassing the sample entirely: rows with `total = 99` in the
  backend versus `CTOTALT = 99` in the source file — **172/172, 166/166, 191/191,
  169/169** across the four years. 698 values that were `NULL` before. Rows with
  `total IS NULL` are 0 on both sides, so nothing was over-corrected.

That last run still reports 9 columns differing on HD2022, which is **correct**: the
`institutions` table has one row per institution, not one per year, so it holds HD2025
values and a 2022 file legitimately disagrees. `carnegie_class` matches because the 2021
classification is published once and does not drift. Worth knowing before reading an
older-year validate run as a fault.

**[verified] `db validate` hits a transient `error decoding response body`** roughly one
run in five against this backend — a truncated response body over the Cloudflare tunnel,
not year-specific (2025 failed then passed; 2024 failed then passed three times). Same
class as the single unexplained `db doctor` failure seen on 2026-09-18. It fails loudly
and a retry succeeds, so it is a nuisance rather than a correctness risk, but
`fetch_completions` has no retry where `send_get` has one for a 401. Worth adding.

### Step 4 — Stop an older year from overwriting a newer one *(code)* — **DONE 2026-09-21**

`ipeds::downgrade_refusal(importing, newest_stored, force)` reads `updated_year` — which
had been written on every row since the beginning and never once read — and refuses an HD
import older than what is already stored, unless `--force`.

Guards only the HD half. Completions carry `year` in their natural key, so years cannot
overwrite one another; institutions have one row per `unitid` and no year dimension, so
the last import wins outright. That asymmetry is what made F2 possible.

Verified live against the freshly loaded backend:

    $ nuanalytics db ipeds-import --institutions HD2022.zip --year 2022
    ✗ refusing to import HD2022: the institutions table already holds data from 2025,
      and this import would overwrite every shared institution with the older year's
      values. Import oldest-year-first, or pass --force if that is what you want.
    exit 1

    $ nuanalytics db ipeds-import --institutions HD2025.zip --year 2025
      ✓ 5985 read, 5985 upserted, 0 skipped      # same or newer is never blocked

The check runs **before** the file is read, so a refusal costs no decompression. An empty
table never refuses, and a probe failure warns and proceeds rather than blocking an import
on a transient read.

### F11 — `db remetric` is blocked on the analysis-pipeline split **[measured]** — NOT BUILT

An in-place metric backfill needs something that analyses a degree and returns its
metrics. There is exactly one such entry point, `build_artifacts`, and it lives in
`src/mcp/tools/analyze.rs` — gated behind the `mcp` feature. The CLI's only entry points,
`run_analyze` and `run_analyze_from_db`, write files and print; they return nothing.

So `remetric` would have to compute through the **MCP** pipeline, which still treats an
OR-group as an AND (`analyze.rs:1370`, adds an edge per in-plan option instead of one).
The corpus was produced by the **CLI** pipeline, and the two disagree — measured 19% apart
on median complexity for the same degree in `clean-up-analysis-todo.md`. A `remetric`
built on it would recompute `complexity_mean`, find it does not match the stored value,
and correctly refuse every run. It would be a command that never succeeds.

Making it "work" would mean dropping the verification step, which is the one thing that
stops it silently overwriting a historical record with numbers from a different pipeline.

**So it is deliberately not built**, and re-import is the supported path instead:

| | re-import | remetric |
|---|---|---|
| corpus (files on disk) | analyse + import | blocked |
| database-only programs | `degree analyze --from-db` | blocked |
| history | appends a run | would patch in place |

Re-import also uses the pipeline that produced the existing numbers, so results stay
comparable — which `remetric` on the MCP path would not.

**This reframes `clean-up-analysis-todo.md`.** That plan reads as a tidiness exercise; it
is also the blocker for in-place metric backfill. `remetric` becomes buildable after its
Step 1 (fix the OR-group DAG) and Step 3 (extract a shared `core::analysis`), at which
point there is one pipeline, one set of numbers, and verification can pass.

### Step 5 — Correct the documentation *(docs)* — **DONE 2026-09-23**
Four places claimed a CS-only or `MAJORNUM=1`-only table. All now state what exists — all
CIP codes, both major numbers, ~313,000 rows per year — and say why the magnitude matters
(it is what makes a `PGRST_DB_MAX_ROWS` cap bite):

- `src/cli/args.rs` — `db ipeds-import` help text ("CS CIP codes only")
- `docs/database/setup.md` — troubleshooting row for `21000 ON CONFLICT affects row twice`,
  which claimed the fix was filtering to `MAJORNUM=1`; the actual fix was adding
  `major_num` to the ON CONFLICT target
- `docs/database/ipeds-data.md` — "After filtering to CS CIP codes: ~15,000–20,000 rows",
  and a sample import transcript showing "18741 matched CS CIP codes" against output the
  importer no longer produces
- `src/core/database/mod.rs` — the `COMPLETIONS` table constant's doc comment

`CHANGELOG.md` turned out to contain no such claim.

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

### Step 8 — Carry `chain_length` through to where it is consumed *(code)* — **DONE**
Three separate gaps, smallest first:
1. **Degree-level report.** — **DONE.** `unified_report.rs` now emits `avg_chain_length`.
   `min_chain_length` was deliberately *not* added: at degree level it was degenerate
   (always 1.0, since some course in any plan has no prerequisites). Per-course min/max/avg
   is the useful form and the aggregator already carries it.
2. **Database column.** — **DONE in the schema file.** `programs-schema.sql:265` has
   `chain_length_mean REAL` and `chain_length` is listed in the `metrics` JSONB comment.
   *Not verified against the live instance* — `db doctor` checks tables, not columns, and
   these tables are still empty (F5), so nothing has exercised it. Confirm when Step 7 is
   decided and the corpus is loaded.
3. **The MCP `None` path** — **resolved: deliberate.** It is the parse-failure response
   constructor, where `plans_analyzed` is 0 and `complexity`, `longest_delay` and
   `total_credits` are all `None` too. `avg_chain_length: None` is consistent, not a gap.

### Step 9 — Re-analyse the corpus *(operational, after Step 8)* — **DONE**
Both variants regenerated into `full_degree/v2/` and `trimmed_degree/v2/` — 1,088 degrees
each, `max_plans = 10000`, seeded per document, `analyzer_version 0.5.4`. The reports carry
`avg_chain_length` and an `analysis.parameters` block recording seed, caps and strategy, so
a run is reproducible from the file alone.

**Still valid after the 2026-09-23 OR-group commit.** That commit changed the MCP DAG to
match the CLI; v2 came from the CLI path, which was already correct. Re-verified: Bowdoin
through the CLI gives the same complexity 87/57/112, delay 5/4/6, chain 2.4319/1.8889/2.9
before and after. The one thing that *would* differ on a re-run is the *drawn* graph in an
HTML report, because the same commit changed equivalence resolution there from "first hash
hit" to "lexicographic minimum" — a determinism fix. Metrics are unaffected.

Original note: the 1,088 degrees were analysed 2026-06-09 with a binary that had no
`chain_length`.
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
