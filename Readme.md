# NuAnalytics

NuAnalytics is a Rust-based tool for analyzing computer science curricula. It computes detailed metrics about curriculum structure including complexity, blocking relationships, delay paths, and centrality measures to help understand how courses are organized and their impact on students.

## Features

- **Curriculum Analysis**: Parse CSV-formatted curriculum files and analyze course dependencies
- **Metric Computation**: Calculate comprehensive metrics including:
  - **Complexity**: Measure of course density and impact on overall curriculum
  - **Blocking**: Count of courses directly blocked by a given course
  - **Delay**: Longest path from a course to course with no prerequisites
  - **Centrality**: Importance of a course in the curriculum network
- **Configuration Management**: Flexible configuration system with CLI overrides

## Quick Start

### Building

```bash
cargo build --release
```

The executable will be at `target/release/nuanalytics`.

### Running

Analyze a curriculum CSV file:

```bash
./nuanalytics planner path/to/curriculum.csv -o result.csv
```

Manage configuration:

```bash
./nuanalytics config get level
./nuanalytics config set level debug
```

## Documentation

- **[Config Command](docs/config.md)** - Configure NuAnalytics settings (logging, database, output directories)
- **[Planner Command](docs/planner.md)** - Analyze curricula and compute metrics for a single degree plan

## Development

For development setup and detailed development information, see [Development.md](Development.md).

## Project Structure

```
├── src/
│   ├── cli/              # Command-line interface
│   ├── core/             # Core analysis engine
│   │   ├── metrics/      # Metric computation
│   │   ├── models/       # Data structures (Course, Degree, Plan, School, DAG)
│   │   ├── planner/      # Curriculum planning and CSV parsing
│   │   └── config.rs     # Configuration management
│   └── lib.rs            # Library exports
├── tests/                # Integration and unit tests
├── samples/              # Sample curriculum files
│   ├── plans/            # Input curriculum CSV files
│   └── correct/          # Reference metric outputs for validation
└── Cargo.toml           # Project manifest
```

## License

See [LICENSE](LICENSE) file for details.
