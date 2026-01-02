//! Config command handler

use crate::args::ConfigSubcommand;
use nu_analytics::config::Config;
use std::io::{self, Write};

/// Dispatch config subcommands
///
/// Routes config subcommands to their appropriate handlers. If no subcommand is provided,
/// displays all configuration values.
///
/// # Arguments
/// * `subcommand` - The config subcommand to execute (None displays all config)
/// * `config` - The current configuration (may be modified by set/unset)
/// * `defaults` - Default configuration values for unset operations
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
///
/// Displays configuration values. If a key is provided, shows only that value.
/// If no key is provided, shows all configuration in formatted layout.
///
/// # Arguments
/// * `config` - The configuration to display
/// * `key` - Optional specific key to display (None shows all)
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
///
/// Sets a configuration value and persists it to disk. Validates the key and value
/// format, exiting with error if invalid.
///
/// # Arguments
/// * `config` - The configuration to modify
/// * `key` - The configuration key to set
/// * `value` - The value to set (as string, will be parsed appropriately)
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
///
/// Resets a configuration value to its default and persists to disk. Exits with
/// error if the key is invalid.
///
/// # Arguments
/// * `config` - The configuration to modify
/// * `defaults` - Default configuration values to reset to
/// * `key` - The configuration key to reset
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
///
/// Resets all configuration to defaults by deleting the config file. Requires user
/// confirmation before proceeding. If the config file doesn't exist, reports success
/// without prompting.
pub fn handle_config_reset() {
    if !Config::get_config_file_path().exists() {
        println!("✓ Config is already at defaults");
        return;
    }

    // Ask for confirmation
    print!("Are you sure you want to reset config to defaults? (y/n): ");
    if io::stdout().flush().is_err() {
        eprintln!("Warning: Failed to flush stdout");
    }

    let mut response = String::new();
    if io::stdin().read_line(&mut response).is_err() {
        eprintln!("Failed to read user input");
        std::process::exit(1);
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Create a test config with known values
    fn test_config() -> Config {
        let mut config = Config::from_defaults();
        config.logging.level = "test_level".to_string();
        config.logging.file = "test_file.log".to_string();
        config.logging.verbose = true;
        config.database.token = "test_token".to_string();
        config.database.endpoint = "https://test.com".to_string();
        config.paths.plans_dir = "/test/plans".to_string();
        config.paths.out_dir = "/test/output".to_string();
        config
    }

    #[test]
    fn test_handle_config_get_specific_key() {
        let config = test_config();

        // Test getting specific keys
        assert_eq!(config.get("level"), Some("test_level".to_string()));
        assert_eq!(config.get("file"), Some("test_file.log".to_string()));
        assert_eq!(config.get("verbose"), Some("true".to_string()));
        assert_eq!(config.get("token"), Some("test_token".to_string()));
        assert_eq!(config.get("endpoint"), Some("https://test.com".to_string()));
        assert_eq!(config.get("plans_dir"), Some("/test/plans".to_string()));
        assert_eq!(config.get("out_dir"), Some("/test/output".to_string()));
    }

    #[test]
    fn test_handle_config_get_unknown_key() {
        let config = test_config();
        assert_eq!(config.get("unknown_key"), None);
    }

    #[test]
    fn test_handle_config_set_valid_key() {
        let mut config = test_config();

        // Set a string value
        assert!(config.set("level", "debug").is_ok());
        assert_eq!(config.logging.level, "debug");

        // Set another string value
        assert!(config.set("token", "new_token").is_ok());
        assert_eq!(config.database.token, "new_token");
    }

    #[test]
    fn test_handle_config_set_verbose_boolean() {
        let mut config = test_config();

        // Set verbose to false
        assert!(config.set("verbose", "false").is_ok());
        assert!(!config.logging.verbose);

        // Set verbose to true
        assert!(config.set("verbose", "true").is_ok());
        assert!(config.logging.verbose);
    }

    #[test]
    fn test_handle_config_set_invalid_boolean() {
        let mut config = test_config();

        // Try to set invalid boolean value
        let result = config.set("verbose", "maybe");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid boolean"));
    }

    #[test]
    fn test_handle_config_set_unknown_key() {
        let mut config = test_config();

        let result = config.set("unknown_key", "value");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown config key"));
    }

    #[test]
    fn test_handle_config_unset_resets_to_default() {
        let mut config = test_config();
        let defaults = Config::from_defaults();

        // Modify a value
        config.logging.level = "custom".to_string();

        // Unset should reset to default
        assert!(config.unset("level", &defaults).is_ok());
        assert_eq!(config.logging.level, defaults.logging.level);
    }

    #[test]
    fn test_handle_config_unset_unknown_key() {
        let mut config = test_config();
        let defaults = Config::from_defaults();

        let result = config.unset("unknown_key", &defaults);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown config key"));
    }

    #[test]
    fn test_handle_config_unset_all_keys() {
        let mut config = test_config();
        let defaults = Config::from_defaults();

        // Unset each key and verify it matches defaults
        assert!(config.unset("level", &defaults).is_ok());
        assert_eq!(config.logging.level, defaults.logging.level);

        assert!(config.unset("file", &defaults).is_ok());
        assert_eq!(config.logging.file, defaults.logging.file);

        assert!(config.unset("verbose", &defaults).is_ok());
        assert_eq!(config.logging.verbose, defaults.logging.verbose);

        assert!(config.unset("token", &defaults).is_ok());
        assert_eq!(config.database.token, defaults.database.token);

        assert!(config.unset("endpoint", &defaults).is_ok());
        assert_eq!(config.database.endpoint, defaults.database.endpoint);

        assert!(config.unset("plans_dir", &defaults).is_ok());
        assert_eq!(config.paths.plans_dir, defaults.paths.plans_dir);

        assert!(config.unset("out_dir", &defaults).is_ok());
        assert_eq!(config.paths.out_dir, defaults.paths.out_dir);
    }
}
