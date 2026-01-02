//! Config command handler

use crate::args::ConfigSubcommand;
use nu_analytics::config::Config;
use std::io::{self, Write};

/// Dispatch config subcommands
pub fn run(subcommand: Option<ConfigSubcommand>, config: &mut Config, defaults: &Config) {
    match subcommand {
        None => handle_config_get(config, None),
        Some(ConfigSubcommand::Get { key }) => handle_config_get(config, key),
        Some(ConfigSubcommand::Set { key, value }) => handle_config_set(config, &key, &value),
        Some(ConfigSubcommand::Unset { key }) => handle_config_unset(config, defaults, &key),
        Some(ConfigSubcommand::Reset) => handle_config_reset(),
    }
}

/// Handle the config get subcommand
pub fn handle_config_get(config: &Config, key: Option<String>) {
    if let Some(k) = key {
        // Print specific config value
        match config.get(&k) {
            Some(value) => println!("{value}"),
            None => eprintln!("Unknown config key: '{k}'"),
        }
    } else {
        // Print all config values
        println!("\n=== Configuration ===\n");
        print!("{config}");
    }
}

/// Handle the config set subcommand
pub fn handle_config_set(config: &mut Config, key: &str, value: &str) {
    if let Err(e) = config.set(key, value) {
        eprintln!("{e}");
        std::process::exit(1);
    }

    if let Err(e) = config.save() {
        eprintln!("Failed to save config: {e}");
        std::process::exit(1);
    }

    println!("✓ Set {key} = {value}");
}

/// Handle the config unset subcommand
pub fn handle_config_unset(config: &mut Config, defaults: &Config, key: &str) {
    if let Err(e) = config.unset(key, defaults) {
        eprintln!("{e}");
        std::process::exit(1);
    }

    if let Err(e) = config.save() {
        eprintln!("Failed to save config: {e}");
        std::process::exit(1);
    }

    println!("✓ Reset {key} to default");
}

/// Handle the config reset subcommand
pub fn handle_config_reset() {
    if !Config::get_config_file_path().exists() {
        println!("✓ Config is already at defaults");
        return;
    }

    // Ask for confirmation
    print!("Are you sure you want to reset config to defaults? (y/n): ");
    io::stdout().flush().ok();

    let mut response = String::new();
    io::stdin().read_line(&mut response).ok();

    if response.trim().eq_ignore_ascii_case("y") || response.trim().eq_ignore_ascii_case("yes") {
        if let Err(e) = Config::reset() {
            eprintln!("Failed to remove config file: {e}");
            std::process::exit(1);
        }
        println!("✓ Config reset to defaults");
    } else {
        println!("✗ Reset cancelled");
    }
}
