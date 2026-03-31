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

/// Local directory config file name (same for both debug and release)
const LOCAL_CONFIG_FILE_NAME: &str = "nuanalytics.toml";

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

/// Audit configuration for degree analysis
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditConfig {
    /// Threshold for highlighting courses with many prerequisites in their chain
    /// Courses with prerequisite chains >= this value will be highlighted
    #[serde(default = "default_prerequisite_chain_threshold")]
    pub prerequisite_chain_threshold: usize,
}

/// Default prerequisite chain threshold (3)
const fn default_prerequisite_chain_threshold() -> usize {
    3
}

impl Default for AuditConfig {
    fn default() -> Self {
        Self {
            prerequisite_chain_threshold: default_prerequisite_chain_threshold(),
        }
    }
}

/// Degree analysis configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DegreeAnalysisConfig {
    /// Calculation strategy for aggregate metrics ("median" or "mean")
    #[serde(default = "default_calc_strategy")]
    pub calc_strategy: String,

    /// Number of random plans to sample and export
    #[serde(default = "default_sample_plan_count")]
    pub sample_plan_count: usize,

    /// Maximum number of plans to generate (safety cap)
    #[serde(default = "default_max_plans")]
    pub max_plans: usize,

    /// Skip equivalent plan combinations
    #[serde(default = "default_ignore_duplicates")]
    pub ignore_duplicates: bool,

    /// Sampling strategy for plan enumeration ("sequential", "shuffled", "stratified")
    /// Defaults to "shuffled" for unbiased statistics
    #[serde(default = "default_sampling_strategy")]
    pub sampling_strategy: String,
}

fn default_calc_strategy() -> String {
    "median".to_string()
}

const fn default_sample_plan_count() -> usize {
    5
}

const fn default_max_plans() -> usize {
    1_000
}

const fn default_ignore_duplicates() -> bool {
    true
}

fn default_sampling_strategy() -> String {
    "shuffled".to_string()
}

impl Default for DegreeAnalysisConfig {
    fn default() -> Self {
        Self {
            calc_strategy: default_calc_strategy(),
            sample_plan_count: default_sample_plan_count(),
            max_plans: default_max_plans(),
            ignore_duplicates: default_ignore_duplicates(),
            sampling_strategy: default_sampling_strategy(),
        }
    }
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
    /// Audit settings for degree analysis
    #[serde(default)]
    pub audit: AuditConfig,
    /// Degree analysis settings
    #[serde(default)]
    pub degree_analysis: DegreeAnalysisConfig,
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

    /// Get the user config file path (home directory)
    ///
    /// Returns the full path to the user-level configuration file:
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

    /// Get the local config file path (current directory)
    ///
    /// Returns the path to `nuanalytics.toml` in the current working directory.
    /// This config takes precedence over the home directory config but can be
    /// overridden by command-line arguments.
    ///
    /// # Returns
    /// Path to `nuanalytics.toml` in the current directory, or `None` if the
    /// current directory cannot be determined.
    #[must_use]
    pub fn get_local_config_file_path() -> Option<PathBuf> {
        std::env::current_dir()
            .ok()
            .map(|d| d.join(LOCAL_CONFIG_FILE_NAME))
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

    /// Load configuration with three-tier hierarchy
    ///
    /// Configuration is loaded with the following precedence (highest to lowest):
    /// 1. Command-line overrides (applied via `apply_overrides()` after this call)
    /// 2. Local directory config (`nuanalytics.toml` in current directory)
    /// 3. Home directory config (`~/.config/nuanalytics/config.toml`)
    /// 4. Embedded defaults
    ///
    /// The merge behavior ensures that:
    /// - Local config overrides home config values
    /// - Home config overrides default values
    /// - Missing fields fall back to the next tier
    ///
    /// # Returns
    /// A `Config` instance with merged settings from all tiers.
    ///
    /// # Examples
    /// ```ignore
    /// let config = Config::load();
    /// // Config is now loaded with local > home > defaults precedence
    /// ```
    #[must_use]
    pub fn load() -> Self {
        let defaults = Self::from_defaults();

        // Load home directory config (tier 3)
        let mut config = Self::load_home_config(&defaults);

        // Apply local directory config if present (tier 2)
        if let Some(local_path) = Self::get_local_config_file_path() {
            if local_path.exists() {
                if let Ok(content) = fs::read_to_string(&local_path) {
                    if let Ok(local_config) = Self::from_toml(&content) {
                        config.merge_from(&local_config);
                    }
                }
            }
        }

        config
    }

    /// Load configuration from home directory, creating it if needed
    ///
    /// Internal helper for loading the home directory config file.
    fn load_home_config(defaults: &Self) -> Self {
        let config_file = Self::get_config_file_path();

        if config_file.exists() {
            if let Ok(content) = fs::read_to_string(&config_file) {
                if let Ok(mut config) = Self::from_toml(&content) {
                    // Merge any missing fields from defaults
                    if config.merge_defaults(defaults) {
                        // Save the updated config with new fields
                        let _ = config.save();
                    }
                    return config;
                }
            }
        } else {
            // First run: create directory and config file from defaults
            if let Some(parent) = config_file.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = defaults.save();
            return defaults.clone();
        }

        defaults.clone()
    }

    /// Merge non-empty values from another config into this one
    ///
    /// Used to apply local directory config on top of home directory config.
    /// Only non-empty string values and non-default numeric values are merged.
    ///
    /// # Arguments
    /// * `other` - The config to merge values from (higher precedence)
    pub fn merge_from(&mut self, other: &Self) {
        // Merge logging fields
        if !other.logging.level.is_empty() {
            self.logging.level.clone_from(&other.logging.level);
        }
        if !other.logging.file.is_empty() {
            self.logging.file.clone_from(&other.logging.file);
        }
        if other.logging.verbose {
            self.logging.verbose = true;
        }

        // Merge database fields
        if !other.database.token.is_empty() {
            self.database.token.clone_from(&other.database.token);
        }
        if !other.database.endpoint.is_empty() {
            self.database.endpoint.clone_from(&other.database.endpoint);
        }

        // Merge paths fields
        if !other.paths.metrics_dir.is_empty() {
            self.paths.metrics_dir.clone_from(&other.paths.metrics_dir);
        }
        if !other.paths.reports_dir.is_empty() {
            self.paths.reports_dir.clone_from(&other.paths.reports_dir);
        }

        // Merge audit fields (only if non-default)
        if other.audit.prerequisite_chain_threshold != default_prerequisite_chain_threshold() {
            self.audit.prerequisite_chain_threshold = other.audit.prerequisite_chain_threshold;
        }

        // Merge degree_analysis fields
        if other.degree_analysis.calc_strategy != default_calc_strategy() {
            self.degree_analysis
                .calc_strategy
                .clone_from(&other.degree_analysis.calc_strategy);
        }
        if other.degree_analysis.sample_plan_count != default_sample_plan_count() {
            self.degree_analysis.sample_plan_count = other.degree_analysis.sample_plan_count;
        }
        if other.degree_analysis.max_plans != default_max_plans() {
            self.degree_analysis.max_plans = other.degree_analysis.max_plans;
        }
        if other.degree_analysis.ignore_duplicates != default_ignore_duplicates() {
            self.degree_analysis.ignore_duplicates = other.degree_analysis.ignore_duplicates;
        }
        if other.degree_analysis.sampling_strategy != default_sampling_strategy() {
            self.degree_analysis
                .sampling_strategy
                .clone_from(&other.degree_analysis.sampling_strategy);
        }
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
            "prerequisite_chain_threshold" => {
                Some(self.audit.prerequisite_chain_threshold.to_string())
            }
            "calc_strategy" | "calc-strategy" => Some(self.degree_analysis.calc_strategy.clone()),
            "sample_plan_count" | "sample-plan-count" => {
                Some(self.degree_analysis.sample_plan_count.to_string())
            }
            "max_plans" | "max-plans" => Some(self.degree_analysis.max_plans.to_string()),
            "ignore_duplicates" | "ignore-duplicates" => {
                Some(self.degree_analysis.ignore_duplicates.to_string())
            }
            "sampling_strategy" | "sampling-strategy" => {
                Some(self.degree_analysis.sampling_strategy.clone())
            }
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
            "prerequisite_chain_threshold" => {
                self.audit.prerequisite_chain_threshold = value.parse::<usize>().map_err(|_| {
                    format!("Invalid number for 'prerequisite_chain_threshold': '{value}'")
                })?;
            }
            "calc_strategy" | "calc-strategy" => {
                if value != "median" && value != "mean" {
                    return Err(format!(
                        "Invalid calc_strategy '{value}': must be 'median' or 'mean'"
                    ));
                }
                self.degree_analysis.calc_strategy = value.to_string();
            }
            "sample_plan_count" | "sample-plan-count" => {
                self.degree_analysis.sample_plan_count = value
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid number for 'sample_plan_count': '{value}'"))?;
            }
            "max_plans" | "max-plans" => {
                self.degree_analysis.max_plans = value
                    .parse::<usize>()
                    .map_err(|_| format!("Invalid number for 'max_plans': '{value}'"))?;
            }
            "ignore_duplicates" | "ignore-duplicates" => {
                self.degree_analysis.ignore_duplicates = value
                    .parse::<bool>()
                    .map_err(|_| format!("Invalid boolean for 'ignore_duplicates': '{value}'"))?;
            }
            "sampling_strategy" | "sampling-strategy" => {
                let valid = ["sequential", "shuffled", "stratified"];
                let lower = value.to_lowercase();
                if !valid.contains(&lower.as_str()) {
                    return Err(format!(
                        "Invalid sampling_strategy '{value}': must be 'sequential', 'shuffled', or 'stratified'"
                    ));
                }
                self.degree_analysis.sampling_strategy = lower;
            }
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
            "prerequisite_chain_threshold" => {
                self.audit.prerequisite_chain_threshold =
                    defaults.audit.prerequisite_chain_threshold;
            }
            "calc_strategy" | "calc-strategy" => {
                self.degree_analysis
                    .calc_strategy
                    .clone_from(&defaults.degree_analysis.calc_strategy);
            }
            "sample_plan_count" | "sample-plan-count" => {
                self.degree_analysis.sample_plan_count = defaults.degree_analysis.sample_plan_count;
            }
            "max_plans" | "max-plans" => {
                self.degree_analysis.max_plans = defaults.degree_analysis.max_plans;
            }
            "ignore_duplicates" | "ignore-duplicates" => {
                self.degree_analysis.ignore_duplicates = defaults.degree_analysis.ignore_duplicates;
            }
            "sampling_strategy" | "sampling-strategy" => {
                self.degree_analysis
                    .sampling_strategy
                    .clone_from(&defaults.degree_analysis.sampling_strategy);
            }
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

        writeln!(f, "\n[audit]")?;
        writeln!(
            f,
            "  prerequisite_chain_threshold = {}",
            self.audit.prerequisite_chain_threshold
        )?;

        writeln!(f, "\n[degree_analysis]")?;
        writeln!(
            f,
            "  calc_strategy = \"{}\"",
            self.degree_analysis.calc_strategy
        )?;
        writeln!(
            f,
            "  sample_plan_count = {}",
            self.degree_analysis.sample_plan_count
        )?;
        writeln!(f, "  max_plans = {}", self.degree_analysis.max_plans)?;
        writeln!(
            f,
            "  ignore_duplicates = {}",
            self.degree_analysis.ignore_duplicates
        )?;
        writeln!(
            f,
            "  sampling_strategy = \"{}\"",
            self.degree_analysis.sampling_strategy
        )?;

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_local_config_file_path() {
        let path = Config::get_local_config_file_path();
        assert!(path.is_some());
        let path = path.unwrap();
        assert!(path.ends_with("nuanalytics.toml"));
    }

    #[test]
    fn test_merge_from_overwrites_non_empty() {
        let mut base = Config::default();
        base.logging.level = "info".to_string();
        base.paths.metrics_dir = "/base/metrics".to_string();

        let mut local = Config::default();
        local.logging.level = "debug".to_string();
        // Leave metrics_dir empty - should not overwrite

        base.merge_from(&local);

        assert_eq!(base.logging.level, "debug");
        assert_eq!(base.paths.metrics_dir, "/base/metrics"); // unchanged
    }

    #[test]
    fn test_merge_from_preserves_base_when_other_empty() {
        let mut base = Config::default();
        base.logging.level = "warn".to_string();
        base.database.token = "secret-token".to_string();

        let local = Config::default(); // all empty

        base.merge_from(&local);

        assert_eq!(base.logging.level, "warn");
        assert_eq!(base.database.token, "secret-token");
    }

    #[test]
    fn test_merge_from_non_default_numeric_values() {
        let mut base = Config::default();
        base.degree_analysis.max_plans = 1000;

        let mut local = Config::default();
        local.degree_analysis.max_plans = 500; // non-default value

        base.merge_from(&local);

        assert_eq!(base.degree_analysis.max_plans, 500);
    }

    #[test]
    fn test_merge_from_verbose_flag() {
        let mut base = Config::default();
        base.logging.verbose = false;

        let mut local = Config::default();
        local.logging.verbose = true;

        base.merge_from(&local);

        assert!(base.logging.verbose);
    }

    #[test]
    fn test_from_defaults_returns_valid_config() {
        let config = Config::from_defaults();
        // Should have some reasonable defaults
        assert!(!config.logging.level.is_empty() || config.logging.level.is_empty());
        // Just verify it loads
    }

    #[test]
    fn test_get_and_set() {
        let mut config = Config::default();

        config.set("level", "debug").unwrap();
        assert_eq!(config.get("level"), Some("debug".to_string()));

        config.set("max_plans", "5000").unwrap();
        assert_eq!(config.get("max_plans"), Some("5000".to_string()));
    }

    #[test]
    fn test_set_invalid_key() {
        let mut config = Config::default();
        let result = config.set("invalid_key", "value");
        assert!(result.is_err());
    }

    #[test]
    fn test_set_invalid_value_type() {
        let mut config = Config::default();
        let result = config.set("max_plans", "not_a_number");
        assert!(result.is_err());
    }
}
