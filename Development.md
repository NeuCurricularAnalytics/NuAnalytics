# Development Guide

This guide provides information for developers working on NuAnalytics, including setup instructions, common development tasks, and contribution policies.

## Getting Started

### Prerequisites

Ensure you have the following installed:

- **Rust 1.70+** - [Install Rust](https://rustup.rs/)
- **Node.js 18+** and npm - [Install Node.js](https://nodejs.org/)
- **Git** - [Install Git](https://git-scm.com/)
- **cargo-watch** (optional) - Install with: `cargo install cargo-watch` (used by `npm run dev:watch`)

### Installation

1. Clone the repository:
   ```bash
   git clone https://github.com/NeuCurricularAnalytics/NuAnalytics.git
   cd NuAnalytics
   ```

2. Install Node.js dependencies:
   ```bash
   npm install
   ```

3. Set up pre-commit hooks:
   ```bash
   pip3 install pre-commit  # if not already installed
   ```
   Then run:
   ```bash
   pre-commit install
   pre-commit install --hook-type commit-msg
   ```

   This ensures code quality checks run automatically before each commit. If you don't have pre-commit installed, you need to install it via pip or pip3 `pip install pre-commit`.

## Common Development Actions

### CLI Development

The CLI is built with Rust using a modular architecture:
- `src/cli/main.rs` - Entry point and startup logic
- `src/cli/args.rs` - CLI argument definitions using clap
- `src/cli/commands/` - Command handlers (config, plan, degree, etc.)
- `src/shared/config/` - Configuration management

**Build the CLI in debug mode:**
```bash
npm run dev
# or
cargo build
```

**Build the CLI for release:**
```bash
npm run build
# or
cargo build --release
```

**Run the CLI:**
```bash
cargo run -- config
```

#### Configuration Management

Configuration is stored in:
- Linux/macOS: `~/.config/nuanalytics/config.toml`
- Windows: `%APPDATA%\nuanalytics\config.toml`

The configuration system supports:
- Persistent settings via TOML file
- CLI overrides (in-memory only, doesn't modify file)
- Automatic merging of missing fields from defaults on upgrades
- Variable expansion (`$NU_ANALYTICS` expands to config directory)

**Configuration commands:**
```bash
cargo run -- config              # Display all config
cargo run -- config get level    # Get specific value
cargo run -- config set level info  # Set value (persists to file)
cargo run -- config unset level  # Reset to default
cargo run -- config reset        # Reset all (with confirmation)
```

**CLI overrides (in-memory only):**
```bash
cargo run -- --db-token <TOKEN> --plans-dir ./my-plans config
cargo run -- --config-level debug config get level  # Shows "debug"
```

You can control logging with either the explicit `--log-level` or shorthand flags:

```bash
# Shorthand flags
cargo run -- -v          # enable verbose
cargo run -- --debug     # enable debug-level + runtime debug

# Explicit level (overrides config)
cargo run -- --log-level warn
cargo run -- --log-level debug

# Falls back to config.logging.level if --log-level not provided
cargo run -- config set level info  # Set in config
cargo run -- config                 # Will use info level from config
```

Tip: For quick CLI testing, prefer `cargo run` so it rebuilds as needed and runs in one step. If you want to skip rebuild when code hasn’t changed, run the compiled binary directly:

```bash
target/nuanalytics --log-level info
```

**Watch for changes and rebuild:**
```bash
npm run dev:watch
```

This uses `cargo-watch` to rebuild with all features enabled during development.

**Run CLI tests:**
```bash
npm run test
# or
cargo test
```



### Build Everything

**Development / Debug:**
```bash
npm run dev
```

**Production / Release:**
```bash
npm run build
```

Feature defaults: During development, debug logging is enabled by default for the CLI.

### Documentation

**Generate Rust documentation:**
```bash
npm run doc
```

Open `docs/rust/index.html` in your browser.

**View docs in browser directly:**
```bash
./scripts/open_docs.py
```

## Code Quality

### Linting

**Run all linters:**
```bash
npm run lint
```

**Run specific linters:**
```bash
npm run lint:clippy   # Rust linting
npm run lint:fmt      # Rust formatting check
npm run lint:doc      # Generate and check docs
```

**Auto-fix issues:**
```bash
npm run lint:fix
```

This will:
- Fix Rust formatting with `cargo fmt`
- Fix Clippy warnings with `--fix`

### Pre-commit Hooks

Pre-commit hooks run automatically before commits and enforce:

- **Rust formatting** - `cargo fmt`
- **Rust linting** - `cargo clippy` (deny warnings)
- **File cleanup** - Trailing whitespace, EOF fixes
- **YAML validation** - `.pre-commit-config.yaml`, etc.
- **Commit message format** - Conventional commits

If a hook fails, fix the issues and try committing again. Most hooks can auto-fix issues:
```bash
# Retry after auto-fixes
git add .
git commit -m "your message"
```

## Testing & Committing Policies

### Testing Requirements

All code changes must include appropriate tests:

- **Unit tests**: Add inline tests in modules (`#[cfg(test)]`)
- **Integration tests**: Add to `tests/` directory
- **Documentation tests**: Add examples in doc comments

**Test organization:**
- `tests/integration.rs` - High-level integration tests
- `tests/rs/` - Rust-specific test modules
- Inline `#[cfg(test)]` modules in source files for unit tests

**Before committing**, ensure all tests pass:

```bash
npm run test
# or
cargo test
```

**Run specific tests:**
```bash
cargo test config         # Tests matching "config"
cargo test --lib          # Only library tests
cargo test --test integration  # Only integration tests
```

**CI/CD will enforce**: All tests must pass before PRs can be merged.

### Commit Message Format

Follow the [Conventional Commits](https://www.conventionalcommits.org/) specification:

```
<type>(<scope>): <subject>

<body>

<footer>
```

**Types:**
- `feat` - A new feature
- `fix` - A bug fix
- `docs` - Documentation changes
- `style` - Code style changes (formatting, etc.)
- `refactor` - Code refactoring without feature changes
- `perf` - Performance improvements
- `test` - Adding or updating tests
- `chore` - Build, tooling, or dependency updates

**Examples:**
```
feat(wasm): add greet function to WASM bindings
fix(cli): handle missing config file gracefully
docs: update installation instructions
chore: upgrade dependencies
test(rs): add smoke tests for get_version
```

**Scope options:**
- `cli` - CLI changes
- `tests` - Test infrastructure
- No scope for general/cross-cutting changes

### Pull Request Process

1. **Create a feature branch**: `git checkout -b feat/your-feature`
2. **Make changes**: Edit files, add tests, update docs as needed
3. **Run linting**: `npm run lint:fix` to auto-fix issues
4. **Run tests**: `npm run test:all` to ensure everything passes
5. **Commit**: Use conventional commit messages
6. **Push**: `git push origin feat/your-feature`
7. **Create PR**: Describe your changes and link any issues
8. **Wait for CI**: GitHub Actions will run tests and lint checks
9. **Address feedback**: Fix any issues raised in review

## Troubleshooting

### Pre-commit hook failures

If a pre-commit hook fails:

1. Read the error message carefully
2. Run the linter locally to see detailed output: `npm run lint`
3. Use `npm run lint:fix` to auto-fix what you can
4. Manually fix remaining issues
5. Re-stage files: `git add .`
6. Retry commit: `git commit -m "message"`

### Build issues

If a build fails:

1. Clean build artifacts: `npm run clean`
2. Rebuild: `npm run build`
3. Check for compilation errors in the output



## Additional Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [Conventional Commits](https://www.conventionalcommits.org/)
- [Pre-commit Documentation](https://pre-commit.com/)
