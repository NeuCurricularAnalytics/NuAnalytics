# NuAnalytics

NuAnalytics is a Rust-based tool for analyzing computer science curricula. It computes detailed metrics about curriculum structure including complexity, blocking relationships, delay paths, and centrality measures to help understand how courses are organized and their impact on students.

It is based off the work of Greg Heileman, and CurricularAnalytics.org. Current version just provides a command line batch capability
and more report options.

## Features

- **Curriculum Analysis**: Parse CSV-formatted curriculum files and analyze course dependencies
- **Metric Computation**: Calculate comprehensive metrics including:
  - **Complexity**: Measure of course density and impact on overall curriculum
  - **Blocking**: Count of courses directly blocked by a given course
  - **Delay**: Longest path from a course to course with no prerequisites
  - **Centrality**: Importance of a course in the curriculum network
- **Report Generation**: Create visual reports in multiple formats:
  - **HTML**: Interactive web-based reports with dependency graphs
  - **PDF**: Print-ready reports via Chrome/Chromium conversion
  - **Markdown**: Text-based reports for documentation
- **Term Scheduling**: Automatic course scheduling respecting prerequisites and credit limits
- **Configuration Management**: Flexible configuration system with CLI overrides

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

Manage configuration:

```bash
nuanalytics config get level
nuanalytics config set level debug
```

## Documentation

- **[Config Command](docs/config.md)** - Configure NuAnalytics settings (logging, database, output directories)
- **[Planner Command](docs/planner.md)** - Analyze curricula for a degree plan, compute metrics, and generate reports

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
- Color-coded complexity indicators
- Term-by-term schedule view
- Detailed metrics table

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
