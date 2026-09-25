# Command Line Program Design


## Commands
The command line program has the following top-level commands:

* `config`   — manage persistent settings stored in `~/.config/nuanalytics/`
* `init`     — scaffold a new research project (degrees/, plans/, MCP wiring, skills)
* `planner`  — handle a single CSV plan, output the traditional curricular-analytics report
* `degree`   — operate on degree YAML files via nested subcommands (see below)
* `db`       — manage and read the Supabase backend (see below)
* `mcp`      — run the Model Context Protocol server over stdio

### `db` subcommands

Access:

* `db login`        — sign in and save a session JWT under `auth_file`; `--email` uses a
  password grant, so it works on a stack with no OAuth app registered
* `db logout`       — clear the saved session (revokes nothing server-side)
* `db whoami`       — show the signed-in user (no DB round-trip)

Deployment:

* `db bootstrap`    — apply the schema and seed files in dependency order; `--print`
  emits them for `psql` and makes no network calls
* `db status`       — endpoint / anon key / auth file / probe. Exits 1 on a failed read
* `db doctor`       — full deployment diagnosis, each check gating the next
* `db exec-sql`     — run arbitrary SQL via the Supabase **Management API**. Cloud only —
  it needs a project ref, which a self-hosted endpoint does not have

Data in:

* `db ipeds-import` — bulk import IPEDS HD + completions CSVs
* `db import`       — import degree analysis reports into the program tables
* `db validate`     — check stored data against the IPEDS file it came from
* `db prune`        — drop old analysis runs, keeping a bounded history per program

Data out:

* `db query`        — read the database (see below)

#### `db query`

Every subcommand prints JSON on stdout by default; `--format table` gives aligned columns
for a terminal. Logs go to stderr, so `db query ... | jq` works unmodified. A failure is
reported as a JSON payload with an `error` key **and** exit status 1, so a script can
trust either.

* `db query schools`      — institutions. `--name` is always a case-insensitive
  substring (`hawaii`), never exact; plus `--state`, `--control`, `--carnegie-class`,
  `--hbcu`, `--tribal`, `--with-programs`, `--limit`
* `db query degrees`      — stored programs: `--school <unitid>`, `--cip`,
  `--catalog-year`, `--degree-type`, `--kind`, `--limit`
* `db query metrics`      — analysis runs for one program: `--degree` (program key or
  degree id), `--variant`, `--all`
* `db query demographics` — IPEDS completions by race and gender: `--school`, `--cip`,
  `--cip-codes`, `--year`, `--award-level`, `--group-by`, `--raw`
* `db query cip`          — the CIP catalogue: `--search`, `--prefix`, `--limit`
* `db query lookup`       — an IPEDS lookup table, i.e. what the numeric codes mean

Two behaviours are worth knowing before reading the output.

**`db query schools --with-programs` restricts as well as annotates.** Only 582 of 6,515
institutions hold a stored program, so a listing that merely attached them would be mostly
empty arrays; the flag returns just the schools that have programs, and `--limit` counts
those. It is a client-side join — `programs.unitid` carries no foreign key, so PostgREST
embedding is unavailable — driven from `programs` and batched, because the alternative
(scanning all 6,515 institutions to intersect) would cross `PGRST_DB_MAX_ROWS` and
truncate silently.

**`db query metrics` defaults to the newest run per variant.** Runs *append* — importing a
program again adds a row rather than replacing one — so a program accumulates runs across
analyzer versions. `--all` shows the history.

**`db query demographics` ratios are never about enrolment.** The baseline is always the
group's share of *all-major completions* (`institution_completion_totals`); this database
holds no enrolment data. The output columns are nonetheless named `enrolled`,
`total_enrolled` and `enrollment_pct` — a historical misnomer, not a second measure. `--group-by`
picks what a row is — one aggregate (`total`), one institution (`school`), or one CIP code
at one school (`cip`) — and the three engines accept different filters. A filter the
chosen grouping cannot honour is **refused by name** rather than silently dropped, because
silently dropping `--hbcu` would answer for every school under a heading that said
otherwise.

### `degree` subcommands
* `degree validate    <FILES>...`         — structural validation (schema, prereq cycles, cross-listings)
* `degree audit       <FILES>...`         — validation + missing prereqs + deep-chain detection
* `degree print-graph <FILES>...`         — print the prerequisite graph as an association list
* `degree analyze     <FILES>...`         — full plan enumeration, metrics, HTML report, CSV exports
* `degree trim        <FILE> [-o <PATH>]` — collapse alternatives to one walkable path per course;
  `-o` accepts a file or a directory (auto-creates `<stem>_trimmed.<ext>` for batches)

> **Breaking change (v0.4.0):** `degree` was previously a flat command with
> action flags (`degree --validate`, `degree --analyze`, …). It is now a
> subcommand dispatcher; the flag form no longer works.


### Future Additions
* `db query --sql <file>` - run a read-only SQL file. Needs a `STABLE` Postgres function
  (`query_readonly`) plus a `DbClient::rpc` path, because PostgREST exposes no SQL
  endpoint and `db exec-sql` is cloud-only. `exec-sql` stays as the cloud write/DDL path
* school - handles schools and programs within schools - degrees are attached to those programs
* stats  - handles some built in queries and stats requests across the various schools and programs stored in db



## Config

The `config` command manages persistent settings stored in
`~/.config/nuanalytics/config.toml` (Linux/macOS) or
`%APPDATA%\nuanalytics\config.toml` (Windows). Any common command-line
argument can be persisted to config to ensure it's always included in a
run. Also holds options such as:
- Supabase project credentials (`endpoint` + `anon_key`) for database tools
- Default paths and directories
- Logging preferences
- Other program-wide settings

Settings can be used by the CLI or other means to access the system (e.g., MCP server).

### Config Subcommands

#### `config` (no args)
Prints the entire current configuration in a readable format.

```bash
$ nuanalytics config
# Output:
# [logging]
# level = "warn"
# file = null
# verbose = false
#
# [database]
# endpoint = "https://abcd.supabase.co"
# anon_key = "eyJhbGc..."
```

#### `config <key>`
Prints the value of a single configuration key.

```bash
$ nuanalytics config log-level
warn

$ nuanalytics config database.anon_key
(prints value or "not set")
```

#### `config set <key> <value>`
Sets a configuration key to a new value and persists it to disk.

```bash
$ nuanalytics config set log-level debug
✓ Updated log-level to "debug"

$ nuanalytics config set database.anon_key "eyJhbGc..."
✓ Updated database.anon_key

$ nuanalytics config set verbose true
✓ Updated verbose to true
```

#### `config unset <key>`
Removes a configuration key (resets to default).

```bash
$ nuanalytics config unset log-file
✓ Removed log-file (will use default)
```

#### `config reset`
Resets all configuration to defaults.

```bash
$ nuanalytics config reset
⚠ This will erase all custom settings. Continue? (y/n)
y
✓ Configuration reset to defaults
```

### Configuration File

Location: `~/.config/nuanalytics/config.toml` (Linux/macOS) or
`%APPDATA%\nuanalytics\config.toml` (Windows). Debug builds use
`~/.config/nuanalytics/dconfig.toml` instead so dev and release configs
don't collide.

Example structure:
```toml
[logging]
level = "warn"
file = ""
verbose = false

[database]
endpoint = "https://abcdefgh.supabase.co"
anon_key = "eyJhbGc..."
enabled = true
# `auth_file` defaults to ~/.config/nuanalytics/auth.json (release)
# or .debug/dauth.json (debug). Set explicitly to override.

[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[degree_analysis]
max_plans = 1000
sample_plans = 5
```

> Setting `endpoint` and `anon_key` enables the database tools but does
> not authorise access on its own. Run `nuanalytics db login` once to
> obtain a user session; the client refreshes the JWT automatically
> when it's near expiry.

### CLI Flag Precedence

1. Command-line flags (highest priority)
2. Environment variables (e.g., `NU_LOG_LEVEL`)
3. Config file values
4. Built-in defaults (lowest priority)
