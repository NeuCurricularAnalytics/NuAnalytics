# Changelog

All notable changes to NuAnalytics are recorded here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project uses semantic versioning.

## [0.6.0] — 2026-10-03

The MCP server is redesigned: fewer, broader tools; SQL access in layers; stored analysis
reachable; skills rewritten. **Breaking for MCP clients**: tools and parameters are renamed
or removed, as below. The CLI breaks in two small places — `degree analyze --metrics-out`'s
file and `--no-csv` — both below. **Analysis figures move** for degrees with an OR between
groups of courses, or with a course that could stand in for its own prerequisite (Fixed,
below); every other degree analyzes exactly as before, and the stored corpus has been
re-imported to match.

### Breaking — MCP tools

| Was | Now |
|---|---|
| `get_degree_schema`, `get_degree_json_schema` | `get_reference(topic="degree-yaml" \| "degree-json-schema" \| "database")` |
| `generate_degree_report` | `render_degree_report` |
| `get_institution` | `search_institutions(unitid=…)` returns the full record |
| `get_institution_completions`, `get_schools_completion_demographics` | `get_completion_demographics(group_by="cip" \| "school")` |
| `cache_yaml` | none: inline `content` is cached by every degree tool, and its handle returned as `source.handle` |
| `degree_pipeline` | none: call `validate_degree`, `audit_degree` and `analyze_degree` in parallel |
| `get_curriculum_visualization` | `render_plan_graph` |
| `scaffold_degree_yaml` | none: it read the legacy `degrees` table |
| `import_degree` | unchanged, but served only with `nuanalytics mcp --allow-writes` |

| Parameter was | Now |
|---|---|
| `yaml_content` | `content` |
| `yaml_path` | `path` |
| `degree_id` (on degree tools) | `degree`, which also takes `sample:<key>`, `cache:<hash>` and a stored `program_key` |
| `trimmed_cache_id` (in `trim_degree`'s response) | `trimmed_degree` |
| `analyze_degree(include_graph_spec, plan_indices)` | removed; `render_plan_graph` draws a plan |
| `compare_degrees(degree_ids, include_metrics)` | `sources: [{label?, degree \| content \| path}]`, with `metrics: stored \| fresh \| none` |

### Breaking — demographics fields (MCP and `db query demographics`)

The baseline a representation ratio compares against is completions in every field at the
same schools. It was named as if it were enrollment; there is no enrollment data.

| Was | Now |
|---|---|
| `enrolled` | `baseline_completions` |
| `total_enrolled` | `baseline_total` |
| `enrollment_pct` | `baseline_pct` |
| `school_pct` (in `group_by=cip` rows) | `baseline_pct` |

A failed call now returns `isError: true` and one envelope,
`{"error": {"code", "message", "next_steps"?, "details"?}}`. An argument a tool does not
take is refused by name instead of ignored.

### Breaking — `degree analyze --metrics-out`

The file is now the degree's report JSON — the `<degree>_report.json` a normal run writes —
with the target course's statistics under `analysis.target_course_stats`. It was the MCP
`analyze_degree` response. `--target-course`'s stdout is unchanged in shape.

### Breaking — `degree analyze --no-csv`

`--no-csv` now skips only the CSV files: the plan CSVs and the `index.csv` row. The report
JSON — the file `db import` reads — and the summary JSONL are written regardless; before,
`--no-csv` skipped them too. The new `--no-metrics` skips every metrics file, a
`--school` roll-up included, so a run that writes no output files is
`--no-metrics --no-report`.

### Added

- **`"{[A, B], C}"` — a choice of course groups — in an `all` list.** Documented since
  schema v5.1 but never parsed: validation reported `[A` as a missing course. The parser
  now expands it into the equivalent `one_of`, one option per group. Supported in the
  course list of an `all` requirement, top-level or inside a `one_of` option; elsewhere
  validation names it instead of reporting a missing course. No stored degree used it:
  all 1,088 stored documents, every fixture and sample parse and serialize byte-identically
  before and after.
- `degree validate --allow-unmatched-patterns`, as the MCP `validate_degree` already had.
- **`get_stored_analysis`**: a stored program's newest run per variant, with its plans and
  course metrics on request. **`render_stored_report`**: the same HTML as `db report`,
  from the stored run.
- **`query_sql`**: one read-only statement, inputs bound through `$1` via the new
  `query_readonly_params` database function, capped at 2,000 rows and 30 seconds.
- `get_reference(topic="database")`: the tables, columns and joins, generated from the
  compiled-in schema files.
- `search_degrees(name=…)` and `db query degrees --name`.
- `output_path` / `output_dir` on the tools that render, refusing to replace a file unless
  `overwrite=true`. `nuanalytics mcp --list-tools`.
- `db doctor` checks that the two query functions are installed.

### Changed

- **`--calc-strategy` / `calc_strategy` are described as what they are:** recorded in each
  run's parameters, read by nothing else. Every report carries both the median and the
  mean; the help text and docs had promised it chose between them.

- **A stored program means its stored run** — breaking for MCP clients that pass a
  `program_key` to the analysis tools. `analyze_degree`, `render_degree_report`,
  `render_plan_graph` and `get_course_detail` read a stored program's newest stored run
  (`variant`, default `full`) instead of enumerating its document afresh; the response's
  `source.run` names the run. `fresh=true` enumerates it afresh, and is required for
  `max_plans`, `include_courses`, `random_seed`, `analysis_timeout_seconds` and
  `target_course`, which are refused for a stored program without it rather than
  ignored. A program with no run of that variant is `source_not_found`, never a silent
  fresh run. `render_degree_report` on a stored program renders the page
  `render_stored_report` and `db report` do. `compare_degrees` without `metrics` now
  reads each stored program's stored run and enumerates the other sources; each entry's
  `metrics_from` says which. Files, inline content and samples are analyzed fresh, as
  before.
- **One analysis pipeline.** The MCP's `analyze_degree`, `render_degree_report`,
  `render_plan_graph`, `get_course_detail` and fresh `compare_degrees` metrics run the
  pipeline `degree analyze` runs (`core::degree::analysis`), so at equal settings their
  figures equal the CLI's — and so the stored runs', which the CLI produced. The MCP had
  its own copy, which built courses, equivalences and prerequisite expansion differently;
  its complexity means differed from the CLI's by up to 2.5% on the test fixtures. The
  CLI's output is unchanged. The MCP's default seed now follows the degree rather than its
  text, so reformatting a file no longer changes which plans are sampled, and `seed_used`
  differs from before.
- `degree analyze --target-course` uses the CLI's configuration (`max_plans`, sampling
  strategy) instead of the MCP's defaults, and works with `--from-db` and in builds
  without the `mcp` feature. `--from-db` prints its "Loaded stored program" line on
  stderr, so stdout carries only the answer.
- **Completion demographics run as one SQL query each**, from compiled-in `.sql` files,
  instead of up to 252 PostgREST calls. They no longer miss the latest year, cap at 5,000
  institutions, or drop a failed batch silently.
- `nuanalytics init` ships five rewritten skills (degree-author, degree-review,
  degree-analyze, stored-programs, curriculum-research) and writes `.mcp.json` plus a
  `settings.json` that approves it. The old `mcpServers` block in `settings.json` was never
  read.

### Fixed

- **`config set` and `config unset` no longer copy a project's settings into the user's
  defaults.** They saved the merged configuration — a project `nuanalytics.toml` and any
  override flags included — to the user file. They now change the one key in the user file
  read on its own, say which file they wrote (noting a project file that outranks it), and
  leave a user file that does not parse untouched.
- **`planner` read no courses when a blank line followed `Courses`**, and reported success
  with a complexity of 0. The header is now the first non-blank line after `Courses`, and
  a file from which no course can be read is an error.
- **`degree print-graph` showed a cycle's first course twice** (`CS152 → CS152 → CS163 →
  CS152`).
- **A stored run that failed to load lost the database error's kind,** so the MCP tools
  answered `tool_error` where they should have said `unreachable`, `not_authenticated` or
  whatever failed.
- **A blank `degree` with a `variant`** was told "a stored program is always analyzed
  afresh"; it is refused as blank.
- **IPEDS archives named `.ZIP`** were read as CSV.
- **A course no longer stands in for its own prerequisite, which silently discarded
  plans.** Northeastern offers `{CS2800, CS4820}` as one slot, and CS4820 requires CS2800.
  A plan taking CS4820 resolved that prerequisite through the equivalence to CS4820
  itself, so expansion never added CS2800, the plan's graph had a self-loop, and the plan
  was dropped without a word. Four corpus degrees lost plans this way — Miami's BS 17%
  (4,889 analyzed of 5,922), Northeastern's BS concentration 8.9% and BA 6.2%, Duke 4.4%.
  Now the plan gets CS2800 and is kept. The other 1,084 full degrees are byte-identical.
- **IPEDS 2022 completions had no count of nonresident-alien women.** The 2022 files pad
  the header `CNRALW` with two trailing spaces and the importer matched names exactly, so
  the column was never found and all 301,055 rows stored NULL (271,668 completions).
  Headers are now trimmed. `db validate` could not catch it: it parses the file with the
  importer's own code. The 2022 completions have been re-imported; all four stored years
  now equal their files in every row count and column sum.
- **A run stopped by the time limit reported itself as the full population.**
  `render_degree_report` said `is_full_population: true` for an MCP run its 180-second
  limit cut short; `analyze_degree` corrected it locally but still reported the plans
  analyzed as the population size. Both now report a clock-stopped run as a sample.
- **`--include`'s exclusion check read a course two branches share as open.** In
  `(A & B) | (A & C)` with A excluded, the second branch found A already visited and
  counted as satisfiable. Only runs with `--include` are affected.
- **An OR between groups of courses is resolved by the branch a plan takes.** CSU's
  MATH156 needs `(MATH124 & MATH126) | MATH127`; the analysis read that as "any one of the
  three", so a plan taking the two-course branch was credited one prerequisite, and
  prerequisite expansion could add MATH124 alone and call MATH156 satisfied (CSU's Longest
  Path did). Each plan now gets every course of the branch it completes, and expansion adds
  a whole branch, the one needing the fewest new courses. 91 of the 1,088 corpus degrees
  move (median complexity +0.4%, 24 by 5% or more); 992 reports are byte-identical,
  including all 958 degrees without such an OR.
- **`A & B | C` was read as `A & (B | C)`.** The analysis's prerequisite parser gave `|`
  the tighter binding — the opposite of every other reader — which mattered for any
  prerequisite stored as a structured tree: those read back without the parentheses
  precedence implies. A stored program re-analysed (`degree analyze --from-db`,
  `fresh=true`) therefore disagreed with the file it came from, BYU by 21%.
- **Fresh analysis is reproducible.** The same degree gave different named plans between
  runs in 21 of 60 sampled corpus degrees, and different random samples in 55; CSU's CLI
  aggregates moved about one run in 20. The causes were hash-order tie-breaks: the order of
  sampled plans, the order OR-groups resolve in, which edge a tied prerequisite cycle
  loses, and which critical path a tie reports. The CLI also never seeded its plan
  selector. After the fix, 0 of 60 vary in anything. Stored runs are read, not re-run, so
  they are unaffected.
- A trimmed run's stored report (`db report --variant trimmed`, `render_stored_report`)
  was built from the program's full document instead of the trimmed degree the run
  analyzed, so its plan graphs could draw prerequisites the run never used — UHM's
  ICS235 from MATH215, where its trimmed degree has MATH203. All 3,948 stored trimmed
  runs carry their own document; it is now used.
- `db report --degree` matched only part of a degree's name, though its help offered a
  program key or degree id too. It now tries those exactly first.
- `degree analyze --target-course` over several files ignored the flag and wrote full
  reports: the worker processes were never given it. A target-course query now always
  runs in-process.
- The verbose plan-validation breakdown credited a course listed under two requirements
  to whichever the hash visited first.
- The server advertised `import_degree` in `tools/list` without `--allow-writes`, because
  `#[tool_handler]` defaulted to a fresh full router.
- `get_reference(topic="database")` invented columns from words in comments (`not`, `so`,
  `which`) and dropped real ones (`complexity_mean`, `random_seed`, …).
- `trim_degree` replaced existing files without asking, and its input guard could be
  bypassed by another spelling of the path.
- A prerequisite cycle broken for analysis was drawn again in the report's graph.
- Course keys containing `_` lost their prerequisites in the report's course list.
- The degree-format reference's quickstart keys no longer look like elective placeholders.
- `institution_completion_totals` ingest no longer adds IPEDS's CIP `99` grand-total rows
  to the totals, which doubled every row. The stored 2022–2025 totals have been rebuilt
  and now equal the detail rows they sum, so the table can be queried directly again; the
  database reference and the `curriculum-research` skill no longer tell models to avoid
  it.

## [0.5.1] — 2026-06-10

Bug-fix release addressing issues found while live-testing the MCP server
against the v0.5.0 stored-program schema. Additive — no breaking changes.

### Fixed

- **Stored-program read tools now query the `programs` table.** `search_degrees`,
  `get_degree`, and `compare_degrees` were still reading the legacy `degrees`
  yaml-blob table, so they returned zero results even after `import_degree` had
  written programs. They now read the normalized `programs` table:
  `search_degrees` gained `degree_type` / `program_kind` / `discipline` filters,
  `get_degree` resolves by `program_key` → `degree_id` → natural key and returns
  the lossless unified-JSON `document`.
- **MCP server JWT auto-refresh.** `DbClient` held a single access token for the
  life of the process, so every database tool started failing ~1 hour in (a
  client re-login didn't help — only a server restart). The client now keeps the
  full session behind a lock and refreshes the token per request: proactively
  when it has expired (re-reading the on-disk auth file, so a fresh
  `nuanalytics db login` is picked up without a restart) and reactively once on a
  `401`. HTTP requests also gained explicit timeouts so a stalled call fails fast
  instead of hanging.
- **`render_plan_graph` no longer hangs** on `plan_category="sample"` /
  `plan_index ≥ 2`. Prerequisite DNF expansion (`parse_to_dnf`) of a pathological
  AND-of-ORs grew as `2^n`; it is now bounded so per-plan graph rendering stays
  fast for every selected plan.
- **`yaml_content` starting with `@`** is rejected fast with a directive error
  (use `yaml_path` / `degree_id`) instead of stalling the YAML parser.
- **Cache-handle errors distinguish expired from unknown**, and the `cache_yaml`
  description's stale "about 1 hour" TTL is corrected to 24 hours.

### Added

- **`analyze_degree` returns `recommended_max_plans`** — a machine-readable
  companion to the truncation follow-up (the current cap when complexity is
  CV-stable, otherwise the doubled-and-capped next value; omitted for a
  full-population run).

## [0.5.0] — 2026-06-09

Adds database-backed degree storage: a normalized, queryable projection of
imported degree programs alongside the lossless source document, plus the CLI,
MCP, and analyze paths that write to and read from it. Additive — no breaking
changes.

### Added

- **Normalized program storage schema** (`docs/database/programs-schema.sql`,
  seeded by `docs/database/program-lookup-seed.sql`). Eight new tables:
  `degree_types` (lookup), `programs` (one row per program, with the lossless
  unified-JSON `document` JSONB as the source of truth plus a queryable scalar
  projection), `courses` (shared per-institution catalog), `program_courses`
  (M:N junction with per-program credit/name overrides), `program_requirements`
  (the requirement tree flattened by `req_path`/`parent_path`, with JSONB
  `selection_spec`/`req_constraints` and `is_impossible`/`allow_double_count`
  flags), `analysis_runs` (one per `degree analyze` run: `variant`, `trimmed`,
  `variations_run`, `sample_type`, the `degree_metrics` JSONB plus promoted
  `complexity_mean`/`delay_mean`/`credits_mean`), `analysis_course_metrics`
  (per run × course), and `analysis_plans` (per run × selected plan). Design is
  hybrid (lossless document + normalized projection) with FK-free natural keys
  (`program_key`, `(institution_ref, course_code)`) matching the existing
  `degrees`/`completions` convention, RLS gated on
  `auth.role() = 'authenticated'` for every read and write, and idempotent
  re-sync via a per-import `generation` stamp. Apply with
  `nuanalytics db exec-sql docs/database/programs-schema.sql` then
  `nuanalytics db exec-sql docs/database/program-lookup-seed.sql`.

- **`db import <FILES>...`** — import degree-first analysis reports
  (`*_report.json`) or plain unified/ai-landscape/YAML degrees into the
  normalized program tables. One report populates the program projection
  (`programs`, `courses`, `program_courses`, `program_requirements`) and, when
  it carries an `analysis` block, one analysis run with its course metrics and
  selected plans. Resolves the institution (`degree.unitid` fast-path → name+CIP
  lookup → name slug; an ambiguous name lists candidate institutions and writes
  nothing). `program_key` includes `catalog_year`, so different years coexist;
  overwriting an existing program needs `--replace` (unverified) or `--force`
  (verified). Flags: `--variant`, `--unitid`, `--institution`, `--cip`,
  `--catalog`, `--degree-id`, `--force`, `--replace`, `--skip-existing`,
  `--dry-run`, `-j/--jobs`. A single file prints a detailed outcome; a
  directory/batch isolates per-file failures to `import_failures.log` with a
  summary.

- **`import_degree` MCP tool** — the same import core over MCP. Takes
  `json_content` or `json_path` (exactly one) plus the import overrides; returns
  a structured result tag (`created`/`updated`/`skipped`/`needs_confirmation`/
  `institution_ambiguous`/`rejected`) with the row counts, and attaches
  `institution_candidates`/`reason`/`errors` on the blocked variants. DB-gated
  (registered only with a logged-in session); `dry_run` previews without
  writing.

- **`degree analyze --from-db <NAME>`** — analyze a stored program pulled from
  the database instead of a file. Resolves by exact `program_key`, then exact
  `degree_id`, then a `name` substring; an ambiguous name lists the candidates
  and stops. Mutually exclusive with positional `FILES`; single-program only
  (no worker pool).

- **`degree.unitid` field** — optional IPEDS unit id on the degree model, set in
  a degree file's `degree:` block, used to link an imported program to its
  institution.

## [0.4.1] — 2026-06-04

This release is additive — no breaking changes from 0.4.0. It introduces a
unified JSON degree format and the tooling around it, parallel/process-isolated
batch analysis, and two new MCP tools, plus two engine fixes that make
machine-converted catalogs analyzable.

### Added

- **Unified JSON degree format.** Degree programs can now be authored and
  consumed as JSON (the `DegreeProgram` model serialized directly) alongside
  YAML. Every `degree` subcommand auto-detects the format on load (content
  starting with `{`/`[` → JSON, otherwise YAML), and raw ai-landscape JSON
  shapes are converted on the fly. Prerequisites serialize as a symmetric
  tagged structure (`{"and"|"or": [...]}`, with a bare string as a leaf), and
  `tags` are available on degrees, requirements, and courses.

- **`degree convert`** — convert ai-landscape program JSON into the unified
  format. Category lists and picklists map to requirements, AND-of-OR
  prerequisites flip into the internal `PrereqExpr` tree, and missing credits
  default to 3 (with warnings). ai-landscape *cluster* pipeline files
  (`course_verifier`/`course_scraper.<program>.results`) expand into one unified
  file per program with collision-safe `<school>__<program>.unified.json` names.
  `-o <PATH>` accepts a file (single input) or directory; `--pretty` pretty-prints.

- **`degree schema`** — emit the unified-degree JSON Schema
  (`src/assets/degree.schema.json`), the same schema the MCP server serves, to
  stdout or `-o <PATH>`. The schema now documents the `from` clause
  (`fromClause`: courses / pattern / include / exclude / groups), confirming the
  unified format supports the same wildcard gen-ed/elective pools as YAML
  (e.g. `"CS:2500+"`, `"*:*"`).

- **Parallel `degree analyze` (`-j`/`--jobs`, default 8).** A multi-file analyze
  now runs as a rolling pool of worker processes, one file per OS process. A
  pathological degree (e.g. a full-catalog scrape) is contained to its own
  process: if it OOMs or crashes, the kernel kills only that child, the parent
  records it in `<metrics-dir>/failures.log` with its exit status (so a
  `SIGKILL` is distinguishable from a non-zero exit), and the rest continue.
  Single-file, `--school`, and `-j 1` runs stay in-process with full per-degree
  output.

- **`--school <NAME>` on `degree analyze`** — treat all inputs as programs of
  one school and emit a combined `<school>_school_report.json` rolling up
  degree-level metrics across the programs.

- **`scripts/analyze-batch.sh`** — process-isolated batch analyze with a
  per-process virtual-memory cap (`ulimit -v`) and a timeout, for running large
  directories of degrees without the OS OOM-killer taking down the whole batch.

- **JSON input for `degree trim`.** Trim now accepts unified (and raw
  ai-landscape) JSON and writes the trimmed program back in the input's format —
  a `.json` input yields a trimmed `.json`; YAML stays YAML.

- **Metrics-rich report JSON.** Output JSON now opens with the degree block and
  is laid out degree → analysis → requirements → selected plans → courses. Each
  selected plan carries its courses, credits, course count, critical path, and a
  term-by-term schedule (mirroring the MCP `analyze_degree` shape). `total_credits`
  surfaces at the top.

- **New MCP tools.** `convert_degree` (ai-landscape JSON → unified JSON +
  warnings, caching the result for chaining by `degree_id`; a cluster file
  returns a bounded program inventory) and `get_degree_json_schema` (returns the
  machine JSON Schema). The existing degree tools
  (`validate_degree` / `analyze_degree` / `audit_degree` / `trim_degree` /
  `get_course_detail`) now accept unified and ai-landscape JSON content — and the
  `cache:<hash>` handle from `convert_degree` — in addition to YAML, via a
  content-level format sniff; `validate_degree` surfaces any
  `conversion_warnings`.

### Fixed

- **Out-of-memory on large select pools.** `RequirementResolver` materialized
  every `C(n, k)` combination of a select pool — a "choose 15 of 42" pool
  (~10¹¹) could allocate tens of GB and get OOM-killed even for an otherwise
  tiny program. Combination generation is now bounded: when `C(n, k)` exceeds
  2000, it deterministically down-samples to that cap. Peak memory on the worst
  catalog programs drops from >6 GB to ~25–120 MB.

- **Converted programs collapsed to a single plan.** Elective-category selects
  were excluded from the plan space (`ENUMERABLE_CATEGORIES` was `["major"]`
  only), so converted programs produced one plan with `std_dev = 0`. Electives
  are now enumerated (plan-count estimation uses saturating multiplication so a
  capped pool can't overflow), restoring real metric spread.

- **JSON parse errors** now report as a distinct `JsonError` rather than
  "YAML Parse Error".

## [0.4.0] — 2026-05-20

### Breaking changes

- **`degree` is now a subcommand dispatcher.** The flag-based form
  (`degree --validate <FILE>`, `degree --analyze <FILE>`, …) has been
  removed. Use explicit subcommands instead:

  | Before                                | After                                  |
  | ------------------------------------- | -------------------------------------- |
  | `degree --validate <FILES>...`        | `degree validate <FILES>...`           |
  | `degree --audit <FILES>...`           | `degree audit <FILES>...`              |
  | `degree --print-graph <FILE>`         | `degree print-graph <FILE>`            |
  | `degree --analyze <FILES>... [opts]`  | `degree analyze <FILES>... [opts]`     |
  | *(no default action)*                 | a subcommand is now required           |

  Combining actions in one call (e.g. `degree --validate --print-graph`)
  is no longer supported — call the relevant subcommands separately.

- **Database access requires authentication for both reads and writes.**
  Supabase row-level security on every table — IPEDS data, stored
  degrees, and the seven lookup tables — now requires
  `auth.role() = 'authenticated'`. The anon key continues to identify
  the project but no longer authorises any operation. Existing
  deployments should re-run `docs/database/rls-patch.sql` (idempotent)
  and update clients to `0.4.0` simultaneously. Users must run
  `nuanalytics db login` once before any database operation.

  The client automatically refreshes the user JWT when it's within 60s
  of expiry, so a single `db login` keeps long-running CLI batches and
  MCP servers usable across the default 1-hour token TTL.

### Added

- **`degree trim`** — collapse a degree YAML to one walkable
  shortest-path-per-course variant. Alternatives outside the major
  collapse to the smallest-prereq-depth choice; equivalents groups
  propagate substitutions to downstream prereq references; pattern
  pools (e.g. `ICS:400+` electives) survive orphan pruning.
  - `--keep-all <SUBJ>` — protect extra subject prefixes beyond
    `major_subjects`.
  - `--include <COURSES>` — pin specific courses as winners at choice
    points.
  - Shell wildcards work for `<FILES>...`; `-o <PATH>` accepts either
    a file (single input) or a directory (any number of inputs —
    auto-creates `<stem>_trimmed.<ext>` per input).
  - Refuses to overwrite the input file.

- **`trim_degree` MCP tool** — the same transform exposed over MCP.
  Returns the trimmed YAML inline alongside a fresh `cache:<hash>`
  handle (`trimmed_cache_id`) so callers can chain `validate_degree`
  / `audit_degree` against the result without re-pasting the body.

- **Token refresh** — `auth.rs` now exchanges the saved refresh token
  for a new access token when the current one is near expiry. Persists
  the refreshed state back to disk for the next process startup.

- **`db status` diagnostics** — prints endpoint / anon-key / auth-file
  state with expiry + email, then probes the database. On 401 it
  surfaces `→ run nuanalytics db login` and exits non-zero.

- **`init` skill updates** — the scaffolded `degree-author` skill now
  documents an optional `trim_degree` step; the scaffolded README +
  `plan-analyze` skill use the new subcommand syntax and list
  `trim_degree` among the available MCP tools.

### Changed

- **`DbClient::new`** requires a non-empty `user_jwt: String` (was
  `Option<String>`). `DbClient::from_config` is now `async` and
  returns `DatabaseError::NotAuthenticated(detail)` when no valid
  session is available.
- **MCP server boot** logs a clear warning and skips registering
  DB-backed tools when the database is unavailable (no config or no
  auth) rather than crashing — the non-DB tools (validate, audit,
  analyze, trim, …) keep working.
- **Trim metric** — the shortest-path choice uses pure upstream
  prerequisite depth (`Course → recurse; All → max; Any → min`) rather
  than the bidirectional `compute_delay`. Downstream blocking no
  longer influences which alternative wins.
- **PostgREST URL building** is in-tree (`build_select_url`) using
  `form_urlencoded::byte_serialize` — the Supabase SDK is no longer
  used for reads. Filter wildcards (`*`) survive encoding; spaces and
  other unsafe characters are encoded normally.

### Migration checklist (0.3.x → 0.4.0)

1. Run `docs/database/rls-patch.sql` against your Supabase project
   (idempotent — safe to re-run).
2. `nuanalytics db login` once on every machine that talks to the
   database (CLI, MCP servers, CI).
3. Update any scripts / docs that call `degree --validate /
   --analyze / --audit / --print-graph` to the new subcommand form.
4. If you have anything reading from your Supabase project with only
   the anon key (e.g. dashboards, downstream tools), provision them a
   real user session.

---

## [0.3.2] — earlier

See `git log v0.3.1..v0.3.2` for details:

- `chore: deny rustdoc::invalid_html_tags + broken intra-doc links`
- `fix(cli): escape <DIR> in init doc comment for rustdoc`
- `chore(release): v0.3.2 — init polish + local-config fix`
- `chore(init): drop +x bit on embedded skill reference files`
- `feat(cli): nuanalytics init <dir> — scaffold a research project`
