# Quick Commands (Cargo Aliases)

Common one-liners for everyday development with the CLI.

```bash
# Build (debug)
cargo build

# Build (release)
cargo build --release

# Run the CLI
cargo run --bin nuanalytics

# Logging examples
cargo run --bin nuanalytics -- --log-level warn
cargo run --bin nuanalytics -- --debug           # enable debug level + runtime flag
cargo run --bin nuanalytics -- -v                # enable verbose output
cargo run --bin nuanalytics -- --log-file ./nu.log

# Tests
cargo test --workspace

# Linting & Formatting
cargo lint                        # clippy with warnings as errors
cargo lint-fix                    # clippy fixes + fmt
cargo fmt-check                   # check formatting without changes
cargo fmt --all                   # format all code

# Docs
cargo doc-private                 # generates docs with private items

# Clean
cargo clean
```
