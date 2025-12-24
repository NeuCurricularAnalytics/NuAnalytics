# Command Line Program Design


## Commands
The command line program will have different command options, they will be

* config - options to update the configuration for the client, often modifying a file in ~/.nuanalytics
* plan   - handles a single plan, and outputs the tradition CSV seen in the original curricular analytics
* degree - takes in a degree, can generate the new numbers /plans based on the degree - generates states files in addition to requests curricular sheets.


### Future Additions
* school - handles schools and programs within schools - degrees are attached to those programs
* stats  - handles some built in queries and stats requests across the various schools and programs stored in db



## Config

The `config` command manages persistent settings stored in `~/.nuanalytics/config.toml` (or similar). Any common command line argument can be persisted to config to ensure it's always included in a run. Also holds options such as:
- Token for access to online database (Firebase, etc.)
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
# token = "..."
```

#### `config <key>`
Prints the value of a single configuration key.

```bash
$ nuanalytics config log-level
warn

$ nuanalytics config database.token
(prints value or "not set")
```

#### `config set <key> <value>`
Sets a configuration key to a new value and persists it to disk.

```bash
$ nuanalytics config set log-level debug
✓ Updated log-level to "debug"

$ nuanalytics config set database.token "secret_xyz"
✓ Updated database.token

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

Location: `~/.nuanalytics/config.toml`  or for windows  `%APPDATA%\nuanalytics\config.toml`

Example structure:
```toml
[logging]
level = "warn"
file = null
verbose = false

[database]
token = ""
endpoint = "https://firebasedb.example.com"

[paths]
plans_dir = "./plans"
output_dir = "./output"
```

### CLI Flag Precedence

1. Command-line flags (highest priority)
2. Environment variables (e.g., `NU_LOG_LEVEL`)
3. Config file values
4. Built-in defaults (lowest priority)
