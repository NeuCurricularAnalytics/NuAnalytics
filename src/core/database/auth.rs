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
use std::fmt;
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
/// Creates the parent directory if it does not exist, and restricts the file to
/// owner-only access — it holds a live access token and refresh token.
///
/// # Errors
///
/// Returns a string describing the failure if the file cannot be written or its
/// permissions cannot be tightened.
pub fn save_auth_state(path: &Path, state: &AuthState) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("Cannot create dir {}: {e}", parent.display()))?;
    }
    let content =
        serde_json::to_string_pretty(state).map_err(|e| format!("Serialization error: {e}"))?;
    std::fs::write(path, content).map_err(|e| format!("Cannot write auth file: {e}"))?;
    restrict_permissions(path)
}

/// Restrict the auth file to `0600` so other users on the machine cannot read
/// the tokens it holds.
#[cfg(unix)]
fn restrict_permissions(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .map_err(|e| format!("Cannot restrict auth file permissions: {e}"))
}

/// No-op on non-Unix platforms, which have no comparable mode bits.
#[cfg(not(unix))]
fn restrict_permissions(_path: &Path) -> Result<(), String> {
    Ok(())
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
// Token grants
// ============================================================================

/// Relative path for Supabase's refresh-token grant endpoint.
const REFRESH_TOKEN_PATH: &str = "/auth/v1/token?grant_type=refresh_token";

/// Relative path for Supabase's password grant endpoint.
///
/// Unlike the OAuth flow this needs no external identity provider, which is what makes it
/// usable on a freshly stood-up stack.
const PASSWORD_GRANT_PATH: &str = "/auth/v1/token?grant_type=password";

/// Response body for either grant at `POST /auth/v1/token`.
///
/// `GoTrue` returns the same session shape for `grant_type=refresh_token` and
/// `grant_type=password`, so both paths deserialise into this. Only the fields we persist
/// into [`AuthState`] are read; Supabase returns extra ones (`token_type`, the full `user`
/// record, …) that we ignore.
#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    /// Seconds until the new access token expires (typically 3600).
    /// Supabase also returns the absolute `expires_at` but it's optional in
    /// some self-hosted setups, so we always recompute from `expires_in`.
    expires_in: i64,
    /// Some Supabase deployments return a refreshed user payload; we only
    /// care about the email for display.
    #[serde(default)]
    user: Option<TokenUser>,
}

#[derive(Debug, Deserialize)]
struct TokenUser {
    #[serde(default)]
    email: Option<String>,
}

impl TokenResponse {
    /// Convert a grant response into the session we persist.
    ///
    /// `expires_at` is computed from `expires_in` rather than read from the response,
    /// because self-hosted `GoTrue` does not always send the absolute field.
    fn into_auth_state(self) -> AuthState {
        AuthState {
            access_token: self.access_token,
            refresh_token: self.refresh_token,
            expires_at: chrono::Utc::now().timestamp() + self.expires_in,
            user_email: self.user.and_then(|u| u.email),
        }
    }
}

/// Why a token refresh failed.
///
/// The distinction is load-bearing: a transport failure means the backend could not be
/// reached, while a rejection means it answered and refused the token. Collapsing both
/// into one string made `DbClient::from_config` report an unreachable backend as
/// "not signed in", telling the user to run `db login` against a host that was down.
#[derive(Debug, Clone)]
pub enum RefreshError {
    /// The request never got an answer (DNS, connection refused, TLS, timeout).
    Transport(String),
    /// The backend answered and refused the refresh token.
    Rejected(String),
    /// The backend answered with a body this client could not parse.
    Malformed(String),
}

impl RefreshError {
    /// The message as the user should see it.
    #[must_use]
    pub fn detail(&self) -> &str {
        match self {
            Self::Transport(m) | Self::Rejected(m) | Self::Malformed(m) => m,
        }
    }
}

impl fmt::Display for RefreshError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.detail())
    }
}

/// Exchange a refresh token for a fresh session.
///
/// Calls `POST {endpoint}/auth/v1/token?grant_type=refresh_token` with the project anon
/// key in the `apikey` header. Keeps long-running sessions (MCP servers in particular)
/// from dead-ending at the 1-hour JWT expiry; [`AuthState::is_expired`] reports `true`
/// 60s ahead of the wall-clock expiry to give callers room to call this.
///
/// # Errors
/// [`RefreshError::Transport`] when the backend could not be reached,
/// [`RefreshError::Rejected`] when it refused the token — revoked elsewhere, or the user
/// was deleted — and [`RefreshError::Malformed`] when its answer could not be parsed.
pub async fn refresh_session(
    endpoint: &str,
    anon_key: &str,
    refresh_token: &str,
) -> Result<AuthState, RefreshError> {
    let url = format!("{}{REFRESH_TOKEN_PATH}", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "refresh_token": refresh_token });

    let response = reqwest::Client::new()
        .post(&url)
        .header("apikey", anon_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| {
            RefreshError::Transport(format!("token refresh request to {url} failed: {e}"))
        })?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        return Err(RefreshError::Rejected(format!(
            "token refresh at {url} rejected ({status}): {body_text}"
        )));
    }

    let parsed: TokenResponse = response.json().await.map_err(|e| {
        RefreshError::Malformed(format!("token refresh returned malformed JSON: {e}"))
    })?;

    Ok(parsed.into_auth_state())
}

// ============================================================================
// Password sign-in
// ============================================================================

/// Why a password sign-in failed.
///
/// Split the same way as [`RefreshError`], for the same reason: a transport failure means
/// the backend was never reached, while a rejection means it answered and refused. The
/// rejection variant carries `GoTrue`'s own words rather than a guess — inventing a cause
/// is the defect that made the OAuth callback expensive to debug.
#[derive(Debug, Clone)]
pub enum SignInError {
    /// The request never got an answer (DNS, connection refused, TLS, timeout).
    Transport(String),
    /// The backend answered and refused the credentials.
    Rejected {
        /// HTTP status `GoTrue` replied with.
        status: u16,
        /// `GoTrue`'s own description of the refusal, most specific field available.
        detail: String,
    },
    /// The backend answered with a body this client could not parse.
    Malformed(String),
}

impl fmt::Display for SignInError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Transport(m) | Self::Malformed(m) => f.write_str(m),
            Self::Rejected { status, detail } => write!(f, "{detail} (HTTP {status})"),
        }
    }
}

/// Pull the most specific human-readable message out of a `GoTrue` error body.
///
/// `GoTrue` has shipped three shapes for these over the versions it supports, and a
/// self-hosted stack may be running any of them: `error_description` (OAuth-style),
/// `msg` with a sibling `error_code` (current), and a bare `message`. Fields are tried
/// most-specific first so the user sees prose rather than a code like
/// `invalid_credentials` when both are present.
fn gotrue_error_text(body: &str) -> Option<String> {
    let parsed: serde_json::Value = serde_json::from_str(body).ok()?;
    for key in ["error_description", "msg", "message", "error_code", "error"] {
        if let Some(text) = parsed.get(key).and_then(serde_json::Value::as_str) {
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
    None
}

/// Sign in with an email and password, returning a session to persist.
///
/// Calls `POST {endpoint}/auth/v1/token?grant_type=password`. This grant needs no
/// external identity provider, so it is the one sign-in path that works on a stack whose
/// operator has not yet registered an OAuth application. It does **not** create accounts:
/// the user must already exist, so enabling this does not enable signup.
///
/// # Errors
/// [`SignInError::Transport`] when the backend could not be reached,
/// [`SignInError::Rejected`] when it refused the credentials — wrong password, unknown
/// user, or an unconfirmed email address, carrying `GoTrue`'s own message — and
/// [`SignInError::Malformed`] when its answer could not be parsed.
pub async fn sign_in_with_password(
    endpoint: &str,
    anon_key: &str,
    email: &str,
    password: &str,
) -> Result<AuthState, SignInError> {
    let url = format!("{}{PASSWORD_GRANT_PATH}", endpoint.trim_end_matches('/'));
    let body = serde_json::json!({ "email": email, "password": password });

    let response = reqwest::Client::new()
        .post(&url)
        .header("apikey", anon_key)
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| SignInError::Transport(format!("sign-in request to {url} failed: {e}")))?;

    let status = response.status();
    if !status.is_success() {
        let body_text = response.text().await.unwrap_or_default();
        let detail = gotrue_error_text(&body_text).unwrap_or_else(|| {
            if body_text.is_empty() {
                format!("{endpoint} refused the sign-in and sent no explanation")
            } else {
                body_text.clone()
            }
        });
        return Err(SignInError::Rejected {
            status: status.as_u16(),
            detail,
        });
    }

    let parsed: TokenResponse = response
        .json()
        .await
        .map_err(|e| SignInError::Malformed(format!("sign-in returned malformed JSON: {e}")))?;

    Ok(parsed.into_auth_state())
}

/// Load the auth file, refreshing the token if it is expired.
///
/// Returns the on-disk state **unchanged** when it is still clock-valid — including a
/// clock-valid token the backend has already revoked, since nothing here asks the
/// backend. For the case where the backend rejected the token we just sent, see
/// `DbClient::force_refresh_from_disk`, which ignores `is_valid()` on purpose.
///
/// Returns `Ok(None)` when no auth file exists yet — a signal for the caller to surface a
/// `db login` prompt rather than an error. On a successful refresh the new state is
/// persisted back to disk so the next process startup also sees a valid session.
///
/// # Errors
/// Returns a [`RefreshError`] when the file exists but the refresh call failed; the
/// variant distinguishes "could not reach the backend" from "the backend refused it".
pub async fn load_and_refresh(
    auth_path: &Path,
    endpoint: &str,
    anon_key: &str,
) -> Result<Option<AuthState>, RefreshError> {
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

    #[cfg(unix)]
    #[test]
    fn test_save_auth_state_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;

        // The file holds a live access + refresh token, so it must not be
        // readable by other users on the machine.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &make_state(3600)).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "unexpected mode {:o}", mode & 0o777);
    }

    #[cfg(unix)]
    #[test]
    fn test_save_auth_state_tightens_an_existing_loose_file() {
        use std::os::unix::fs::PermissionsExt;

        // A file written by an older build (or `umask 022`) is already 0644 on
        // disk; overwriting it must still bring the mode back down.
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("auth.json");
        std::fs::write(&path, "{}").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();

        save_auth_state(&path, &make_state(3600)).unwrap();

        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "unexpected mode {:o}", mode & 0o777);
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
    // --- password sign-in ---------------------------------------------------

    /// Canned-response server that records the raw request it received.
    ///
    /// The recording is the point: the interesting assertions are about *which* grant
    /// endpoint was called and what was sent with it, neither of which is observable
    /// from the returned `AuthState`.
    async fn recording_stub(
        status_line: &'static str,
        body: &'static str,
    ) -> (String, std::sync::Arc<std::sync::Mutex<String>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(String::new()));
        let recorder = std::sync::Arc::clone(&seen);
        tokio::spawn(async move {
            let Ok((mut stream, _)) = listener.accept().await else {
                return;
            };
            let mut buf = [0u8; 8192];
            let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                .await
                .unwrap_or(0);
            *recorder.lock().expect("record request") =
                String::from_utf8_lossy(&buf[..n]).to_string();
            let response = format!(
                "HTTP/1.1 {status_line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
        });
        (url, seen)
    }

    const PASSWORD_SESSION: &str = r#"{"access_token":"at","refresh_token":"rt","expires_in":3600,"user":{"email":"you@example.edu"}}"#;

    #[tokio::test]
    async fn sign_in_with_password_calls_the_password_grant_with_the_anon_key() {
        let (url, seen) = recording_stub("200 OK", PASSWORD_SESSION).await;
        sign_in_with_password(&url, "anon-key", "you@example.edu", "hunter2")
            .await
            .expect("stub returns a valid session");

        let request = seen.lock().expect("read recorded request").clone();
        assert!(
            request.contains("grant_type=password"),
            "must use the password grant, not the refresh grant: {request}"
        );
        assert!(
            request.contains("apikey: anon-key"),
            "GoTrue rejects the grant without the project anon key: {request}"
        );
        assert!(
            request.contains("you@example.edu") && request.contains("hunter2"),
            "credentials must reach the backend: {request}"
        );
    }

    #[tokio::test]
    async fn sign_in_with_password_recomputes_expiry_from_expires_in() {
        // Self-hosted GoTrue does not always send the absolute `expires_at`, so the
        // session's expiry has to come from `expires_in`. The stub body omits it.
        let before = chrono::Utc::now().timestamp();
        let (url, _seen) = recording_stub("200 OK", PASSWORD_SESSION).await;
        let state = sign_in_with_password(&url, "anon", "you@example.edu", "pw")
            .await
            .expect("stub returns a valid session");

        assert!(
            state.expires_at >= before + 3600 && state.expires_at <= before + 3605,
            "expires_at should be ~1h out, got {} (now {before})",
            state.expires_at
        );
        assert_eq!(state.user_email.as_deref(), Some("you@example.edu"));
        assert!(
            state.is_valid(),
            "a fresh 1h session must not read as expired"
        );
    }

    #[tokio::test]
    async fn sign_in_with_password_surfaces_gotrues_own_refusal_message() {
        let (url, _seen) = recording_stub(
            "400 Bad Request",
            r#"{"code":400,"error_code":"invalid_credentials","msg":"Invalid login credentials"}"#,
        )
        .await;
        let error = sign_in_with_password(&url, "anon", "you@example.edu", "wrong")
            .await
            .expect_err("a 400 must not read as success");

        match error {
            SignInError::Rejected { status, detail } => {
                assert_eq!(status, 400);
                assert_eq!(
                    detail, "Invalid login credentials",
                    "the user should see GoTrue's prose, not its error code"
                );
            }
            other => panic!("expected Rejected, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn sign_in_with_password_separates_unreachable_from_refused() {
        // Port 1 on loopback refuses instantly. A transport failure must not be reported
        // as a credential problem — that is the mistake `RefreshError` exists to avoid.
        let error = sign_in_with_password("http://127.0.0.1:1", "anon", "you@example.edu", "pw")
            .await
            .expect_err("nothing is listening on port 1");
        assert!(
            matches!(error, SignInError::Transport(_)),
            "expected Transport, got {error:?}"
        );
    }

    #[tokio::test]
    async fn sign_in_with_password_reports_an_unreadable_success_body() {
        let (url, _seen) = recording_stub("200 OK", "this is not json").await;
        let error = sign_in_with_password(&url, "anon", "you@example.edu", "pw")
            .await
            .expect_err("a 200 with a junk body is not a session");
        assert!(
            matches!(error, SignInError::Malformed(_)),
            "expected Malformed, got {error:?}"
        );
    }

    #[test]
    fn gotrue_error_text_reads_every_shape_gotrue_ships() {
        // GoTrue has used all three over the versions a self-hosted stack might run.
        let cases = [
            (
                r#"{"error_description":"Email not confirmed"}"#,
                "Email not confirmed",
            ),
            (
                r#"{"msg":"Invalid login credentials"}"#,
                "Invalid login credentials",
            ),
            (
                r#"{"message":"signups not allowed"}"#,
                "signups not allowed",
            ),
            (r#"{"error":"invalid_grant"}"#, "invalid_grant"),
        ];
        for (body, expected) in cases {
            assert_eq!(
                gotrue_error_text(body).as_deref(),
                Some(expected),
                "failed to read {body}"
            );
        }
    }

    #[test]
    fn gotrue_error_text_prefers_prose_over_a_code() {
        // Current GoTrue sends both; `invalid_credentials` tells the user less than the
        // sentence next to it does.
        let body =
            r#"{"code":400,"error_code":"invalid_credentials","msg":"Invalid login credentials"}"#;
        assert_eq!(
            gotrue_error_text(body).as_deref(),
            Some("Invalid login credentials")
        );
    }

    #[test]
    fn gotrue_error_text_declines_bodies_it_cannot_read() {
        // Falling back to the raw body is the caller's job, so these must be None rather
        // than a guess.
        for body in [
            "",
            "<html>502 Bad Gateway</html>",
            "{}",
            r#"{"unexpected":"shape"}"#,
            r#"{"msg":""}"#,
        ] {
            assert!(
                gotrue_error_text(body).is_none(),
                "should not have extracted a message from {body:?}"
            );
        }
    }

    #[test]
    fn sign_in_error_display_keeps_the_status_visible() {
        let rejected = SignInError::Rejected {
            status: 400,
            detail: "Invalid login credentials".to_string(),
        };
        let shown = rejected.to_string();
        assert!(shown.contains("Invalid login credentials"), "{shown}");
        assert!(shown.contains("400"), "{shown}");

        // Transport and Malformed already carry a full sentence, so Display must not
        // decorate them with a status they do not have.
        assert_eq!(
            SignInError::Transport("host is down".to_string()).to_string(),
            "host is down"
        );
    }
}
