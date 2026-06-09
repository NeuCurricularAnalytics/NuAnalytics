# Changelog

All notable changes to NuAnalytics are recorded here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project uses semantic versioning.

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
