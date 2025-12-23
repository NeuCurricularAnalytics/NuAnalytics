# Quick Commands (CLI-only)

Common one-liners for everyday development with the CLI.

```bash
# Build (debug)
npm run dev

# Build (release)
npm run build

# Run the CLI
npm run run
# or
cargo run --bin nuanalytics-cli

# Logging examples
cargo run --bin nuanalytics-cli -- --log-level warn
cargo run --bin nuanalytics-cli -- --debug           # enable debug level + runtime flag
cargo run --bin nuanalytics-cli -- -v                # enable verbose output
cargo run --bin nuanalytics-cli -- --log-file ./nu.log

# Tests
npm run test                      # cargo test --workspace

# Linting
npm run lint                      # clippy + fmt + doc
npm run lint:clippy
npm run lint:fmt
npm run lint:doc

# Docs
npm run doc                       # generates docs into docs/rust

# Clean
npm run clean
```
