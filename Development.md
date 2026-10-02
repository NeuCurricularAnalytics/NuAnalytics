# Development Guide

How to build, test and change NuAnalytics from a checkout, and the conventions a change
is expected to follow.

## Setup

You need a current stable Rust toolchain ([rustup](https://rustup.rs/)), Git, and
[pre-commit](https://pre-commit.com/) (a Python tool). Python 3 runs the MCP test script,
and Node runs the optional MCP Inspector.

```bash
git clone https://github.com/NeuCurricularAnalytics/NuAnalytics.git
cd NuAnalytics
pip3 install pre-commit
pre-commit install
pre-commit install --hook-type commit-msg
```

The hooks run `cargo fmt`, `cargo clippy` with warnings as errors, whitespace and
end-of-file fixes, a YAML check, a large-file check, and a Conventional Commits check on
the message. If a hook fails, fix what it reports (most fixes are automatic), stage the
files again and commit again.

## Building and running

```bash
cargo build                 # debug: target/debug/nuanalytics
cargo build --release       # release: target/release/nuanalytics
cargo run -- planner samples/plans/BSCS_Hawaii_Manoa.csv
cargo run -- degree analyze samples/degrees/csu-cs-bscs-general.yaml
cargo run -- mcp
```

`cargo run` rebuilds when needed; run `target/debug/nuanalytics` directly to skip the
check.

### Features

The default features are `log-info`, `log-debug`, `verbose`, `file-logging`, `database`
and `mcp`. `database` adds the Supabase client and the `db` commands; `mcp` adds the
server. **CI tests three feature sets, and the middle one matters most:**

```bash
cargo test --all-features
cargo test --no-default-features --features database   # the CLI without the MCP server
cargo test --no-default-features
```

The database queries live in `src/core/query/` so the CLI can use them without `mcp`. If
code under `src/core/` ever imports `crate::mcp`, only the middle build fails.

### Build memory

A release build uses several gigabytes. On a machine already running editors and
language servers, limit parallel jobs (`cargo build -j 4`) or run the build in its own
memory-limited cgroup (`systemd-run --user --scope -p MemoryMax=8G -- cargo build`).

## Testing

- Unit tests sit in a `#[cfg(test)] mod tests` block in the file they test.
- Integration tests are in `tests/rs/`, compiled as the one `integration` target through
  `tests/integration.rs`. `tests/config_tests.rs` is a separate target for configuration.
- `tests/assets/degrees/` holds 13 real degrees that the integration tests compile in with
  `include_str!`, so a missing fixture is a build error. Its `Readme.md` says where each
  came from.

```bash
cargo test                          # everything, default features
cargo test config                   # tests whose name contains "config"
cargo test --lib                    # unit tests only
cargo test --test integration       # the integration target only
```

`tests/scripts/test_mcp_server.py` drives a running MCP server over stdio; see
[tests/scripts/README.md](tests/scripts/README.md) for its options. The
[MCP Inspector](https://github.com/modelcontextprotocol/inspector) gives a web UI for
calling tools by hand:

```bash
npx @modelcontextprotocol/inspector cargo run -- mcp
```

## Code quality

Cargo aliases, defined in `.cargo/config.toml`:

| alias | runs |
|---|---|
| `cargo fmt-check` | `cargo fmt --all -- --check` |
| `cargo lint` | `clippy` on all targets and features, warnings as errors |
| `cargo lint-fix` | `clippy --fix` on all targets and features |
| `cargo doc-private` | `cargo doc --no-deps --all-features` (output in `target/doc/nu_analytics/`) |

CI runs clippy with warnings as errors, `cargo fmt --check`, `cargo doc`, and the three
test builds above. `clippy.toml` requires a doc comment on every public item.

## Layout

- `src/cli/` — the command line: `main.rs` (startup), `args.rs` (clap definitions) and
  `commands/` (one handler per command: `config`, `init`, `planner`, `degree`, `db`,
  `mcp`).
- `src/core/` — the library. `degree/analysis.rs` is the one degree-analysis pipeline,
  shared by `degree analyze` and every MCP analysis tool; give it an `AnalysisConfig`
  option rather than adding a step to either caller. `database/` is the Supabase client,
  sign-in, and the IPEDS and degree importers; `query/` holds the database queries both
  front ends use.
- `src/mcp/` — the MCP server. `server.rs` registers the tools, parses their arguments
  and calls one engine each; `tools/` holds a module per degree tool; `envelope.rs` turns
  `{"error", "code"}` payloads into protocol errors.

### Adding an MCP tool

1. Write the engine in `src/core/` if the tool needs logic the CLI could also use.
2. Add the request type (deriving `schemars::JsonSchema`) and a module under
   `src/mcp/tools/` if needed.
3. Register it in `src/mcp/server.rs` with `#[tool]`, and add it to `CAPABILITIES` —
   a test fails if `CAPABILITIES` and the router disagree.
4. If it writes to the database, serve it only under `--allow-writes`.
5. Test it. The skills that `nuanalytics init` ships are also tested against the server:
   any tool or argument a skill names must exist.

## Configuration while developing

A debug build keeps its sign-in session in `.debug/dauth.json` in the working directory;
a release build keeps it in `$NU_ANALYTICS/auth.json`, where `$NU_ANALYTICS` is the
configuration directory (`~/.config/nuanalytics` on Linux and macOS,
`%APPDATA%\nuanalytics` on Windows). Global flags override configuration for one run
without writing it:

```bash
cargo run -- --log-level debug degree validate samples/degrees/csu-cs-bscs-general.yaml
cargo run -- -v degree analyze samples/degrees/csu-cs-bscs-general.yaml   # verbose
cargo run -- --debug config                      # debug logging and runtime debug output
cargo run -- --db-endpoint http://localhost:8000 db status
```

See [docs/config.md](docs/config.md) for every setting.

## Commits and pull requests

Commit messages follow [Conventional Commits](https://www.conventionalcommits.org/):
`<type>(<scope>): <subject>`, with an optional body and footer. Types are `feat`, `fix`,
`docs`, `style`, `refactor`, `perf`, `test` and `chore`; the scope is optional — `cli`,
`mcp`, `db`, `degree`, `tests`, or none for a cross-cutting change.

```
feat(degree): read {[A, B], C} as a choice of course groups
fix(db): trim padded IPEDS headers
docs: correct the metric definitions in the README
```

For a pull request: branch from `main` (`git checkout -b feat/short-description`), make
the change with its tests and documentation, run `cargo fmt`, `cargo lint` and the three
test builds, push, and open the PR. CI must pass before it is merged.

## Troubleshooting

- **A pre-commit hook fails:** run the failing tool yourself for the full output
  (`cargo lint`), apply `cargo lint-fix`, fix the rest by hand, stage and commit again.
- **A build fails oddly after switching branches:** `cargo clean`, then build again.
