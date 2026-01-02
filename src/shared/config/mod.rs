//! Configuration module for `NuAnalytics`

use serde::{Deserialize, Serialize};

/// Logging configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    /// Log level (error, warn, info, debug)
    pub level: String,
    /// Log file path
    pub file: String,
    /// Enable verbose output
    pub verbose: bool,
}

/// Database configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Database token/connection string
    pub token: String,
    /// Database endpoint
    pub endpoint: String,
}

/// Paths configuration
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PathsConfig {
    /// Directory for curriculum plans
    pub plans_dir: String,
    /// Directory for output files
    pub out_dir: String,
}

/// Main configuration structure
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    /// Logging settings
    #[serde(rename = "Logging", default)]
    pub logging: LoggingConfig,
    /// Database settings
    #[serde(default)]
    pub database: DatabaseConfig,
    /// Path settings
    #[serde(default)]
    pub paths: PathsConfig,
}

impl Config {
    /// Initialize config from a TOML string
    ///
    /// # Errors
    /// Returns an error if the TOML cannot be parsed
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(toml_str)
    }

    /// Initialize config from defaults (TOML string)
    ///
    /// # Panics
    /// Panics if the compiled-in defaults TOML cannot be parsed
    #[must_use]
    pub fn from_defaults(defaults_toml: &str) -> Self {
        Self::from_toml(defaults_toml).expect("Failed to parse compiled-in default configuration")
    }
}
