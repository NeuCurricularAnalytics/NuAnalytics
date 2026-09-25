# NuAnalytics — notes for Claude

Rust CLI + MCP server for curricular analytics. Crate `nu-analytics`, library
`nu_analytics`, binary `nuanalytics` (`src/cli/main.rs`).

This file covers the things that are easy to get wrong. It is not a full tour of the
codebase — run `/init` if you want that.

## Build and test

Default features: `log-info`, `log-debug`, `verbose`, `file-logging`, `database`, `mcp`.

    cargo test --all-features                       # 1340 tests, clean
    cargo test --no-default-features --features database   # 1092 — the CLI's feature set
    cargo build --features database

**Three feature sets are tested in CI, and the middle one is load-bearing.** The query
engines live in `src/core/query/` so the CLI can call them without `mcp`; if `core` ever
imports `crate::mcp` again, only `--no-default-features --features database` catches it.

**Degree fixtures are compiled in, so a missing one is a build error, not a test
failure.** `tests/rs/degree_fixtures.rs` `include_str!`s 13 real degree builds from
`tests/assets/degrees/`; renaming or deleting one takes out the whole `integration`
target rather than failing a test. That directory's `Readme.md` records where each came
from (the `WebScrappedCombinedDataMetrics` corpus) and how to refresh one.

**Only `earliest_term` is baselined in the target-course cases, deliberately.** The other
population figures move when the enumeration seed does. Read the module doc in
`tests/rs/target_course_population.rs` before adding an assertion on them.

**Builds are memory-hungry on a loaded machine.** A release build alongside several
`rust-analyzer` instances has OOM-killed this box. Prefer running long builds in their own
cgroup:

    systemd-run --user --scope -p MemoryMax=8G -p MemoryHigh=6G -- cargo build --features database

## Quality gate

`/check-rs` (`.claude/skills/check-rs/`) is the project's Rust quality pass — fmt, clippy,
then review agents. Run it between steps rather than at the end.

## The database is an optional, swappable backend

Everything except the database features works with no backend at all. Two deployments are
supported and must behave the same: Supabase cloud, and a self-hosted Supabase stack.

**The client speaks PostgREST over HTTP, never the SQL wire protocol.** `DbClient`
(`src/core/database/client.rs`) issues `GET`/`POST` against `/rest/v1/...`
(`REST_API_PREFIX`, `:46`); auth goes to `/auth/v1/...` via `supabase-client-auth`. There is
no connection string and no SQL. Anything that assumes a Postgres connection is wrong.

    src/core/database/client.rs   REST calls, batching (WRITE_BATCH_SIZE = 500), 60s timeout
    src/core/database/auth.rs     session file, load_and_refresh, 60s expiry buffer
    src/cli/commands/db.rs        db login/status/import/exec-sql, OAuth PKCE flow
    src/core/config.rs            precedence, merge_defaults, save
    src/mcp/server.rs             builds DbClient ONCE at startup

## Do not put credentials in the default assets

`src/assets/DefaultCLIConfigRelease.toml` and `DefaultCLIConfigDebug.toml` are compiled in
with `include_str!` and **this repo is public**. They ship `endpoint = ""` and
`anon_key = ""` and must stay that way. Each file carries a SECURITY comment explaining it.

This is not cosmetic. `merge_defaults` (`config.rs:285-290`) refills an *empty* endpoint or
anon key from these defaults and `load_home_config` then **saves** the result, so any value
here silently re-points a self-hosted user at that backend and persists it.
`config unset database.endpoint` does the same. Blank means "configure it explicitly",
which is the intended behaviour: the tool reports

    Database not configured. Set `endpoint` and `anon_key` in [database] config.

## Config precedence surprises

Local `nuanalytics.toml` > home `config.toml` > compiled defaults — but `config set`
writes to **home**. A user with a project-local file can run `config set`, see it succeed,
and still hit the old backend. `config.toml` is written at default umask (0644), unlike
the auth file (0600, `auth.rs:88-93`).

**The tiers are merged as `toml::Table`s, before anything is deserialised**
(`Config::merge_tables`). This is load-bearing, not a style choice: once serde has run, a
key the file never mentioned is indistinguishable from one written with its default value,
and the previous struct-level merge had to guess from the value — wrongly, for three
different kinds of value. A local file that mentioned only `endpoint` silently reset
`auth_file` to its non-empty default, so `db whoami` reported a signed-in user as signed
out; `max_plans = 1000` written explicitly was discarded for equalling the default; and
`verbose = false` could not turn off a `true` from a lower tier. Add a field and it is
handled automatically — there is no per-field merge list any more.

**An explicitly blank string is treated as absent and never overrides.** Blank means "not
configured" everywhere in this tool, so a blank in one tier erasing a working value from
another could only be a footgun. Writing a value is how you override a lower tier;
blanking a key is how you say nothing. `Config::table_sets_endpoint` follows the same
rule, so `db status` does not name a file that blanked the endpoint as the file that set
it.

## Current work

**The self-hosting migration is finished** and `docs/db-migration-todo.md` is retired
(2026-09-24). The durable findings live where they are used: the setup walkthrough and the
two traps a self-hoster hits — seed tables are read-only through the API, and you need the
`supabase/postgres` image, not a stock one — are in `docs/database/setup.md`.

`docs/database-audit-todo.md` is the live backend work list, trimmed to what is left:
re-import the corpus (it predates both the recovered degree fields and the OR-group
tie-break fix), tidy the corpus repo, and why `db remetric` is deliberately not built.
It also records the settled decisions that are easy to re-litigate — notably that **writes
stay open to any authenticated member**: ownership policies were implemented and reverted
because they stop `db import --replace` overwriting another member's row. `created_by`
remains on four tables as attribution only, read by no policy.

**Every deployment is a fresh install** (true as of 2026-09-18 — the author's is the only
one). So there is no schema migration path to preserve, and
`docs/database/rls-patch.sql` and `schema-patch-v2.sql` are obsolete: both say in their
own headers that they are for pre-existing databases and that `schema.sql` already
contains everything in them.

**Standing up a new backend is four commands**, none of which need a checkout or an OAuth
app: `config set database.endpoint`, `config set database.anon_key`,
`db bootstrap --print | psql "$DATABASE_URL"`, `db login --email <addr>`, then
`db doctor` to confirm.

**`db bootstrap` owns the file order, not the connection.** `--print` emits the five
schema/seed files in dependency order and makes no network calls and reads no config —
that is the state someone is in before setup. Plain `db bootstrap` applies them via the
Management API on Supabase cloud only, and refuses on a self-hosted endpoint rather than
inventing a project ref. The files are `include_str!`'d from `docs/database/`, so there is
one copy and no drift; `bootstrap::SCHEMA_FILES` is guarded by a test that diffs it
against `docs/database/*.sql`, so a new file added there must be wired in or moved to
`historical/`.

**There are two ways to sign in and OAuth is only one of them.** `db login --email <addr>`
uses GoTrue's `grant_type=password`, which needs no external identity provider, so it is
the way into a stack whose operator has not registered an OAuth app. Both grants return
the same body and share one `TokenResponse`/`into_auth_state` in `auth.rs`, which is where
`expires_at` is recomputed from `expires_in` (self-hosted GoTrue does not always send the
absolute field). Sign-in failures are a typed `SignInError` split like `RefreshError` —
`Transport` means never reached, `Rejected` means answered and refused — and rejections
quote GoTrue's own text via `gotrue_error_text`, which tries `error_description`, `msg`,
`message`, `error_code`, `error` in that order because GoTrue has shipped all of those
shapes. Do not add a cause of our own to a rejection. The password is prompted via
`rpassword` and must never become an argument.

**`nuanalytics db doctor` is the first thing to run against an unfamiliar deployment.** It
walks configuration → reachability → anon-key read → session → authenticated read → schema
(all 20 tables) → seed data → row limit, each check gating the next so the report names one
cause rather than repeating it. Logic is in `src/core/database/doctor.rs`; the CLI only
formats. A check added to `diagnose` must also be added to the three `*_DEPENDENTS` arrays,
or it silently disappears from the report when an earlier check fails — the unreachable
test asserts the report length to catch exactly that.

**The row-limit check is the one that catches wrong answers rather than errors.** A
`PGRST_DB_MAX_ROWS` below 5,000 truncates the MCP completions queries with an HTTP 200 and
no indication. It is detected by disagreement between `count_rows` (`count=exact`, which
the cap does not apply to) and `DbClient::rows_returned` (a real select), so the reported
cap is observed, not guessed. Probing `completions` covers the full 5,000; the `cip_codes`
fallback only proves a cap is above 2,173 and **says so** rather than implying coverage it
does not have.

**Failure messages are shared, not per-call-site.** `DatabaseError::next_steps(endpoint)`
(`src/core/database/error.rs`) owns the remediation text for every variant, and both
`db status` and the MCP server's `db_not_configured_response` render it. Add wording there
rather than at a call site, and keep to the rule the TODO sets: name the backend, name
what failed, name the next step, and assert no cause the code has not established.

`docs/clean-up-analysis-todo.md` plans the merge of the degree analysis pipeline, which
still exists twice (CLI and MCP). It opens with evidence that the two are the same level
of analysis, and that `planner` is not, so `planner` stays out of it. **Step 1 is done**:
the per-plan DAG now exists once, in `src/core/degree/plan_dag.rs`, called by both — an
OR-group contributes at most one edge. Steps 2–7 are not started.

Two things in that document are load-bearing before touching metrics. The tie-break change
in step 1 moved **307 of 1,088 degrees** (2.8% by ≥5%), so the stored corpus is a
generation behind. And section 6 records, with measurements, why the OR-of-ANDs gap in the
flat edge model should *not* be "fixed" casually: the real defect is 0.09% of
course-instances, while the option-choice heuristic it would drag along is 15× more
frequent and moves metrics far more.
