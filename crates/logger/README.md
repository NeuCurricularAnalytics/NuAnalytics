# Logger

Lightweight cross-platform logging crate with feature-gated levels.

- `error!` and `warn!` are always enabled
- `info!` is enabled by the `log-info` feature (default)
- `debug!` is enabled by the `log-debug` feature (default) and a runtime flag
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
```

## Usage

```rust
use logger::{Level, set_level, enable_debug};
use logger::{error, warn, info, debug};

fn main() {
    // Set log level programmatically
    set_level(Level::Info);

    error!("Something went wrong: {}", 42);
    warn!("Heads up: {}", "value");
    info!("FYI: {}", "details");

    // Enable debug at runtime (requires `log-debug` feature)
    enable_debug();
    set_level(Level::Debug);
    debug!("Extra diagnostic info");
}
```

## Features

- `log-info`: enables `info!` macro output
- `log-debug`: enables `debug!` macro output and runtime flag control

## WASM Notes

When targeting `wasm32`, messages are emitted via `web_sys::console`. Styling is applied to
`[ERROR]` and `[WARN]` prefixes to make them stand out in the browser console.

## License

MIT or Apache-2.0, at your option.
