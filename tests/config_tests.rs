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
    assert_eq!(config.database.anon_key, "test_token");
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
    assert_eq!(config.database.anon_key, ""); // Default empty
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
        db_anon_key: Some("override_token".to_string()),
        db_endpoint: Some("https://override.com".to_string()),
        metrics_dir: Some("./custom_metrics".to_string()),
        reports_dir: Some("./custom_reports".to_string()),
    };

    config.apply_overrides(&overrides);

    assert_eq!(config.logging.level, "error");
    assert_eq!(config.logging.file, "/custom/path.log");
    assert!(config.logging.verbose);
    assert_eq!(config.database.anon_key, "override_token");
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
        db_anon_key: None,
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

#[test]
fn test_get_local_config_file_path() {
    let path = Config::get_local_config_file_path();

    // Should return Some path ending with nuanalytics.toml
    assert!(path.is_some());
    let path = path.unwrap();
    assert!(path.ends_with("nuanalytics.toml"));
}

#[test]
fn test_merge_from_all_fields() {
    let mut base = Config::default();
    base.logging.level = "info".to_string();
    base.paths.metrics_dir = "/base/metrics".to_string();
    base.degree_analysis.max_plans = 1000;

    let mut local = Config::default();
    local.logging.level = "debug".to_string();
    local.degree_analysis.max_plans = 500; // non-default

    base.merge_from(&local);

    assert_eq!(base.logging.level, "debug"); // overwritten
    assert_eq!(base.paths.metrics_dir, "/base/metrics"); // preserved (local empty)
    assert_eq!(base.degree_analysis.max_plans, 500); // overwritten (non-default)
}

#[test]
fn test_merge_from_verbose_only_sets_true() {
    let mut base = Config::default();
    base.logging.verbose = true;

    let local = Config::default(); // verbose is false (default)

    base.merge_from(&local);

    // verbose should stay true - we only set to true, never to false
    assert!(base.logging.verbose);
}

#[test]
fn test_normalize_key_strips_database_prefix() {
    let mut config = Config::from_defaults();
    // Set with dotted key
    config
        .set("database.endpoint", "https://example.supabase.co")
        .unwrap();
    // Retrieve with bare key — should be identical
    assert_eq!(config.get("endpoint"), config.get("database.endpoint"));
    assert_eq!(
        config.get("endpoint").unwrap(),
        "https://example.supabase.co"
    );
}

#[test]
fn test_normalize_key_bare_key_unchanged() {
    let mut config = Config::from_defaults();
    config.set("endpoint", "https://bare.supabase.co").unwrap();
    assert_eq!(config.get("endpoint").unwrap(), "https://bare.supabase.co");
}

#[test]
fn test_normalize_key_all_section_prefixes() {
    let mut config = Config::from_defaults();
    config.set("logging.level", "info").unwrap();
    assert_eq!(config.get("level"), config.get("logging.level"));

    config.set("paths.metrics_dir", "/tmp/metrics").unwrap();
    assert_eq!(config.get("metrics_dir"), config.get("paths.metrics_dir"));

    config
        .set("audit.prerequisite_chain_threshold", "5")
        .unwrap();
    assert_eq!(
        config.get("prerequisite_chain_threshold"),
        config.get("audit.prerequisite_chain_threshold")
    );

    config.set("degree_analysis.max_plans", "200").unwrap();
    assert_eq!(
        config.get("max_plans"),
        config.get("degree_analysis.max_plans")
    );
}

#[test]
fn test_normalize_key_management_key() {
    let mut config = Config::from_defaults();
    config
        .set("database.management_key", "sbp_test_token")
        .unwrap();
    assert_eq!(config.get("management_key").unwrap(), "sbp_test_token");
    assert_eq!(
        config.get("database.management_key"),
        config.get("management_key")
    );
}
