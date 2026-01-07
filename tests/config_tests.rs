//! Integration tests for configuration management

use nu_analytics::config::{Config, ConfigOverrides};
use std::fs;
use std::path::PathBuf;
use tempfile::TempDir;

/// Helper to create a temporary config directory
fn setup_temp_config() -> (TempDir, PathBuf) {
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let config_file = temp_dir.path().join("config.toml");
    (temp_dir, config_file)
}

#[test]
fn test_config_from_defaults() {
    let config = Config::from_defaults();

    // Should have non-empty defaults for critical fields
    assert!(
        !config.logging.level.is_empty(),
        "Default log level should not be empty"
    );
    assert!(
        !config.paths.metrics_dir.is_empty(),
        "Default metrics_dir should not be empty"
    );
    assert!(
        !config.paths.reports_dir.is_empty(),
        "Default reports_dir should not be empty"
    );
}

#[test]
fn test_config_from_toml_basic() {
    let toml_str = r#"
[logging]
level = "info"
file = "/tmp/test.log"
verbose = true

[database]
token = "test_token"
endpoint = "https://example.com"

[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"
"#;

    let config = Config::from_toml(toml_str).expect("Failed to parse TOML");

    assert_eq!(config.logging.level, "info");
    assert_eq!(config.logging.file, "/tmp/test.log");
    assert!(config.logging.verbose);
    assert_eq!(config.database.token, "test_token");
    assert_eq!(config.database.endpoint, "https://example.com");
    assert_eq!(config.paths.metrics_dir, "./metrics");
    assert_eq!(config.paths.reports_dir, "./reports");
}

#[test]
fn test_config_from_toml_partial() {
    // Test that missing fields within sections use defaults
    let toml_str = r#"
[logging]
level = "error"

[database]

[paths]
"#;

    let config = Config::from_toml(toml_str).expect("Failed to parse partial TOML");

    assert_eq!(config.logging.level, "error");
    assert_eq!(config.logging.file, ""); // Default empty
    assert!(!config.logging.verbose); // Default false
    assert_eq!(config.database.token, ""); // Default empty
}

#[test]
fn test_config_variable_expansion() {
    let toml_str = r#"
[logging]
file = "$NU_ANALYTICS/test.log"

[database]
endpoint = "$NU_ANALYTICS/db"

[paths]
"#;

    let config = Config::from_toml(toml_str).expect("Failed to parse TOML with variables");

    // Variable should be expanded to actual path
    assert!(config.logging.file.contains("nuanalytics"));
    assert!(!config.logging.file.contains("$NU_ANALYTICS"));
    assert!(config.database.endpoint.contains("nuanalytics"));
    assert!(!config.database.endpoint.contains("$NU_ANALYTICS"));
}

#[test]
fn test_config_get_set() {
    let mut config = Config::from_defaults();

    // Test get
    let level = config.get("level");
    assert!(level.is_some());

    // Test set
    config.set("level", "debug").expect("Failed to set level");
    assert_eq!(config.get("level").unwrap(), "debug");

    config
        .set("verbose", "true")
        .expect("Failed to set verbose");
    assert_eq!(config.get("verbose").unwrap(), "true");
    assert!(config.logging.verbose);

    // Test unknown key
    assert!(config.get("unknown_key").is_none());
    assert!(config.set("unknown_key", "value").is_err());
}

#[test]
fn test_config_unset() {
    let mut config = Config::from_defaults();
    let defaults = Config::from_defaults();

    // Change a value
    config.set("level", "debug").expect("Failed to set level");
    assert_eq!(config.logging.level, "debug");

    // Unset should restore default
    config
        .unset("level", &defaults)
        .expect("Failed to unset level");
    assert_eq!(config.logging.level, defaults.logging.level);
}

#[test]
fn test_config_save_and_load() {
    let (_temp_dir, config_file) = setup_temp_config();

    // Create and save a config
    let mut config = Config::from_defaults();
    config.set("level", "info").expect("Failed to set level");

    // Manually save to our test location
    if let Some(parent) = config_file.parent() {
        fs::create_dir_all(parent).expect("Failed to create dir");
    }
    let toml_str = toml::to_string_pretty(&config).expect("Failed to serialize");
    fs::write(&config_file, toml_str).expect("Failed to write config");

    // Load and verify
    let content = fs::read_to_string(&config_file).expect("Failed to read config");
    let loaded_config = Config::from_toml(&content).expect("Failed to parse loaded config");

    assert_eq!(loaded_config.logging.level, "info");
}

#[test]
fn test_config_overrides_apply() {
    let mut config = Config::from_defaults();

    let overrides = ConfigOverrides {
        level: Some("error".to_string()),
        file: Some("/custom/path.log".to_string()),
        verbose: Some(true),
        db_token: Some("override_token".to_string()),
        db_endpoint: Some("https://override.com".to_string()),
        metrics_dir: Some("./custom_metrics".to_string()),
        reports_dir: Some("./custom_reports".to_string()),
    };

    config.apply_overrides(&overrides);

    assert_eq!(config.logging.level, "error");
    assert_eq!(config.logging.file, "/custom/path.log");
    assert!(config.logging.verbose);
    assert_eq!(config.database.token, "override_token");
    assert_eq!(config.database.endpoint, "https://override.com");
    assert_eq!(config.paths.metrics_dir, "./custom_metrics");
    assert_eq!(config.paths.reports_dir, "./custom_reports");
}

#[test]
fn test_config_overrides_partial() {
    let mut config = Config::from_defaults();

    // Apply partial overrides - only level changes
    let overrides = ConfigOverrides {
        level: Some("debug".to_string()),
        file: None,
        verbose: None,
        db_token: None,
        db_endpoint: None,
        metrics_dir: None,
        reports_dir: None,
    };

    config.apply_overrides(&overrides);

    assert_eq!(config.logging.level, "debug");
}

#[test]
fn test_config_display_format() {
    let config = Config::from_defaults();
    let display_str = format!("{config}");

    // Should contain section headers (lowercase)
    assert!(display_str.contains("[logging]"));
    assert!(display_str.contains("[database]"));
    assert!(display_str.contains("[paths]"));

    // Should contain field names
    assert!(display_str.contains("level"));
    assert!(display_str.contains("file"));
    assert!(display_str.contains("verbose"));
}

#[test]
fn test_merge_defaults_adds_missing_fields() {
    // Create a minimal config with empty fields
    let toml_str = r#"
[logging]
level = "error"
file = ""
verbose = false

[database]
token = ""
endpoint = ""

[paths]
metrics_dir = ""
reports_dir = ""
"#;

    let mut config = Config::from_toml(toml_str).expect("Failed to parse minimal config");
    let defaults = Config::from_defaults();

    // Before merge, non-logging fields should be empty

    // Merge should add missing fields from defaults
    let changed = config.merge_defaults(&defaults);

    assert!(
        changed,
        "merge_defaults should return true when fields are added"
    );
}

#[test]
fn test_merge_defaults_preserves_existing() {
    let toml_str = r#"
[logging]
level = "error"
file = "/my/custom/path.log"
verbose = false

[database]
token = ""
endpoint = ""

[paths]
metrics_dir = ""
reports_dir = ""
"#;

    let mut config = Config::from_toml(toml_str).expect("Failed to parse config");
    let defaults = Config::from_defaults();

    config.merge_defaults(&defaults);

    // Custom values should be preserved
    assert_eq!(config.logging.level, "error");
    assert_eq!(config.logging.file, "/my/custom/path.log");
}

#[test]
fn test_get_nuanalytics_dir() {
    let dir = Config::get_nuanalytics_dir();

    // Should contain "nuanalytics" in the path
    assert!(dir.to_string_lossy().contains("nuanalytics"));

    // Should not be empty or just "."
    assert_ne!(dir, PathBuf::from("."));
}

#[test]
fn test_get_config_file_path() {
    let path = Config::get_config_file_path();

    // Should end with config.toml or dconfig.toml
    let path_str = path.to_string_lossy();
    assert!(path_str.ends_with("config.toml") || path_str.ends_with("dconfig.toml"));
}
