# Config Command

The `config` command manages NuAnalytics configuration settings. Configuration is stored in your user's config directory and can be overridden via CLI flags.

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
nuanalytics config get out_dir
```

**Example Output:**

```
=== Configuration ===

[logging]
  level = "warn"
  file = ""
  verbose = false

[database]
  token = ""
  endpoint = ""

[paths]
  metrics_dir = "metrics"
  reports_dir = "reports"
```

### `config set <KEY> <VALUE>`

Set a configuration value that persists in the config file.

**Usage:**

```bash
nuanalytics config set level debug
nuanalytics config set out_dir /path/to/output
nuanalytics config set token your-api-token
```

**Supported Configuration Keys:**

- `level` - Set logging verbosity (error, warn, info, debug)
- `verbose` - Enable verbose output (true/false)
- `file` - Path to log file
- `metrics_dir` - Default output directory for CSV metrics files
- `reports_dir` - Default output directory for report files (HTML, PDF, Markdown)
- `token` - API token for database integration  (Does nothing at this point - future update)
- `endpoint` - Database API endpoint URL        (Does nothing at this point - future update)

### `config unset <KEY>`

Reset a configuration value to its default.

**Usage:**

```bash
nuanalytics config unset level
nuanalytics config unset token
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
2. **Config Overrides** - Flags like `--config-level` that write to config file
3. **Config File** - Values persisted via `config set`
4. **Defaults** - Built-in defaults (lowest priority)

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
- `--config-out-dir <DIR>` - Set output directory and save to config file
- `--db-token <TOKEN>` - Override database token at runtime (short form)
- `--db-endpoint <URL>` - Override database endpoint at runtime (short form)
- `--config-db-token <TOKEN>` - Set database token and save to config file
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

## Configuration File Location

Configuration is stored in:

- **Linux/macOS**: `~/.config/nuanalytics/config.toml`
- **Windows**: `%APPDATA%\nuanalytics\config.toml`

To view your config file path:

```bash
nuanalytics config get
# The file location is displayed in the output
```

## Default Configuration

When you first run NuAnalytics, it uses these defaults (which depend on build mode):

**Release Mode:**
```toml
[logging]
level = "warn"
verbose = false
file = "$NU_ANALYTICS/nuanalytics.log"

[database]
token = ""
endpoint = ""

[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"
```

**Debug Mode:**
```toml
[logging]
level = "debug"
verbose = true
file = ".debug/nuanalytics.debug.log"

[database]
token = ""
endpoint = ""

[paths]
metrics_dir = ".debug/metrics"
reports_dir = ".debug/reports"
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
nuanalytics config set endpoint https://your-api.example.com
nuanalytics config set token your-secret-token
```

### Debug a Problem

```bash
# Enable debug logging for investigation
nuanalytics config set level debug
nuanalytics planner input.csv

# View the output
cat ~/.logs/nuanalytics.log
```
