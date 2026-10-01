# MCP Server

`nuanalytics mcp` is an MCP (Model Context Protocol) server. It lets a model author,
validate, analyze and render degree programs; read the stored programs and the analysis
stored with them; and query IPEDS completion data, through typed tools or read-only SQL.

- [Quick start](#quick-start)
- [Installation](#installation)
- [Connecting a client](#connecting-a-client)
- [How the tools work](#how-the-tools-work)
- [Skills](#skills)
- [Testing](#testing)
- [Validation reference](#validation-reference)
- [Troubleshooting](#troubleshooting)

## Quick start

```bash
cargo install nu-analytics          # MCP is included by default
nuanalytics init my-study           # a project wired to the server, with five skills
cd my-study && claude
```

`nuanalytics mcp --list-tools` prints the tools this build serves, with their
descriptions. That list is authoritative; this page does not repeat it.

## Installation

```bash
cargo install nu-analytics                                        # from crates.io
cargo install --git https://github.com/NeuCurricularAnalytics/NuAnalytics --bin nuanalytics
```

From a checkout, `cargo run -- mcp` runs the server and rebuilds as needed.

## Connecting a client

The server speaks MCP over stdio. Any client that can launch a command works.

### Claude Code

`nuanalytics init <dir>` writes `.mcp.json` at the project root, which registers the
server. It also writes `.claude/settings.json`, which approves the server through
`enabledMcpjsonServers`. Claude Code applies project settings only in a trusted folder,
so accept the trust prompt the first time you open the project. `/mcp` lists the
connected servers.

To add the server to an existing project, write `.mcp.json` yourself:

```json
{
  "mcpServers": {
    "nuanalytics": {
      "type": "stdio",
      "command": "nuanalytics",
      "args": ["mcp"]
    }
  }
}
```

Servers belong in `.mcp.json`. Claude Code does not read a `mcpServers` block from
`.claude/settings.json`.

### Claude Desktop

Add the same `mcpServers` block to `claude_desktop_config.json`:
`~/Library/Application Support/Claude/` on macOS, `%APPDATA%\Claude\` on Windows. Use an
absolute `command` path if `nuanalytics` is not on the PATH Claude Desktop sees.

### Tools that write

`import_degree`, which writes a degree and its analysis into the database, is served
only when the server is started with `nuanalytics mcp --allow-writes`. The flag goes in
the `args` of `.mcp.json`, where it is visible. Every other tool reads the database, if
it touches it at all. Tools that write files say so, and refuse to replace a file unless
passed `overwrite=true`.

## How the tools work

### What there is

| Group | Tools |
|---|---|
| Reference | `get_reference`: the degree format, its JSON Schema, and the database schema |
| Author and check | `validate_degree`, `audit_degree`, `find_courses_matching`, `get_course_detail`, `convert_degree`, `trim_degree` |
| Analyze | `analyze_degree`, `render_degree_report`, `render_plan_graph` |
| Samples | `list_sample_degrees` |
| IPEDS | `search_institutions`, `search_cip_codes`, `get_lookup_codes`, `get_completion_demographics` |
| Stored programs | `search_degrees`, `get_degree`, `get_stored_analysis`, `render_stored_report`, `compare_degrees` |
| SQL | `query_sql` |
| Writes | `import_degree` (with `--allow-writes` only) |

The server's instructions to the model carry the same map, generated from one table
that a test keeps equal to the tools served.

### Passing a degree

Every tool that takes a degree takes exactly one of three arguments:

- **`degree`**, a reference. There are three kinds:
  - `sample:csu` is a bundled sample (`list_sample_degrees` lists them).
  - `cache:<hash>` is a degree an earlier call cached.
  - Anything else is a stored program's `program_key`, or a `degree_id` that names one
    program. A `degree_id` that matches several programs is refused, with their
    `program_key`s listed.
- **`content`**: the degree inline, as YAML or unified JSON. It is cached as it passes
  through, and the response's `source.handle` is a `cache:` reference to pass next time
  instead of the text.
- **`path`**: a file on the server's filesystem.

Every response says where its degree came from, in `source`.

### Stored or fresh

**A stored program means its stored run.** Its plans were enumerated when it was
imported, so `analyze_degree`, `render_degree_report`, `render_plan_graph`,
`get_course_detail` and `compare_degrees` read that run and enumerate nothing. The
response's `source.run` names the run (`run_key`, `variant`, `created_at`,
`analyzer_version`). `variant` picks `full` (the default) or `trimmed`.

- **`fresh=true`** enumerates a stored program's degree afresh instead — with
  `compare_degrees`, `metrics="fresh"`. It is also what the settings that shape a run
  need: `max_plans`, `include_courses`, `random_seed`, `analysis_timeout_seconds` and
  `target_course` are refused for a stored program without it, rather than ignored.
- **A program with no run of that variant** is reported as `source_not_found`. It is
  never replaced by a fresh run silently.
- **Files, inline content and samples** are always enumerated afresh.

A fresh run is reproducible: the same degree, settings and analyzer give the same
figures. It can still differ from a stored run analyzed with other settings or an older
analyzer. A stored run records its cap but not the size of its population, so a stored
run that reached its cap reports `is_full_population: false` and `population_size` equal
to the plans it analyzed.

Runs append, so a program can hold several runs per variant. The stored tools report
the newest run of each variant unless asked for the history.

### Errors

A call that fails comes back with `isError: true` and one envelope:

```json
{"error": {"code": "source_not_found", "message": "...", "next_steps": ["..."], "details": {}}}
```

| code | meaning |
|---|---|
| `bad_arguments` | The arguments are invalid, or name one the tool does not take |
| `source_not_found` | No such sample, handle, file or stored program |
| `ambiguous_reference` | A `degree_id` matches several programs; `details` lists them |
| `cache_expired` | The `cache:` handle is more than 24 hours old; send the degree again |
| `db_unavailable` | The database is not configured, not reachable or not signed in; `next_steps` says which |
| `unreachable`, `query_failed`, … | The database was reached and failed; the message names the operation |
| `sql_rejected` | `query_sql` refused the statement before sending it: not one read-only statement |
| `sql_backend` | The database refused the SQL; `details.sqlstate` has its code |
| `write_failed` | An output file could not be written |

An argument the tool does not take is refused by name, rather than ignored: a misspelt
filter would otherwise return unfiltered results that look like an answer.

A degree that fails validation is a finding, not an error: `validate_degree` returns it
under `errors` and `warnings`.

### The database

The IPEDS, stored-program and SQL tools need a backend (see
[database/setup.md](database/setup.md)). The server builds its database client once, at
startup. When that fails, every database tool reports the reason recorded then, with the
steps to fix it, and the server has to be restarted afterwards. The other tools work with
no backend at all.

`query_sql` runs one `SELECT` or `WITH` inside a read-only transaction. It returns at most
2,000 rows (200 by default), says when that cap truncated the result, and stops a
statement after 30 seconds. Its inputs go in `params`, read in the SQL as `$1`.
`get_reference(topic="database")` describes the tables, how they join, and the rows not
to sum.

## Skills

`nuanalytics init` ships five skills in `.claude/skills/`. Each loads when a request
matches its description, and each pre-approves only tools that read:

| Skill | For |
|---|---|
| `degree-author` | Turning a catalog into a degree file that validates |
| `degree-review` | Checking, fixing or updating a degree file |
| `degree-analyze` | A degree's metrics explained; its report and plan graphs |
| `stored-programs` | Finding, exporting, analyzing and comparing stored programs |
| `curriculum-research` | IPEDS completions and cross-school questions, with SQL when needed |

Tests keep the skills honest: every tool and argument a skill names must exist, no
retired name may appear, every file a skill points to is shipped, and the example degree
must validate.

## Testing

```bash
nuanalytics mcp --list-tools                          # what this build serves
npx @modelcontextprotocol/inspector nuanalytics mcp   # interactive
python3 tests/scripts/test_mcp_server.py              # end-to-end over stdio
cargo test mcp::                                      # unit tests
```

## Validation reference

`validate_degree` reports these as `error_type` and `warning_type`.

### Errors

| Type | Meaning | Usual fix |
|---|---|---|
| `CircularPrerequisite` | Prerequisites form a cycle (A→B→C→A) | Remove one prerequisite |
| `MissingCourse` | A requirement names a course not under `courses:` | Add the course, or fix the key |
| `MissingPrerequisite` | A prerequisite is not under `courses:` | Add it, or fix the reference |
| `MissingCorequisite` | A corequisite is not under `courses:` | Add it |
| `InvalidPattern` | A pattern is malformed | Use `CS:3000+`, `MATH:300-499`, `*:100+` |
| `PatternMatchesNoCourses` | A pattern matches no listed course | List the courses, or pass `allow_unmatched_patterns=true` for an external pool |
| `InvalidRequirement` | A requirement's fields do not fit its type | Check `type` and its required fields |
| `UnidirectionalCrossListing` | A cross-listing is one-way | Add the reciprocal `cross_listed_as` |

### Warnings

| Type | Meaning |
|---|---|
| `UnreferencedCourse` | A course no requirement or prerequisite uses |
| `IsolatedCourse` | A course with no prerequisites and no dependents |
| `BroadPattern` | A pattern that matches very many courses |
| `HiddenRequirement` | A course required only through a prerequisite chain |
| `HiddenRequirementOption` | A prerequisite choice none of whose options the degree lists |
| `HiddenPrerequisite` | An upper-level course that declares no prerequisites |
| `MissingCrossListedCourse` | A `cross_listed_as` course that does not exist |
| `PatternMatchesNoCoursesAllowed` | An unmatched pattern, allowed by `allow_unmatched_patterns` |
| `ImpliedElectiveConstraint` | An elective whose prerequisite is itself only in an optional pool |
| `DuplicateRequirementMembership` | A course in two requirements when `allow_double_counting` is false |
| `CreditTotalUnreachable` | Fixed credits plus the minimum electives already exceed `total_credits` |
| `CreditTotalImplausible` | Fixed credits plus the maximum electives still fall short of `total_credits` |

## Troubleshooting

- **The client does not list the server.** Check that `nuanalytics mcp --list-tools`
  runs from the same PATH the client uses, or use an absolute `command`. In Claude Code,
  accept the folder's trust prompt, then check `/mcp`.
- **Database tools report `db_unavailable`.** Follow the `next_steps`. `nuanalytics db
  doctor` walks configuration, reachability, sign-in and schema in order. Restart the
  server afterwards.
- **A tool refuses an argument.** The message names the arguments it does take.
- **Debug logging:** `nuanalytics --log-level debug mcp 2>debug.log`.

## See also

- [Degree command](degree.md): the CLI counterparts of the degree tools
- [Database setup](database/setup.md)
- [Sample degrees](../samples/degrees/)
- [MCP specification](https://modelcontextprotocol.io/)
