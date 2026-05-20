# Changelog

All notable changes to NuAnalytics are recorded here. Format loosely
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); the
project uses semantic versioning.

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
