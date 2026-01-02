# Assets


Various assets to be included into the compile. To include the asset, you can use the following

```rust
// Embed a file as a byte array
const LOGO: &[u8] = include_bytes!("assets/logo.png");

// Embed a file as a string (must be valid UTF-8)
const CONFIG: &str = include_str!("assets/config.toml");
```


If we end up wanting to migrate to a more complete solution, we can use the `rust-embed` crates (will need to be added to the TOMAL).


```rust
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "assets/"]
struct Assets;

fn main() {
    let logo = Assets::get("logo.png").unwrap();
    let data: std::borrow::Cow<[u8]> = logo.data;
}
```
