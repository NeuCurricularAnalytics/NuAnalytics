//! Config command handler

use crate::args::ConfigSubcommand;
use nu_analytics::config::Config;
use std::io::{self, Write};
use std::path::Path;

/// Dispatch config subcommands
///
/// Routes config subcommands to their appropriate handlers. If no subcommand is provided,
/// displays all configuration values.
///
/// # Arguments
/// * `subcommand` - The config subcommand to execute (None displays all config)
/// * `config` - The configuration in effect, for display
/// * `defaults` - Default configuration values for unset operations
///
/// `set` and `unset` change the user file only, read on its own: `config` is the merged
/// view, project file and override flags included, and must never be what is saved.
pub fn run(subcommand: Option<ConfigSubcommand>, config: &Config, defaults: &Config) {
    let user_file = Config::get_config_file_path();
    match subcommand {
        None => handle_config_get(config, None),
        Some(ConfigSubcommand::Get { key }) => handle_config_get(config, key),
        Some(ConfigSubcommand::Set { key, value }) => {
            exit_on_error(handle_config_set(&user_file, &key, &value));
        }
        Some(ConfigSubcommand::Unset { key }) => {
            exit_on_error(handle_config_unset(&user_file, defaults, &key));
        }
        Some(ConfigSubcommand::Reset) => handle_config_reset(),
    }
}

/// Print a handler's message, or its error and exit 1.
fn exit_on_error(result: Result<String, String>) {
    match result {
        Ok(message) => println!("{message}"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}

/// The note `set` and `unset` add when a project file is present: it outranks the user
/// file for every key it sets, so the change may not be the one in effect here.
fn project_file_note() -> String {
    Config::get_local_config_file_path()
        .filter(|p| p.exists())
        .map(|p| {
            format!(
                "\n  note: {} takes precedence over the user file for any key it sets",
                p.display()
            )
        })
        .unwrap_or_default()
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

/// Handle the config set subcommand: write one key to the user file at `user_file`.
///
/// Returns the confirmation to print, or why nothing was written: an unknown key, a
/// value of the wrong type, a user file that does not parse, or a failed write.
///
/// # Errors
/// See above; the user file is left unchanged.
pub fn handle_config_set(user_file: &Path, key: &str, value: &str) -> Result<String, String> {
    edit_user_file(user_file, |user| user.set(key, value))?;
    Ok(confirmation(&format!("Set {key} = {value}"), user_file))
}

/// Handle the config unset subcommand: reset one key to its default in the user file.
///
/// # Errors
/// As [`handle_config_set`]; the user file is left unchanged.
pub fn handle_config_unset(
    user_file: &Path,
    defaults: &Config,
    key: &str,
) -> Result<String, String> {
    edit_user_file(user_file, |user| user.unset(key, defaults))?;
    Ok(confirmation(&format!("Reset {key} to default"), user_file))
}

/// Read the user file on its own, apply `change`, and save it — or, when `change` or the
/// read fails, leave the file as it was.
fn edit_user_file(
    user_file: &Path,
    change: impl FnOnce(&mut Config) -> Result<(), String>,
) -> Result<(), String> {
    let mut user = Config::load_user_file(user_file)?;
    change(&mut user)?;
    user.save_to(user_file)
        .map_err(|e| format!("Failed to save {}: {e}", user_file.display()))
}

/// `✓ <what> in <file>`, with the note about a project file that outranks it.
fn confirmation(what: &str, user_file: &Path) -> String {
    format!("✓ {what} in {}{}", user_file.display(), project_file_note())
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

    /// A user file in a temporary directory, never the developer's real one.
    fn user_file(contents: Option<&str>) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        if let Some(text) = contents {
            std::fs::write(&path, text).expect("write user file");
        }
        (dir, path)
    }

    #[test]
    fn test_handle_config_set_changes_only_the_named_key_in_the_user_file() {
        // The defect: `set` saved the merged configuration, so a project file's values and
        // any override flags landed in the user's defaults. Now the user file is read on
        // its own, and only the named key changes.
        let (_dir, path) = user_file(Some("[logging]\nlevel = \"info\"\n"));
        handle_config_set(&path, "max_plans", "2000").expect("sets");
        let saved = Config::load_user_file(&path).expect("reloads");
        assert_eq!(saved.degree_analysis.max_plans, 2000);
        assert_eq!(saved.logging.level, "info", "an unrelated key is untouched");
        assert_eq!(
            saved.paths.reports_dir,
            Config::from_defaults().paths.reports_dir
        );

        handle_config_unset(&path, &Config::from_defaults(), "max_plans").expect("unsets");
        let saved = Config::load_user_file(&path).expect("reloads");
        assert_eq!(
            saved.degree_analysis.max_plans,
            Config::from_defaults().degree_analysis.max_plans
        );
        assert_eq!(saved.logging.level, "info");
    }

    #[test]
    fn test_handle_config_set_creates_a_missing_user_file_from_defaults() {
        let (_dir, path) = user_file(None);
        // `error` is neither build's default, so this passes only if the file was written.
        let message = handle_config_set(&path, "level", "error").expect("sets");
        assert!(path.exists(), "the user file is created");
        assert!(message.contains(&path.display().to_string()), "{message}");
        assert_eq!(
            Config::load_user_file(&path).unwrap().logging.level,
            "error"
        );
    }

    #[test]
    fn test_handle_config_set_writes_nothing_when_it_refuses() {
        let (_dir, path) = user_file(Some("this is = = not toml"));
        let err = handle_config_set(&path, "level", "debug").expect_err("refuses");
        assert!(err.contains(&path.display().to_string()), "{err}");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "this is = = not toml"
        );

        let (_dir, path) = user_file(None);
        assert!(handle_config_set(&path, "no_such_key", "1").is_err());
        assert!(!path.exists(), "a refused set writes nothing");
    }

    /// Create a test config with known values
    fn test_config() -> Config {
        let mut config = Config::from_defaults();
        config.logging.level = "test_level".to_string();
        config.logging.file = "test_file.log".to_string();
        config.logging.verbose = true;
        config.database.anon_key = "test_token".to_string();
        config.database.endpoint = "https://test.com".to_string();
        config.paths.metrics_dir = "/test/metrics".to_string();
        config.paths.reports_dir = "/test/reports".to_string();
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
        assert_eq!(config.get("metrics_dir"), Some("/test/metrics".to_string()));
        assert_eq!(config.get("reports_dir"), Some("/test/reports".to_string()));
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
        assert_eq!(config.database.anon_key, "new_token");
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
        assert_eq!(config.database.anon_key, defaults.database.anon_key);

        assert!(config.unset("endpoint", &defaults).is_ok());
        assert_eq!(config.database.endpoint, defaults.database.endpoint);

        assert!(config.unset("metrics_dir", &defaults).is_ok());
        assert_eq!(config.paths.metrics_dir, defaults.paths.metrics_dir);

        assert!(config.unset("reports_dir", &defaults).is_ok());
        assert_eq!(config.paths.reports_dir, defaults.paths.reports_dir);
    }
}
