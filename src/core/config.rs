//! Configuration module for `NuAnalytics`

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::PathBuf;

/// Default CLI configuration loaded based on build profile.
/// Uses release defaults in release mode, debug defaults in debug mode.
#[cfg(not(debug_assertions))]
const CONFIG_DEFAULTS: &str = include_str!("../assets/DefaultCLIConfigRelease.toml");

#[cfg(debug_assertions)]
const CONFIG_DEFAULTS: &str = include_str!("../assets/DefaultCLIConfigDebug.toml");

#[cfg(not(debug_assertions))]
const CONFIG_FILE_NAME: &str = "config.toml";

#[cfg(debug_assertions)]
const CONFIG_FILE_NAME: &str = "dconfig.toml";

/// Logging configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (error, warn, info, debug)
    #[serde(default)]
    pub level: String,
    /// Log file path
    #[serde(default)]
    pub file: String,
    /// Enable verbose output
    #[serde(default)]
    pub verbose: bool,
}

/// Database configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database token/connection string
    #[serde(default)]
    pub token: String,
    /// Database endpoint
    #[serde(default)]
    pub endpoint: String,
}

/// Paths configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Directory for metrics CSV output files
    #[serde(default)]
    pub metrics_dir: String,
    /// Directory for report output files
    #[serde(default)]
    pub reports_dir: String,
}

/// Main configuration structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Logging settings
    pub logging: LoggingConfig,
    /// Database settings
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Path settings
    #[serde(default)]
    pub paths: PathsConfig,
}

/// Optional CLI overrides for configuration values
#[derive(Debug, Clone, Default)]
pub struct ConfigOverrides {
    /// Override logging level
    pub level: Option<String>,
    /// Override log file path
    pub file: Option<String>,
    /// Override verbose flag
    pub verbose: Option<bool>,
    /// Override database token
    pub db_token: Option<String>,
    /// Override database endpoint
    pub db_endpoint: Option<String>,
    /// Override metrics output directory
    pub metrics_dir: Option<String>,
    /// Override reports output directory
    pub reports_dir: Option<String>,
}

impl Config {
    /// Get the `$NU_ANALYTICS` directory path
    ///
    /// Returns:
    /// - Linux: `~/.config/nuanalytics`
    /// - macOS: `~/Library/Application Support/nuanalytics`
    /// - Windows: `%APPDATA%\nuanalytics`
    #[must_use]
    pub fn get_nuanalytics_dir() -> PathBuf {
        dirs::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("nuanalytics")
    }

    /// Merge missing fields from defaults into this config
    ///
    /// This method is used when loading configuration to ensure that newly added
    /// configuration fields are populated with their default values. Only fields
    /// that are empty in the current config and non-empty in defaults are updated.
    ///
    /// # Returns
    ///
    /// `true` if any fields were added/changed, `false` otherwise
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut config = Config::from_toml(old_config_str)?;
    /// let defaults = Config::from_defaults();
    /// if config.merge_defaults(&defaults) {
    ///     // Config was updated with new fields
    ///     config.save()?;
    /// }
    /// ```
    #[allow(clippy::useless_let_if_seq)]
    pub fn merge_defaults(&mut self, defaults: &Self) -> bool {
        let mut changed = false;

        // Merge logging fields - only if they're empty (use defaults for empty values)
        if self.logging.level.is_empty() && !defaults.logging.level.is_empty() {
            self.logging.level.clone_from(&defaults.logging.level);
            changed = true;
        }
        if self.logging.file.is_empty() && !defaults.logging.file.is_empty() {
            self.logging.file.clone_from(&defaults.logging.file);
            changed = true;
        }

        // Merge database fields - only add if default is non-empty
        if self.database.token.is_empty() && !defaults.database.token.is_empty() {
            self.database.token.clone_from(&defaults.database.token);
            changed = true;
        }
        if self.database.endpoint.is_empty() && !defaults.database.endpoint.is_empty() {
            self.database
                .endpoint
                .clone_from(&defaults.database.endpoint);
            changed = true;
        }

        // Merge paths fields
        if self.paths.metrics_dir.is_empty() && !defaults.paths.metrics_dir.is_empty() {
            self.paths
                .metrics_dir
                .clone_from(&defaults.paths.metrics_dir);
            changed = true;
        }
        if self.paths.reports_dir.is_empty() && !defaults.paths.reports_dir.is_empty() {
            self.paths
                .reports_dir
                .clone_from(&defaults.paths.reports_dir);
            changed = true;
        }

        changed
    }

    /// Apply CLI-provided overrides onto the loaded configuration
    ///
    /// This allows command-line arguments to override configuration file values
    /// without modifying the persistent configuration file. Only non-`None` values
    /// in the overrides struct will replace config values.
    ///
    /// # Arguments
    ///
    /// * `overrides` - A `ConfigOverrides` struct with optional override values
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let mut config = Config::load();
    /// let overrides = ConfigOverrides {
    ///     level: Some("debug".to_string()),
    ///     ..Default::default()
    /// };
    /// config.apply_overrides(&overrides);
    /// // config.logging.level is now "debug" for this run only
    /// ```
    pub fn apply_overrides(&mut self, overrides: &ConfigOverrides) {
        if let Some(level) = &overrides.level {
            self.logging.level.clone_from(level);
        }
        if let Some(file) = &overrides.file {
            self.logging.file.clone_from(file);
        }
        if let Some(verbose) = overrides.verbose {
            self.logging.verbose = verbose;
        }

        if let Some(token) = &overrides.db_token {
            self.database.token.clone_from(token);
        }
        if let Some(endpoint) = &overrides.db_endpoint {
            self.database.endpoint.clone_from(endpoint);
        }

        if let Some(metrics_dir) = &overrides.metrics_dir {
            self.paths.metrics_dir.clone_from(metrics_dir);
        }
        if let Some(reports_dir) = &overrides.reports_dir {
            self.paths.reports_dir.clone_from(reports_dir);
        }
    }

    /// Get the user config file path
    ///
    /// Returns the full path to the configuration file:
    /// - `config.toml` for release builds
    /// - `dconfig.toml` for debug builds (allows separate debug config)
    ///
    /// The file is located in the directory returned by [`get_nuanalytics_dir`].
    ///
    /// [`get_nuanalytics_dir`]: Self::get_nuanalytics_dir
    #[must_use]
    pub fn get_config_file_path() -> PathBuf {
        Self::get_nuanalytics_dir().join(CONFIG_FILE_NAME)
    }

    /// Expand `$NU_ANALYTICS` variable in a string
    ///
    /// Replaces occurrences of `$NU_ANALYTICS` with the actual nuanalytics
    /// directory path. This allows configuration values to reference the
    /// config directory dynamically.
    ///
    /// # Arguments
    ///
    /// * `value` - The string potentially containing `$NU_ANALYTICS`
    ///
    /// # Returns
    ///
    /// The string with `$NU_ANALYTICS` expanded to the actual path
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let expanded = Config::expand_variables("$NU_ANALYTICS/logs/app.log");
    /// // Returns something like "/home/user/.config/nuanalytics/logs/app.log"
    /// ```
    #[must_use]
    fn expand_variables(value: &str) -> String {
        if value.contains("$NU_ANALYTICS") {
            let nu_analytics_dir = Self::get_nuanalytics_dir();
            value.replace("$NU_ANALYTICS", nu_analytics_dir.to_str().unwrap_or("."))
        } else {
            value.to_string()
        }
    }

    /// Initialize config from a TOML string
    ///
    /// Parses a TOML configuration string and expands any `$NU_ANALYTICS` variables
    /// in the values. Missing fields will use their serde defaults (typically empty
    /// strings or false).
    ///
    /// # Arguments
    ///
    /// * `toml_str` - A TOML-formatted configuration string
    ///
    /// # Errors
    ///
    /// Returns an error if the TOML cannot be parsed or doesn't match the expected schema
    ///
    /// # Examples
    ///
    /// ```ignore
    /// let config = Config::from_toml(r#"
    /// [Logging]
    /// level = "info"
    /// file = "$NU_ANALYTICS/app.log"
    /// "#)?;
    /// ```
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(toml_str)?;

        // Expand variables in config values
        config.logging.file = Self::expand_variables(&config.logging.file);
        config.database.token = Self::expand_variables(&config.database.token);
        config.database.endpoint = Self::expand_variables(&config.database.endpoint);
        config.paths.metrics_dir = Self::expand_variables(&config.paths.metrics_dir);
        config.paths.reports_dir = Self::expand_variables(&config.paths.reports_dir);

        Ok(config)
    }

    /// Load configuration from embedded defaults
    ///
    /// Loads the compiled-in default configuration that is bundled with the binary.
    /// The defaults differ between debug and release builds:
    /// - Debug: Uses `DefaultCLIConfigDebug.toml`
    /// - Release: Uses `DefaultCLIConfigRelease.toml`
    ///
    /// # Returns
    /// A `Config` instance with all values set to their defaults.
    ///
    /// # Panics
    /// Panics if the embedded default configuration is invalid TOML or cannot be parsed.
    /// This should never happen in practice since the defaults are compiled into the binary.
    ///
    /// # Examples
    /// ```ignore
    /// let config = Config::from_defaults();
    /// assert_eq!(config.logging.level, "info");
    /// ```
    #[must_use]
    pub fn from_defaults() -> Self {
        Self::from_toml(CONFIG_DEFAULTS).expect("Failed to parse compiled-in default configuration")
    }

    /// Load configuration from file, or create from defaults if not found
    ///
    /// This is the primary way to load configuration. It handles several scenarios:
    /// - If config file exists: Loads from file, merges missing fields from defaults, saves updated config
    /// - If config file doesn't exist (first run): Creates config directory if needed, loads defaults, saves to file
    ///
    /// The merge behavior ensures that upgrading the application automatically adds new config
    /// fields while preserving existing user settings.
    ///
    /// # Returns
    /// A `Config` instance loaded from file or defaults. Falls back to defaults if any error occurs
    /// during loading.
    ///
    /// # Examples
    /// ```ignore
    /// let config = Config::load();
    /// // Config is now loaded from ~/.config/nuanalytics/config.toml (or defaults if first run)
    /// ```
    #[must_use]
    pub fn load() -> Self {
        let config_file = Self::get_config_file_path();
        let defaults = Self::from_defaults();

        if config_file.exists() {
            if let Ok(content) = fs::read_to_string(&config_file) {
                if let Ok(mut config) = Self::from_toml(&content) {
                    // Merge any missing fields from defaults
                    if config.merge_defaults(&defaults) {
                        // Save the updated config with new fields
                        let _ = config.save();
                    }
                    return config;
                }
            }
        } else {
            // First run: create directory and config file from defaults

            // Create the directory if it doesn't exist
            if let Some(parent) = config_file.parent() {
                let _ = fs::create_dir_all(parent);
            }

            // Save the default config
            let _ = defaults.save();

            return defaults;
        }

        defaults
    }

    /// Save configuration to file
    ///
    /// Serializes the current configuration to TOML format and writes it to the
    /// platform-specific config file. The config directory will be created if it
    /// doesn't exist.
    ///
    /// The saved file will use the format:
    /// ```toml
    /// [Logging]
    /// level = "info"
    /// file = "$NU_ANALYTICS/logs/nuanalytics.log"
    /// verbose = false
    ///
    /// [Database]
    /// token = "your-token"
    /// endpoint = "https://api.example.com"
    ///
    /// [Paths]
    /// metrics_dir = "$NU_ANALYTICS/metrics"
    /// reports_dir = "$NU_ANALYTICS/reports"
    /// ```
    ///
    /// # Errors
    /// Returns an error if:
    /// - The config cannot be serialized to TOML (shouldn't happen)
    /// - The config directory cannot be created
    /// - The file cannot be written (permissions, disk full, etc.)
    ///
    /// # Examples
    /// ```ignore
    /// let mut config = Config::load()?;
    /// config.logging.level = "debug".to_string();
    /// config.save()?;
    /// ```
    pub fn save(&self) -> Result<(), Box<dyn std::error::Error>> {
        let config_file = Self::get_config_file_path();
        if let Some(parent) = config_file.parent() {
            fs::create_dir_all(parent)?;
        }
        let toml_str = toml::to_string_pretty(self)?;
        fs::write(&config_file, toml_str)?;
        Ok(())
    }

    /// Get a configuration value by key
    ///
    /// Retrieves a configuration value using a string key that maps to the config structure.
    /// Supports all config fields in the format `section.field` or just `field` for top-level fields.
    ///
    /// Supported keys:
    /// - `level`: Logging level ("debug", "info", "warn", "error")
    /// - `file`: Log file path
    /// - `verbose`: Verbose logging boolean
    /// - `token`: Database authentication token
    /// - `endpoint`: Database API endpoint
    /// - `metrics_dir`: Metrics output directory path
    /// - `reports_dir`: Reports output directory path
    ///
    /// # Arguments
    /// - `key`: The configuration key to retrieve
    ///
    /// # Returns
    /// - `Some(String)`: The configuration value as a string
    /// - `None`: If the key is not recognized
    ///
    /// # Examples
    /// ```ignore
    /// let config = Config::load()?;
    /// if let Some(level) = config.get("level") {
    ///     println!("Current log level: {}", level);
    /// }
    /// ```
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "level" => Some(self.logging.level.clone()),
            "file" => Some(self.logging.file.clone()),
            "verbose" => Some(self.logging.verbose.to_string()),
            "token" => Some(self.database.token.clone()),
            "endpoint" => Some(self.database.endpoint.clone()),
            "metrics_dir" | "metrics-dir" => Some(self.paths.metrics_dir.clone()),
            "reports_dir" | "reports-dir" => Some(self.paths.reports_dir.clone()),
            _ => None,
        }
    }

    /// Set a configuration value by key
    ///
    /// Updates a configuration value using a string key and value. The value will be
    /// validated and converted to the appropriate type.
    ///
    /// Supported keys and their value formats:
    /// - `level`: String ("debug", "info", "warn", "error", "trace", "off")
    /// - `file`: String (file path, can include `$NU_ANALYTICS`)
    /// - `verbose`: Boolean ("true" or "false")
    /// - `token`: String (any value)
    /// - `endpoint`: String (typically a URL)
    /// - `metrics_dir`: String (directory path for metrics CSV files)
    /// - `reports_dir`: String (directory path for report files)
    ///
    /// Note: This method updates the in-memory config. Call [`save()`](Config::save) to persist changes.
    ///
    /// # Arguments
    /// - `key`: The configuration key to set
    /// - `value`: The new value as a string
    ///
    /// # Errors
    /// Returns an error if:
    /// - The key is not recognized
    /// - The value cannot be parsed (e.g., "maybe" for verbose boolean)
    ///
    /// # Examples
    /// ```ignore
    /// let mut config = Config::load()?;
    /// config.set("level", "debug")?;
    /// config.set("verbose", "true")?;
    /// config.save()?;
    /// ```
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        match key {
            "level" => self.logging.level = value.to_string(),
            "file" => self.logging.file = value.to_string(),
            "verbose" => {
                self.logging.verbose = value
                    .parse::<bool>()
                    .map_err(|_| format!("Invalid boolean value for 'verbose': '{value}'"))?;
            }
            "token" => self.database.token = value.to_string(),
            "endpoint" => self.database.endpoint = value.to_string(),
            "metrics_dir" | "metrics-dir" => self.paths.metrics_dir = value.to_string(),
            "reports_dir" | "reports-dir" => self.paths.reports_dir = value.to_string(),
            _ => return Err(format!("Unknown config key: '{key}'")),
        }
        Ok(())
    }

    /// Unset a configuration value by key (reset to default)
    ///
    /// Resets a single configuration value to its default value. This is useful for
    /// reverting individual settings without losing all customizations.
    ///
    /// The default value is taken from the provided defaults config (typically from
    /// [`from_defaults()`](Config::from_defaults)).
    ///
    /// Note: This method updates the in-memory config. Call [`save()`](Config::save) to persist changes.
    ///
    /// # Arguments
    /// - `key`: The configuration key to reset
    /// - `defaults`: A config instance containing default values
    ///
    /// # Errors
    /// Returns an error if the key is not recognized.
    ///
    /// # Examples
    /// ```ignore
    /// let mut config = Config::load()?;
    /// let defaults = Config::from_defaults();
    ///
    /// config.set("level", "trace")?;
    /// config.unset("level", &defaults)?;  // Resets to "info"
    /// config.save()?;
    /// ```
    pub fn unset(&mut self, key: &str, defaults: &Self) -> Result<(), String> {
        match key {
            "level" => self.logging.level.clone_from(&defaults.logging.level),
            "file" => self.logging.file.clone_from(&defaults.logging.file),
            "verbose" => self.logging.verbose = defaults.logging.verbose,
            "token" => self.database.token.clone_from(&defaults.database.token),
            "endpoint" => self
                .database
                .endpoint
                .clone_from(&defaults.database.endpoint),
            "metrics_dir" | "metrics-dir" => self
                .paths
                .metrics_dir
                .clone_from(&defaults.paths.metrics_dir),
            "reports_dir" | "reports-dir" => self
                .paths
                .reports_dir
                .clone_from(&defaults.paths.reports_dir),
            _ => return Err(format!("Unknown config key: '{key}'")),
        }
        Ok(())
    }

    /// Reset all configuration to defaults
    ///
    /// Deletes the configuration file, causing the next [`load()`](Config::load) call to
    /// recreate it from defaults. This is a destructive operation that removes all user
    /// customizations.
    ///
    /// If the config file doesn't exist, this method succeeds without doing anything.
    ///
    /// # Safety
    /// This is a destructive operation. The CLI typically requires user confirmation
    /// before calling this method.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The config file exists but cannot be deleted (permissions, file locked, etc.)
    ///
    /// # Examples
    /// ```ignore
    /// // Typically preceded by user confirmation
    /// Config::reset()?;
    /// println!("Configuration reset to defaults");
    ///
    /// // Next load will recreate from defaults
    /// let config = Config::load()?;
    /// ```
    pub fn reset() -> Result<(), std::io::Error> {
        let config_file = Self::get_config_file_path();
        if config_file.exists() {
            fs::remove_file(config_file)?;
        }
        Ok(())
    }
}

impl fmt::Display for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        writeln!(f, "[logging]")?;
        writeln!(f, "  level = \"{}\"", self.logging.level)?;
        writeln!(f, "  file = \"{}\"", self.logging.file)?;
        writeln!(f, "  verbose = {}", self.logging.verbose)?;

        writeln!(f, "\n[database]")?;
        writeln!(f, "  token = \"{}\"", self.database.token)?;
        writeln!(f, "  endpoint = \"{}\"", self.database.endpoint)?;

        writeln!(f, "\n[paths]")?;
        writeln!(f, "  metrics_dir = \"{}\"", self.paths.metrics_dir)?;
        writeln!(f, "  reports_dir = \"{}\"", self.paths.reports_dir)?;

        Ok(())
    }
}
