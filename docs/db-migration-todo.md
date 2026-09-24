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

**Every deployment is a fresh install.** As of 2026-09-18 the only existing installation
is the author's. Nothing here needs a migration path from an older schema, which retires
two files and changes the shape of §5 (below). Anything framed as "migrate an existing
database" can be folded into the schema it patches.

Consequently `docs/database/rls-patch.sql` and `docs/database/schema-patch-v2.sql` are
**obsolete**. Both say so in their own headers -- *"Run against an existing database"*,
*"If you are setting up a fresh database, run schema.sql instead -- it already includes
all of the below"* -- so every object in them is already in `schema.sql`. They are seven
files in a directory where the documented order names five, which is a trap for exactly
the person this document is written for. Delete them, or move them under
`docs/database/historical/`.

---

## Where a fresh installer gets stuck (measured 2026-09-18)

Walked end to end with an isolated `HOME`, so this is what a new user actually meets
rather than what the code looks like it should do.

**Configuring the tool is no longer the hard part.** Two commands point it at any backend,
and `db doctor` then reports the state of that backend check by check:

    nuanalytics config set database.endpoint <url>
    nuanalytics config set database.anon_key <key>

Unconfigured is reported correctly and exits 1 -- *"no backend is configured"*, the two
commands to fix it, and the warning that a project-local `nuanalytics.toml` outranks what
`config set` writes. Nothing is silently defaulted: `endpoint from: nothing — no tier set
an endpoint`. That is §2 and §3 having landed.

Signing in no longer needs an OAuth application either, as of 2026-09-18:
`db login --email <addr>` uses GoTrue's password grant, which every stack has out of the
box. That was the one barrier with no workaround outside the tool.

Nor is applying the schema, as of 2026-09-18: `db bootstrap --print | psql "$DATABASE_URL"`
emits all five files in the order they require, with no config and no network needed.

So the answer to "can someone configure the NuAnalytics side of a self-hosted or remote
backend without it being a hassle" is **yes**, in four commands:

    nuanalytics config set database.endpoint <url>
    nuanalytics config set database.anon_key <key>
    nuanalytics db bootstrap --print | psql "$DATABASE_URL"
    nuanalytics db login --email <addr>
    nuanalytics db doctor            # confirms all eight checks

Nothing known is in the way any more. The project-local config defect that used to be
listed here -- a `nuanalytics.toml` resetting `auth_file` and turning a working session
into *"Not signed in"* -- is fixed in §3.

The walkthrough is what found the sign-in barrier and the row cap; neither was in this
document before 2026-09-18.

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

- [x] **A project-local config silently reset every field it did not mention.** *(done)*
      -- the tiers are now merged as `toml::Table`s **before** deserialisation
      (`Config::merge_tables`), so "the file did not mention this key" is observable
      again. After serde runs it is not, which is why the old struct-level `merge_from`
      had to guess from the value whether a key had been written -- and each guess was
      wrong for some legitimate value. `merge_from` is deleted; there is no per-field
      merge list to maintain, so a new field is handled automatically.

      **Three symptoms, one cause.** Reproduced in an isolated `HOME` with a local file
      setting only `[database] endpoint`, then re-run after the fix:

      | Local file says | Home has | Before | After |
      |---|---|---|---|
      | nothing about `auth_file` | `/home/me/auth.json` | `.debug/dauth.json` | `/home/me/auth.json` |
      | `max_plans = 1000` | `9999` | `9999` (discarded) | `1000` |
      | `verbose = false` | `true` | `true` (cannot disable) | `false` |

      Only the first was recorded here before; the other two were found by reproducing it.
      The second and third are the *opposite* failure -- a value the user wrote being
      thrown away -- and came from the non-string branches comparing against the default
      and testing truthiness.

      **An explicitly blank string is treated as absent and never overrides**, which is
      the semantic decision this needed. Blank means "not configured" everywhere in this
      tool, so a blank in one tier erasing a working value from another could only ever be
      a footgun; the point of the endpoint being blank is to drive the user to configure
      one, not to unconfigure someone else's. Writing a value is how you override a lower
      tier; blanking a key is how you say nothing. `table_sets_endpoint` follows the same
      rule, so `db status` no longer names a file that blanked the endpoint as the file
      that set it.

      A local file that is malformed, or that parses but does not describe a valid config,
      is reported through `SourceStatus::Malformed` and leaves the tiers below it standing
      rather than taking the whole configuration down.

      Mutation-tested: replacing nested tables wholesale instead of merging them fails
      three tests, and dropping the blank-is-absent rule fails two.

## 4. Making self-hosting reproducible — DONE

- [x] **`db login` was OAuth-only, so nobody could sign in to a new stack until an OAuth
      app existed.** *(done)* -- `nuanalytics db login --email <addr>` now uses GoTrue's
      `grant_type=password`, which needs no external identity provider and is therefore
      the way in to a stack whose operator has not registered an OAuth application. OAuth
      stays the default, so nothing changes for the existing deployment, and the two flags
      conflict at the arg parser rather than one silently winning.

      The password is prompted for on the tty via `rpassword` and never accepted as an
      argument -- an argument persists in shell history and is visible in `ps` for the
      life of the process. `rpassword` was preferred over a hand-rolled `stty -echo`
      because it restores the terminal on a signal; `stty` leaves echo off after a Ctrl-C
      at the prompt. Verified not to echo: a pty test types a known string and asserts it
      never appears in the output.

      Neither path creates an account, so this did not enable signup -- the user must
      already exist, which is what `add-member.sh` does today.

      Both grants return the same session body, so they now share one `TokenResponse` and
      one `into_auth_state`, which is where the `expires_at`-from-`expires_in`
      recomputation lives (self-hosted GoTrue does not always send the absolute field).
      Failures are a typed `SignInError` split the same way as `RefreshError`, for the
      same reason: `Transport` means the backend was never reached, `Rejected` means it
      answered and refused. Rejections quote GoTrue's own words -- `gotrue_error_text`
      reads `error_description`, `msg`, `message`, `error_code` and `error`, most
      specific first, because GoTrue has shipped all of those shapes and a self-hosted
      stack may run any of them.

      `run_login` now exits 1 on failure. It was the only `db` subcommand that did not,
      and it is the first step of setting a deployment up, so `db login && db import`
      would previously have carried on against a backend it never signed in to.

      **[verified 2026-09-18]** Live against `nu.lionelle.com` through a pty: the grant
      reaches GoTrue and its own refusal surfaces --
      `https://nu.lionelle.com refused the sign-in: Invalid login credentials (HTTP 400)`,
      exit 1. Also verified not-configured (exits 1 with the shared remediation) and the
      no-tty case, which reports *"`--email` needs an interactive terminal"* rather than a
      credential problem.
- [x] **`db doctor` could not see a `PGRST_DB_MAX_ROWS` cap, and the cap returns wrong
      answers rather than errors.** *(done)* -- new **row limit** check, the eighth.
      Upstream Supabase defaults the setting to **1000** while the MCP completions tools
      request up to 5,000 rows (`src/mcp/tools/completions.rs:557,1063,1099`). PostgREST
      truncates to the cap with an HTTP 200 and nothing to say it did, so representation
      ratios and totals come out confidently short. It was the worst failure mode in this
      document because every other one announces itself.

      Detection compares two requests whose disagreement *is* the diagnosis:
      `count_rows` uses `count=exact`, which the cap does not apply to, and the new
      `DbClient::rows_returned` does a real select of one narrow column. Fewer rows back
      than asked for, against a table known to hold more, means the cap is exactly the
      number returned -- so the check reports the observed value rather than guessing it.

      Probes `completions` first and falls back to `cip_codes`
      (`ROW_LIMIT_PROBES`). `completions` is what the MCP tools actually query at 5,000,
      so it gives full coverage; `cip_codes` is seeded on every install and its 2,173 rows
      still catch the upstream default of 1000, which is the case that matters on a fresh
      stack. **The check says so when its reach is limited** -- on a `cip_codes`-only
      deployment it passes with *"a cap above 2173 cannot be seen until more data is
      imported"*, because a cap of 3000 is genuinely invisible there and implying
      otherwise would be the same class of error as asserting an unestablished cause.
      An empty deployment warns rather than failing: nothing to probe is not a fault.

      This is a `Fail`, not a `Warn`, unlike the seed-data check. A short optional seed
      leaves a usable deployment; a backend that answers large queries with wrong numbers
      does not.

      **[verified 2026-09-18]** Live against `nu.lionelle.com`:
      `✓ row limit  completions returned all 5000 rows of a 5000-row request`, 8 passed.
      The cap is not set there. The failure path cannot be exercised against that stack
      without reconfiguring it, so it is covered by a stub that answers `count=exact`
      truthfully while truncating the select -- the asymmetry the real setting produces.
      Mutation-tested: weakening the comparison to `got == 0`, and reversing the probe
      order, each fail a test.
- [x] **`db bootstrap`** *(done)* -- applies the five schema/seed files in the order they
      require, or emits them with `--print`. Implemented as **option 2 folded with 1**, as
      recommended: the ordering is what actually goes wrong, and neither half requires the
      tool to learn SQL connectivity.

      `--print` writes all five to stdout in order and **makes no network calls and reads
      no config** -- which matters, because that is the exact state someone is in before
      any of this is set up. Pipe it into `psql`, redirect it to review, or paste it into
      the cloud SQL Editor. On Supabase cloud, plain `db bootstrap` applies the files
      itself through the Management API, gated on an explicit `project_ref` and
      `management_key`; on a self-hosted endpoint it refuses and points at
      `--print | psql` rather than sending a meaningless project ref.

      The files are `include_str!`'d (~166 KB) rather than read from disk, so the command
      works for someone who installed the binary and has no checkout, and so there is
      exactly one copy of each file -- editing `docs/database/schema.sql` changes what the
      tool emits, with no drift possible. Verified byte-for-byte: all five appear
      verbatim, in order, and the only additions are `--` comments (165,784 bytes of SQL +
      1,237 of banners).

      A failed step stops the run and names the file, rather than continuing and burying
      the cause under a cascade of missing-relation errors from the files that depend on
      it. It says the files are idempotent so a re-run is safe.

      `SCHEMA_FILES` is guarded by a test that reads `docs/database/*.sql` and fails if the
      directory and the list disagree -- so a sixth file added without being wired in is
      caught rather than silently producing an incomplete schema. Mutation-tested by
      dropping a file from the list: two tests fail.

      The `project_ref`/`management_key` checks were **extracted** into
      `management_api_credentials` and are now shared with `db exec-sql`, which had the
      same thirty lines. Both now exit 1 on failure; `db exec-sql` previously returned 0.

- [x] **`db doctor`** *(done)* — `nuanalytics db doctor`. Checks run outwards from
      configuration to data, and each one gates the next so the report names a single
      cause instead of repeating it six times: configuration → reachability → anon-key
      read → session → authenticated read → schema → seed data. Verified against the live
      self-hosted backend (7 passed) and against the not-configured, disabled and
      unreachable cases, which exit 1 with the rest marked skipped.

      Logic lives in `src/core/database/doctor.rs` returning a `Report` of `Check`s, so
      ordering and the pass/fail judgement are testable without a terminal; the CLI only
      formats. Warnings do not fail the command — a deployment whose optional seed is
      short is still usable, one missing tables is not.

      Three probes were added to `DbClient` for it: `table_exists` (keys off SQLSTATE
      `42P01`, so a permission error is *not* read as "table missing"), `count_rows` (uses
      `Prefer: count=exact` and reads `Content-Range`, so counting `completions` does not
      transfer a million rows), and `anon_read_status` (sends the anon key with no user
      JWT, which is the only way to observe what RLS does to an unauthenticated read).
      `tables::ALL` now lists all 20, including the seven lookup tables from
      `lookup-seed.sql` whose absence means a half-finished bootstrap.

      Not covered, deliberately: **cert validity** as a distinct check. A bad certificate
      surfaces as a reachability failure with rustls' own message, which is accurate; a
      separate expiry check would need to parse the chain and would risk asserting a cause
      the code has not established.
- [x] **`deploy/selfhost/`** *(done, as documentation)* -- resolved the scope question the
      second way: the four verified gotchas are now written up in
      `docs/database/setup.md` under **Self-hosting notes**, and the compose stack stays
      where it is. Vendoring it into this public repo would have duplicated a live
      deployment that cannot be verified from here, and the local notes put operational
      work on the `archon` side rather than this repo.

      `setup.md` now opens by stating that both deployments are supported and that the one
      real difference is how the schema gets applied, and sends a self-hoster to the new
      section before Step 1. Documented there:
      - **`PGRST_DB_MAX_ROWS` defaults to 1000** while the MCP completions tools request
        5,000 -- the only one of the four that produces wrong answers instead of an error,
        and now also caught by `db doctor`'s row-limit check.
      - **`podman-compose` 1.6.0 hangs during image pull** (sits in `ep_poll`, no child
        process, no storage growth, exits 0 when killed, so it looks like it worked).
        Workaround: `podman pull` each image first. **[verified]**
      - **SELinux-enforcing hosts need `:ro,Z`** on the gateway config mounts; upstream
        ships plain `:ro`, so the container cannot read its own entrypoint and
        crash-loops. **[verified]**
      - **The gateway is Envoy (`api-gw`), not Kong**, and there is no `vector` or
        `analytics` service; the only dependency edge to cut when dropping Studio is
        `api-gw <- studio`. Kong-era guides do not match. **[verified]**

      Also: where to get the endpoint and anon key on a self-hosted stack
      (`API_EXTERNAL_URL` / `ANON_KEY` in the stack `.env`, not the Supabase dashboard),
      and that `db exec-sql` does not apply there.

## 5. Data integrity on a shared instance

- [ ] **Writes are last-writer-wins on natural keys.** `upsert_batch` sends
      `Prefer: resolution=merge-duplicates` (`client.rs:330,373-376`) against `program_key`
      for `programs` (`import.rs:1168,1172`) and `degree_id` for `degrees`
      (`mcp/tools/degrees.rs:567`), and only checks `status.is_success()` (`client.rs:386`).
      Two people importing their own version of the same program means the second silently
      overwrites the first's `document` JSONB. `analysis_runs` is unaffected — `run_key` is
      a content hash, so the overwrite is a no-op.
      **Superseded 2026-09-24 — enforcement is not wanted at this stage.** The fix below
      was implemented and then reverted: an ownership `USING` clause stops
      `db import --replace` overwriting a row another member created, and that has to
      keep working. `created_by` remains on the four tables as **attribution only**, with
      no policy reading it. Audit Step 6 records the decision. The original write-up
      follows; it is still the right shape if enforcement is ever turned on.

      **Fix (edit the schema, plus one small Rust change):** add
      `created_by uuid DEFAULT auth.uid()` to `programs`, `degrees`, `program_courses`,
      `program_requirements`, and replace their `FOR ALL` policies with
      `USING (created_by = auth.uid() OR created_by IS NULL)` plus a matching `WITH CHECK`.
      Leave the IPEDS reference tables and `courses` shared by design
      (`src/core/database/mod.rs:56`).

      *Revised for fresh-installs-only:* this was written as a standalone migration file
      to avoid breaking existing databases. With no such databases, edit
      `programs-schema.sql` in place instead -- a sixth file in a directory whose
      documented order already trips people is a worse outcome than a schema that is
      correct on first application. The author's own instance is the one thing that then
      needs the `ALTER`, applied by hand, which is a single known case rather than a
      supported path. The `IS NULL` disjunct stays: it is what keeps rows created before
      the column existed readable, including that instance's.
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
