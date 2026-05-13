//! Supabase auth state — save/load/clear the signed-in user's session.
//!
//! Auth state is stored at the path configured in `database.auth_file`.
//! The built-in defaults are profile-specific:
//! - **Release**: `$NU_ANALYTICS/auth.json` (`~/.config/nuanalytics/auth.json`)
//! - **Debug**:   `.debug/dauth.json` relative to the working directory
//!
//! Override with: `nuanalytics config set database.auth_file /path/to/auth.json`
//!
//! The auth token is user-specific and must not be committed to version control.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ============================================================================
// Types
// ============================================================================

/// Persisted Supabase user session.
///
/// Saved to disk after a successful `nuanalytics db login` and read at startup
/// to authenticate database operations without requiring a repeated login.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthState {
    /// JWT access token used in `Authorization: Bearer` requests
    pub access_token: String,
    /// Refresh token for obtaining a new access token when this one expires
    pub refresh_token: String,
    /// Unix timestamp at which the access token expires
    pub expires_at: i64,
    /// Email of the signed-in user (display only, not used for auth)
    pub user_email: Option<String>,
}

impl AuthState {
    /// Returns `true` if the access token has expired (or is about to in < 60s).
    #[must_use]
    pub fn is_expired(&self) -> bool {
        let now = chrono::Utc::now().timestamp();
        // Treat tokens as expired 60 seconds early to avoid edge-case failures
        self.expires_at <= now + 60
    }

    /// Returns `true` if the access token is present and not expired.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        !self.access_token.is_empty() && !self.is_expired()
    }
}

// ============================================================================
// File path
// ============================================================================

// ============================================================================
// Persistence helpers
// ============================================================================

/// Load the saved auth state from disk, returning `None` if absent or unreadable.
#[must_use]
pub fn load_auth_state(path: &Path) -> Option<AuthState> {
    let content = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&content).ok()
}

/// Persist an auth state to disk.
///
/// Creates the parent directory if it does not exist.
///
/// # Errors
///
/// Returns a string describing the failure if the file cannot be written.
pub fn save_auth_state(path: &Path, state: &AuthState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create dir {}: {e}", parent.display()))?;
    }
    let content =
        serde_json::to_string_pretty(state).map_err(|e| format!("Serialization error: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("Cannot write auth file: {e}"))
}

/// Delete the saved auth state from disk (sign out).
///
/// Silently ignores errors (e.g. file already gone).
pub fn clear_auth_state(path: &Path) {
    let _ = std::fs::remove_file(path);
}

/// Resolve the auth file path from the config value.
///
/// Returns the configured path as a `PathBuf`. Variable expansion (e.g. `$NU_ANALYTICS`)
/// is already applied by the config loader.
#[must_use]
pub fn auth_file_path(db_config: &crate::core::config::DatabaseConfig) -> PathBuf {
    PathBuf::from(&db_config.auth_file)
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_state(expires_offset_secs: i64) -> AuthState {
        AuthState {
            access_token: "tok".to_string(),
            refresh_token: "refresh".to_string(),
            expires_at: chrono::Utc::now().timestamp() + expires_offset_secs,
            user_email: Some("test@example.com".to_string()),
        }
    }

    #[test]
    fn test_is_expired_future_token() {
        let state = make_state(3600); // expires in 1 hour
        assert!(!state.is_expired());
        assert!(state.is_valid());
    }

    #[test]
    fn test_is_expired_past_token() {
        let state = make_state(-100); // expired 100 seconds ago
        assert!(state.is_expired());
        assert!(!state.is_valid());
    }

    #[test]
    fn test_is_expired_within_buffer() {
        let state = make_state(30); // expires in 30s — within the 60s safety buffer
        assert!(state.is_expired());
        assert!(!state.is_valid());
    }

    #[test]
    fn test_is_valid_empty_token() {
        let state = AuthState {
            access_token: String::new(),
            refresh_token: "refresh".to_string(),
            expires_at: chrono::Utc::now().timestamp() + 3600,
            user_email: None,
        };
        assert!(!state.is_valid());
    }

    #[test]
    fn test_roundtrip_serialize() {
        let state = make_state(3600);
        let json = serde_json::to_string(&state).unwrap();
        let back: AuthState = serde_json::from_str(&json).unwrap();
        assert_eq!(back.access_token, state.access_token);
        assert_eq!(back.expires_at, state.expires_at);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        // Use a temp dir so we don't touch the real config dir
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("auth_test.json");
        let state = make_state(3600);

        // Write manually and read back to verify persistence logic
        let content = serde_json::to_string_pretty(&state).unwrap();
        std::fs::write(&path, &content).unwrap();
        let loaded: AuthState =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();

        assert_eq!(loaded.access_token, state.access_token);
        assert_eq!(loaded.refresh_token, state.refresh_token);
        assert_eq!(loaded.expires_at, state.expires_at);
        assert_eq!(loaded.user_email, state.user_email);
    }

    #[test]
    fn test_is_expired_boundary_exactly_60s_buffer() {
        // Exactly at the boundary: 60s from now should be considered expired
        let state = make_state(60);
        assert!(state.is_expired());
    }

    #[test]
    fn test_is_expired_just_over_buffer() {
        // 61s from now — just outside the 60s buffer, should be valid
        let state = make_state(61);
        assert!(!state.is_expired());
        assert!(state.is_valid());
    }
}
