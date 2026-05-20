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
// Token refresh
// ============================================================================

/// Relative path for Supabase's refresh-token grant endpoint.
const REFRESH_TOKEN_PATH: &str = "/auth/v1/token?grant_type=refresh_token";

/// Response body for `POST /auth/v1/token?grant_type=refresh_token`.
///
/// Only the fields we persist into [`AuthState`] are deserialised; Supabase
/// returns extra fields (`token_type`, `user`, …) that we ignore.
#[derive(Debug, Deserialize)]
struct RefreshResponse {
    access_token: String,
    refresh_token: String,
    /// Seconds until the new access token expires (typically 3600).
    /// Supabase also returns the absolute `expires_at` but it's optional in
    /// some self-hosted setups, so we always recompute from `expires_in`.
    expires_in: i64,
    /// Some Supabase deployments return a refreshed user payload; we only
    /// care about the email for display.
    #[serde(default)]
    user: Option<RefreshUser>,
}

#[derive(Debug, Deserialize)]
struct RefreshUser {
    #[serde(default)]
    email: Option<String>,
}

/// Exchange a refresh token for a fresh access token.
///
/// Calls Supabase's `POST {endpoint}/auth/v1/token?grant_type=refresh_token`
/// with the project anon key in the `apikey` header. Used by the database
/// client to keep long-running sessions (MCP servers in particular) from
/// dead-ending at the 1-hour JWT expiry — `is_expired()` already returns
/// `true` 60s ahead of the wall-clock expiry to give callers a buffer.
///
/// # Errors
/// Returns a string describing the failure if the HTTP request fails or
/// Supabase rejects the refresh token (e.g. it was revoked by `db logout`
/// on another machine, or the user was deleted).
pub async fn refresh_session(
    endpoint: &str,
    anon_key: &str,
    refresh_token: &str,
) -> Result<AuthState, String> {
    let url = format!("{}{REFRESH_TOKEN_PATH}", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "refresh_token": refresh_token });

    let response = reqwest::Client::new()
        .post(&url)
        .header("apikey", anon_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("token refresh request to {url} failed: {e}"))?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(format!(
            "token refresh at {url} rejected ({status}): {body_text}"
        ));
    }

    let parsed: RefreshResponse = response
        .json()
        .await
        .map_err(|e| format!("token refresh returned malformed JSON: {e}"))?;

    Ok(AuthState {
        access_token: parsed.access_token,
        refresh_token: parsed.refresh_token,
        expires_at: chrono::Utc::now().timestamp() + parsed.expires_in,
        user_email: parsed.user.and_then(|u| u.email),
    })
}

/// Load the auth file, refreshing the token if it's expired.
///
/// Returns the freshest available [`AuthState`], or `Ok(None)` when no auth
/// file exists yet — that's a signal for the caller to surface a `db login`
/// prompt rather than an error. On a successful refresh the new state is
/// persisted back to disk so the next process startup also sees a valid
/// session.
///
/// # Errors
/// Returns a string when the file exists but the refresh call failed
/// (network error, revoked refresh token, malformed response).
pub async fn load_and_refresh(
    auth_path: &Path,
    endpoint: &str,
    anon_key: &str,
) -> Result<Option<AuthState>, String> {
    let Some(state) = load_auth_state(auth_path) else {
        return Ok(None);
    };
    if state.is_valid() {
        return Ok(Some(state));
    }
    let refreshed = refresh_session(endpoint, anon_key, &state.refresh_token).await?;
    // Best-effort persist — even if we can't write to disk, the in-memory
    // state is still usable for this process.
    let _ = save_auth_state(auth_path, &refreshed);
    Ok(Some(refreshed))
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

    #[tokio::test]
    async fn load_and_refresh_returns_none_when_auth_file_missing() {
        // load_and_refresh treats "no file" as the signal-to-prompt case,
        // not an error — callers turn it into `db login` guidance.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nope.json");
        let result = load_and_refresh(&path, "https://example.supabase.co", "anon").await;
        assert!(
            matches!(result, Ok(None)),
            "expected Ok(None), got {result:?}"
        );
    }

    #[tokio::test]
    async fn load_and_refresh_returns_existing_state_when_token_is_fresh() {
        // No network round-trip should happen for a non-expired token.
        // Pointing at an obviously invalid endpoint proves the refresh
        // call wasn't attempted (otherwise it would error out).
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("auth.json");
        let state = make_state(3600);
        save_auth_state(&path, &state).unwrap();

        let result = load_and_refresh(
            &path,
            "http://127.0.0.1:1/this-should-not-be-called",
            "anon",
        )
        .await
        .expect("fresh tokens must not trigger a refresh call");
        let returned = result.expect("auth state should be loaded from disk");
        assert_eq!(returned.access_token, state.access_token);
        assert_eq!(returned.refresh_token, state.refresh_token);
    }
}
