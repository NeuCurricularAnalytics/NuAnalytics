//! Configuration module for `NuAnalytics`

use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseConfig {
    /// Supabase project URL (e.g. `https://abcdefgh.supabase.co`)
    #[serde(default)]
    pub endpoint: String,
    /// Supabase anonymous (public) key — identifies the project to Supabase
    /// in the `apikey` header on every request, and is also used to bootstrap
    /// the OAuth login flow. Does **not** authorise database access on its own;
    /// every read and write also needs the user JWT stored in `auth_file`
    /// after `nuanalytics db login`.
    ///
    /// Accepts the legacy TOML key `token` for backward compatibility.
    #[serde(default, alias = "token")]
    pub anon_key: String,
    /// Enable database integration (requires `endpoint` and `anon_key` to be set)
    #[serde(default)]
    pub enabled: bool,
    /// Path to the auth session file saved by `nuanalytics db login`.
    ///
    /// Supports `$NU_ANALYTICS` variable expansion.
    /// Default differs by build profile:
    /// - Release: `$NU_ANALYTICS/auth.json`
    /// - Debug:   `.debug/dauth.json` (local to the working directory)
    #[serde(default = "default_auth_file")]
    pub auth_file: String,
    /// Supabase Personal Access Token (PAT) for the Management API.
    ///
    /// Required for `nuanalytics db exec-sql` which executes DDL and arbitrary SQL
    /// via `api.supabase.com`. Different from `anon_key` (project data API) and
    /// the OAuth session token. Generate one at:
    /// <https://app.supabase.com/account/tokens>
    #[serde(default)]
    pub management_key: String,
    /// Supabase project reference for the Management API, e.g. `abcdefgh`.
    ///
    /// Explicit rather than derived from `endpoint`: the project ref used to be the first
    /// subdomain label, which yielded `Some("db")` for `https://db.example.edu` and
    /// `Some("localhost:8000")` for a local stack — a meaningless ref sent to
    /// `api.supabase.com`. Host sniffing also breaks a legitimate cloud project served
    /// through a custom domain.
    ///
    /// Blank means "not a Supabase-cloud project", which is the correct default for a
    /// self-hosted deployment. Only `db exec-sql` uses it.
    #[serde(default)]
    pub project_ref: String,
}

/// Default auth-file path: `.debug/dauth.json` in debug builds,
/// `$NU_ANALYTICS/auth.json` (expanded at load time) in release.
fn default_auth_file() -> String {
    #[cfg(debug_assertions)]
    return ".debug/dauth.json".to_string();
    #[cfg(not(debug_assertions))]
    "$NU_ANALYTICS/auth.json".to_string()
}

/// Default Supabase Management API key — empty; must be set explicitly.
const fn default_management_key() -> String {
    String::new()
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            endpoint: String::new(),
            anon_key: String::new(),
            enabled: false,
            auth_file: default_auth_file(),
            management_key: default_management_key(),
            project_ref: String::new(),
        }
    }
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
    #[serde(default)]
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
    /// Override database anon key
    pub db_anon_key: Option<String>,
    /// Override database endpoint
    pub db_endpoint: Option<String>,
    /// Override metrics output directory
    pub metrics_dir: Option<String>,
    /// Override reports output directory
    pub reports_dir: Option<String>,
}

/// Whether a config file was read, and why not when it was not.
///
/// Read failures used to be swallowed by `if let Ok(...)`, so a malformed home config
/// silently produced default settings — including a blank endpoint that
/// [`Config::merge_defaults`] then refilled and saved.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum SourceStatus {
    /// No file at this path.
    #[default]
    Missing,
    /// Read and parsed.
    Loaded,
    /// Present but could not be read (permissions, I/O).
    Unreadable(String),
    /// Present and readable but not valid TOML for this schema.
    Malformed(String),
}

/// Which precedence tier supplied `database.endpoint`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EndpointSource {
    /// No tier set it.
    #[default]
    Unset,
    /// A `--db-endpoint` flag, which outranks every file.
    CliOverride,
    /// A project-local `nuanalytics.toml` in the working directory.
    LocalConfig,
    /// The home config (`config.toml`, or `dconfig.toml` for debug builds).
    HomeConfig,
    /// The compiled-in defaults, i.e. neither a home nor a local file supplied one.
    CompiledDefaults,
}

/// Where the effective configuration came from.
#[derive(Debug, Clone, Default)]
pub struct ConfigSources {
    /// Home config path for this build profile.
    pub home: PathBuf,
    /// Outcome of reading [`Self::home`].
    pub home_status: SourceStatus,
    /// Project-local config path, when one exists in the working directory.
    pub local: Option<PathBuf>,
    /// Outcome of reading [`Self::local`].
    pub local_status: SourceStatus,
    /// Which tier supplied the database endpoint.
    pub endpoint_from: EndpointSource,
}

impl ConfigSources {
    /// Human-readable provenance lines, most specific first.
    ///
    /// Includes any read/parse failure, because a config that could not be read is the
    /// likeliest reason the effective settings are not what the user expects.
    #[must_use]
    pub fn describe(&self) -> Vec<String> {
        let mut out = Vec::new();
        let winner = match self.endpoint_from {
            EndpointSource::CliOverride => {
                "--db-endpoint (overrides every config file)".to_string()
            }
            EndpointSource::LocalConfig => self.local.as_ref().map_or_else(
                || "project-local config".to_string(),
                |p| p.display().to_string(),
            ),
            EndpointSource::HomeConfig => self.home.display().to_string(),
            EndpointSource::CompiledDefaults => {
                "compiled-in defaults (no config file supplied one)".to_string()
            }
            EndpointSource::Unset => "nothing — no tier set an endpoint".to_string(),
        };
        out.push(format!("endpoint from: {winner}"));

        if let Some(ref local) = self.local {
            if self.endpoint_from != EndpointSource::LocalConfig
                && self.local_status == SourceStatus::Loaded
            {
                out.push(format!(
                    "note: {} exists and outranks the home config, but did not set an endpoint",
                    local.display()
                ));
            }
        }
        for (label, path, status) in [
            (
                "home config",
                self.home.display().to_string(),
                &self.home_status,
            ),
            (
                "project-local config",
                self.local
                    .as_ref()
                    .map_or_else(String::new, |p| p.display().to_string()),
                &self.local_status,
            ),
        ] {
            match status {
                SourceStatus::Unreadable(e) => {
                    out.push(format!("{label} at {path} could not be read: {e}"));
                }
                SourceStatus::Malformed(e) => {
                    out.push(format!("{label} at {path} is not valid TOML: {e}"));
                }
                SourceStatus::Loaded | SourceStatus::Missing => {}
            }
        }
        out
    }
}

impl DatabaseConfig {
    /// The endpoint, or an explicit marker when blank.
    ///
    /// One definition: the `is_empty()` branch was written out at six call sites across
    /// three files, and `db status` rendered the same blank state two different ways.
    #[must_use]
    pub fn endpoint_label(&self) -> &str {
        endpoint_label(&self.endpoint)
    }
}

/// Strip trailing slashes from a backend endpoint.
///
/// Applied once where the endpoint enters the system rather than at each URL builder:
/// `auth` trimmed it and the SDK trimmed it, but the `PostgREST` URL builders did not, so
/// an endpoint stored as `https://host/` produced `https://host//rest/v1/...`. A
/// hand-edited config is the likeliest source, so the client normalises defensively too.
#[must_use]
pub fn normalize_endpoint(endpoint: &str) -> &str {
    endpoint.trim_end_matches('/')
}

/// The endpoint, or an explicit marker when blank.
///
/// Free function so `core::database::error` can use it without depending on config.
#[must_use]
pub const fn endpoint_label(endpoint: &str) -> &str {
    if endpoint.is_empty() {
        "(no endpoint configured)"
    } else {
        endpoint
    }
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
        if self.database.anon_key.is_empty() && !defaults.database.anon_key.is_empty() {
            self.database
                .anon_key
                .clone_from(&defaults.database.anon_key);
            changed = true;
        }
        if self.database.endpoint.is_empty() && !defaults.database.endpoint.is_empty() {
            self.database
                .endpoint
                .clone_from(&defaults.database.endpoint);
            changed = true;
        }
        if self.database.auth_file.is_empty() && !defaults.database.auth_file.is_empty() {
            self.database
                .auth_file
                .clone_from(&defaults.database.auth_file);
            changed = true;
        }
        if self.database.management_key.is_empty() && !defaults.database.management_key.is_empty() {
            self.database
                .management_key
                .clone_from(&defaults.database.management_key);
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

        if let Some(key) = &overrides.db_anon_key {
            self.database.anon_key.clone_from(key);
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
    /// [logging]
    /// level = "info"
    /// file = "$NU_ANALYTICS/app.log"
    /// "#)?;
    /// ```
    pub fn from_toml(toml_str: &str) -> Result<Self, toml::de::Error> {
        let mut config: Self = toml::from_str(toml_str)?;

        // Expand variables in config values
        config.logging.file = Self::expand_variables(&config.logging.file);
        config.database.anon_key = Self::expand_variables(&config.database.anon_key);
        config.database.endpoint = Self::expand_variables(&config.database.endpoint);
        config.database.auth_file = Self::expand_variables(&config.database.auth_file);
        config.database.management_key = Self::expand_variables(&config.database.management_key);
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
        Self::load_with_sources().0
    }

    /// Load configuration and report where each part came from.
    ///
    /// Precedence is unchanged: local `nuanalytics.toml` > home config > compiled
    /// defaults. The [`ConfigSources`] half exists because "which backend am I talking
    /// to" is unanswerable without also knowing *which file said so* — the home file is
    /// `config.toml` for release builds and `dconfig.toml` for debug builds, and a
    /// project-local file silently outranks both.
    #[must_use]
    pub fn load_with_sources() -> (Self, ConfigSources) {
        let home = Self::get_config_file_path();
        let local = Self::get_local_config_file_path();
        Self::load_with_sources_from(&home, local.as_deref(), true)
    }

    /// [`Self::load_with_sources`] with the paths injected, for tests.
    ///
    /// `create_home_if_missing` exists because the production path creates and saves the
    /// home config on a first run. A test must never do that: it would write to the
    /// developer's real `~/.config/nuanalytics/`, through the same
    /// `merge_defaults`-then-save path that can silently re-point a configured backend.
    #[must_use]
    pub fn load_with_sources_from(
        home: &Path,
        local: Option<&Path>,
        create_home_if_missing: bool,
    ) -> (Self, ConfigSources) {
        let defaults = Self::from_defaults();

        // `existed` is captured *before* loading, because the production loader creates
        // the file from defaults on a first run — reporting it as "loaded" would name a
        // file the tool itself had just written.
        let existed = home.exists();
        let mut config = if create_home_if_missing {
            Self::load_home_config(&defaults)
        } else {
            Self::read_config_file(home)
                .0
                .unwrap_or_else(|| defaults.clone())
        };
        let home_status = if existed {
            Self::read_config_file(home).1
        } else {
            SourceStatus::Missing
        };
        let mut sources = ConfigSources {
            home: home.to_path_buf(),
            home_status,
            ..ConfigSources::default()
        };

        // Tier 2: project-local config.
        let mut local_set_endpoint = false;
        if let Some(local_path) = local {
            if local_path.exists() {
                sources.local = Some(local_path.to_path_buf());
                let (parsed, status) = Self::read_config_file(local_path);
                sources.local_status = status;
                if let Some(local_config) = parsed {
                    // Recorded, not inferred by diffing the merged value: a local file
                    // that repeats the home endpoint would otherwise be reported as
                    // "sets no endpoint", pointing the user at the wrong file to edit.
                    local_set_endpoint = !local_config.database.endpoint.is_empty();
                    config.merge_from(&local_config);
                }
            }
        }

        sources.endpoint_from = if config.database.endpoint.is_empty() {
            EndpointSource::Unset
        } else if local_set_endpoint {
            EndpointSource::LocalConfig
        } else if matches!(sources.home_status, SourceStatus::Loaded) {
            EndpointSource::HomeConfig
        } else {
            // No usable home file, yet a non-empty endpoint: it came from the
            // compiled-in defaults.
            EndpointSource::CompiledDefaults
        };

        (config, sources)
    }

    /// Read and parse one config file, reporting why it could not be used.
    ///
    /// One definition for both tiers: the read/parse/classify sequence was written three
    /// times, and the home file was parsed twice per load.
    fn read_config_file(path: &Path) -> (Option<Self>, SourceStatus) {
        match fs::read_to_string(path) {
            Ok(content) => match Self::from_toml(&content) {
                Ok(config) => (Some(config), SourceStatus::Loaded),
                Err(e) => (None, SourceStatus::Malformed(e.to_string())),
            },
            Err(e) => (None, SourceStatus::Unreadable(e.to_string())),
        }
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
        if !other.database.anon_key.is_empty() {
            self.database.anon_key.clone_from(&other.database.anon_key);
        }
        if !other.database.endpoint.is_empty() {
            self.database.endpoint.clone_from(&other.database.endpoint);
        }
        if !other.database.auth_file.is_empty() {
            self.database
                .auth_file
                .clone_from(&other.database.auth_file);
        }
        if !other.database.management_key.is_empty() {
            self.database
                .management_key
                .clone_from(&other.database.management_key);
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
    /// [logging]
    /// level = "info"
    /// file = "$NU_ANALYTICS/logs/nuanalytics.log"
    /// verbose = false
    ///
    /// [database]
    /// anon_key = "your-anon-key"
    /// endpoint = "https://your-project.supabase.co"
    ///
    /// [paths]
    /// metrics_dir = "$NU_ANALYTICS/metrics"
    /// reports_dir = "$NU_ANALYTICS/reports"
    ///
    /// [audit]
    /// # ...
    ///
    /// [degree_analysis]
    /// # ...
    /// ```
    ///
    /// Section names are lower-case, matching the field names — TOML keys are
    /// case-sensitive and no field carries a `serde(rename)`, so `[Logging]` parses as
    /// an unknown section and every field silently falls back to its default.
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

    /// Strip an optional section prefix from a config key so both bare
    /// keys (`"endpoint"`) and dotted keys (`"database.endpoint"`) are
    /// accepted by `get`, `set`, and `unset`.
    fn normalize_key(key: &str) -> &str {
        for prefix in &[
            "database.",
            "logging.",
            "paths.",
            "audit.",
            "degree_analysis.",
        ] {
            if let Some(rest) = key.strip_prefix(prefix) {
                return rest;
            }
        }
        key
    }

    /// Retrieve a configuration value by key.
    ///
    /// Accepts both bare keys (`"endpoint"`) and dotted section keys
    /// (`"database.endpoint"`). Returns `None` for unrecognised keys.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        let key = Self::normalize_key(key);
        match key {
            "level" => Some(self.logging.level.clone()),
            "file" => Some(self.logging.file.clone()),
            "verbose" => Some(self.logging.verbose.to_string()),
            "anon_key" | "anon-key" | "token" => Some(self.database.anon_key.clone()),
            "endpoint" => Some(self.database.endpoint.clone()),
            "auth_file" | "auth-file" => Some(self.database.auth_file.clone()),
            "management_key" | "management-key" => Some(self.database.management_key.clone()),
            "project_ref" | "project-ref" => Some(self.database.project_ref.clone()),
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
    /// - `anon_key` (or legacy `token`): String — Supabase anonymous key
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
        let key = Self::normalize_key(key);
        match key {
            "level" => self.logging.level = value.to_string(),
            "file" => self.logging.file = value.to_string(),
            "verbose" => {
                self.logging.verbose = value
                    .parse::<bool>()
                    .map_err(|_| format!("Invalid boolean value for 'verbose': '{value}'"))?;
            }
            "anon_key" | "anon-key" | "token" => self.database.anon_key = value.to_string(),
            "endpoint" => {
                self.database.endpoint = normalize_endpoint(value).to_string();
            }
            "auth_file" | "auth-file" => self.database.auth_file = value.to_string(),
            "management_key" | "management-key" => {
                self.database.management_key = value.to_string();
            }
            "project_ref" | "project-ref" => {
                self.database.project_ref = value.to_string();
            }
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
        let key = Self::normalize_key(key);
        match key {
            "level" => self.logging.level.clone_from(&defaults.logging.level),
            "file" => self.logging.file.clone_from(&defaults.logging.file),
            "verbose" => self.logging.verbose = defaults.logging.verbose,
            "anon_key" | "anon-key" | "token" => {
                self.database
                    .anon_key
                    .clone_from(&defaults.database.anon_key);
            }
            "endpoint" => self
                .database
                .endpoint
                .clone_from(&defaults.database.endpoint),
            "auth_file" | "auth-file" => self
                .database
                .auth_file
                .clone_from(&defaults.database.auth_file),
            "management_key" | "management-key" => self
                .database
                .management_key
                .clone_from(&defaults.database.management_key),
            "project_ref" | "project-ref" => self
                .database
                .project_ref
                .clone_from(&defaults.database.project_ref),
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
        writeln!(f, "  endpoint = \"{}\"", self.database.endpoint)?;
        writeln!(f, "  anon_key = \"{}\"", self.database.anon_key)?;
        writeln!(f, "  auth_file = \"{}\"", self.database.auth_file)?;
        writeln!(f, "  management_key = \"{}\"", self.database.management_key)?;

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
        base.database.anon_key = "secret-token".to_string();

        let local = Config::default(); // all empty

        base.merge_from(&local);

        assert_eq!(base.logging.level, "warn");
        assert_eq!(base.database.anon_key, "secret-token");
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
        assert!(
            !config.logging.level.is_empty(),
            "the compiled-in defaults must set a log level"
        );
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

    // ── unset ───────────────────────────────────────────────────────────────

    #[test]
    fn test_unset_resets_to_default_value() {
        let mut config = Config::default();
        let defaults = Config::from_defaults();

        config.set("level", "trace").unwrap();
        config.unset("level", &defaults).unwrap();

        assert_eq!(config.logging.level, defaults.logging.level);
    }

    #[test]
    fn test_unset_unknown_key_returns_err() {
        let mut config = Config::default();
        let defaults = Config::default();
        assert!(config.unset("nope", &defaults).is_err());
    }

    #[test]
    fn test_unset_accepts_dotted_keys() {
        let mut config = Config::default();
        let mut defaults = Config::default();
        defaults.database.endpoint = "https://default.supabase.co".to_string();
        config.database.endpoint = "https://custom.supabase.co".to_string();

        config.unset("database.endpoint", &defaults).unwrap();

        assert_eq!(config.database.endpoint, "https://default.supabase.co");
    }

    #[test]
    fn test_unset_via_legacy_token_alias() {
        let mut config = Config::default();
        let mut defaults = Config::default();
        defaults.database.anon_key = "default-key".to_string();
        config.database.anon_key = "custom-key".to_string();

        config.unset("token", &defaults).unwrap();

        assert_eq!(config.database.anon_key, "default-key");
    }

    // ── set: validation paths ───────────────────────────────────────────────

    #[test]
    fn test_set_calc_strategy_accepts_median_and_mean() {
        let mut config = Config::default();
        assert!(config.set("calc_strategy", "median").is_ok());
        assert!(config.set("calc_strategy", "mean").is_ok());
    }

    #[test]
    fn test_set_calc_strategy_rejects_invalid() {
        let mut config = Config::default();
        let err = config.set("calc_strategy", "average").unwrap_err();
        assert!(err.contains("calc_strategy"));
    }

    #[test]
    fn test_set_sampling_strategy_lowercases_value() {
        let mut config = Config::default();
        config.set("sampling_strategy", "SHUFFLED").unwrap();
        assert_eq!(config.degree_analysis.sampling_strategy, "shuffled");
    }

    #[test]
    fn test_set_sampling_strategy_rejects_unknown() {
        let mut config = Config::default();
        assert!(config.set("sampling_strategy", "random").is_err());
    }

    #[test]
    fn test_set_prerequisite_chain_threshold_accepts_number() {
        let mut config = Config::default();
        config.set("prerequisite_chain_threshold", "5").unwrap();
        assert_eq!(config.audit.prerequisite_chain_threshold, 5);
    }

    #[test]
    fn test_set_prerequisite_chain_threshold_rejects_non_numeric() {
        let mut config = Config::default();
        assert!(config
            .set("prerequisite_chain_threshold", "not-a-number")
            .is_err());
    }

    // ── merge_defaults ──────────────────────────────────────────────────────

    #[test]
    fn test_merge_defaults_fills_only_empty_fields() {
        let mut config = Config::default();
        config.logging.level = "warn".to_string();

        let mut defaults = Config::default();
        defaults.logging.level = "info".to_string();
        defaults.logging.file = "/var/log/app.log".to_string();

        let changed = config.merge_defaults(&defaults);

        assert!(changed);
        assert_eq!(config.logging.level, "warn"); // not overwritten
        assert_eq!(config.logging.file, "/var/log/app.log"); // filled
    }

    #[test]
    fn test_merge_defaults_returns_false_when_nothing_to_fill() {
        let mut config = Config::default();
        config.logging.level = "warn".to_string();

        let defaults = Config::default(); // all empty — nothing to merge

        assert!(!config.merge_defaults(&defaults));
    }

    // ── get: aliases and unknown keys ───────────────────────────────────────

    #[test]
    fn test_get_returns_none_for_unknown_key() {
        let config = Config::default();
        assert_eq!(config.get("does_not_exist"), None);
    }

    #[test]
    fn test_get_accepts_dotted_kebab_and_legacy_aliases() {
        let mut config = Config::default();
        config.database.anon_key = "abc".to_string();

        assert_eq!(config.get("anon_key"), Some("abc".to_string()));
        assert_eq!(config.get("anon-key"), Some("abc".to_string()));
        assert_eq!(config.get("token"), Some("abc".to_string())); // legacy
        assert_eq!(config.get("database.anon_key"), Some("abc".to_string()));
    }

    // ── Display ─────────────────────────────────────────────────────────────

    #[test]
    fn test_display_includes_all_section_headers() {
        let config = Config::default();
        let s = format!("{config}");
        assert!(s.contains("[logging]"));
        assert!(s.contains("[database]"));
        assert!(s.contains("[paths]"));
        assert!(s.contains("[audit]"));
        assert!(s.contains("[degree_analysis]"));
    }

    #[test]
    fn test_from_toml_with_missing_logging_section_uses_defaults() {
        // Regression guard for `#[serde(default)]` on `Config.logging`: a
        // project-local `nuanalytics.toml` that omits `[logging]` must still
        // deserialize, otherwise `Config::load` silently drops the local
        // overrides — see the `nuanalytics init` template.
        let toml_str = r#"
[paths]
metrics_dir = "./metrics"
reports_dir = "./reports"
"#;
        let cfg = Config::from_toml(toml_str)
            .expect("missing [logging] must fall back to LoggingConfig::default()");
        let default_logging = LoggingConfig::default();
        assert_eq!(cfg.logging.level, default_logging.level);
        assert_eq!(cfg.logging.file, default_logging.file);
        assert_eq!(cfg.logging.verbose, default_logging.verbose);
        assert_eq!(cfg.paths.metrics_dir, "./metrics");
    }

    /// SECURITY: neither compiled-in default asset may ship database credentials.
    ///
    /// This repo is public. `merge_defaults` refills an *empty* `endpoint`/`anon_key`
    /// from these assets and `load_home_config` then saves the result, so any value
    /// committed here silently re-points a self-hosted user at that backend and
    /// persists it.
    ///
    /// Both files are read as raw text on purpose: only one is `include_str!`d per
    /// build profile, so asserting through `Config::from_defaults()` would leave the
    /// other completely unchecked.
    #[test]
    fn default_assets_never_ship_database_credentials() {
        const RELEASE: &str = include_str!("../assets/DefaultCLIConfigRelease.toml");
        const DEBUG: &str = include_str!("../assets/DefaultCLIConfigDebug.toml");

        for (name, src) in [
            ("DefaultCLIConfigRelease.toml", RELEASE),
            ("DefaultCLIConfigDebug.toml", DEBUG),
        ] {
            let config =
                Config::from_toml(src).unwrap_or_else(|e| panic!("{name} must parse as TOML: {e}"));
            assert!(
                config.database.endpoint.is_empty(),
                "{name} ships a non-blank database.endpoint ({:?}). This repo is public, \
                 and merge_defaults will silently re-point and persist a self-hosted \
                 user's config to it.",
                config.database.endpoint
            );
            assert!(
                config.database.anon_key.is_empty(),
                "{name} ships a non-blank database.anon_key"
            );
        }
    }

    #[test]
    fn merge_defaults_neither_overwrites_a_configured_endpoint_nor_invents_one() {
        // The refill-then-save path is what makes the guard above load-bearing, so pin
        // its two edges: a configured endpoint must survive, and a blank one must stay
        // blank rather than acquiring a value from the compiled-in defaults.
        let defaults = Config::from_defaults();

        let mut configured = Config::default();
        configured.database.endpoint = "https://nu.example.com".to_string();
        configured.database.anon_key = "local-anon-key".to_string();
        configured.merge_defaults(&defaults);
        assert_eq!(
            configured.database.endpoint, "https://nu.example.com",
            "a self-hosted endpoint must survive merge_defaults untouched"
        );
        assert_eq!(configured.database.anon_key, "local-anon-key");

        let mut blank = Config::default();
        blank.merge_defaults(&defaults);
        assert!(
            blank.database.endpoint.is_empty(),
            "an empty endpoint was back-filled from the compiled-in defaults, which is \
             exactly how a committed credential would reach a user's saved config"
        );
        assert!(blank.database.anon_key.is_empty());
    }

    // ---- config provenance --------------------------------------------------

    #[test]
    fn describe_names_the_file_that_supplied_the_endpoint() {
        let sources = ConfigSources {
            home: PathBuf::from("/home/u/.config/nuanalytics/dconfig.toml"),
            home_status: SourceStatus::Loaded,
            local: None,
            local_status: SourceStatus::Missing,
            endpoint_from: EndpointSource::HomeConfig,
        };
        let lines = sources.describe();
        assert!(
            lines[0].contains("dconfig.toml"),
            "must name the exact file, since the home file differs by build profile: {lines:?}"
        );
    }

    #[test]
    fn describe_names_a_project_local_file_when_it_won() {
        let sources = ConfigSources {
            home: PathBuf::from("/home/u/.config/nuanalytics/config.toml"),
            home_status: SourceStatus::Loaded,
            local: Some(PathBuf::from("/work/proj/nuanalytics.toml")),
            local_status: SourceStatus::Loaded,
            endpoint_from: EndpointSource::LocalConfig,
        };
        let joined = sources.describe().join("\n");
        assert!(
            joined.contains("/work/proj/nuanalytics.toml"),
            "the local file outranks the home one and must be named: {joined}"
        );
    }

    #[test]
    fn describe_warns_when_a_local_file_outranks_but_sets_no_endpoint() {
        // The documented trap: `config set` writes to the home config, so a user with a
        // project-local file can set an endpoint, see it succeed, and still be told the
        // home value is in effect. Saying the local file exists explains why.
        let sources = ConfigSources {
            home: PathBuf::from("/home/u/.config/nuanalytics/config.toml"),
            home_status: SourceStatus::Loaded,
            local: Some(PathBuf::from("/work/proj/nuanalytics.toml")),
            local_status: SourceStatus::Loaded,
            endpoint_from: EndpointSource::HomeConfig,
        };
        let joined = sources.describe().join("\n");
        assert!(
            joined.contains("outranks"),
            "must mention that a local file exists and outranks the home config: {joined}"
        );
    }

    #[test]
    fn describe_reports_an_unusable_config_instead_of_staying_silent() {
        // These used to be swallowed by `if let Ok(...)`, so a malformed config produced
        // default settings with no indication why.
        let malformed = ConfigSources {
            home: PathBuf::from("/home/u/.config/nuanalytics/config.toml"),
            home_status: SourceStatus::Malformed("expected `=` at line 3".to_string()),
            local: None,
            local_status: SourceStatus::Missing,
            endpoint_from: EndpointSource::CompiledDefaults,
        };
        let joined = malformed.describe().join("\n");
        assert!(joined.contains("not valid TOML"), "got: {joined}");
        assert!(joined.contains("expected `=` at line 3"), "got: {joined}");

        let unreadable = ConfigSources {
            home_status: SourceStatus::Unreadable("permission denied".to_string()),
            ..ConfigSources::default()
        };
        let joined = unreadable.describe().join("\n");
        assert!(joined.contains("could not be read"), "got: {joined}");
        assert!(joined.contains("permission denied"), "got: {joined}");
    }

    #[test]
    fn describe_states_plainly_when_no_tier_set_an_endpoint() {
        let sources = ConfigSources {
            endpoint_from: EndpointSource::Unset,
            ..ConfigSources::default()
        };
        let joined = sources.describe().join("\n");
        assert!(
            joined.contains("no tier set an endpoint"),
            "a blank endpoint must be stated, not left implicit: {joined}"
        );
    }

    // ---- provenance over real files ----------------------------------------
    //
    // These use `load_with_sources_from(.., create_home_if_missing = false)`. The
    // production entry point creates and re-saves the home config, so a test calling it
    // would write to the developer's real ~/.config/nuanalytics/ through the same
    // merge_defaults-then-save path CLAUDE.md warns about.

    fn write(path: &Path, body: &str) {
        fs::write(path, body).expect("write test config");
    }

    #[test]
    fn a_local_file_outranks_the_home_file_and_is_named() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, local) = (
            dir.path().join("config.toml"),
            dir.path().join("nuanalytics.toml"),
        );
        write(
            &home,
            "[database]\nendpoint = \"https://home.example.com\"\nanon_key = \"k\"\n",
        );
        write(
            &local,
            "[database]\nendpoint = \"https://local.example.com\"\n",
        );

        let (config, sources) = Config::load_with_sources_from(&home, Some(&local), false);
        assert_eq!(config.database.endpoint, "https://local.example.com");
        assert_eq!(sources.endpoint_from, EndpointSource::LocalConfig);
        assert!(
            sources.describe().join("\n").contains("nuanalytics.toml"),
            "the winning file must be named"
        );
    }

    #[test]
    fn a_local_file_repeating_the_home_endpoint_is_not_called_empty() {
        // Provenance used to be inferred by comparing the merged value against the
        // post-home value, so a local file setting the *same* endpoint was reported as
        // "did not set an endpoint" — pointing the user at the wrong file to edit.
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, local) = (
            dir.path().join("config.toml"),
            dir.path().join("nuanalytics.toml"),
        );
        write(
            &home,
            "[database]\nendpoint = \"https://same.example.com\"\nanon_key = \"k\"\n",
        );
        write(
            &local,
            "[database]\nendpoint = \"https://same.example.com\"\n",
        );

        let (_, sources) = Config::load_with_sources_from(&home, Some(&local), false);
        assert_eq!(
            sources.endpoint_from,
            EndpointSource::LocalConfig,
            "the local file did set the endpoint, even though the value matches home"
        );
        assert!(
            !sources
                .describe()
                .join("\n")
                .contains("did not set an endpoint"),
            "must not claim the local file is silent: {:?}",
            sources.describe()
        );
    }

    #[test]
    fn a_local_file_that_sets_no_endpoint_is_flagged_as_outranking_anyway() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, local) = (
            dir.path().join("config.toml"),
            dir.path().join("nuanalytics.toml"),
        );
        write(
            &home,
            "[database]\nendpoint = \"https://home.example.com\"\nanon_key = \"k\"\n",
        );
        write(&local, "[logging]\nlevel = \"debug\"\n");

        let (config, sources) = Config::load_with_sources_from(&home, Some(&local), false);
        assert_eq!(config.database.endpoint, "https://home.example.com");
        assert_eq!(sources.endpoint_from, EndpointSource::HomeConfig);
        let joined = sources.describe().join("\n");
        assert!(
            joined.contains("did not set an endpoint"),
            "a local file that outranks but is silent must be called out: {joined}"
        );
    }

    #[test]
    fn a_malformed_home_config_is_reported_not_swallowed() {
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("config.toml");
        write(&home, "this is not toml =\n");

        let (_, sources) = Config::load_with_sources_from(&home, None, false);
        assert!(
            matches!(sources.home_status, SourceStatus::Malformed(_)),
            "got {:?}",
            sources.home_status
        );
        let joined = sources.describe().join("\n");
        assert!(joined.contains("not valid TOML"), "got: {joined}");
        assert!(
            joined.contains(&home.display().to_string()),
            "the failure must name the file: {joined}"
        );
    }

    #[test]
    fn a_malformed_local_config_does_not_hide_the_working_home_config() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (home, local) = (
            dir.path().join("config.toml"),
            dir.path().join("nuanalytics.toml"),
        );
        write(
            &home,
            "[database]\nendpoint = \"https://home.example.com\"\nanon_key = \"k\"\n",
        );
        write(&local, "[database\n");

        let (config, sources) = Config::load_with_sources_from(&home, Some(&local), false);
        assert_eq!(
            config.database.endpoint, "https://home.example.com",
            "an unusable local file must not blank the endpoint"
        );
        assert_eq!(sources.endpoint_from, EndpointSource::HomeConfig);
        let joined = sources.describe().join("\n");
        assert!(joined.contains("not valid TOML"), "got: {joined}");
        assert!(
            joined.contains("nuanalytics.toml"),
            "must name the unusable file: {joined}"
        );
    }

    #[test]
    fn a_missing_home_config_is_not_reported_as_loaded() {
        // The production loader creates the file on a first run; reporting "loaded" would
        // name a file the tool itself had just written.
        let dir = tempfile::tempdir().expect("tempdir");
        let home = dir.path().join("config.toml");
        let (_, sources) = Config::load_with_sources_from(&home, None, false);
        assert_eq!(sources.home_status, SourceStatus::Missing);
        assert_eq!(sources.endpoint_from, EndpointSource::Unset);
        assert!(
            sources
                .describe()
                .join("\n")
                .contains("no tier set an endpoint"),
            "{:?}",
            sources.describe()
        );
    }
}
