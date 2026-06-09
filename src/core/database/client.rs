//! Supabase database client.
//!
//! ## How authentication works
//!
//! Every Supabase `PostgREST` request carries two headers:
//!
//! ```text
//! apikey:        <project anon key>   — identifies the project
//! Authorization: Bearer <user JWT>    — identifies the signed-in user
//! ```
//!
//! Both reads and writes use the same split-header pattern with a real
//! user JWT — there is no anon-only read path. Row-level security on
//! every table requires `auth.role() = 'authenticated'`, so the client
//! refuses to build without a valid session. The session is loaded via
//! [`super::auth::load_and_refresh`], which exchanges the saved refresh
//! token for a fresh access token whenever the current one is within
//! the [`AuthState::is_expired`] 60s safety buffer.
//!
//! ## Long-lived sessions (MCP server)
//!
//! The whole [`AuthState`] (access **and** refresh token) is held behind an
//! `Arc<RwLock<…>>`, and every request first calls [`DbClient::current_token`],
//! which refreshes proactively when the cached access token is expired. This
//! keeps a long-running process (the MCP server in particular) alive past the
//! 1-hour JWT expiry without a restart — and, because the refresh path re-reads
//! the on-disk auth file first, a fresh `db login` by the user is picked up
//! mid-process. A 401 from `PostgREST` triggers one reactive refresh + retry.

use super::auth::{
    auth_file_path, load_and_refresh, load_auth_state, refresh_session, save_auth_state, AuthState,
};
use super::error::{DatabaseError, DatabaseResult};
use super::query::{FilterKind, QueryFilters};
use super::tables;
use crate::core::config::DatabaseConfig;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// Batch size for upsert HTTP requests.
const WRITE_BATCH_SIZE: usize = 500;

/// Relative path under which `PostgREST` exposes the tables.
const REST_API_PREFIX: &str = "/rest/v1";

/// Total per-request timeout. A stalled `PostgREST` call (e.g. against a
/// half-open connection after a laptop sleep) returns a clean error well under
/// the 4-minute MCP ceiling instead of hanging the tool call.
const HTTP_TIMEOUT: Duration = Duration::from_mins(1);

/// Connection-establishment timeout — fail fast when the endpoint is
/// unreachable rather than waiting out the full request budget.
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Database client backed by Supabase.
///
/// Wraps a single `reqwest` client and carries the credentials for every
/// request. Constructed via [`DbClient::from_config`] — which loads the
/// saved session and refreshes it if expired — or [`DbClient::new`] for
/// callers that already hold a token.
#[derive(Debug, Clone)]
pub struct DbClient {
    /// HTTP client shared across reads and writes
    http: reqwest::Client,
    /// Supabase project URL (e.g. `https://xyz.supabase.co`)
    endpoint: String,
    /// Project anon key — goes in the `apikey` header on every request
    anon_key: String,
    /// The signed-in session (access + refresh token + expiry), behind a lock
    /// so it can be refreshed in place across a long-lived process. Cloned
    /// `DbClient`s share the same session via the `Arc`.
    session: Arc<RwLock<AuthState>>,
    /// Path to the on-disk auth file, when known. Used to persist refreshed
    /// tokens back to disk so the next process startup also sees a valid
    /// session, and to pick up a fresh `db login` mid-process. `None` when the
    /// client was constructed with an explicit JWT (e.g. in tests).
    auth_path: Option<PathBuf>,
}

impl DbClient {
    /// Create a new client from configuration, loading and refreshing the
    /// saved session as needed.
    ///
    /// # Errors
    ///
    /// - [`DatabaseError::Disabled`] if `config.enabled` is false.
    /// - [`DatabaseError::NotConfigured`] if endpoint or anon key are empty.
    /// - [`DatabaseError::NotAuthenticated`] if the auth file is missing,
    ///   the refresh token was rejected, or the network is unreachable. The
    ///   inner string carries diagnostic detail for the caller to surface.
    pub async fn from_config(config: &DatabaseConfig) -> DatabaseResult<Self> {
        if !config.enabled {
            return Err(DatabaseError::Disabled);
        }
        if config.endpoint.is_empty() || config.anon_key.is_empty() {
            return Err(DatabaseError::NotConfigured);
        }
        let auth_path = auth_file_path(config);
        let state = load_and_refresh(&auth_path, &config.endpoint, &config.anon_key)
            .await
            .map_err(DatabaseError::NotAuthenticated)?
            .ok_or_else(|| {
                DatabaseError::NotAuthenticated(format!("no auth file at {}", auth_path.display()))
            })?;
        Self::new_with_session(&config.endpoint, &config.anon_key, state, Some(auth_path))
    }

    /// Create a client with an explicit JWT. Useful for tests and for callers
    /// that already hold a token. The synthesised session carries no refresh
    /// token and never expires, so such a client never attempts a refresh.
    ///
    /// # Errors
    ///
    /// - [`DatabaseError::NotConfigured`] if endpoint or anon key are empty.
    /// - [`DatabaseError::NotAuthenticated`] if `user_jwt` is empty.
    pub fn new(endpoint: &str, anon_key: &str, user_jwt: String) -> DatabaseResult<Self> {
        let state = AuthState {
            access_token: user_jwt,
            refresh_token: String::new(),
            // Far-future expiry → `is_valid()` is always true → no refresh path.
            expires_at: i64::MAX,
            user_email: None,
        };
        Self::new_with_session(endpoint, anon_key, state, None)
    }

    fn new_with_session(
        endpoint: &str,
        anon_key: &str,
        state: AuthState,
        auth_path: Option<PathBuf>,
    ) -> DatabaseResult<Self> {
        if endpoint.is_empty() || anon_key.is_empty() {
            return Err(DatabaseError::NotConfigured);
        }
        if state.access_token.is_empty() {
            return Err(DatabaseError::NotAuthenticated(
                "empty user JWT".to_string(),
            ));
        }
        let http = reqwest::Client::builder()
            .timeout(HTTP_TIMEOUT)
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .build()
            .map_err(|e| {
                DatabaseError::ConnectionError(format!("HTTP client build failed: {e}"))
            })?;
        Ok(Self {
            http,
            endpoint: endpoint.to_string(),
            anon_key: anon_key.to_string(),
            session: Arc::new(RwLock::new(state)),
            auth_path,
        })
    }

    /// Return a usable access token, refreshing proactively when the cached one
    /// is expired (or within the 60s safety buffer).
    ///
    /// # Errors
    /// [`DatabaseError::NotAuthenticated`] if a refresh was needed but failed
    /// (network error, revoked refresh token, missing auth file).
    async fn current_token(&self) -> DatabaseResult<String> {
        {
            let session = self.session.read().await;
            if session.is_valid() {
                return Ok(session.access_token.clone());
            }
        }
        self.reauthenticate(false).await
    }

    /// Refresh the in-memory session and return the new access token.
    ///
    /// `force` skips the "another task already refreshed" fast path so a 401 on
    /// a clock-valid token still triggers a re-auth. Holds the write lock across
    /// the network call so concurrent callers don't stampede the refresh.
    async fn reauthenticate(&self, force: bool) -> DatabaseResult<String> {
        let mut guard = self.session.write().await;
        if !force && guard.is_valid() {
            return Ok(guard.access_token.clone());
        }
        let fresh = self.load_fresh_state(&guard).await?;
        let token = fresh.access_token.clone();
        *guard = fresh;
        drop(guard);
        Ok(token)
    }

    /// Obtain a fresh [`AuthState`]. Prefers the on-disk auth file (so a fresh
    /// `db login` is picked up without a restart) via [`load_and_refresh`],
    /// which also persists the refreshed token; falls back to refreshing the
    /// in-memory refresh token for clients with no tracked auth file.
    async fn load_fresh_state(&self, current: &AuthState) -> DatabaseResult<AuthState> {
        if let Some(path) = self.auth_path.as_deref() {
            return load_and_refresh(path, &self.endpoint, &self.anon_key)
                .await
                .map_err(DatabaseError::NotAuthenticated)?
                .ok_or_else(|| {
                    DatabaseError::NotAuthenticated(format!(
                        "auth file disappeared at {}; run `nuanalytics db login`",
                        path.display()
                    ))
                });
        }
        if current.refresh_token.is_empty() {
            return Err(DatabaseError::NotAuthenticated(
                "session expired and no refresh token available".to_string(),
            ));
        }
        refresh_session(&self.endpoint, &self.anon_key, &current.refresh_token)
            .await
            .map_err(DatabaseError::NotAuthenticated)
    }

    /// Refresh the cached email associated with the current session, if any.
    /// Returns `None` when no auth file is tracked (e.g. test clients) or
    /// when the on-disk file no longer carries an email.
    #[must_use]
    pub fn signed_in_email(&self) -> Option<String> {
        self.auth_path
            .as_deref()
            .and_then(load_auth_state)
            .and_then(|s| s.user_email)
    }

    /// Path to the on-disk auth file backing this client, if any.
    #[must_use]
    pub fn auth_path(&self) -> Option<&std::path::Path> {
        self.auth_path.as_deref()
    }

    /// Persist a freshly-issued [`AuthState`] back to the on-disk auth file,
    /// when one is tracked. Best-effort: missing path or write failure is
    /// swallowed so the in-memory client keeps working.
    pub fn persist_session(&self, state: &AuthState) {
        if let Some(path) = self.auth_path.as_deref() {
            let _ = save_auth_state(path, state);
        }
    }

    /// Check database connectivity with a minimal authenticated read.
    ///
    /// # Errors
    /// Returns [`DatabaseError::ConnectionError`] when the underlying read
    /// fails (including a 401 if the session is no longer valid).
    pub async fn ping(&self) -> DatabaseResult<()> {
        let filters = QueryFilters::new();
        match self
            .select(tables::INSTITUTIONS, "unitid", &filters, Some(1))
            .await
        {
            Ok(_) => Ok(()),
            Err(DatabaseError::QueryError(msg)) => Err(DatabaseError::ConnectionError(msg)),
            Err(e) => Err(e),
        }
    }

    /// Query a table with filters, returning results as a JSON array.
    ///
    /// Sends `apikey: <anon key>` + `Authorization: Bearer <user JWT>` so RLS
    /// sees the request as `authenticated`. `select_cols` is comma-separated
    /// (use `"*"` for all).
    ///
    /// # Errors
    /// - [`DatabaseError::QueryError`] if `PostgREST` returns a non-success
    ///   status (e.g. 401 when the token expired mid-request, 400 for
    ///   malformed filters).
    /// - [`DatabaseError::ParseError`] if the response is not valid JSON.
    pub async fn select(
        &self,
        table: &str,
        select_cols: &str,
        filters: &QueryFilters,
        limit: Option<usize>,
    ) -> DatabaseResult<serde_json::Value> {
        let url = build_select_url(&self.endpoint, table, select_cols, filters, limit);

        let mut token = self.current_token().await?;
        let mut response = self.send_get(&url, &token).await?;
        // A 401 here means the token was rejected despite looking valid by the
        // clock (revoked, clock skew). Force one reauth + retry before giving up.
        if response.status().as_u16() == 401 {
            token = self.reauthenticate(true).await?;
            response = self.send_get(&url, &token).await?;
        }

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(DatabaseError::QueryError(format!(
                "PostgREST error ({status}): {body}"
            )));
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| DatabaseError::ParseError(e.to_string()))
    }

    /// Issue a single authenticated `GET`. Split out so [`Self::select`] can
    /// reissue it with a fresh token after a 401.
    async fn send_get(&self, url: &str, token: &str) -> DatabaseResult<reqwest::Response> {
        self.http
            .get(url)
            .header("apikey", &self.anon_key)
            .header("Authorization", format!("Bearer {token}"))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| DatabaseError::QueryError(format!("HTTP request failed: {e}")))
    }

    /// Issue a single authenticated upsert `POST` for one chunk. Split out so
    /// [`Self::upsert_batch`] can reissue it with a fresh token after a 401.
    async fn send_upsert(
        &self,
        url: &str,
        token: &str,
        chunk: &[serde_json::Value],
    ) -> DatabaseResult<reqwest::Response> {
        self.http
            .post(url)
            .header("apikey", &self.anon_key)
            .header("Authorization", format!("Bearer {token}"))
            .header("Content-Type", "application/json")
            .header("Prefer", "resolution=merge-duplicates")
            .json(chunk)
            .send()
            .await
            .map_err(|e| DatabaseError::QueryError(format!("HTTP request failed: {e}")))
    }

    /// Upsert a batch of records.
    ///
    /// Requires the same authenticated session as [`Self::select`]; with the
    /// auth-required RLS model both paths use the same headers.
    ///
    /// `on_conflict` is the column(s) for upsert conflict resolution
    /// (e.g. `&["unitid"]` or `&["unitid", "cip_code", "award_level", "year"]`).
    ///
    /// `None`-valued fields in serialised records are kept so that `PostgREST`
    /// sees a uniform key set across the entire batch — PGRST102 fires when
    /// records in the same request have different key sets. Nulls map to
    /// SQL `NULL`, which is correct for optional IPEDS fields.
    ///
    /// # Errors
    /// - [`DatabaseError::ParseError`] if records cannot be serialised.
    /// - [`DatabaseError::QueryError`] if the HTTP request fails or Supabase
    ///   returns an error status.
    pub async fn upsert_batch<T>(
        &self,
        table: &str,
        records: Vec<T>,
        on_conflict: &[&str],
    ) -> DatabaseResult<()>
    where
        T: serde::Serialize,
    {
        if records.is_empty() {
            return Ok(());
        }

        let json_records: Vec<serde_json::Value> = records
            .into_iter()
            .map(|r| serde_json::to_value(r).map_err(|e| DatabaseError::ParseError(e.to_string())))
            .collect::<DatabaseResult<Vec<_>>>()?;

        let conflict_param = on_conflict.join(",");
        let url = format!(
            "{}{REST_API_PREFIX}/{table}?on_conflict={conflict_param}",
            self.endpoint
        );

        for chunk in json_records.chunks(WRITE_BATCH_SIZE) {
            let mut token = self.current_token().await?;
            let mut response = self.send_upsert(&url, &token, chunk).await?;
            if response.status().as_u16() == 401 {
                token = self.reauthenticate(true).await?;
                response = self.send_upsert(&url, &token, chunk).await?;
            }

            if !response.status().is_success() {
                let status = response.status().as_u16();
                let body = response.text().await.unwrap_or_default();
                return Err(DatabaseError::QueryError(format!(
                    "PostgREST error ({status}): {body}"
                )));
            }
        }

        Ok(())
    }
}

/// Build a `PostgREST` query URL from a table, column list, filter set, and
/// optional limit. Values are percent-encoded via the `form_urlencoded`
/// serialiser, which intentionally leaves `*` literal — `PostgREST` treats
/// `*` as the SQL `%` wildcard in `like` / `ilike` filters.
fn build_select_url(
    endpoint: &str,
    table: &str,
    select_cols: &str,
    filters: &QueryFilters,
    limit: Option<usize>,
) -> String {
    let mut url = format!(
        "{endpoint}{REST_API_PREFIX}/{table}?select={}",
        url_encode(select_cols)
    );
    for (kind, col, val) in &filters.entries {
        let encoded = url_encode(&filter_value(kind, val));
        url.push('&');
        url.push_str(col);
        url.push('=');
        url.push_str(&encoded);
    }
    if let Some(n) = limit {
        use std::fmt::Write as _;
        // Writing into a `String` never errors — the unwrap is on the
        // `fmt::Error`, not the formatting itself.
        write!(&mut url, "&limit={n}").unwrap();
    }
    url
}

/// Render a filter value into the `PostgREST` `op.value` form (e.g.
/// `eq.MA`, `in.(1,2,3)`, `ilike.*northeastern*`).
fn filter_value(kind: &FilterKind, val: &str) -> String {
    match kind {
        FilterKind::Eq => format!("eq.{val}"),
        FilterKind::Ilike => format!("ilike.{val}"),
        FilterKind::StartsWith => format!("like.{val}"),
        FilterKind::Gte => format!("gte.{val}"),
        FilterKind::Lte => format!("lte.{val}"),
        // `in` wants parenthesised list: `in.(v1,v2,v3)`.
        FilterKind::In => format!("in.({val})"),
    }
}

/// Percent-encode a query-string value using `application/x-www-form-urlencoded`
/// rules — preserves `*` (`PostgREST` wildcard) while still encoding spaces,
/// commas inside non-list values, and other unsafe characters.
fn url_encode(value: &str) -> String {
    form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_rejects_empty_endpoint() {
        let err =
            DbClient::new("", "anon", "jwt".to_string()).expect_err("empty endpoint must error");
        assert!(matches!(err, DatabaseError::NotConfigured));
    }

    #[test]
    fn new_rejects_empty_anon_key() {
        let err = DbClient::new("https://example.supabase.co", "", "jwt".to_string())
            .expect_err("empty anon key must error");
        assert!(matches!(err, DatabaseError::NotConfigured));
    }

    #[test]
    fn new_rejects_empty_user_jwt() {
        let err = DbClient::new("https://example.supabase.co", "anon", String::new())
            .expect_err("empty user JWT must error");
        assert!(matches!(err, DatabaseError::NotAuthenticated(_)));
    }

    #[test]
    fn new_succeeds_with_complete_credentials() {
        let client = DbClient::new("https://example.supabase.co", "anon", "jwt".to_string())
            .expect("complete credentials must produce a client");
        assert_eq!(client.endpoint, "https://example.supabase.co");
        assert_eq!(client.anon_key, "anon");
        let session = client
            .session
            .try_read()
            .expect("uncontended session read in test");
        assert_eq!(session.access_token, "jwt");
        // The explicit-JWT constructor synthesises a non-expiring session with
        // no refresh token, so such a client never attempts a refresh.
        assert!(session.refresh_token.is_empty());
        assert!(session.is_valid());
        drop(session);
        assert!(client.auth_path().is_none());
    }

    #[test]
    fn build_select_url_renders_filters_and_limit() {
        let filters = QueryFilters::new()
            .eq("state", Some("MA"))
            .ilike("name", Some("northeastern"));
        let url = build_select_url(
            "https://example.supabase.co",
            "institutions",
            "unitid,name",
            &filters,
            Some(50),
        );
        assert!(url.starts_with("https://example.supabase.co/rest/v1/institutions?select="));
        assert!(url.contains("state=eq.MA"), "missing state filter: {url}");
        assert!(
            url.contains("name=ilike.*northeastern*"),
            "ilike value must keep `*` literals: {url}"
        );
        assert!(url.ends_with("&limit=50"));
    }

    #[test]
    fn build_select_url_in_list_uses_paren_list() {
        let filters = QueryFilters::new().in_list("unitid", &[167_358_i32, 166_629]);
        let url = build_select_url(
            "https://example.supabase.co",
            "institutions",
            "*",
            &filters,
            None,
        );
        assert!(
            url.contains("unitid=in.%28167358%2C166629%29")
                || url.contains("unitid=in.(167358,166629)"),
            "in-list must percent-encode parens and commas: {url}"
        );
    }

    #[test]
    fn build_select_url_no_filters_no_limit() {
        let filters = QueryFilters::new();
        let url = build_select_url(
            "https://example.supabase.co",
            "cip_codes",
            "cip_code,title",
            &filters,
            None,
        );
        assert_eq!(
            url,
            "https://example.supabase.co/rest/v1/cip_codes?select=cip_code%2Ctitle"
        );
    }

    #[test]
    fn build_select_url_encodes_spaces_in_filter_values() {
        // A real-world filter — institution names contain spaces. The
        // serialiser must encode them so PostgREST sees the literal value.
        let filters = QueryFilters::new().eq("name", Some("The State University of New York"));
        let url = build_select_url(
            "https://example.supabase.co",
            "institutions",
            "*",
            &filters,
            None,
        );
        assert!(
            url.contains("name=eq.The+State+University+of+New+York"),
            "spaces must be encoded as `+` in form-urlencoded values: {url}"
        );
    }

    #[test]
    fn build_select_url_preserves_wildcard_star_in_ilike() {
        // `*` is the PostgREST wildcard; the serialiser must keep it literal.
        let filters = QueryFilters::new().ilike("name", Some("northeastern"));
        let url = build_select_url(
            "https://example.supabase.co",
            "institutions",
            "*",
            &filters,
            None,
        );
        assert!(
            url.contains("ilike.*northeastern*"),
            "wildcards must survive encoding: {url}"
        );
    }

    #[test]
    fn signed_in_email_is_none_for_test_clients() {
        let client = DbClient::new("https://example.supabase.co", "anon", "jwt".to_string())
            .expect("test client");
        assert!(client.signed_in_email().is_none());
        assert!(client.auth_path().is_none());
    }

    use crate::core::config::DatabaseConfig;

    fn test_config(auth_file: &str) -> DatabaseConfig {
        DatabaseConfig {
            enabled: true,
            endpoint: "https://example.supabase.co".to_string(),
            anon_key: "anon".to_string(),
            auth_file: auth_file.to_string(),
            management_key: String::new(),
        }
    }

    #[tokio::test]
    async fn from_config_returns_disabled_when_feature_off() {
        let mut config = test_config("/tmp/never-read.json");
        config.enabled = false;
        let err = DbClient::from_config(&config)
            .await
            .expect_err("disabled config must error");
        assert!(matches!(err, DatabaseError::Disabled));
    }

    #[tokio::test]
    async fn from_config_returns_not_configured_when_endpoint_missing() {
        let mut config = test_config("/tmp/never-read.json");
        config.endpoint = String::new();
        let err = DbClient::from_config(&config)
            .await
            .expect_err("empty endpoint must error");
        assert!(matches!(err, DatabaseError::NotConfigured));
    }

    #[tokio::test]
    async fn from_config_returns_not_authenticated_when_auth_file_missing() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("does-not-exist.json");
        let config = test_config(&path.to_string_lossy());
        let err = DbClient::from_config(&config)
            .await
            .expect_err("missing auth file must error");
        match err {
            DatabaseError::NotAuthenticated(detail) => {
                assert!(
                    detail.contains("no auth file"),
                    "detail should mention the missing file: {detail}"
                );
            }
            other => panic!("expected NotAuthenticated, got {other:?}"),
        }
    }

    fn auth_state(access: &str, refresh: &str, expires_offset_secs: i64) -> AuthState {
        AuthState {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at: chrono::Utc::now().timestamp() + expires_offset_secs,
            user_email: None,
        }
    }

    #[tokio::test]
    async fn current_token_returns_cached_token_without_network_when_valid() {
        // `new` synthesises a far-future expiry. Pointing at an unreachable
        // endpoint proves no refresh round-trip is attempted for a valid token.
        let client = DbClient::new(
            "http://127.0.0.1:1/never",
            "anon",
            "valid-token".to_string(),
        )
        .expect("test client");
        let token = client
            .current_token()
            .await
            .expect("a valid token must not trigger a refresh");
        assert_eq!(token, "valid-token");
    }

    #[tokio::test]
    async fn explicit_jwt_client_never_refreshes() {
        let client =
            DbClient::new("http://127.0.0.1:1/never", "anon", "tok".to_string()).expect("client");
        // Both the proactive and the non-forced reauth paths short-circuit on a
        // session that is valid by the clock.
        assert_eq!(client.current_token().await.unwrap(), "tok");
        assert_eq!(client.reauthenticate(false).await.unwrap(), "tok");
    }

    #[tokio::test]
    async fn current_token_picks_up_fresh_disk_session_when_cached_is_expired() {
        // Mirrors the field report's "user re-ran `db login` but the MCP server
        // didn't notice" case: a fresh, valid session on disk must be adopted
        // without a network refresh (the unreachable endpoint proves it).
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &auth_state("disk-fresh", "r", 3600)).expect("write disk session");

        let stale = auth_state("stale", "old-refresh", -100);
        let client =
            DbClient::new_with_session("http://127.0.0.1:1/never", "anon", stale, Some(path))
                .expect("client");

        let token = client
            .current_token()
            .await
            .expect("must adopt the fresh disk session");
        assert_eq!(
            token, "disk-fresh",
            "a valid on-disk re-login must be picked up without a restart or network call"
        );
    }

    #[tokio::test]
    async fn current_token_errors_when_expired_and_no_refresh_available() {
        // Expired in-memory session, no auth file, empty refresh token → there
        // is nothing to refresh with, so it must fail fast rather than hang.
        let expired = auth_state("x", "", -100);
        let client =
            DbClient::new_with_session("https://example.supabase.co", "anon", expired, None)
                .expect("client");
        let err = client
            .current_token()
            .await
            .expect_err("expired session with no refresh path must error");
        assert!(
            matches!(err, DatabaseError::NotAuthenticated(_)),
            "got {err:?}"
        );
    }
}
