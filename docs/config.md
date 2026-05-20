# Config Command

The `config` command manages NuAnalytics configuration settings. Configuration is loaded from multiple sources with a clear precedence hierarchy.

## Configuration Hierarchy

NuAnalytics uses a three-tier configuration system with the following precedence (highest to lowest):

1. **Command-line arguments** - Override any config value for a single run
2. **Local config** (`nuanalytics.toml` in current directory) - Project-specific settings
3. **Home config** (`~/.config/nuanalytics/config.toml`) - User-wide defaults
4. **Built-in defaults** - Fallback values

This allows you to:
- Set user-wide defaults in your home config
- Override those with project-specific settings via local `nuanalytics.toml`
- Further override for a single run via CLI flags

### Local Configuration File

Create a `nuanalytics.toml` file in your project directory to set project-specific settings:

```toml
# nuanalytics.toml - Local project configuration
[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[logging]
level = "debug"

[degree_analysis]
max_plans = 500
sampling_strategy = "stratified"
```

The local config only overrides values that are explicitly set - empty or default values are ignored, allowing the home config to provide fallback values.

## Overview

The `config` command allows you to:

- **View** current configuration values
- **Set** configuration values that persist across runs
- **Unset** configuration values to reset them to defaults
- **Reset** all configuration to defaults

## Subcommands

### `config get [KEY]`

Display configuration values.

**Usage:**

```bash
# Display all configuration
nuanalytics config get

# Display a specific configuration value
nuanalytics config get level
nuanalytics config get file
nuanalytics config get metrics_dir
```

**Example Output:**

```
=== Configuration ===

[logging]
  level = "warn"
  file = ""
  verbose = false

[database]
  endpoint = ""
  anon_key = ""
  enabled = false
  auth_file = "~/.config/nuanalytics/auth.json"

[paths]
  metrics_dir = "metrics"
  reports_dir = "reports"
```

### `config set <KEY> <VALUE>`

Set a configuration value that persists in the config file.

**Usage:**

```bash
nuanalytics config set level debug
nuanalytics config set metrics_dir /path/to/metrics
nuanalytics config set database.anon_key eyJhbGc...
```

**Supported Configuration Keys:**

- `level` - Set logging verbosity (error, warn, info, debug)
- `verbose` - Enable verbose output (true/false)
- `file` - Path to log file
- `metrics_dir` - Default output directory for CSV metrics files
- `reports_dir` - Default output directory for report files (HTML, PDF, Markdown)
- `database.endpoint` - Supabase project URL (e.g. `https://abcdefgh.supabase.co`)
- `database.anon_key` - Supabase anonymous (public) key, JWT format starting with `eyJhbGc...` (legacy alias: `database.token`)
- `database.enabled` - Whether to enable database tools (true/false)
- `database.auth_file` - Path to the auth session file populated by `nuanalytics db login`

> Setting `endpoint` and `anon_key` enables the database tools but
> does not authorise access on its own. After configuring, run
> `nuanalytics db login` once to save your OAuth session; the client
> refreshes the JWT automatically near expiry.

### `config unset <KEY>`

Reset a configuration value to its default.

**Usage:**

```bash
nuanalytics config unset level
nuanalytics config unset database.anon_key
```

### `config reset`

Reset all configuration values to their defaults. Requires confirmation.

**Usage:**

```bash
nuanalytics config reset
```

This command will prompt you to confirm before resetting all settings.

## Configuration Priority

When NuAnalytics runs, configuration is applied in this order (highest priority first):

1. **CLI Flags** - Runtime flags like `--log-level` (most specific, highest priority)
2. **Local Config** - `nuanalytics.toml` in the current directory (project-specific)
3. **Home Config** - `~/.config/nuanalytics/config.toml` (user defaults)
4. **Built-in Defaults** - Compiled-in defaults (lowest priority)

### Example: Priority in Action

```bash
# Set logging level in config file
nuanalytics config set level warn

# Override at runtime (takes precedence over config file)
nuanalytics planner input.csv --log-level debug

# Override and persist in config file
nuanalytics config set level debug
```

## Runtime config Flags

In addition to `config` subcommands, you can control config at runtime:

- `--log-level <LEVEL>` - Set runtime log level without saving to config (error, warn, info, debug)
- `--verbose` / `-v` - Enable verbose output for current run
- `--debug` - Enable debug-level logging and runtime debug mode
- `--log-file <PATH>` - Write logs to a file for current run
- `--config-level <LEVEL>` - Set logging level and save to config file
- `--config-verbose` - Set verbose flag and save to config file
- `--config-log-file <PATH>` - Set log file path and save to config file
- `--metrics-dir <DIR>` - Override metrics output directory for this run
- `--reports-dir <DIR>` - Override reports output directory for this run
- `--db-anon-key <KEY>` - Override database anon key at runtime (short form)
- `--db-endpoint <URL>` - Override database endpoint at runtime (short form)
- `--config-db-anon-key <KEY>` - Set database anon key and save to config file
- `--config-db-endpoint <URL>` - Set database endpoint and save to config file


### Examples:

```bash
# Runtime logging (doesn't modify config)
nuanalytics planner input.csv --log-level debug

# Persistent logging (saves to config)
nuanalytics config set level debug

# Both: Set config AND use different level for this run
nuanalytics --log-level info planner input.csv   # Uses info just this time

# Enable debug mode (both logging and runtime)
nuanalytics -debug planner input.csv -
```

## Configuration File Locations

Configuration is loaded from these locations:

### Home Configuration (user defaults)
- **Linux**: `~/.config/nuanalytics/config.toml` (or `dconfig.toml` in debug builds)
- **macOS**: `~/Library/Application Support/nuanalytics/config.toml`
- **Windows**: `%APPDATA%\nuanalytics\config.toml`

### Local Configuration (project-specific)
- **All platforms**: `nuanalytics.toml` in the current working directory

To view your home config file path:

```bash
nuanalytics config get
# The file location is displayed in the output
```

## Default Configuration

NuAnalytics embeds two default config files in the binary — one for
release builds, one for debug — and falls back to them when neither the
home nor the local config file overrides a given key. The defaults
below are the actual contents of `src/assets/DefaultCLIConfigRelease.toml`
and `DefaultCLIConfigDebug.toml`.

**Release Mode** (used by the installed `nuanalytics` binary):
```toml
[logging]
level = "warn"
file = "$NU_ANALYTICS/nuanalytics.log"
verbose = false

[database]
endpoint = "https://oaaqxtzkfcjcosilpbwi.supabase.co"
anon_key = "eyJhbGciOiJIUzI1NiI..."   # JWT-format anon key — see below
enabled = true
auth_file = "$NU_ANALYTICS/auth.json"
management_key = ""                    # set this only if you run `db exec-sql`

[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[audit]
prerequisite_chain_threshold = 4

[degree_analysis]
calc_strategy = "median"
sample_plan_count = 5
max_plans = 1000
ignore_duplicates = true
sampling_strategy = "shuffled"
```

**Debug Mode** (used by `cargo run` and other debug builds — output goes
under `.debug/` to avoid mixing with release output):
```toml
[logging]
level = "debug"
file = ".debug/nuanalytics.debug.log"
verbose = true

[database]
endpoint = "https://oaaqxtzkfcjcosilpbwi.supabase.co"
anon_key = "eyJhbGciOiJIUzI1NiI..."
enabled = true
auth_file = ".debug/dauth.json"        # separate from any active release session
management_key = ""

[paths]
metrics_dir = ".debug/metrics"
reports_dir = ".debug/reports"

[audit]
prerequisite_chain_threshold = 4

[degree_analysis]
calc_strategy = "median"
sample_plan_count = 3                  # smaller for faster debug iteration
max_plans = 1000
ignore_duplicates = true
sampling_strategy = "shuffled"
```

> The shipped `anon_key` and `endpoint` point at the project's shared
> development database. They identify the project but do not grant access
> — you still need `nuanalytics db login` to obtain a user JWT before any
> database tool will work (see [Database setup](database/setup.md) for
> the full flow). `management_key` is a separate Supabase Personal Access
> Token used by `db exec-sql` for DDL; generate one at
> <https://app.supabase.com/account/tokens> only if you need it.

## Audit Configuration

The `[audit]` section controls the `degree audit` command:

| Key | Description | Default |
|-----|-------------|---------|
| `prerequisite_chain_threshold` | Minimum chain depth to flag as "deep". Courses whose longest prereq chain is at least this many steps long are highlighted in audit reports. | 4 |

## Degree Analysis Configuration

The `[degree_analysis]` section controls the `degree analyze` command:

| Key | Description | Default (release) |
|-----|-------------|-------------------|
| `calc_strategy` | Aggregate metric strategy across generated plans: `"median"` or `"mean"`. Median is more robust to outlier plans. | `"median"` |
| `max_plans` | Hard cap on plan generation. Programs with many electives explode combinatorially; this stops runaway generation. | 1000 |
| `sample_plan_count` | How many random plans to export in full (term schedules + CSVs). Does not affect aggregate stats — those use every analysed plan. | 5 (release) / 3 (debug) |
| `sampling_strategy` | Plan enumeration order before sampling: `"sequential"`, `"shuffled"`, or `"stratified"`. | `"shuffled"` |
| `ignore_duplicates` | Skip plans that are permutations of the same course set. Strongly recommended — reduces noise. | true |

**Sampling Strategies:**

- `sequential` - Enumerate plans in natural order (may bias statistics toward early options)
- `shuffled` - Randomize order before sampling (recommended for unbiased stats)
- `stratified` - Ensure coverage across elective option space

**Examples:**

```bash
# Set maximum plans to generate
nuanalytics config set degree_analysis.max_plans 5000

# Use mean instead of median for aggregate metrics
nuanalytics config set degree_analysis.calc_strategy mean

# Use stratified sampling for better coverage
nuanalytics config set degree_analysis.sampling_strategy stratified

# Export more random samples
nuanalytics config set degree_analysis.sample_plan_count 20

# Disable deduplication for full enumeration
nuanalytics config set degree_analysis.ignore_duplicates false
```

## Common Workflows

### Set Up Logging to File

```bash
nuanalytics config set file ~/.logs/nuanalytics.log
nuanalytics config set level debug
```

### Configure Default Output Directories

```bash
# Set metrics output directory
nuanalytics config set metrics_dir /home/user/analysis/metrics

# Set reports output directory
nuanalytics config set reports_dir /home/user/analysis/reports
```

### Set Database Credentials

```bash
nuanalytics config set database.endpoint https://abcdefgh.supabase.co
nuanalytics config set database.anon_key eyJhbGc...
nuanalytics config set database.enabled true

# Then obtain a user session (saved to auth_file):
nuanalytics db login
```

### Debug a Problem

```bash
# Enable debug logging for investigation
nuanalytics config set level debug
nuanalytics planner input.csv

# View the output
cat ~/.logs/nuanalytics.log
```
