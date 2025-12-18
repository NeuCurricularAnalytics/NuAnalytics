//! Command-line interface entry point for `NuAnalytics`

use nu_analytics::get_version;

fn main() {
    println!("NuAnalytics CLI v{}", get_version());
    println!("Hello from the command-line interface!");
}
