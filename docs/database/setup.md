# Database Setup — Supabase

NuAnalytics keeps its IPEDS institution data, completion demographics, and stored degree
programs in [Supabase](https://supabase.com) — an open-source Firebase alternative built
on PostgreSQL.

**Two deployments are supported and behave identically: Supabase cloud, and a stack you
host yourself.** The client only ever speaks PostgREST over HTTP, so pointing it at a
different backend is configuration, not code. The one real difference is *how the schema
gets applied*, which is [Step 4](#step-4--create-the-database-schema).

If you are self-hosting, read [Self-hosting notes](#self-hosting-notes) first, then skip
Steps 1–2 and start at Step 3.

---

## Overview of credentials

There are **two separate credentials** you will deal with:

| Credential | What it is | Where it lives | Used for |
|-----------|-----------|---------------|---------|
| **Anon key** (JWT) | Public project key, starts with `eyJ...` | Config file | All requests — identifies the project to Supabase |
| **User session token** | Your personal OAuth JWT after signing in | `auth.json` (auto-managed) | Write requests — lets RLS see you as `authenticated` |

The anon key is set once per environment and stays in config.
The session token is managed automatically by `nuanalytics db login` / `logout`.

> **Important — key format:** Supabase's data API (PostgREST) requires the **JWT-format
> anon key** (`eyJhbGc...`), not the newer publishable key (`sb_publishable_...`).
> Find it under **Project Settings → API → Project API keys → anon / public**.

### Access levels

| State              | Reads | Writes |
|--------------------|-------|--------|
| Not signed in      | ✗     | ✗      |
| Signed in (`db login`) | ✓ | ✓ IPEDS import, degree storage |

Every database access — read or write — requires a logged-in user. The anon
key identifies the project to Supabase but does **not** authorise access on
its own; row-level security gates every table on
`auth.role() = 'authenticated'`. The client refreshes the access token
automatically when it's within 60s of expiry, so a single `db login` per
machine keeps long-running sessions (CLI batches, MCP servers) usable
without manual re-auth.

---

## Self-hosting notes

Skip this section if you are using Supabase cloud.

Everything below was hit in practice standing up a self-hosted stack. None of it is
NuAnalytics-specific — it is all upstream behaviour that costs an afternoon to rediscover.

**Get your endpoint and anon key from your own stack**, not the Supabase dashboard: they
are `API_EXTERNAL_URL` and `ANON_KEY` in the stack's `.env`. Then pick up at Step 3.

### Row cap — the one that returns wrong answers instead of errors

`PGRST_DB_MAX_ROWS` defaults to **1000** upstream, while the MCP completions tools request
up to 5,000 rows. PostgREST truncates to the cap with an HTTP 200 and no indication, so
totals and representation ratios come out confidently short rather than failing.

**Leave it unset, or set it to at least 10000.** `nuanalytics db doctor` checks for this
explicitly — see the **row limit** check.

### `podman-compose` 1.6.0 hangs during image pull

It sits in `ep_poll` with no child process and no storage growth, then exits 0 when killed,
which makes it look like it succeeded. Pull the images first, then bring the stack up:

```sh
podman pull <each image in the compose file>
podman-compose up -d          # completes in seconds once images are local
```

### SELinux-enforcing hosts need `:ro,Z` on the gateway config mounts

Upstream ships the Envoy config mounts as plain `:ro`. On an enforcing host the container
cannot read its own entrypoint, crash-loops, and floods the desktop with AVC denials. The
`db` mounts already carry `:Z`; the gateway ones need the same.

### The gateway is Envoy, not Kong

Current upstream uses Envoy as `api-gw`, and there is no `vector` or `analytics` service.
If you are trimming the stack — dropping Studio, say — the only dependency edge to cut is
`api-gw <- studio`. Guides written against the Kong-era compose file will not match.

### Schema and seeds

There is no Management API on a self-hosted stack, so `db exec-sql` does not apply. Use:

```sh
nuanalytics db bootstrap --print | psql "$DATABASE_URL"
```

See [Step 4](#step-4--create-the-database-schema).

---

## Step 1 — Create a Supabase account and project

1. Go to <https://supabase.com> and sign up for a free account.
2. Click **New project**.
3. Choose an **Organisation** (or create one), give the project a name (e.g. `nuanalytics-dev`), set a **database password**, and pick a region close to you.
4. Click **Create new project** and wait ~2 minutes for provisioning.

> **Tip — separate dev and prod projects:** NuAnalytics uses different config files for
> debug builds (`~/.config/nuanalytics/dconfig.toml`) and release builds
> (`~/.config/nuanalytics/config.toml`). Create two Supabase projects — one for
> development and one for production — and put each project's URL and anon key in the
> corresponding config file.

---

## Step 2 — Find your project URL and anon key

In the Supabase dashboard:

1. Go to **Project Settings** → **API** (left sidebar).
2. Under **Project URL**, copy the URL — it looks like `https://abcdefgh.supabase.co`.
3. Under **Project API keys**, copy the **anon / public** key.
   - Do **not** use the `service_role` key for NuAnalytics; that key bypasses row-level
     security and is for admin operations only.

---

## Step 3 — Configure NuAnalytics

Set your Supabase credentials in the config:

```sh
# Set the project URL
nuanalytics config set database.endpoint https://abcdefgh.supabase.co

# Set the anon key
nuanalytics config set database.anon_key eyJhbGc...

# Enable database integration
nuanalytics config set database.enabled true
```

For **debug builds** (run from source with `cargo run`), the config file is
`~/.config/nuanalytics/dconfig.toml`. For **release builds** (installed binary), it is
`~/.config/nuanalytics/config.toml`. Run the commands above in the appropriate
environment.

Verify the config was written:

```sh
nuanalytics config get database
```

---

## Step 4 — Create the database schema

Five files must be applied, **in order** — the seed files insert into tables the schema
files create, so out of sequence they fail on a missing relation. `db bootstrap` owns that
ordering so you do not have to:

```sh
# Any deployment — emits all five files in order, makes no network calls
nuanalytics db bootstrap --print | psql "$DATABASE_URL"

# Supabase cloud — applies them for you, needs project_ref + management_key
nuanalytics db bootstrap
```

`--print` works with no configuration and no session, which is the situation you are in
before any of this is set up. Pipe it to `psql`, redirect it to a file to review, or paste
the output into the Supabase **SQL Editor** (Dashboard → SQL Editor → New query → Run).

If a step fails during `db bootstrap`, it stops there and names the file rather than
carrying on and burying the cause under a cascade of missing-relation errors. Everything
is idempotent, so fix the problem and re-run.

Verify the result with `nuanalytics db doctor`, which checks all 20 tables and the seed
row counts.

### What gets created

The schema creates:
- **7 lookup tables** — `award_levels`, `institution_control`, `institution_level`, `institution_sector`, `carnegie_class`, `institution_locale`, `institution_size`
- **5 data/cache tables** — `cip_codes`, `institutions`, `completions`, `institution_completion_totals`, `degrees`
- **Row-Level Security** — RLS is enabled on every table (lookup + data) with `auth.role() = 'authenticated'` read policies and matching write policies on the four writable tables. `cip_codes` and the lookup tables have no write policy — they're seeded via SQL.

> **Key design decision — no cross-table foreign keys on IPEDS data:** IPEDS surveys
> don't guarantee that every UNITID in completions exists in the HD directory, and CIP
> codes evolve between taxonomy versions. All joins use `LEFT JOIN` rather than FK
> constraints.

### Step 4b — The five files, if you are applying them by hand

`db bootstrap` applies these for you. This is what it applies, and why the order is what
it is:

| # | File | Creates / seeds |
|---|------|-----------------|
| 1 | `docs/database/schema.sql` | Tables, indexes, RLS policies |
| 2 | `docs/database/cip-seed.sql` | `cip_codes` — 2,173 CIP 2020 codes |
| 3 | `docs/database/lookup-seed.sql` | `award_levels`, `carnegie_class`, locale, … |
| 4 | `docs/database/programs-schema.sql` | Stored-programs tables |
| 5 | `docs/database/program-lookup-seed.sql` | `degree_types` |

Files 2 and 3 insert into tables file 1 creates; file 5 seeds a table file 4 creates.

The first three must be applied **before** importing IPEDS data. The last two add the
normalized stored-programs tables; apply them before `db import` / `import_degree` (see
[Stored programs (normalized)](#stored-programs-normalized)).

On Supabase cloud an individual file can also be applied with
`nuanalytics db exec-sql <file>`. That is a cloud-only path — it uses the Management API,
which a self-hosted stack does not have.

Every object uses `IF NOT EXISTS` and policies are dropped before being recreated, so all
of this is safe to re-run on a live database.

> `docs/database/historical/` holds two migrations for databases created before a schema
> change. **A fresh install must not run them** — everything in them is already in
> `schema.sql`.

---

## Step 5 — Row-Level Security

RLS policies are included in `schema.sql` — no separate step needed for fresh installs.

**Existing database (set up before this was added):** Run
`docs/database/historical/rls-patch.sql` in the SQL Editor to add the missing policies
without touching any data. Fresh installs must not run it — see
`docs/database/historical/Readme.md`.

To verify policies are in place:

```sql
SELECT tablename, policyname, cmd
FROM pg_policies
WHERE tablename IN (
    'institutions', 'completions', 'institution_completion_totals',
    'cip_codes', 'degrees',
    'award_levels', 'institution_control', 'institution_level',
    'institution_sector', 'carnegie_class', 'institution_locale',
    'institution_size'
)
ORDER BY tablename, cmd;
```

Every table should have a `SELECT` (auth read) policy. The four writable
tables (`institutions`, `completions`, `institution_completion_totals`,
`degrees`) should also have an `ALL` (auth write) entry. `cip_codes` and
the seven lookup tables are seeded via SQL and have no write policy.

---

## Step 6 — Test connectivity

```sh
nuanalytics db status
```

Expected output when **not signed in**:
```
endpoint:      https://abcdefgh.supabase.co ✓
anon key:      set ✓
auth file:     /home/you/.config/nuanalytics/auth.json ✗ (missing)
ping:          ✗ Not signed in (no auth file at /home/you/.config/nuanalytics/auth.json). Run `nuanalytics db login` first.
→ run `nuanalytics db login`
```

Expected output when **signed in**:
```
endpoint:      https://abcdefgh.supabase.co ✓
anon key:      set ✓
auth file:     /home/you/.config/nuanalytics/auth.json ✓ (expires in 47 min as you@northeastern.edu)
ping:          ✓ authenticated read succeeded
```

If you see `401 Invalid API key`, your anon key is the wrong format — see the key format
note in the credentials section above.

### Check the whole deployment, not just the connection

`db status` answers "can I reach it". `db doctor` answers "is it set up right", which is
the question if you did not build the backend yourself:

```sh
nuanalytics db doctor
```

```
Backend:  https://db.example.edu
  endpoint from: /home/you/.config/nuanalytics/config.toml

✓ configuration        endpoint https://db.example.edu, anon key set, enabled
✓ reachability         https://db.example.edu answered
✓ anon-key read        200 — row-level security filters an unauthenticated read rather than rejecting it
✓ session              valid, you@example.edu
✓ authenticated read   succeeded
✓ schema               all 20 tables present
✓ seed data            cip_codes has all 2173 rows
✓ row limit            completions returned all 5000 rows of a 5000-row request

8 passed, 0 warning(s), 0 failed, 0 skipped
```

Each check gates the next, so a failure is reported once with the rest marked `-` skipped
rather than six errors that all share one cause. Exits 1 on any failure. Warnings do not
fail it — a short optional seed still leaves a working deployment.

Run this before Step 4 as well as after: on an unconfigured schema it tells you exactly
which of the 20 tables are missing, which is faster than reading SQL errors.

---

## Step 7 — Enable an OAuth provider (optional)

IPEDS data import requires an authenticated session. There are two ways to get one, and
neither stores a password locally:

| | Setup needed | Use when |
|---|---|---|
| **OAuth** (`db login`) | Register an OAuth app with the provider, allowlist the redirect URL — this step | You want browser sign-in, or several people share the deployment |
| **Password** (`db login --email`) | None. Every GoTrue ships the password grant | You are standing up a stack and just need in |

**If you are self-hosting and only want to get going, skip to Step 8 and use
`--email`.** This step exists because OAuth is nicer day to day, not because sign-in
requires it. Neither method creates accounts — the user must already exist on the
backend, so using `--email` does not mean enabling signup.

> **Symptom check:** If `nuanalytics db login` prints a URL and you get a **404** when
> you open it, the OAuth provider is not yet enabled in Supabase. Complete this step
> first before running `db login`, or sign in with `--email` instead.

### 7a — Add the redirect URL allowlist entry

NuAnalytics starts a temporary local HTTP server on a random port to receive the OAuth
callback. You must tell Supabase that redirecting to `127.0.0.1` is allowed.

1. In the Supabase dashboard go to **Authentication → URL Configuration**.
2. Under **Redirect URLs**, click **Add URL**.
3. Enter: `http://127.0.0.1:*`

   The `*` wildcard matches any port number, so it covers whichever random port the tool
   picks each time you log in.
4. Click **Save**.

### 7b — Configure a GitHub OAuth provider (recommended)

GitHub is the default provider for `nuanalytics db login`.

**In GitHub** (do this first):

1. Go to <https://github.com/settings/developers> → **OAuth Apps** → **New OAuth App**.
2. Fill in:
   - **Application name**: `NuAnalytics` (or any name)
   - **Homepage URL**: your Supabase project URL, e.g. `https://abcdefgh.supabase.co`
   - **Authorization callback URL**: copy this from the Supabase dashboard in the next step
     (it looks like `https://abcdefgh.supabase.co/auth/v1/callback`)
3. Click **Register application**.
4. On the next page, copy the **Client ID**.
5. Click **Generate a new client secret** and copy the secret immediately (shown once).

**In Supabase**:

1. Go to **Authentication → Providers** → find **GitHub** → toggle it **on**.
2. Paste the **Client ID** and **Client Secret** from GitHub.
3. The **Callback URL (for OAuth)** shown on this page is what you paste into GitHub's
   Authorization callback URL field above.
4. Click **Save**.

### 7c — Verify the provider is working

Open this URL in a browser (replace with your project ID):
```
https://<your-project-id>.supabase.co/auth/v1/authorize?provider=github
```

- If you see a GitHub login page → the provider is correctly configured. ✓
- If you see a **404** → the Client ID/Secret aren't saved yet, or the provider toggle is off.
- If you see a **redirect URI mismatch** error from GitHub → double-check the callback URL
  in your GitHub OAuth App settings.

### Other providers

Google, GitLab, Discord, and Azure follow the same pattern — enable in Supabase and
create an OAuth app in their respective developer consoles. Supabase's documentation
for each provider is at **Authentication → Providers** → click the provider name for
setup instructions.

## Step 8 — Sign in

Signing in is required for any **write** operation (IPEDS import, storing degrees).
Read access (MCP query tools) works without signing in.

```sh
nuanalytics db login                          # OAuth, opens browser with GitHub (default)
nuanalytics db login --provider google        # OAuth with another enabled provider
nuanalytics db login --email you@example.edu  # password, prompted — needs no OAuth provider
```

**OAuth.** NuAnalytics opens your Windows browser (on WSL/Windows: via
`powershell.exe Start-Process`). After you approve access on GitHub/Google, the browser
redirects to a temporary local server and the terminal shows
`✓ Signed in as you@email.com`. The session is saved automatically.

**Password.** `--email` prompts for the password and does not echo it. Pass only the
address — never the password, which would land in your shell history and be visible in
`ps`. This path needs no OAuth provider, which makes it the way into a stack you have
just stood up. It requires an interactive terminal, so use OAuth for unattended sign-in.

Both paths save the same session file and refresh the same way, so it makes no difference
to anything downstream which one you used. A refusal quotes the backend's own words:

```
✗ Login failed: https://db.example.edu refused the sign-in: Invalid login credentials (HTTP 400)
  The account must already exist on the backend — `--email` never creates one.
```

Verify read-write access:
```sh
nuanalytics db status
# Auth: read-write  (signed in as you@northeastern.edu)
```

Check who is signed in:
```sh
nuanalytics db whoami
```

Sign out when done:
```sh
nuanalytics db logout
```

---

## Stored programs (normalized)

`docs/database/programs-schema.sql` (+ `docs/database/program-lookup-seed.sql`,
applied in [Step 4b](#step-4b--seed-the-lookup-and-cip-tables)) add a
normalized, **queryable** projection of imported degree programs alongside the
lossless source document. Eight tables:

| Table | What it holds |
|-------|---------------|
| `degree_types` | Lookup for normalized `degree_type` codes (`BS`, `BA`, `MINOR`, …); seeded by `program-lookup-seed.sql` |
| `programs` | One row per program. The lossless unified-JSON `document` (JSONB) is the source of truth; the other columns are a queryable scalar projection. `program_key` is the deterministic idempotency key |
| `courses` | Shared per-institution course catalog, partitioned by `institution_ref` and keyed `(institution_ref, course_code)` |
| `program_courses` | M:N junction programs ↔ courses, with per-program `credit_hours_override` / `name_as_listed` |
| `program_requirements` | The requirement tree flattened by `req_path` / `parent_path`, with JSONB `selection_spec` / `req_constraints` and `is_impossible` / `allow_double_count` flags |
| `analysis_runs` | One row per `degree analyze` run of a program: `variant`, `trimmed`, `variations_run`, `sample_type`, the `degree_metrics` JSONB, plus promoted `complexity_mean` / `delay_mean` / `credits_mean` |
| `analysis_course_metrics` | Per run × course metric breakdown (promoted `*_mean` columns + full JSONB) |
| `analysis_plans` | Per run × selected exemplar plan (shortest / longest / sample) |

Design mirrors the IPEDS tables: rows link by **natural keys** (`program_key`,
`(institution_ref, course_code)`) with no cross-table foreign keys, RLS gates
every read and write on `auth.role() = 'authenticated'`, and a per-import
`generation` stamp makes re-syncing idempotent (the `programs` row is the commit
marker, bumped last).

### Importing programs

Once the tables exist and you're signed in, load degree reports with the CLI
`db import` command (or the `import_degree` MCP tool). A degree-first analysis
report (`*_report.json`) populates the program projection and, when it carries
an `analysis` block, one analysis run; a plain unified/YAML degree populates
just the program. The institution is resolved against `institutions` (the
report's `unitid`, then a name + CIP lookup); an ambiguous name lists candidate
institutions and writes nothing.

```sh
# Import one report
nuanalytics db import metrics/neu-khoury-bscs-boston_report.json

# Preview a directory of reports without writing
nuanalytics db import metrics/ --dry-run

# Pin the institution and overwrite an existing (unverified) program
nuanalytics db import report.json --unitid 167358 --replace
```

`--dry-run` reports the row counts without writing:

```
ℹ <report>.json: skipped (dry-run)
  institution:    Colorado State University (unitid 126818)
  program_key:    prog:126818|11.0701|2025-2026|BS
  variant:        full
  analysis:       10000 variations (shuffled), 7 sample plans
  rows:           82 courses, 32 requirements, 72 course-metrics
```

Overwriting an existing program requires `--replace` (unverified) or `--force`
(verified). Stored programs can then be analyzed without re-reading the file via
`degree analyze --from-db <NAME>` (see [Degree Command](../degree.md#analyze-a-stored-program---from-db)).
The full flag set (`--variant`, `--unitid`, `--institution`, `--cip`,
`--catalog`, `--degree-id`, `--force`, `--replace`, `--skip-existing`,
`--dry-run`, `-j/--jobs`) is available via `nuanalytics db import --help`; the
[MCP `import_degree` tool](../mcp.md#import_degree) exposes the same options.

---

## Configuration reference

| Config key | Description | Example |
|-----------|-------------|---------|
| `database.endpoint` | Supabase project URL | `https://abcdefgh.supabase.co` |
| `database.anon_key` | Anon (public) API key | `eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9...` |
| `database.enabled` | Enable database integration | `true` |

Config files:
- **Debug builds**: `~/.config/nuanalytics/dconfig.toml`
- **Release builds**: `~/.config/nuanalytics/config.toml`
- **Auth session (release)**: `~/.config/nuanalytics/auth.json` — auto-managed, do not edit
- **Auth session (debug)**: `.debug/dauth.json` in the working directory — auto-managed, do not commit

---

## Two things that will bite a self-hoster

Both observed in practice while standing this stack up, not inferred from reading code.

**The seed tables are read-only through the API — seed them with SQL.** Every RLS policy
in the schema is `auth.role() = 'authenticated'`, and only 12 of the 31 are write
(`FOR ALL`) policies. `cip_codes`, the seven lookup tables and `degree_types` have a read
policy and no write policy, so an authenticated `POST` to `cip_codes` returns `42501`.
That is deliberate: they are reference data. `db bootstrap` applies the seed files as SQL
for exactly this reason.

**You need the `supabase/postgres` image, not a plain `postgres:N`.** There is no `GRANT`
statement anywhere in the schema files. The tables are reachable at all only because the
Supabase image sets `ALTER DEFAULT PRIVILEGES` for `anon` / `authenticated` /
`service_role`. On a stock Postgres image every table will exist and every request will
be denied.

## Troubleshooting

| Problem | Solution |
|---------|----------|
| `Database not configured` | Set `endpoint`, `anon_key`, and `enabled = true` — see Step 3 |
| `401 Invalid API key` | Using the wrong key format — must be the JWT anon key (`eyJhbGc...`), not the publishable key (`sb_publishable_...`). Find it under **Project Settings → API → Project API keys → anon / public** |
| `404` when opening the login URL | OAuth provider not enabled — complete Step 7, or sign in with `db login --email <addr>`, which needs no provider |
| `refused the sign-in: Invalid login credentials` | The address or password is wrong, or the account has no password because it was created through OAuth. `--email` never creates an account |
| `refused the sign-in: Email not confirmed` | The account exists but its address was never confirmed — confirm it, or disable confirmation on the stack |
| `--email` needs an interactive terminal | The password prompt needs a tty, so it cannot run in CI or a pipeline. Use OAuth for unattended sign-in |
| GitHub error: `redirect_uri_mismatch` | Authorization callback URL in your GitHub OAuth App doesn't match `https://<project>.supabase.co/auth/v1/callback` |
| Login URL opens but `site can't be reached` (WSL) | Browser is Windows-side but can't reach WSL listener — ensure `WSL_DISTRO_NAME` env var is set; rebuilding from source will pick it up automatically |
| Login URL opens but callback fails | `http://127.0.0.1:*` is not in Supabase → Authentication → URL Configuration → Redirect URLs |
| `error_code=signup_disabled` in callback URL | Go to **Authentication → Settings** → enable "Allow new users to sign up", log in once, then disable again |
| `access_denied` from OAuth | Either the GitHub app denied access or the signup was blocked — check the full callback URL for `error_description` |
| `Login failed: Cannot create auth client` | Check that `database.endpoint` is the correct Supabase project URL |
| Write fails: `requires authentication` | Not signed in — run `nuanalytics db login` first |
| `Failed to ping: 404` | The `institutions` table doesn't exist — run `schema.sql` in Step 4 |
| Import fails: `PGRST102 All object keys must match` | Upgrade to latest build — older versions stripped nulls inconsistently |
| Import fails: `23502 null value in column "id"` | Upgrade to latest build — `id` field now skips serialization when null |
| Import fails: `23503 Key not present in cip_codes` | Run `cip-seed.sql` first (Step 4b), then re-import |
| Import fails: `23503 Key not present in institutions` | Some IPEDS survey UNITIDs don't appear in HD — run `schema.sql` fresh (no FK constraints) |
| Import fails: `21000 ON CONFLICT affects row twice` | Upgrade to latest build — the completions ON CONFLICT target now includes `major_num`, so a CIP appearing at both `MAJORNUM=1` and `2` is two rows rather than a conflict. Nothing is filtered out: the table holds every row of the file, ~313,000 per year |
| Upsert fails with `42501 permission denied` | RLS INSERT policy is missing — see Step 5 |
| `db doctor`: `PGRST_DB_MAX_ROWS is capping responses` | Self-hosted only. Upstream defaults the cap to 1000 while the MCP tools request 5,000, so large queries truncate silently with HTTP 200 and analytics come out wrong. Leave `PGRST_DB_MAX_ROWS` unset, or set it to at least 10000 |
| Completions totals look too low but nothing errored | Run `nuanalytics db doctor` and read the **row limit** check — a row cap produces short answers, not failures |
| Timed out waiting for browser callback | OAuth flow didn't complete in 2 minutes — run `db login` again |
