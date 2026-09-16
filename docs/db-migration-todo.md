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

## 1. Importer defects [verified]

Found by running `db ipeds-import` for 2022/2023/2024/2025.

- [ ] **Non-UTF-8 source files are rejected outright.** `HD2022.zip` failed with
      `Cannot decode zip entry: stream did not contain valid UTF-8`. The file is CP1252;
      the offending byte is `\xe9` (the `é` in a trustee name) at byte 64468. One accented
      character aborts a 6,256-row import. IPEDS has historically shipped CP1252.
      **Fix:** fall back to CP1252/Latin-1 when UTF-8 decoding fails.
      Workaround today: `iconv -f CP1252 -t UTF-8` then repack the zip.
- [ ] **Partial failure exits 0.** For 2022, institutions failed while completions
      succeeded and the process still exited 0, so a batch loop reports success on a
      half-finished import. The only symptom was 494 orphaned `completions` rows referencing
      `unitid`s absent from `institutions`.
      **Fix:** return non-zero when any sub-import fails.
- [ ] **IPEDS filename casing is inconsistent.** `hd2022.csv` / `hd2025.csv` are lowercase;
      `HD2023.csv` / `HD2024.csv` are uppercase. `--dir` auto-detection must be
      case-insensitive or it silently skips files.
- [ ] **`docs/database/ipeds-data.md:121-122` direct-download URLs fail for the newest
      release.** `datacenter/data/HD2025.zip` 404s even with browser headers and a referer,
      while `HD2024.zip` succeeds by the same method. The current year must come through the
      Data Center UI. Document that rather than implying `curl -O` always works.

## 2. Backend portability

- [ ] **`db exec-sql` assumes Supabase cloud.** `SUPABASE_MGMT_API_BASE`
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
- [ ] **Normalize a trailing slash on `database.endpoint`.** `auth.rs:165` trims it and the
      SDK does too, but `client.rs:374` and `:411` do not, so an endpoint ending in `/`
      produces `…//rest/v1/…`. `Config::set` does no validation. Normalize in one place.
- [ ] **Remove the baked-in default endpoint and anon key** from
      `src/assets/DefaultCLIConfigRelease.toml` and `DefaultCLIConfigDebug.toml`. With a
      user-supplied backend there is no correct default, and shipping one is what makes the
      next two bugs possible:
      - `merge_defaults` (`src/core/config.rs:285-290`) refills an *empty* endpoint from the
        compiled-in default, and `load_home_config` (`:539-554`) then saves it — so clearing
        the endpoint silently restores the default and persists it.
      - `config unset database.endpoint` resets to that default (pinned by the test at
        `config.rs:1115-1123`). Users must be told to `set`, never `unset`.

## 3. Diagnosability

- [ ] **`db whoami` does not print the endpoint** (`db.rs:355-372`). `db status` already
      does (`:482`). With two possible backends, "which database am I talking to" is the
      first question in any support exchange.
- [ ] **Report which config file won.** Precedence is local `nuanalytics.toml` > home
      `config.toml` > compiled defaults (`config.rs:492-530`), but `config set` writes to
      *home* (`:667-673`). A user with a project-local file can run `config set`, see it
      succeed, and still hit the old backend. `db status` should name the source file.
- [ ] **`db status` help text is wrong.** `args.rs:641` advertises row counts it doesn't
      display.
- [ ] **The forced token refresh cannot force.** `client.rs:285` catches a 401 and calls
      `reauthenticate(true)`, but the path runs through `load_and_refresh`, which
      short-circuits on `state.is_valid()` (`auth.rs:214-217`) and returns the same dead
      token. A stale-but-clock-valid token yields a bare `PostgREST error (401)` with no
      "run `db login`" hint, and the retry is wasted.
      **Fix:** call `refresh_session` directly when `force` is set, and map a post-retry 401
      to `DatabaseError::NotAuthenticated`.
- [ ] **MCP database outages are misreported as user error.** `run_server` builds the
      `DbClient` once at startup (`src/mcp/server.rs:879-891`) and on failure sets it to
      `None` for the process lifetime; every tool then returns `db_not_configured_response`
      (`:795-806`), which blames config or login. It is also intermittent: a fresh token
      starts fine and fails per-call, an hour-old token starts with no database tools at
      all. Name the endpoint and distinguish unreachable / not-configured / not-logged-in.

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

## Notes

- All 31 RLS policies are `auth.role() = 'authenticated'` — 16 in `schema.sql`, 15 in
  `programs-schema.sql`. 12 are write (`FOR ALL`) policies, so `cip_codes`, the 7 lookup
  tables, and `degree_types` are read-only through the API and **must** be seeded via SQL,
  not the client. Confirmed in practice: an authenticated POST to `cip_codes` returns
  `42501`. **[verified]**
- There is no `GRANT` statement in any schema file. The tables are usable only because the
  `supabase/postgres` image sets `ALTER DEFAULT PRIVILEGES` for `anon`/`authenticated`/
  `service_role`. A plain `postgres:N` image will not work. **[verified]**
