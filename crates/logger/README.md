# Logger

Lightweight cross-platform logging crate with feature-gated levels.

- `error!` and `warn!` are always enabled
- `info!` is enabled by the `log-info` feature (default)
- `debug!` is enabled by the `log-debug` feature (default) and a runtime flag
- `verbose!` is enabled by the `verbose` feature - simple printer with no tags
- `file-logging` enables writing logs to a file (verbose does NOT go to file)
- Works on native and `wasm32` targets (`web_sys::console` on WASM)

## Install

Add to your Cargo.toml (path dependency in this workspace):

```toml
[dependencies]
logger = { path = "crates/logger" }

[features]
# Optionally forward or configure features in the parent crate
log-info = ["logger/log-info"]
log-debug = ["logger/log-debug"]
verbose = ["logger/verbose"]
file-logging = ["logger/file-logging"]
```

## Usage

```rust
use logger::{Level, set_level, enable_debug, enable_verbose, init_file_logging};
use logger::{error, warn, info, debug, verbose};
use std::path::PathBuf;

fn main() {
    // Set log level programmatically
    set_level(Level::Info);

    // Enable file logging (requires `file-logging` feature)
    // All log levels (error, warn, info, debug) go to file, but verbose does NOT
    init_file_logging(&PathBuf::from("app.log"));

    error!("Something went wrong: {}", 42);
    warn!("Heads up: {}", "value");
    info!("FYI: {}", "details");

    // Enable debug at runtime (requires `log-debug` feature)
    enable_debug();
    set_level(Level::Debug);
    debug!("Extra diagnostic info");

    // Enable verbose output (requires `verbose` feature)
    // This is a simple printer with no tags, separate from log levels
    enable_verbose();
    verbose!("Processing item {} of {}", 1, 100);
}
```

## Features

- `log-info`: enables `info!` macro output
- `log-debug`: enables `debug!` macro output and runtime flag control
- `verbose`: enables `verbose!` macro - simple printer without tags, controlled independently
- `file-logging`: enables writing log messages to a file (verbose does NOT go to file)

## WASM Notes

When targeting `wasm32`, messages are emitted via `web_sys::console`. Styling is applied to
`[ERROR]` and `[WARN]` prefixes to make them stand out in the browser console.

## License

MIT or Apache-2.0, at your option.
