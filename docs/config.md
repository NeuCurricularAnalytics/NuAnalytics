# Config Command

`nuanalytics config` reads and changes NuAnalytics settings. Settings come from several
places; this page says which wins, what each setting does, and the surprises.

## Where settings come from

First match wins:

1. **Command-line flags**, for one run (`--log-level debug`, `--metrics-dir ./out`).
2. **The project file**, `nuanalytics.toml` in the current directory.
3. **The user file**: `~/.config/nuanalytics/config.toml` on Linux,
   `~/Library/Application Support/nuanalytics/config.toml` on macOS,
   `%APPDATA%\nuanalytics\config.toml` on Windows. Debug builds read `dconfig.toml` in the
   same directory instead.
4. **Built-in defaults**, compiled into the binary.

A key a file mentions overrides the tiers below it, whatever its value — writing
`max_plans = 1000` or `verbose = false` in a project file takes effect even though both
equal a default. The one exception: **a blank string counts as not set**, so a blank
`endpoint` in one file never erases a working endpoint from another. To override a lower
tier, write a value; to say nothing, leave the key out or blank.

Surprises:

- **`config set` writes the user file, never the project file.** With a project file that
  sets the same key, `config set` succeeds and the project file still wins. Edit
  `nuanalytics.toml` directly to change a project setting.
- **`config set` and `config unset` change one key and nothing else.** They read the user
  file on its own — never the merged view, so neither a project file's values nor override
  flags reach it — change the key, and save. The confirmation names the file written, and
  adds a note when a `nuanalytics.toml` here outranks it. A user file that does not parse
  is reported and left as it is. (Before 0.6.0 they saved the merged configuration, copying
  a project's settings into the user's defaults.)
- **The user file is created on first run**, and keys added in a newer version are filled
  in from the defaults and saved back. It is written with default permissions (typically
  0644); the sign-in session file is written 0600.

`nuanalytics db status` names the file that supplied the database endpoint, which is the
quickest way to see which tier is in effect.

A project file:

```toml
# nuanalytics.toml
[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[logging]
level = "debug"

[degree_analysis]
max_plans = 10000
```

## Subcommands

### `config` / `config get [KEY]`

With no key, prints every setting. With a key, prints that value.

```bash
nuanalytics config
nuanalytics config get level
nuanalytics config get degree_analysis.max_plans
```

### `config set <KEY> <VALUE>`

Writes a value to the user file.

```bash
nuanalytics config set level debug
nuanalytics config set metrics_dir /path/to/metrics
nuanalytics config set database.endpoint https://nu.example.org
```

### `config unset <KEY>`

Resets one value to its default, in the user file.

### `config reset`

Resets every value to its default, after asking for confirmation.

## Keys

Keys are accepted bare (`level`) or with their section (`logging.level`).

| Key | Section | Meaning |
|---|---|---|
| `level` | `logging` | Log level: `error`, `warn`, `info` or `debug`. |
| `file` | `logging` | Log file path. `$NU_ANALYTICS` expands to the configuration directory. |
| `verbose` | `logging` | Extra detail on stdout (`true`/`false`). |
| `endpoint` | `database` | The backend's URL — a Supabase cloud project or a self-hosted stack. |
| `anon_key` | `database` | The backend's anonymous key (alias: `token`). It identifies the project; it does not grant access. |
| `auth_file` | `database` | Where `db login` saves the session. Supports `$NU_ANALYTICS`. |
| `management_key` | `database` | A Supabase Personal Access Token, used only by `db exec-sql` and plain `db bootstrap` on Supabase cloud. |
| `project_ref` | `database` | The Supabase cloud project reference those two commands use. Leave blank for a self-hosted backend. |
| `metrics_dir` | `paths` | Where `planner` and `degree analyze` write metrics. |
| `reports_dir` | `paths` | Where reports are written. |
| `prerequisite_chain_threshold` | `audit` | `degree audit` flags prerequisite chains at least this long. Default 4. |
| `calc_strategy` | `degree_analysis` | `median` (default) or `mean`, recorded in each run's parameters. Nothing else reads it yet: every report carries both the median and the mean. |
| `max_plans` | `degree_analysis` | The most plans `degree analyze` analyzes. A degree with more is sampled: under `shuffled`, a random sample of this size, seeded so the same degree gives the same sample. Default 1000. |
| `sample_plan_count` | `degree_analysis` | How many random plans to export in full (term schedules and CSVs). Statistics use every analyzed plan regardless. Default 5. |
| `ignore_duplicates` | `degree_analysis` | Skip a plan whose set of courses equals one already analyzed. Default `true`. |
| `sampling_strategy` | `degree_analysis` | Plan order before sampling: `shuffled` (default) or `sequential`, which favours the first options listed. `stratified` is accepted but not implemented; it behaves as `shuffled`. |

`project_ref` is never derived from `endpoint`: a custom-domain cloud project has no
`.supabase.co` in its URL, and a self-hosted host would give a meaningless value.

The files also accept `[database] enabled = false`, which turns every database feature
off; it has no `config set` key.

## Command-line overrides

These flags apply to one run and are never saved:

| Flag | Overrides |
|---|---|
| `--log-level <LEVEL>`, `--config-level <LEVEL>` | `logging.level` |
| `-v`, `--verbose`; `--config-verbose <true\|false>` | `logging.verbose` |
| `--debug` | debug logging plus runtime debug output |
| `--log-file <PATH>`, `--config-log-file <PATH>` | `logging.file` |
| `--db-endpoint <URL>`, `--config-db-endpoint <URL>` | `database.endpoint` |
| `--db-anon-key <KEY>`, `--config-db-anon-key <KEY>` | `database.anon_key` |
| `--metrics-dir <DIR>` | `paths.metrics_dir` |
| `--reports-dir <DIR>` | `paths.reports_dir` |

They are global, so they go before the subcommand:

```bash
nuanalytics --log-level debug planner input.csv
nuanalytics --db-endpoint http://localhost:8000 db status
```

## The built-in defaults

Two default files are compiled in: `src/assets/DefaultCLIConfigRelease.toml` for release
builds and `DefaultCLIConfigDebug.toml` for debug builds. They differ only in where output
goes and how chatty they are:

| | release | debug |
|---|---|---|
| `logging.level` | `warn` | `debug` |
| `logging.file` | `$NU_ANALYTICS/nuanalytics.log` | `.debug/nuanalytics.debug.log` |
| `logging.verbose` | `false` | `true` |
| `database.auth_file` | `$NU_ANALYTICS/auth.json` | `.debug/dauth.json` |
| `paths.metrics_dir` | `./metrics` | `.debug/metrics` |
| `paths.reports_dir` | `./reports` | `.debug/reports` |
| `degree_analysis.sample_plan_count` | 5 | 3 |

**Both ship `endpoint` and `anon_key` blank, and must.** The repository is public, and a
non-blank default would be worse than leaked: an empty endpoint or anon key in a user's
file is refilled from the defaults and saved, so a default value would silently re-point a
self-hosted user at that backend. With them blank, the database commands report
`Database not configured. Set endpoint and anon_key in [database] config.` until you set
them. Every other feature works with no backend.

## Common tasks

Point at a backend and sign in:

```bash
nuanalytics config set database.endpoint https://nu.example.org
nuanalytics config set database.anon_key <anon key>
nuanalytics db login --email you@example.org      # or `db login` for OAuth
nuanalytics db status
```

See [Database setup](database/setup.md) for standing up the backend itself.

Log to a file while investigating a problem:

```bash
nuanalytics config set file ~/logs/nuanalytics.log
nuanalytics config set level debug
```

Analyze more plans per degree:

```bash
nuanalytics config set degree_analysis.max_plans 10000
```
