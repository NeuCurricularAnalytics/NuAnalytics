# Database migration — TODO

Work items for making the database backend a user-supplied deployment target, so a group can
run their own instance instead of a shared cloud project. The tool already works without a
database; this makes "cloud" and "self-hosted" behave the same.

Findings below were produced by standing up a self-hosted Supabase stack and running the
real bootstrap + IPEDS import against it on 2026-09-15. Items marked **[verified]** were
observed in practice, not inferred from reading code.

## Context that shapes all of this

The client never speaks SQL over the wire. `DbClient` has three public methods — `ping`
(`src/core/database/client.rs:249`), `select` (`:272`), `upsert_batch` (`:354`) — all HTTP
against `{endpoint}/rest/v1`, plus GoTrue at `{endpoint}/auth/v1`. There is no `sqlx` or
`tokio-postgres`. So pointing at a different backend is configuration, and the query layer,
models, and `QueryFilters` need no changes at all.

---

## Start here — make failures legible (DONE)

All five items below landed. Verification: `cargo fmt --check` clean, `cargo clippy
--all-targets -- -D warnings` clean under `--no-default-features`, `--features database`
and `--all-features`; 1172 tests on default features (4 consecutive clean runs), 707 with
no default features. The HTTP boundary now has its first tests — a path-aware
`tokio::net::TcpListener` stub in `client.rs`, no new dev-dependency.

What is left in §3 is the four *missing-information* items (`db whoami` endpoint, naming
the winning config file, the `db status` help text, `db logout --remote`), which were
always scheduled after these.

The numbered sections below are grouped by **category, not priority**.

Why this first: every item in it has already cost real debugging time during the
migration, the changes are small and mutually independent, and until a failure names its
own cause, verifying any *later* item is harder than it needs to be. None of it changes
behaviour on the happy path, so the risk is low.

Order, with the reasoning:

1. **Surface `error_description` on a failed OAuth callback** — §3, `db.rs:294-297`.
   Best value for effort in the whole document; it is a few lines. The only item that has
   misdirected debugging *twice*. A precise GoTrue error —
   `sql: Scan error on column index 3, name "confirmation_token": converting NULL to
   string is unsupported` — reached the user as *"Check that the provider is enabled in
   your Supabase project"*, a cause the code cannot know and which was simply wrong. Print
   `error_description` and `error_code`; stop asserting a cause.
2. **Branch the `db status` hint on the error variant** — §3. It currently tells a
   *not-configured* user to run `db login`, when there is no backend to log in to.
   `Disabled`, `NotConfigured` and `NotAuthenticated` each have a different fix and should
   each say so.
3. **Make the forced token refresh actually force** — §3, `client.rs:285`,
   `auth.rs:214-217`. A stale-but-clock-valid token produces a bare
   `PostgREST error (401)` with no "run `db login`" hint, and the retry is wasted.
   Depends on nothing else here.
4. **Name the endpoint in MCP failures** — §3, `mcp/server.rs:879-891`, `:795-806`.
   The worst user-facing case: the MCP server builds its client once at startup, so a
   transient outage makes it blame configuration for the entire process lifetime.
5. **Fix the `db status` stale-expiry line** — §3. Cosmetic, but it prints
   `(expired N min ago)` directly above `ping: ✓ authenticated read succeeded`, which
   undermines trust in everything else the command reports.

§3 holds nine items; the five above are the ones where a failure *reports itself wrongly*.
The other four — `db whoami` not printing the endpoint, naming the winning config file,
the wrong `db status` help text, and `db logout` not revoking remotely — are diagnosability
too, but they are missing information rather than actively misleading output. Do them after,
or fold them in opportunistically while touching the same files.

**Acceptance criteria for this step.** For each failure mode, the message should name:
(a) which backend was being talked to, (b) what actually failed, and (c) what the user
should do next. No message may assert a cause the code has not established — that is the
specific defect that made items 1 and 2 expensive.

**Decide the test question before starting.** These auth/config paths are exactly the
ones worth integration-testing, and `cargo test --features database` now runs the full
suite (see "Blocker for anyone picking up Part B -- RESOLVED").

---

## 1. Importer defects [verified] — DONE

Found by running `db ipeds-import` for 2022/2023/2024/2025.

- [x] **Non-UTF-8 source files are rejected outright.** *(done)* — `decode_ipeds_bytes`
      tries UTF-8 and falls back to CP1252, and **both** read paths now use it. They had
      disagreed: the zip path used a strict UTF-8 read and aborted, while the loose-CSV
      path used `from_utf8_lossy` and silently replaced the byte with `U+FFFD`, corrupting
      the name with no signal. CP1252 rather than Latin-1 because they differ over
      `0x80..=0x9F`, where CP1252 carries the smart quotes and em dash that appear in
      institution names. `encoding_rs` was already in the lock file via `reqwest` under
      the same feature, so this added no new crate. The fallback is reported, with the
      byte offset. Five tests, including the real `\xe9` row and the
      CP1252-vs-Latin-1 range.
      ~~Original:~~ `HD2022.zip` failed with
      `Cannot decode zip entry: stream did not contain valid UTF-8`. The file is CP1252;
      the offending byte is `\xe9` (the `é` in a trustee name) at byte 64468. One accented
      character aborts a 6,256-row import. IPEDS has historically shipped CP1252.
      **Fix:** fall back to CP1252/Latin-1 when UTF-8 decoding fails.
      Workaround today: `iconv -f CP1252 -t UTF-8` then repack the zip.
- [x] **Partial failure exits 0.** *(done)* — failures are collected and the command
      exits 1 with a per-file summary. It also calls out the specific dangerous
      combination explicitly: completions succeeding while institutions failed leaves rows
      referencing absent `unitid`s, which is exactly how the 494 orphans appeared.
      ~~Original:~~ For 2022, institutions failed while completions
      succeeded and the process still exited 0, so a batch loop reports success on a
      half-finished import. The only symptom was 494 orphaned `completions` rows referencing
      `unitid`s absent from `institutions`.
      **Fix:** return non-zero when any sub-import fails.
- [x] **IPEDS filename casing is inconsistent.** *(done)* — `auto_detect_file` matches
      case-insensitively, keeping a fast path for an exactly-cased name. Directory entries
      are sorted first: `read_dir` order is arbitrary, so with both `HD2022.csv` and
      `hd2022.csv` present an unsorted scan picked a different file per run. Five tests,
      one of which pins the determinism.
      ~~Original:~~ `hd2022.csv` / `hd2025.csv` are lowercase;
      `HD2023.csv` / `HD2024.csv` are uppercase. `--dir` auto-detection must be
      case-insensitive or it silently skips files.
- [x] **`docs/database/ipeds-data.md` direct-download URLs fail for the newest
      release.** *(done)* — documented that the current year must come through the Data
      Center UI, alongside the casing and CP1252 notes so the next person meets them in
      the docs rather than in a failed import.
      ~~Original:~~ `datacenter/data/HD2025.zip` 404s even with browser headers and a referer,
      while `HD2024.zip` succeeds by the same method. The current year must come through the
      Data Center UI. Document that rather than implying `curl -O` always works.

## 2. Backend portability — DONE

- [x] **`db exec-sql` assumes Supabase cloud.** *(done)* — gated on an explicit
      `database.project_ref`, checked **before** the `management_key` check so a
      self-hosted user is told the command does not apply rather than sent to create a
      Supabase PAT first. Hostname sniffing is gone from the decision path;
      `extract_project_ref` became `suggest_project_ref`, which only fills in a hint and
      only for a genuine `*.supabase.co` host with a single label — so `https://db.example.edu`,
      `http://localhost:8000` and `https://a.b.supabase.co` all yield no suggestion
      instead of a bogus ref. Help text reworded, and it now says what a self-hosted user
      should do instead (`psql -f <file>`). Verified live against `nu.lionelle.com` and a
      simulated cloud endpoint.
      ~~Original:~~ `SUPABASE_MGMT_API_BASE`
      (`src/cli/commands/db.rs:20`) posts to `api.supabase.com`, and `extract_project_ref`
      (`:383-393`) takes the first subdomain label — so `https://db.example.edu` yields
      `Some("db")` rather than `None`, and the request goes out with a meaningless project
      ref. It also mangles ports and paths (`http://localhost:8000` → `Some("localhost:8000")`).
      **Fix:** gate on an explicit signal (a `database.project_ref` config key or a
      `--project-ref` flag), not on hostname sniffing — host matching breaks legitimate
      cloud projects served through a custom domain. Place the check *before* the
      `management_key` check at `:397-403`, otherwise self-hosted users are told to go
      create a Supabase PAT. Reword the `ExecSql` help text at `src/cli/args.rs:628` to
      match.
- [x] **Normalize a trailing slash on `database.endpoint`.** *(done)* — one
      `config::normalize_endpoint`, applied where the endpoint enters the system: the
      client constructor (so a hand-edited config is also covered) and `Config::set` (so
      the stored value stays clean). Not applied per URL builder, which is what let the
      three sites disagree in the first place.
      ~~Original:~~ `auth.rs:165` trims it and the
      SDK does too, but `client.rs:374` and `:411` do not, so an endpoint ending in `/`
      produces `…//rest/v1/…`. `Config::set` does no validation. Normalize in one place.
- [x] **Remove the baked-in default endpoint and anon key** from
      `src/assets/DefaultCLIConfigRelease.toml` and `DefaultCLIConfigDebug.toml`.
      *(done — B6)* Both ship `endpoint`, `anon_key` and `management_key` blank, and
      `config::tests::default_assets_never_ship_database_credentials` now guards it by
      reading **both** files as raw text — only one is compiled per build profile, so
      asserting through `Config::from_defaults()` would leave the other unchecked. The two
      sub-bullets below remain true and are the reason the guard exists.
      ~~Original:~~ With a
      user-supplied backend there is no correct default, and shipping one is what makes the
      next two bugs possible:
      - `merge_defaults` (`src/core/config.rs:285-290`) refills an *empty* endpoint from the
        compiled-in default, and `load_home_config` (`:539-554`) then saves it — so clearing
        the endpoint silently restores the default and persists it.
      - `config unset database.endpoint` resets to that default (pinned by the test at
        `config.rs:1115-1123`). Users must be told to `set`, never `unset`.

## 3. Diagnosability — DONE

- [x] **`db whoami` does not print the endpoint** *(done)* — `whoami` now prints a
      `Backend:` line, and both not-signed-in arms name the endpoint too ("Not signed in
      to https://…"). Shared `endpoint_or_unset` renders `(no endpoint configured)`
      rather than a blank.
- [x] **Report which config file won.** *(done)* — `Config::load_with_sources()` returns
      a `ConfigSources { home, home_status, local, local_status, endpoint_from }` and
      `db status` prints `endpoint from: <path>`. It also reports a config that exists but
      could not be read or parsed, which `load()` previously swallowed via
      `if let Ok(...)`, and warns when a project-local file outranks the home config but
      sets no endpoint.

      **This mattered immediately.** The home file differs by build profile —
      `config.toml` for release, `dconfig.toml` for debug (`config.rs:20`) — so the same
      machine reports two different backends depending on which binary is run. That cost
      a wrong report during this very work: `db status` from `target/debug` said the
      backend was the cloud project while `config.toml` had long pointed at
      `nu.lionelle.com`. Naming the file makes the split visible.
- [x] **`db status` help text is wrong.** *(done)* — now describes what it actually
      reports: backend, which config file supplied it, session validity, authenticated
      read, and that it exits 1 on failure.
- [x] **The forced token refresh cannot force.** *(done)* `reauthenticate(force)` now
      routes to `DbClient::force_refresh_from_disk`, which re-reads the auth file and
      adopts a newer token if another process wrote one, otherwise exchanges the refresh
      token directly. A 401 that survives the retry is classified by
      `DbClient::classify_failure` as `NotAuthenticated` naming the endpoint and
      `db login`, not a bare `QueryError`. Covered by five `#[tokio::test]`s over a
      path-aware stub server (`client.rs`), no new dev-dependency.
      ~~Original:~~ `client.rs:285` catches a 401 and calls
      `reauthenticate(true)`, but the path runs through `load_and_refresh`, which
      short-circuits on `state.is_valid()` (`auth.rs:214-217`) and returns the same dead
      token. A stale-but-clock-valid token yields a bare `PostgREST error (401)` with no
      "run `db login`" hint, and the retry is wasted.
      **Fix:** call `refresh_session` directly when `force` is set, and map a post-retry 401
      to `DatabaseError::NotAuthenticated`.
- [x] **MCP database outages are misreported as user error.** *(done)* `run_server` now
      keeps the startup failure in `DbUnavailable { endpoint, kind, detail, next_steps }`
      instead of discarding it, and `db_not_configured_response` emits
      `{backend, reason, detail, next_steps}` — including an explicit "the client is
      created at startup; restart after fixing" step. An unreachable backend is no longer
      reported as a config or login problem.
      ~~Original:~~ `run_server` builds the
      `DbClient` once at startup (`src/mcp/server.rs:879-891`) and on failure sets it to
      `None` for the process lifetime; every tool then returns `db_not_configured_response`
      (`:795-806`), which blames config or login. It is also intermittent: a fresh token
      starts fine and fails per-call, an hour-old token starts with no database tools at
      all. Name the endpoint and distinguish unreachable / not-configured / not-logged-in.
- [x] **`accept_oauth_callback` throws away the only useful part of an OAuth failure**
      *(done)* `CallbackFailure` now captures `error`, `error_description` and
      `error_code` and renders them in both the terminal message and the browser page
      (HTML-escaped — the text arrives via a URL). The invented "check that the provider
      is enabled" wording is gone from both. The real cutover failure is pinned as a test
      fixture so it cannot regress.
      ~~Original:~~
      [verified — cost hours]. `db.rs:294-297` keeps the `error` query param and discards
      `error_description`, so every failed callback collapses to
      *"Sign in failed — check that the provider is enabled in your Supabase project"*.
      During the self-hosted cutover the real cause was
      `sql: Scan error on column index 3, name "confirmation_token": converting NULL to
      string is unsupported` — a malformed `auth.users` row, nothing to do with the
      provider being enabled. GoTrue had sent it in `?error_description=`; it was only
      recoverable by reading the browser's address bar.
      **Fix:** print `error_description` (and `error_code` when present) in both the
      terminal message and the browser page, and stop asserting a cause the code cannot
      know. The current wording actively misdirects: it sent us to audit GoTrue config and
      the GitHub OAuth app registration twice.
- [x] **`db status` reports a stale expiry next to a successful ping** *(done)* — the
      auth-file line is now rendered *after* the ping, so it reflects the refreshed
      session. Verified against the live backend: `expires in 60 min` beside
      `ping: ✓ authenticated read succeeded`.
      ~~Original:~~ [verified]. It
      renders the auth-file line from the on-disk `expires_at` *before* the ping refreshes
      the token, so a session that refreshes fine prints
      `✓ (expired 29827635 min ago as …)` immediately above
      `ping: ✓ authenticated read succeeded`. Harmless but it reads like a bug in the tool
      and undermines trust in the rest of the output. Render the line after the refresh, or
      label it "token age (auto-refreshes)".
- [x] **No `db logout --remote`, and `db login` cannot re-auth a live session.** *(noted,
      not implemented)* — `db logout`'s help and output now state plainly that it removes
      the local token only, that the token stays valid at the named backend until it
      expires, and that offboarding means deleting the `auth.users` row. The output line
      changed from "Signed out", which read as revocation.

      A real `--remote` is still absent and is a deliberate non-goal for now: GoTrue
      validates only the JWT signature and `exp`, so even a server-side revocation leaves
      up to a one-hour tail. Anything that claimed to revoke immediately would be
      asserting more than it can deliver.
- [x] **`db status` tells a not-configured user to log in** *(done)* — remediation now
      comes from `DatabaseError::next_steps(endpoint)` in the library, shared with the MCP
      server so the two cannot drift. Verified on a blank config: it points at
      `config set database.endpoint/anon_key` and notes that `config set` writes to the
      home config while a project-local `nuanalytics.toml` wins.
      ~~Original:~~ [verified]. On a fresh install
      with blank defaults (post-B6) it prints `endpoint (unset) ✗`, `anon key (unset) ✗`,
      `ping: ✗ Database not configured...` and then `→ run \`nuanalytics db login\``.
      There is nothing to log in to. The hint should branch: not-configured -> point at
      `config set database.endpoint/anon_key` (or `db bootstrap`, B9); not-authenticated ->
      `db login`.

### Found while doing §3 [verified]

- [ ] **A project-local config silently resets every field it does not mention.**
      `merge_from` (`config.rs:568`) overrides whenever the incoming value is non-empty,
      but by the time it runs, serde has already filled absent keys with their
      `#[serde(default = ...)]` values — which for `auth_file` is a *non-empty* relative
      path (`default_auth_file`, `config.rs:77`). So a `nuanalytics.toml` containing only

          [database]
          endpoint = "https://nu.lionelle.com"

      silently replaces the home config's `auth_file` with `.debug/dauth.json`, resolved
      against the current directory. `db whoami` then reports **"Not signed in"** for a
      user who is signed in; the tool simply looked in the wrong file.
      **[verified]** Reproduced against this machine's own configuration.

      `endpoint`, `anon_key` and `management_key` escape this only by accident — their
      defaults are `""`, and an empty value cannot override.
      Every other field with a non-empty default is affected, including `logging.level`,
      `logging.file` and both `paths.*` entries.

      **Fix:** merge only keys the file actually contains. Either deep-merge at the
      `toml::Table` level before deserializing (removes the empty-string heuristic
      entirely and is the conventional shape for layered config), or pass the parsed
      table alongside and gate each field on key presence. Prefer the former, but note it
      changes semantics for an explicit `endpoint = ""` in a local file, which currently
      cannot override — and per `CLAUDE.md` an empty endpoint is refilled from the
      compiled defaults and then **saved**, so that interaction needs a test before the
      change lands.

## 4. Making self-hosting reproducible

- [ ] **`db bootstrap`** — apply the five schema/seed files in the documented order
      (`docs/database/setup.md:114-121`) through whichever path is available: Management API
      when a project ref is configured, direct `psql` otherwise. Applying the schema is the
      *only* real asymmetry between cloud and self-hosted; closing it is what makes the two
      interchangeable.
- [ ] **`db doctor`** — report, for whatever backend is configured: which config file
      supplied the endpoint, reachability, cert validity, whether all 20 tables exist,
      whether an anon-key-only read returns `200 []` (RLS filtering) rather than an error,
      session validity, and row-count sanity (`cip_codes` = 2173). This is what lets a group
      that is not the author diagnose their own deployment.
- [ ] **`deploy/selfhost/`** — pinned compose file, env template with empty secrets, the
      five-file bootstrap, and a README. Verified gotchas to document:
      - `podman-compose` 1.6.0 hangs during image pull (sits in `ep_poll`, no child process,
        no storage growth, then exits 0 when killed). Workaround: `podman pull` each image
        first, then `up -d` completes in seconds. **[verified]**
      - On SELinux-enforcing hosts the Envoy config mounts need `:ro,Z`. Upstream ships them
        as plain `:ro`, so the container cannot read its own entrypoint, crash-loops, and
        floods the desktop with AVC denials. The `db` mounts already carry `:Z`. **[verified]**
      - `PGRST_DB_MAX_ROWS` defaults to **1000** upstream, while the MCP completions tools
        request up to 5,000 rows (`src/mcp/tools/completions.rs:557,1063,1099`). Leave it
        unset or set it ≥ 10000, or large queries truncate silently with no error.
      - Upstream now uses Envoy (`api-gw`), not Kong, and there is no `vector` or
        `analytics` service. The only dependency edge to cut when dropping Studio is
        `api-gw <- studio`. **[verified]**

## 5. Data integrity on a shared instance

- [ ] **Writes are last-writer-wins on natural keys.** `upsert_batch` sends
      `Prefer: resolution=merge-duplicates` (`client.rs:330,373-376`) against `program_key`
      for `programs` (`import.rs:1168,1172`) and `degree_id` for `degrees`
      (`mcp/tools/degrees.rs:567`), and only checks `status.is_success()` (`client.rs:386`).
      Two people importing their own version of the same program means the second silently
      overwrites the first's `document` JSONB. `analysis_runs` is unaffected — `run_key` is
      a content hash, so the overwrite is a no-op.
      **Fix (one migration, no Rust):** add `created_by uuid DEFAULT auth.uid()` to
      `programs`, `degrees`, `program_courses`, `program_requirements`, and replace their
      `FOR ALL` policies with `USING (created_by = auth.uid() OR created_by IS NULL)` plus a
      matching `WITH CHECK`. Leave the IPEDS reference tables and `courses` shared by design
      (`src/core/database/mod.rs:56`).
      **[verified]** The resulting error is *not* the expected `42501` RLS violation. Because
      RLS hides the other user's row from the UPDATE path, the `merge-duplicates` upsert
      falls through to an INSERT and trips the natural-key unique constraint instead:
      `HTTP 409  {"code":"23505","message":"duplicate key value violates unique constraint
      \"degrees_degree_id_key\""}`. Reads are unaffected — the other user can still SELECT
      the row, so collaboration is preserved. Translate `23505` on these four tables into
      "program_key / degree_id X is owned by another user", because "duplicate key" gives
      the user no hint that ownership is involved.

### Blocker for anyone picking up Part B -- RESOLVED

`cargo test` used to fail to compile the integration target: the target-course cases
`include_str!`d absolute `/tmp/first_sem_unified/` paths for fixtures that no longer
existed. The full suite now builds and runs.

Thirteen real degree builds are vendored under `tests/assets/degrees/` (not
`tests/fixtures/`, as originally proposed) and referenced by paths relative to the test
source. `tests/assets/degrees/Readme.md` records which upstream corpus build each file is,
how the mapping was recovered, and how to refresh one.

Two things surfaced while doing it, both fixed:

- The cases asserted nothing -- they printed computed-vs-expected term numbers and always
  passed. They now assert, and `earliest_term` baselines are recorded per case.
- `build_artifacts` left `PlanGeneratorConfig.random_seed` at `None` while setting
  `SamplingStrategy::Shuffled`, so *which* plans were enumerated under a `max_plans` cap
  came from thread-local entropy and analysis output differed run to run. The seed it
  already computes is now passed through, making runs reproducible. This changed none of
  the term-placement baselines.

## Notes

- All 31 RLS policies are `auth.role() = 'authenticated'` — 16 in `schema.sql`, 15 in
  `programs-schema.sql`. 12 are write (`FOR ALL`) policies, so `cip_codes`, the 7 lookup
  tables, and `degree_types` are read-only through the API and **must** be seeded via SQL,
  not the client. Confirmed in practice: an authenticated POST to `cip_codes` returns
  `42501`. **[verified]**
- There is no `GRANT` statement in any schema file. The tables are usable only because the
  `supabase/postgres` image sets `ALTER DEFAULT PRIVILEGES` for `anon`/`authenticated`/
  `service_role`. A plain `postgres:N` image will not work. **[verified]**
