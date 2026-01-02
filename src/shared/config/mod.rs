//! Configuration module for `NuAnalytics`

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::PathBuf;

/// Default CLI configuration loaded based on build profile.
/// Uses release defaults in release mode, debug defaults in debug mode.
#[cfg(not(debug_assertions))]
const CONFIG_DEFAULTS: &str = include_str!("../../assets/DefaultCLIConfigRelease.toml");

#[cfg(debug_assertions)]
const CONFIG_DEFAULTS: &str = include_str!("../../assets/DefaultCLIConfigDebug.toml");

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
    /// Directory for curriculum plans
    #[serde(default)]
    pub plans_dir: String,
    /// Directory for output files
    #[serde(default)]
    pub out_dir: String,
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
    /// Returns true if any fields were added
    #[allow(clippy::useless_let_if_seq)]
    fn merge_defaults(&mut self, defaults: &Self) -> bool {
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
        if self.paths.plans_dir.is_empty() && !defaults.paths.plans_dir.is_empty() {
            self.paths.plans_dir.clone_from(&defaults.paths.plans_dir);
            changed = true;
        }
        if self.paths.out_dir.is_empty() && !defaults.paths.out_dir.is_empty() {
            self.paths.out_dir.clone_from(&defaults.paths.out_dir);
            changed = true;
        }

        changed
    }

    /// Get the user config file path
    ///
    /// return config.toml for release
    ///        dconfig.toml for debug
    #[must_use]
    pub fn get_config_file_path() -> PathBuf {
        #[cfg(debug_assertions)]
        {
            Self::get_nuanalytics_dir().join("dconfig.toml")
        }
        #[cfg(not(debug_assertions))]
        {
            Self::get_nuanalytics_dir().join("config.toml")
        }
    }

    /// Expand `$NU_ANALYTICS` variable in a string
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
    /// # Errors
    /// Returns an error if the TOML cannot be parsed
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(toml_str)?;

        // Expand variables in config values
        config.logging.file = Self::expand_variables(&config.logging.file);
        config.database.token = Self::expand_variables(&config.database.token);
        config.database.endpoint = Self::expand_variables(&config.database.endpoint);
        config.paths.plans_dir = Self::expand_variables(&config.paths.plans_dir);
        config.paths.out_dir = Self::expand_variables(&config.paths.out_dir);

        Ok(config)
    }

    /// Initialize config from defaults (TOML string)
    ///
    /// # Panics
    /// Panics if the compiled-in defaults TOML cannot be parsed
    #[must_use]
    pub fn from_defaults() -> Self {
        Self::from_toml(CONFIG_DEFAULTS).expect("Failed to parse compiled-in default configuration")
    }

    /// Load config from user config file, creating it from defaults on first run
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

    /// Save config to user config file
    ///
    /// # Errors
    /// Returns an error if the config cannot be saved
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
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        match key {
            "level" => Some(self.logging.level.clone()),
            "file" => Some(self.logging.file.clone()),
            "verbose" => Some(self.logging.verbose.to_string()),
            "token" => Some(self.database.token.clone()),
            "endpoint" => Some(self.database.endpoint.clone()),
            "plans_dir" => Some(self.paths.plans_dir.clone()),
            "out_dir" => Some(self.paths.out_dir.clone()),
            _ => None,
        }
    }

    /// Set a configuration value by key
    ///
    /// # Errors
    /// Returns an error if the key is unknown or the value is invalid
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
            "plans_dir" => self.paths.plans_dir = value.to_string(),
            "out_dir" => self.paths.out_dir = value.to_string(),
            _ => return Err(format!("Unknown config key: '{key}'")),
        }
        Ok(())
    }

    /// Unset a configuration value by key (reset to default)
    ///
    /// # Errors
    /// Returns an error if the key is unknown
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
            "plans_dir" => self.paths.plans_dir.clone_from(&defaults.paths.plans_dir),
            "out_dir" => self.paths.out_dir.clone_from(&defaults.paths.out_dir),
            _ => return Err(format!("Unknown config key: '{key}'")),
        }
        Ok(())
    }

    /// Reset all configuration to defaults
    ///
    /// # Errors
    /// Returns an error if the config file cannot be deleted
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
        writeln!(f, "  plans_dir = \"{}\"", self.paths.plans_dir)?;
        writeln!(f, "  out_dir = \"{}\"", self.paths.out_dir)?;

        Ok(())
    }
}
