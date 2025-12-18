# Development Guide

This guide provides information for developers working on NuAnalytics, including setup instructions, common development tasks, and contribution policies.

## Getting Started

### Prerequisites

Ensure you have the following installed:

- **Rust 1.70+** - [Install Rust](https://rustup.rs/)
- **Node.js 18+** and npm - [Install Node.js](https://nodejs.org/)
- **Git** - [Install Git](https://git-scm.com/)
- **wasm-pack** - Install with: `cargo install wasm-pack`

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
   pre-commit install
   pre-commit install --hook-type commit-msg
   ```

   This ensures code quality checks run automatically before each commit.

## Common Development Actions

### CLI Development

The CLI is built with Rust and can be found in `src/native/`.

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
cargo run --bin nuanalytics-cli
```

**Watch for changes and rebuild:**
```bash
npm run dev:watch
```

**Run CLI tests:**
```bash
npm run test
# or
cargo test
```

### WASM Development

The WASM module consists of Rust bindings (`src/wasm/rs/`), TypeScript (`src/wasm/typescript/`), and Sass styles (`src/wasm/sass/`).

**Build WASM in development mode (with sourcemaps):**
```bash
npm run dev:wasm:build
```

**Build WASM for production (minified):**
```bash
npm run build:wasm
```

**Watch WASM build continuously:**
```bash
npm run dev:wasm
```

This runs concurrent watch tasks for:
- `dev:wasm:watch` - Rust compilation
- `dev:typescript:watch` - TypeScript bundling with sourcemaps
- `dev:sass:watch` - Sass compilation

**Serve the built WASM app locally:**
```bash
npm run serve
```

Open http://localhost:4173 in your browser to view the app.

**Run WASM tests:**
```bash
npm run test:wasm
```

**Run TypeScript tests:**
```bash
npm run test:ts
# Watch mode:
npm run test:ts:watch
```

**Run all tests:**
```bash
npm run test:all
```

### Build Everything

**Development (unminified, with sourcemaps):**
```bash
npm run dev:all
```

**Production (minified, optimized):**
```bash
npm run build:all
```

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
npm run lint:clippy      # Rust linting
npm run lint:fmt         # Rust formatting check
npm run lint:typescript  # TypeScript linting
npm run lint:styles      # Sass linting
npm run lint:doc         # Generate and check docs
```

**Auto-fix issues:**
```bash
npm run lint:fix
```

This will:
- Fix Rust formatting with `cargo fmt`
- Fix Clippy warnings with `--fix`
- Fix ESLint issues in TypeScript
- Fix Stylelint issues in Sass

### Pre-commit Hooks

Pre-commit hooks run automatically before commits and enforce:

- **Rust formatting** - `cargo fmt`
- **Rust linting** - `cargo clippy` (deny warnings)
- **SCSS linting** - `stylelint`
- **TypeScript linting** - `eslint`
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

- **Rust changes**: Add tests to `tests/rs/` or inline in Rust files
- **TypeScript changes**: Add tests to `tests/typescript/`
- **WASM changes**: Test both Rust and TypeScript integration

**Before committing**, ensure all tests pass:

```bash
npm run test:all
```

This runs:
- Rust tests: `cargo test`
- TypeScript tests: `vitest run tests/typescript`

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
- `wasm` - WASM module changes
- `cli` - CLI changes
- `typescript` - TypeScript/JS changes
- `sass` - Styling changes
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

### WASM build issues

If WASM build fails:

1. Clean build artifacts: `npm run clean`
2. Rebuild: `npm run build:wasm`
3. Check for compilation errors in the output

### Port already in use

If port 4173 is already in use when running `npm run serve`:

```bash
npx --yes serve dist -l 3000  # Use different port
```

## Additional Resources

- [Rust Book](https://doc.rust-lang.org/book/)
- [wasm-bindgen Guide](https://rustwasm.github.io/docs/wasm-bindgen/)
- [TypeScript Handbook](https://www.typescriptlang.org/docs/)
- [Conventional Commits](https://www.conventionalcommits.org/)
- [Pre-commit Documentation](https://pre-commit.com/)
