# Quick Commands

Common one-liners for everyday development:

```bash
# Build everything for development
npm run dev:all

# Run CLI with debug
cargo run --bin nuanalytics-cli -- --debug

# Run CLI with info via -v
cargo run --bin nuanalytics-cli -- -v

# Build WASM dev and serve locally
npm run dev:wasm:build && npm run serve

# Build production bundles (CLI + WASM)
npm run build:all

# Run all tests
npm run test:all

# Run linters
npm run lint
```
