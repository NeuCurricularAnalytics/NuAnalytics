# Take the CIP out of `program_key`, then make CIPs correctable

Written 2026-10-25. Status: **planned, nothing started.**

## Why

`program_key` is a program's identity, and for the institution form it embeds the CIP:

    prog:<unitid or ref>|<cip>|<catalog_year>|<degree_type>|<degree_id or name>[|v<date>]

But most stored CIPs are guesses. Measured 2026-10-25 on the live backend: 1,487 of 1,963
programs (76%) carry a `conversion_warnings` entry saying the CIP was "inferred from program
name" or is "the supplied guess, not stated". 11.0701 alone covers 1,076 programs.

Correcting a guessed CIP is therefore impossible without breaking identity:

- **Re-import under the right CIP creates a second program.** New CIP, new key, so
  `ON CONFLICT` inserts; the old row and its courses, requirements and runs stay behind
  (`program_key` is linked by convention, not FK, so nothing cascades).
- **`db import --cip X` does not reach the pulled degree** (a bug, independent of the key).
  The override goes into the column and the key (`import.rs` `base_program_key`, and the
  `StoredProgram` build), but `document` is `to_unified_value(&program)` of the parsed
  degree, which still has the old CIP. `degree analyze --from-db` and `get_degree` read
  `document`, so they return the old value.
- **A hand `UPDATE`** can fix column + document, but leaves the old CIP in the key and a
  stale `document_hash`, so the next import of that file treats it as changed.

## What was checked (2026-10-25)

- **Only one function builds the key** (`base_program_key`, `src/core/database/import.rs`).
  Everything else in this repo treats it as opaque: equality lookups, `ORDER BY` in
  `catalog/institutions_with_programs.sql`. Nothing splits it or reads the CIP out of it.
- **`run_key` is random** (`new_run_id`), not derived from the program key.
- **No collisions.** All 1,963 stored programs are institution-form keys; with the CIP
  segment dropped, all 1,963 stay distinct (the trailing `degree_id or name` already
  separates them). The id-only form `prog:<id>|<cat>` and the `fp:` form never had a CIP.
- **Dependent rows** (no FK, no cascade):

  | table | rows | unique constraint involving the key |
  |---|---|---|
  | programs | 1,963 | `program_key` |
  | program_courses | 77,058 | `(program_key, course_code)` |
  | program_requirements | 20,561 | `(program_key, req_path)` |
  | analysis_runs | 11,831 | — (`run_key`) |
  | analysis_plans | 74,185 | — (`run_key, plan_index`) |
  | analysis_course_metrics | 413,864 | — (`run_key, course_code`) |

- **Outside this repo, the nightly-sync checkout (sibling directory) stores keys.**
  `state/queue.csv`: 1,105 rows with a `program_key`, 1,101 not `done`;
  `nightly/runner.py` `_stored_document` fetches `programs.document` by that exact key, so
  stale keys fail with "stored program … not found". The same 1,105 rows have ids
  `revisit:prog:…` that embed the key (`nightly/lists.py`), so a key change makes them look
  new and drops `attempts`/`last_checked`. Its dated `reports/*.md` are history; leave them.
  Its code treats the key as opaque (`importer.py` `_program_key` only scans output for a
  `prog:`/`fp:` token).

## Decisions (all settled 2026-10-25)

1. **Version — decided 2026-10-25: 0.7.0.** Commit the version-date / intl work (0.6.1) on
   `feature/version-date-intl` first, then branch for this.

   What actually breaks for a 0.6.x binary against a migrated database: reads all keep
   working (listing, `db query`, metrics, lookup by `degree_id`, `--from-db` with a key the
   database returned — the key is opaque). Keys saved before the migration stop
   resolving, on any binary. The one real break is **import of an already-stored
   degree**: the old binary computes the CIP key, finds no row, and inserts a duplicate
   program with its own courses, requirements and run — no error. `--replace` and the
   `verified` gate are bypassed, and 0.6.1's newest-version rule makes a pull by
   `degree_id` return the duplicate. The reverse (new binary, unmigrated database) forks
   every re-imported program the same way, so binary and migration move together.

   The CLI surface does not change; what changes is the stored key's meaning and that old
   importers must not run against a migrated database. Pre-1.0 semver puts that in the
   minor version, and the 0.6.1 entry promises compatibility with 0.6.0 clients, so this
   is 0.7.0. The phase 2 constraint turns the silent fork into a refused write, so the
   CHANGELOG entry can say exactly what an old binary does: reads work, imports are refused.
2. **Collision semantics.** After the change, two programs at one school, same catalog
   year, type and id/name, but different CIPs, are one program. None exist today; that is
   the intended meaning (CIP is an attribute, not identity). The test
   `two_programs_sharing_cip_year_and_type_get_different_keys` keeps its point (the
   discriminator), but re-check its wording.
3. **What a CIP correction is keyed on — decided 2026-10-25: `(institution_ref,
   degree_id)`.** Corrections arrive one school at a time, as the user works with each,
   and a school's answer is about a *program*, not one catalog year. Measured that day:
   `degree_id` is never NULL, unique within a school (1,961 school/id pairs, none shared
   across schools), and stays the same across catalog years and dated versions (Wayne
   State's CS BS: one id, 2025-2026 and 2026-2027 rows). It is also what
   `programs_latest` groups on.
   - `program_key` is too narrow: it names one year/version, so next year's import (or
     a nightly `|v<date>` version) would come in with the guess again.
   - School + name is ambiguous: 4 schools have two programs named "Bachelor of Science
     in Computer Science" with different ids.
   - Risk: `degree_id` is often a slug of the name, so a rebuild that renames the program
     gets a new id and loses the correction. `db cip list` / `db doctor` report
     corrections whose `(institution_ref, degree_id)` matches no stored program (3.5).
   - Record provenance with each correction (where it came from, who, when), and mark
     programs whose CIP is confirmed rather than guessed, so analyses can filter on it.
     Without that, a correct CIP and a guess look identical in `programs.cip_code`.

## Phase 1 — code (this repo)

1. `base_program_key`: drop `{cip}` from the institution form →
   `prog:{inst}|{cat}|{dtype}|{discriminator}`. Remove the now-unused `cip` local. Keep the
   long comment about the discriminator; add one sentence on why the CIP is not in it
   (mostly guessed; must be correctable without changing identity).
2. Fix the `--cip` bug: apply `opts.cip_code` to the degree **before** `document` and
   `document_hash` are built, so column, document and hash agree. Add a test that imports
   with `--cip` and asserts `document.degree.cip_code`.

   The same defect covers `--institution-ref` and `--unitid` (found in the 0.6.1 review,
   2026-10-26, deliberately left out of 0.6.1): the option keys the row but the stored
   document keeps the file's value, so re-importing the stored document without the flag
   resolves the school differently and gets another key. Fix all the identity overrides
   in one place — CIP, catalog year, degree id, unitid / institution_ref (setting one
   clears the other) — and note that rows imported with an override will hash differently
   on their next import (reported `updated`, harmless).
3. Update the key-format tests in `import.rs` (about 7 asserts: the `167358|11.0701|…`
   and `ca:university-of-toronto|11.0701|…` strings, plain and `|v<date>`). Fixture keys
   elsewhere (`degree.rs`, `degrees.rs`, `mcp/tools/import.rs`) are opaque test data —
   update the ones that look like real keys so they don't teach the old format.
4. Docs: `programs-schema.sql` key comment and the `program_key_has_no_cip` constraint
   (see phase 2), `docs/database/setup.md` (example around the `program_key:` line and
   the key-form table), `docs/degree.md` examples.
5. Version 0.7.0: bump `Cargo.toml` (and `Cargo.lock`) and `Readme.md` where it names the
   version, and add a `## [0.7.0]` CHANGELOG entry: **Changed** — the CIP is no longer
   part of `program_key`, the run-once migration file, and that 0.6.x binaries can still
   read a migrated database but their imports are refused by the constraint;
   **Fixed** — `db import --cip` now reaches the stored document.
6. `/check-rs`; `cargo test --all-features` and
   `cargo test --no-default-features --features database` (in a cgroup).

## Phase 2 — migration (user runs it; `psql` on the server, one transaction)

Put the file in `docs/database/historical/` — **not** `docs/database/`, whose `*.sql` must
match `bootstrap::SCHEMA_FILES` (a test diffs them). Fresh installs never need it.

Re-key only rows whose key is provably the institution form, built from the row's own
columns — a blind regex would also strip the catalog year from `prog:<id>|<cat>|v<date>`.

```sql
BEGIN;
SET LOCAL statement_timeout = 0;   -- ~600k rows; don't let a role default cut it off
CREATE TEMP TABLE rekey ON COMMIT DROP AS
SELECT program_key AS old_key,
       'prog:' || coalesce(unitid::text, institution_ref) || '|' ||
       substr(program_key,
              length('prog:' || coalesce(unitid::text, institution_ref) || '|' ||
                     coalesce(cip_code, '') || '|') + 1) AS new_key
  FROM programs
 WHERE program_key LIKE 'prog:' || coalesce(unitid::text, institution_ref) || '|' ||
                        coalesce(cip_code, '') || '|%';
-- expect 1,963 (all rows today); stop if not
SELECT count(*), count(DISTINCT new_key) FROM rekey;

UPDATE programs                p SET program_key = r.new_key FROM rekey r WHERE p.program_key = r.old_key;
UPDATE program_courses         t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE program_requirements    t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_runs           t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_plans          t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;
UPDATE analysis_course_metrics t SET program_key = r.new_key FROM rekey r WHERE t.program_key = r.old_key;

-- checks before COMMIT: no child row of any of the five tables points at a missing
-- program (repeat this for program_courses, program_requirements, analysis_plans,
-- analysis_course_metrics); all must be 0
SELECT count(*) FROM analysis_runs r WHERE NOT EXISTS (SELECT 1 FROM programs p WHERE p.program_key = r.program_key);

-- Export the mapping NOW: the temp table is dropped at COMMIT. Phase 2b needs it.
\copy rekey TO 'program-key-rekey.csv' CSV HEADER

-- Refuse old-form keys from now on, so a 0.6.x import fails loudly instead of inserting
-- a duplicate. Only rows with a CIP are checked: with an empty CIP the old pattern is
-- `prog:<inst>||%`, which a NEW key also produces when catalog_year is blank (49
-- programs have a blank catalog year today) — checking those would refuse valid rows.
-- A non-empty CIP cannot match a new key unless it equals the catalog year.
-- The cost: an old binary importing a program with no CIP is not caught (1 row today).
ALTER TABLE programs ADD CONSTRAINT program_key_has_no_cip CHECK (
  coalesce(cip_code, '') = ''
  OR program_key NOT LIKE 'prog:' || coalesce(unitid::text, institution_ref) || '|'
                          || cip_code || '|%');
COMMIT;
```

**Review fixes (2026-10-25), recorded so they are not undone:**

- The first draft of the constraint checked empty CIPs too, and claimed a new key could
  never contain `||`. It can: `prog:<inst>||BS|…` when the catalog year is blank (e.g.
  today's `prog:476975|11.0101||BS|…` becomes `prog:476975||BS|…`). Combined with a blank
  CIP, the constraint would have refused a valid new-binary import.
- The first draft exported the mapping after `COMMIT`, by which time `ON COMMIT DROP` had
  removed it.
- `analysis_runs.config_fingerprint` hashes the program key in
  (`import.rs`, `config_fingerprint`). It is advisory (schema comment: "NOT the
  identity"), and nothing in this repo or nightly-sync looks runs up by it, so stale
  fingerprints are left as they are: the only effect is that a re-run of an identical
  configuration is not recognised as a duplicate of a pre-migration run — it costs a
  redundant row, which is the failure mode that column was designed for. Do not try to
  recompute it in SQL.

The constraint also belongs in `programs-schema.sql` (phase 1), so fresh installs get it;
add it there idempotently (`DROP CONSTRAINT IF EXISTS` then `ADD`), after the
`ALTER TABLE … ADD COLUMN` block. Add a test, if one is practical against the schema
text, that a key built by `program_key` never matches the pattern. Confirm on the server
that an import by the old binary is refused with a readable error (PostgREST returns 400
with code `23514`), and record the wording it prints in the CHANGELOG entry.

Verify the `cip_code` column equals the CIP inside each key before trusting the `LIKE`
guard (checked 2026-10-25 only that column = document; the key itself is built from the
same `opts.cip_code.or(degree.cip_code)` as the column, so it should hold — the
`expect 1,963` count is the check).

`analysis_plans` and `analysis_course_metrics` both carry `program_key` (confirmed in
`programs-schema.sql`). Before writing the final file, grep the schema once more for any
other `program_key` column or view (`programs_latest` selects `p.*`, so it follows).

### Phase 2b — nightly-sync, same window

**Data only; no nightly code change is needed for the re-key.** Checked 2026-10-25: its
code never builds or parses a key (`importer.py` `_program_key` only scans import output
for a `prog:`/`fp:` token; `runner.py` and `lists.py` pass keys through). What must move:

- `state/queue.csv` — the `program_key` column and the `revisit:<key>` ids (1,105 rows).
  Rewrite through the exported mapping; small Python script, run with `-I`, dry-run diff
  first. `lists/revisit.csv` carries the same ids — rewrite it too, or regenerate it.
- The `nuanalytics` binary it calls (`~/.cargo/bin`) — the 0.7.0 install in step 1 of the
  cutover.

Leave alone: `reports/*.md` and the 875 `corpus/**/import.log` files (history), and the
`tests/test_core.py` fixture id (opaque test data).

### WebScrappedCombinedDataMetrics — unaffected by the re-key

It stores no program keys (checked 2026-10-25). It matters only to phase 3: its degree
JSONs carry the CIP guesses (its README counts 660 programs in 11.0701), so re-importing
one would reapply its guess over a correction.

### Cutover order

1. Pause the nightly job — **before** installing anything: a 0.7.0 binary importing into
   an unmigrated database forks programs just as an old one does into a migrated one.
2. Phase 1 merged; install the 0.7.0 binary in `~/.cargo/bin` (nightly-sync calls it).
   Restart any running `nuanalytics mcp` servers (Claude sessions keep the old process;
   one started with `--allow-writes` can import).
3. Phase 2 transaction (mapping exported inside it).
4. Phase 2b.
5. Smoke: `db doctor`; `degree analyze --from-db <new key>`; re-import one corpus degree
   and confirm it **updates** the existing row (same key, no new row); try one import
   with a 0.6.x binary and confirm the constraint refuses it.
6. Resume nightly.

## Phase 3 — correcting CIPs (the original goal)

**Release:** phase 3 is additive — a new table, new commands — so it ships after the
cutover as 0.7.1, following the 0.6.1 precedent (`institutions_intl` was additive too).
Its CHANGELOG entry says to apply `programs-schema.sql` first: `db doctor`'s schema check
requires every table in `tables::ALL`, so it fails on a database without `program_cips`.

The user's workflow: CIPs start as guesses; as the user works with each school, the
school gives the real CIPs, and those must stick — through re-imports, new catalog years
and nightly versions — and must be usable to line a program up with its IPEDS
demographics, **including programs not built yet**.

### 3.1 The confirmed-CIP registry: `program_cips`

One table, in `programs-schema.sql`, identifying a program by `(institution_ref,
degree_id)` (decision 3), with **one row per CIP the program has had** (3.1a). It does
**not** reference `programs` (the repo's FK-free convention), so a row can exist before,
after, or without the program it names.

    institution_ref  TEXT NOT NULL      -- unitid-as-text, or 'cc:slug' for non-US
    degree_id        TEXT NOT NULL
    from_year        INTEGER            -- start of the academic year the CIP applies from
                                        -- (2024 = 2024-2025); NULL = all years
    unitid           INTEGER            -- NULL for non-US; what demographics join on
    cip_code         TEXT NOT NULL      -- dot notation, must exist in cip_codes (checked by the CLI)
    award_level      INTEGER            -- IPEDS level (5 = bachelor's …); derived from degree_type when omitted
    name             TEXT NOT NULL      -- the program as the school names it
    degree_type      TEXT
    source           TEXT NOT NULL      -- where the CIP came from: "registrar email 2026-11-02", a URL, …
    note             TEXT
    created_by       UUID DEFAULT auth.uid()   -- attribution only, as on programs
    created_at / updated_at
    UNIQUE (institution_ref, degree_id, coalesce(from_year, 0))   -- a unique index; NULL can't sit in a PK

### 3.1a CIP history — schools recode programs

Schools do change a program's CIP in some years, usually from a year forward. So a
program has a history of rows, and each row applies **from its `from_year` until the next
row's**:

- **No `from_year` (NULL): all years.** The common case — one row, applies to everything.
  In a history it means "everything before the first dated row".
- **A dated row:** applies from that academic year forward, until a later dated row
  takes over.
- **Resolving a year Y:** the row with the greatest `from_year ≤ Y`, NULL counting as
  earliest. If every row starts after Y and there is no NULL row, that year has **no
  confirmed CIP** — fall back to the guess and say so. Not the earliest row: a school
  saying "11.0102 from 2024-2025" says nothing about 2022-2023.

Example: `NULL → 11.0701`, `2024 → 11.0102` means 11.0701 through 2023-2024 and 11.0102
from 2024-2025 on.

**One year axis, academic years.** `from_year` is the academic year's start (2024 =
2024-2025). The two things resolved against it count years differently, and each converts:

- **A stored program** uses the first four-digit year in its `catalog_year`. Stored values
  today (2026-10-25): mostly `2026-2027` / `2025-2026` / `2024-2025`, but also 49 blank,
  two-year spans (`2026-2028`), bare years (`2025`) and `Fall 2026`. A span resolves by its
  first year. A blank or unparseable `catalog_year` uses the `version_date`'s academic
  year when there is one (July onward starts a new year), else the **latest** row (the
  current CIP); either way with a warning in the document metadata. `Spring/Summer 2027` belongs to 2026-2027
  but parses as 2027 — rare enough to warn about rather than special-case.
- **IPEDS** survey year Y (`completions.year`, from `C{Y}_A`) counts awards from July Y-1
  to June Y, i.e. academic year Y-1. Resolve it with `from_year ≤ Y-1`. Confirm that
  convention against NCES's file description before relying on it, and pin it with a
  test, because getting it backwards shifts every recode by a year.

The CLI takes `--from 2024-2025` (or `--all-years`). It refuses a bare `--from 2024`,
which is ambiguous between the 2024-2025 catalog year and IPEDS 2024 (= 2023-2024).

RLS like the other program tables (read and write for any authenticated member — the
settled decision in `database-audit-todo.md`). Add `tables::PROGRAM_CIPS` to
`src/core/database/mod.rs` and to `tables::ALL`, which `db doctor`'s schema check reads
(its test fails if a constant is missing from `ALL`).

### 3.2 A CIP for a degree that is not in the database

The registry row **is** the "empty degree" — no placeholder goes into `programs`. A stub
there would be wrong in many places at once: `document` is `NOT NULL` and every reader
parses it, `programs_latest` / `db query degrees` / `schools --with-programs` would list
it, the analysis tools would try to analyze it, and `db doctor`'s counts would include it.
Keeping it out of `programs` means nothing downstream has to learn a new "empty" case.

So a registered program (all its history rows together) is in one of two states,
derived by a join and never stored:

- **linked** — a `programs` row has the same `(institution_ref, degree_id)`;
- **unbuilt** — none does (yet).

When the degree is built later it links automatically, **provided it is imported under
the same `degree_id`**. That is the one thing to get right:

- When registering an unbuilt program, the CLI proposes the id the builders would
  produce (the slug convention: `<school-slug>-<program-slug>`) and prints it.
- `db import` already accepts `--degree-id`; nightly can pass it. Optional nightly change:
  turn unbuilt registry rows into build items (it already generates items per school and
  CIP in `lists.py`), so a confirmed program is queued with its id fixed in advance.
- `db cip relink <school> <old-id> <new-id>` for when a build arrives under another id.

### 3.3 Applying it

- **On import:** `db import` looks up `(institution_ref, degree_id)` in `program_cips` and
  resolves the row for the degree's catalog year (3.1a); a hit overrides the degree's
  CIP **before** `document` and `document_hash` are built (the same place as the phase 1
  `--cip` fix), so a re-import of an unchanged source hashes the same and is reported
  unchanged rather than rewritten. An explicit `--cip` still wins over the registry (the
  caller said so), with a warning when they disagree. It records the fact in the document's
  `conversion_warnings`-style metadata ("cip_code 11.0701 confirmed: <source>"). Every
  importer — CLI, MCP, nightly, a WebScrapped JSON — honours it with no change to them.
- **On the stored rows:** adding or changing a registry row re-resolves every *existing*
  program with that pair (each catalog year and version against its own year) and updates
  in place the ones whose CIP changes: `cip_code` column,
  `document.degree.cip_code`, `document_hash`. Never the key — phase 1 is what makes that
  safe. The stored runs are left alone: the CIP is not an analysis input, so a CIP-only
  change does not make a run stale. (For `full` runs `analyzed_document_hash` then no
  longer equals the program's `document_hash`; nothing compares the two today — keep it
  that way, or have such a comparison ignore the CIP.)
- **Confirmed vs guessed is queryable:** a stored program's CIP is confirmed exactly when
  a registry row *resolves for its catalog year* (3.1a) — a program registered only from
  2024-2025 has its 2023-2024 row still guessed. Surface it as `cip_confirmed` in `db query degrees` and the
  MCP degree summaries; no extra column on `programs` to keep in sync.

### 3.4 Demographics — the reason this matters

IPEDS completions are keyed `(unitid, cip_code, award_level, major_num, year)`; there is
no program in them. The registry is the bridge: `(school, degree_id) → (unitid, cip,
award_level)`.

- `db query demographics --school <unitid> --degree <degree_id>` (and the MCP
  equivalent): resolve the CIP from `program_cips` if present, else from the stored
  program's guess (the stored row for that academic year, else the latest), and **say
  which** — every result carries `cip_source: "confirmed" |
  "guessed"`. Works for unbuilt programs, since only the registry is needed.
- **A recode splits the series.** Each IPEDS year is resolved on its own (3.1a), so a
  program recoded in 2024-2025 reads 11.0701 for IPEDS 2022–2024 and 11.0102 from IPEDS
  2025. Every row of the result carries the CIP it used, and the result says where the
  CIP changed, so a jump in the numbers isn't mistaken for a change in enrolment. A year
  with no confirmed CIP falls back to the guess, marked `guessed` on that row only.
- **Many programs, one CIP.** A BS and a BA in CS are often both filed under 11.0701 at
  award level 5; IPEDS counts them together and cannot split them. When other programs
  at the school share the `(cip, award_level)`, the result lists them, so the numbers are
  never presented as one program's own.
- **Empty result.** When the confirmed CIP has no completions at that school,
  `completions_by_cip.sql` already returns `nearby_cips_with_data`; pass it through — a
  confirmed CIP with no IPEDS rows is worth flagging back to the school.
- **Award level is not always derivable.** BS/BA → 5 and MS → 7 are safe; certificates
  span IPEDS levels 1, 2, 4, 6 and 8, so `CERT` needs an explicit `--award-level`.
  `degree_types.level` is only undergraduate/graduate and cannot supply it.
- **Minors, concentrations and tracks are not IPEDS awards.** A minor has no completions
  line at all; a concentration or track is counted under its parent major's CIP. The
  answer says so, rather than returning an empty table or the parent's numbers as the
  track's own.
- **Major number.** IPEDS counts first and second majors separately (`major_num` 1/2).
  Use the same default as the existing `demographics` query and state it in the output.
- Non-US schools have no IPEDS data; the registry still holds their CIPs, and the
  demographics answer says "no IPEDS coverage" rather than "no rows".

### 3.5 Commands

- `db cip set <school> <degree_id> <cip> --source <text> [--from 2024-2025 | --all-years]
  [--name … --degree-type … --award-level …]` — one correction; `--name` required when
  the program is unbuilt. Without `--from`, all years. Setting a row for a `from_year`
  that already has one replaces it; other years' rows are untouched.
- `db cip import <csv>` — the same, in bulk (columns as the table, `from_year` blank =
  all years); dry run by default, prints linked / unbuilt / changed per row.
- `db cip list [--school <unitid>] [--unbuilt]` — the registry with state; a program
  with a history prints it as a timeline (`all years → 11.0701`, `2024-2025 → 11.0102`).
- `db cip relink` (3.2) and `db cip remove`.
- `db doctor`: report registry rows still unbuilt (informational — expected for
  programs not yet built) and rows whose school has stored programs but none with that id
  (likely a rebuild under a new id). Any new doctor check must also go into the three
  `*_DEPENDENTS` arrays in `doctor.rs`, or it vanishes from the report when an earlier
  check fails (`every_early_exit_lists_the_same_checks_as_a_full_run` catches it).
- MCP: read-only — `cip_confirmed` and `cip_source` in existing outputs. Writes stay CLI
  (or behind `--allow-writes` if wanted later).

### 3.6 Out of scope for now

- Writing confirmed CIPs back into the WebScrappedCombinedDataMetrics source JSONs —
  separate, opt-in; the import-time override already makes it unnecessary for correctness.
