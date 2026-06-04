# NuAnalytics

**Version 0.4.1**

NuAnalytics is a Rust-based tool for analyzing computer science curricula. It computes detailed metrics about curriculum structure including complexity, blocking relationships, delay paths, and centrality measures to help understand how courses are organized and their impact on students.

It is based off the work of Greg Heileman, and CurricularAnalytics.org. Current version provides command line capabilities for curriculum analysis, degree program validation, and comprehensive reporting.

> **0.4.0 has two breaking changes** (the `degree` command became a
> subcommand dispatcher and database access now requires login on every
> call). See [CHANGELOG.md](./CHANGELOG.md) for the full migration
> checklist before upgrading from 0.3.x.

## Features

- **Curriculum Analysis**: Parse CSV-formatted curriculum files and analyze course dependencies
- **Degree Program Validation**: Validate YAML-based degree programs with comprehensive checks
- **Degree Plan Generation**: Generate all possible degree plans and compute aggregate statistics
- **Unified JSON Degree Format** *(new in 0.4.1)*: Author and analyze degrees as JSON as well as YAML — every `degree` subcommand auto-detects the format on load. `degree convert` turns ai-landscape program JSON into the unified format, and `degree schema` emits its JSON Schema for downstream validation.
- **Parallel Batch Analysis** *(new in 0.4.1)*: `degree analyze -j N` runs a process-isolated worker pool (default 8) so a pathological degree (e.g. a full-catalog scrape) can't take down the whole batch; failures are logged and the run continues.
- **Degree Trim** *(new in 0.4.0)*: Collapse a degree to one walkable shortest-path-per-course variant — useful for visualisation and downstream tools that don't reason about alternatives. Protected major subjects keep all options; pattern pools survive orphan pruning. Accepts YAML or JSON and round-trips the input format.
- **Gen-Ed Tracking**: Track how major courses satisfy general education requirements
- **Graph Analysis**: Build and analyze course prerequisite graphs
- **Curriculum Auditing**: Identify issues like missing prerequisites and complex prerequisite chains
- **Metric Computation**: Calculate comprehensive metrics including:
  - **Complexity**: Measure of course density and impact on overall curriculum
  - **Blocking**: Count of courses directly blocked by a given course
  - **Delay**: Longest path from a course to course with no prerequisites
  - **Centrality**: Importance of a course in the curriculum network
- **Report Generation**: Create visual reports in multiple formats:
  - **HTML**: Interactive web-based reports with box plots and statistics
  - **PDF**: Print-ready reports via Chrome/Chromium conversion
  - **Markdown**: Text-based reports for documentation
- **Term Scheduling**: Automatic course scheduling respecting prerequisites and credit limits
- **Configuration Management**: Flexible configuration system with CLI overrides
- **MCP Server** (optional): AI model integration for interactive degree building via Model Context Protocol. Degree tools accept YAML, unified JSON, or raw ai-landscape JSON; `convert_degree` *(new in 0.4.1)* converts ai-landscape JSON to the unified format and `get_degree_json_schema` *(new in 0.4.1)* returns the machine schema. The `trim_degree` tool pipes a fresh `cache:<hash>` handle back for chained `validate_degree` / `audit_degree` calls.
- **Authenticated Database Access** *(behavior change in 0.4.0)*: Supabase reads and writes both require a logged-in user (`nuanalytics db login`); session tokens auto-refresh.

## Quick Start

## Installation

### Stable (crates.io) - Recommended

Install the CLI via Cargo:

```bash
cargo install nu-analytics
```

This provides the `nuanalytics` binary globally.

### From Git (for latest)

Install directly from Git:

```bash
cargo install --git https://github.com/NeuCurricularAnalytics/NuAnalytics --bin nuanalytics
```

### Building

```bash
cargo build --release
```

The executable will be at `target/release/nuanalytics`.

### Running (installed)

Analyze a curriculum CSV file and generate both metrics CSV and HTML report:

```bash
nuanalytics planner path/to/curriculum.csv
```

Generate only a PDF report:

```bash
nuanalytics planner path/to/curriculum.csv --no-csv --report-format pdf
```

Generate only CSV metrics (no report):

```bash
nuanalytics planner path/to/curriculum.csv --no-report
```

Analyze a degree program (generates plans, metrics, and reports):

```bash
nuanalytics degree analyze samples/degrees/csu-cs-bscs-general.yaml
```

Validate a degree program only:

```bash
nuanalytics degree validate samples/degrees/csu-cs-bscs-general.yaml
```

Audit a degree program for issues:

```bash
nuanalytics degree audit samples/degrees/csu-cs-bscs-general.yaml
```

Trim a degree to one walkable path per course (collapses alternatives
outside the major):

```bash
nuanalytics degree trim samples/degrees/csu-cs-bscs-general.yaml
```

Scaffold a new research project (creates `degrees/`, `plans/`, MCP
wiring, and three SKILL.md skills for Claude Code under `.claude/`):

```bash
nuanalytics init my-research-project
cd my-research-project && claude
```

Sign in to the Supabase-backed IPEDS database (required for any DB
tool — both reads and writes):

```bash
nuanalytics db login
nuanalytics db status
```

Manage configuration:

```bash
nuanalytics config get level
nuanalytics config set level debug
nuanalytics config set degree_analysis.max_plans 5000
```

## Configuration

NuAnalytics uses a three-tier configuration system with the following precedence (highest to lowest):

1. **Command-line arguments** - Override any config value for a single run
2. **Local config** (`nuanalytics.toml` in current directory) - Project-specific settings
3. **Home config** (`~/.config/nuanalytics/config.toml`) - User-wide defaults
4. **Built-in defaults** - Fallback values

This allows you to set project-specific configurations by creating a `nuanalytics.toml` file in your project directory:

```toml
# nuanalytics.toml - Local project configuration
[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"

[degree_analysis]
max_plans = 500
sampling_strategy = "stratified"
```

## Documentation

- **[Config Command](docs/config.md)** - Configure NuAnalytics settings (logging, database, output directories, degree analysis)
- **[Planner Command](docs/planner.md)** - Analyze curricula for a degree plan, compute metrics, and generate reports
- **[Degree Command](docs/degree.md)** - Validate, analyze, and audit degree program YAML files
- **[MCP Server](docs/mcp.md)** - AI model integration via Model Context Protocol (optional feature)

## Development

If you are contributing or working locally, see [Development.md](Development.md) for `cargo run` workflows and setup.

## Project Structure

```
├── src/
│   ├── cli/              # Command-line interface
│   │   ├── main.rs       # CLI entry point
│   │   ├── args.rs       # Argument definitions (clap)
│   │   └── commands/     # Command handlers
│   ├── core/             # Core analysis engine
│   │   ├── config.rs     # Configuration management
│   │   ├── metrics.rs    # Metric computation algorithms
│   │   ├── metrics_export.rs  # CSV export functionality
│   │   ├── models/       # Data structures (Course, Degree, Plan, School, DAG)
│   │   ├── degree/       # Degree program analysis
│   │   │   ├── gen_ed_tracker.rs  # Gen-ed satisfaction tracking
│   │   │   ├── plan_generator.rs  # Plan enumeration
│   │   │   └── plan_validation.rs # Plan validation
│   │   ├── planner/      # CSV parsing and planning
│   │   └── report/       # Report generation
│   │       ├── formats/  # HTML, PDF, Markdown reporters
│   │       └── term_scheduler.rs  # Course scheduling algorithm
│   ├── logger/           # Internal logging module
│   └── lib.rs            # Library exports
├── tests/                # Integration and unit tests
├── docs/                 # Documentation
└── Cargo.toml            # Project manifest
```

## Report Formats

### HTML Reports
Self-contained HTML files with:
- Interactive course dependency visualization
- Box plots for curriculum metrics distribution
- Term-by-term schedule view
- Detailed metrics table
- Plan comparison (shortest/longest/random samples)

### PDF Reports
Generated via headless Chrome/Chromium:
- Same content as HTML reports
- Print-optimized layout
- Requires Chrome, Chromium, or custom converter

### Markdown Reports
Simple text-based reports suitable for:
- Documentation systems
- Version control diffing
- Email attachments

## License

See [LICENSE](LICENSE) file for details.
