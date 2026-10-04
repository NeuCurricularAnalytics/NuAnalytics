# NuAnalytics

**Version 0.6.0**

NuAnalytics is a Rust command-line tool and MCP server for analyzing degree curricula. It
measures how a curriculum is structured — the complexity, blocking, delay and centrality
of each course — and how that structure plays out across every plan a student could take.
It builds on the work of Greg Heileman and [CurricularAnalytics.org](https://curricularanalytics.org).

See [CHANGELOG.md](./CHANGELOG.md) for what changed between versions, including breaking
changes.

## Features

- **Curriculum analysis** (`planner`): read a Curricular Analytics CSV, compute per-course
  metrics, schedule the courses into terms, and write CSV metrics and a report.
- **Degree programs** (`degree`): author a degree in YAML or the unified JSON format
  (detected automatically), then validate, audit, convert, trim or analyze it.
  - `degree analyze` enumerates the degree's plans — every one, or a seeded sample when
    there are more than `--max-plans` — and reports statistics across them, along with
    the shortest and longest plans and a few random ones. `-j N` analyzes many degrees in
    isolated worker processes, so one pathological degree cannot stop a batch.
  - `degree trim` collapses a degree to one walkable path per course, for visualization
    and tools that do not reason about alternatives.
  - `degree convert` turns scraped program JSON into the unified format, and
    `degree schema` prints its JSON Schema.
- **Metrics**, per course and summed over a curriculum:
  - **Delay**: the number of courses on the longest prerequisite path through the course.
  - **Blocking**: the number of courses that cannot be taken until this one is passed.
  - **Complexity**: delay plus blocking.
  - **Centrality**: the total length of the source-to-sink paths through the course.
  - **Chain length**: the number of courses in the longest prerequisite chain ending at
    the course.
- **Reports**: HTML (interactive, with box plots and a term-by-term schedule), PDF (through
  headless Chrome or Chromium) and Markdown.
- **Database** (optional): a Supabase backend, cloud or self-hosted, holding IPEDS
  institutions and completion demographics and a store of degree programs with their
  analysis runs. `db import` loads analyzed degrees; `degree analyze --from-db` and the MCP
  tools read them back. Reads and writes require signing in (`nuanalytics db login`). See
  [Database Setup](docs/database/setup.md).
- **MCP server** (optional): tools that let a model author, validate, analyze and render
  degrees, read stored programs and their analysis, and query completion demographics or
  run read-only SQL. Tools that write to the database are served only with
  `nuanalytics mcp --allow-writes`. See [docs/mcp.md](docs/mcp.md).

## Installation

From crates.io:

```bash
cargo install nu-analytics
```

From Git, for the latest:

```bash
cargo install --git https://github.com/NeuCurricularAnalytics/NuAnalytics --bin nuanalytics
```

From a checkout:

```bash
cargo build --release      # the binary is target/release/nuanalytics
```

## Quick start

Analyze a curriculum CSV, writing CSV metrics and an HTML report:

```bash
nuanalytics planner path/to/curriculum.csv
nuanalytics planner path/to/curriculum.csv --no-csv --report-format pdf   # PDF report only
nuanalytics planner path/to/curriculum.csv --no-report                     # CSV metrics only
```

Work with a degree program:

```bash
nuanalytics degree validate samples/degrees/csu-cs-bscs-general.yaml
nuanalytics degree audit    samples/degrees/csu-cs-bscs-general.yaml
nuanalytics degree analyze  samples/degrees/csu-cs-bscs-general.yaml
nuanalytics degree trim     samples/degrees/csu-cs-bscs-general.yaml
```

Start a research project — `degrees/`, `plans/`, an `.mcp.json`, and five Claude Code
skills under `.claude/skills/`:

```bash
nuanalytics init my-research-project
cd my-research-project && claude
```

Sign in to the database (needed for every database command and tool):

```bash
nuanalytics db login
nuanalytics db status
```

Read and change configuration:

```bash
nuanalytics config get level
nuanalytics config set level debug
nuanalytics config set degree_analysis.max_plans 5000
```

## Configuration

Settings are resolved in this order, first match winning:

1. command-line arguments, for a single run;
2. `nuanalytics.toml` in the current directory, for a project;
3. `~/.config/nuanalytics/config.toml`, for the user;
4. built-in defaults.

`config set` always writes the user file, so a project file that sets the same key still
overrides it. A project file looks like this:

```toml
[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[degree_analysis]
max_plans = 10000
```

See [docs/config.md](docs/config.md) for every setting.

## Documentation

- [Degree command](docs/degree.md): the degree format, and validating, auditing,
  trimming and analyzing degrees.
- [Planner command](docs/planner.md): analyzing a curriculum CSV.
- [Config command](docs/config.md): settings and where they are read from.
- [MCP server](docs/mcp.md): the tools a model can call.
- [Database setup](docs/database/setup.md): standing up a Supabase backend, cloud or
  self-hosted, and the stored-program tables.
- [IPEDS data](docs/database/ipeds-data.md): which survey files to download and how to
  import them.
- [Development.md](Development.md): building, testing and running from a checkout.

## Project structure

```
src/
├── cli/                  command-line interface (clap arguments, command handlers)
├── core/
│   ├── degree/           degree parsing, validation, plan generation and the one
│   │                     analysis pipeline (analysis.rs)
│   ├── models/           courses, degrees, plans, prerequisite graphs
│   ├── metrics.rs        delay, blocking, complexity, centrality, chain length
│   ├── planner/          Curricular Analytics CSV input
│   ├── report/           HTML, PDF and Markdown reports; term scheduling
│   ├── database/         Supabase client, auth, IPEDS import, degree import
│   ├── query/            database queries shared by the CLI and the MCP server
│   ├── statistics/       streaming summary statistics
│   └── config.rs         configuration
├── mcp/                  MCP server and its tools
└── lib.rs
tests/                    integration tests and fixtures
docs/                     documentation
```

## License

See [LICENSE](LICENSE).
