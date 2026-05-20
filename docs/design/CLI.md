# Command Line Program Design


## Commands
The command line program has the following top-level commands:

* `config`   — manage persistent settings stored in `~/.config/nuanalytics/`
* `init`     — scaffold a new research project (degrees/, plans/, MCP wiring, skills)
* `planner`  — handle a single CSV plan, output the traditional curricular-analytics report
* `degree`   — operate on degree YAML files via nested subcommands (see below)
* `db`       — manage Supabase access: `login`, `logout`, `whoami`, `status`, `ipeds-import`, `exec-sql`
* `mcp`      — run the Model Context Protocol server over stdio

### `db` subcommands
* `db login`        — OAuth sign-in, saves a session JWT under `auth_file`
* `db logout`       — clear the saved session
* `db whoami`       — show the signed-in user (no DB round-trip)
* `db status`       — diagnostic: endpoint / anon key / auth file / probe
* `db ipeds-import` — bulk import IPEDS HD + completions CSVs into Supabase
* `db exec-sql`     — run arbitrary SQL via the Supabase Management API (admin)

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
