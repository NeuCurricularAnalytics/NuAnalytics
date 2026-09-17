# NuAnalytics — notes for Claude

Rust CLI + MCP server for curricular analytics. Crate `nu-analytics`, library
`nu_analytics`, binary `nuanalytics` (`src/cli/main.rs`).

This file covers the things that are easy to get wrong. It is not a full tour of the
codebase — run `/init` if you want that.

## Build and test

Default features: `log-info`, `log-debug`, `verbose`, `file-logging`, `database`, `mcp`.

    cargo test --features database --lib --bins     # 961 tests, clean
    cargo build --features database

**`cargo test` (the full suite) does not compile, and this is pre-existing.**
`tests/rs/first_sem_cases.rs` has 29 `include_str!("/tmp/first_sem_unified/*.unified.json")`
calls — absolute paths into `/tmp` for fixtures that no longer exist:

    error: couldn't read `/tmp/first_sem_unified/Syracuse_University_...unified.json`

Use `--lib --bins` until those fixtures are vendored under `tests/fixtures/` and referenced
relative to `CARGO_MANIFEST_DIR`. Don't be misled into thinking your change broke it.

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

Local `nuanalytics.toml` > home `config.toml` > compiled defaults
(`config.rs:492-530`) — but `config set` writes to **home**. A user with a project-local
file can run `config set`, see it succeed, and still hit the old backend. `config.toml` is
written at default umask (0644), unlike the auth file (0600, `auth.rs:88-93`).

## Current work

`docs/db-migration-todo.md` is the live work list for making the backend a true deployment
target — importer defects, backend portability, diagnosability, reproducibility, and data
integrity on a shared instance. Items marked `[verified]` were observed in practice, not
theorised. Start there.
