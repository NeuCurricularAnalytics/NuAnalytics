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
//! `Arc<RwLock<…>>`, and every request first calls `current_token` (private),
//! which refreshes proactively when the cached access token is expired. This
//! keeps a long-running process (the MCP server in particular) alive past the
//! 1-hour JWT expiry without a restart — and, because the refresh path re-reads
//! the on-disk auth file first, a fresh `db login` by the user is picked up
//! mid-process. A 401 from `PostgREST` triggers one reactive refresh + retry.

use super::auth::{
    auth_file_path, load_and_refresh, load_auth_state, refresh_session, save_auth_state, AuthState,
    RefreshError,
};
use super::error::{DatabaseError, DatabaseResult};
use super::query::{FilterKind, QueryFilters};
use super::tables;
use crate::core::config::DatabaseConfig;
use std::path::{Path, PathBuf};
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

/// Map an auth-layer [`RefreshError`] onto the database error the caller should see.
///
/// A transport failure becomes [`DatabaseError::ConnectionError`] so its remediation says
/// "could not reach the backend" rather than "run `db login`" — the mis-diagnosis this
/// distinction exists to prevent.
fn refresh_error_to_database_error(error: RefreshError) -> DatabaseError {
    match error {
        RefreshError::Transport(msg) => DatabaseError::ConnectionError(msg),
        RefreshError::Rejected(msg) | RefreshError::Malformed(msg) => {
            DatabaseError::NotAuthenticated(msg)
        }
    }
}

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
            .map_err(refresh_error_to_database_error)?
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
        let fresh = self.load_fresh_state(&guard, force).await?;
        let token = fresh.access_token.clone();
        *guard = fresh;
        drop(guard);
        Ok(token)
    }

    /// Obtain a fresh [`AuthState`]. Prefers the on-disk auth file (so a fresh
    /// `db login` is picked up without a restart) via [`load_and_refresh`],
    /// which also persists the refreshed token; falls back to refreshing the
    /// in-memory refresh token for clients with no tracked auth file.
    ///
    /// `force` — set after the backend rejected the token we just sent — diverts the
    /// on-disk path to [`Self::force_refresh_from_disk`], which ignores `is_valid()`.
    /// Without that divert, `load_and_refresh`'s short-circuit returns the same rejected
    /// token and the caller's retry is wasted.
    async fn load_fresh_state(
        &self,
        current: &AuthState,
        force: bool,
    ) -> DatabaseResult<AuthState> {
        if let Some(path) = self.auth_path.as_deref() {
            if force {
                return self.force_refresh_from_disk(path, current).await;
            }
            return load_and_refresh(path, &self.endpoint, &self.anon_key)
                .await
                .map_err(refresh_error_to_database_error)?
                .ok_or_else(|| {
                    DatabaseError::NotAuthenticated(format!(
                        "auth file disappeared at {}",
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
            .map_err(refresh_error_to_database_error)
    }

    /// Obtain a token after the backend *rejected* the one we just used.
    ///
    /// [`load_and_refresh`] cannot serve this case: it short-circuits on
    /// `state.is_valid()`, which is true for a token that is clock-valid but revoked, so
    /// it hands back the same dead token and the caller's retry is wasted.
    ///
    /// Two steps, in order:
    /// 1. Re-read the auth file. If it now holds a *different* access token, another
    ///    process refreshed or re-logged-in; use that rather than burning a refresh.
    /// 2. Otherwise exchange the refresh token directly.
    async fn force_refresh_from_disk(
        &self,
        path: &Path,
        rejected: &AuthState,
    ) -> DatabaseResult<AuthState> {
        let on_disk = load_auth_state(path).ok_or_else(|| {
            DatabaseError::NotAuthenticated(format!("auth file disappeared at {}", path.display()))
        })?;

        // Must also be clock-valid: adopting a *different but expired* token spends the
        // caller's single retry on a token that cannot work either.
        if on_disk.access_token != rejected.access_token && on_disk.is_valid() {
            return Ok(on_disk);
        }

        if on_disk.refresh_token.is_empty() {
            return Err(DatabaseError::NotAuthenticated(format!(
                "{} holds no refresh token",
                path.display()
            )));
        }

        let refreshed = refresh_session(&self.endpoint, &self.anon_key, &on_disk.refresh_token)
            .await
            .map_err(|e| match e {
                // Unreachable is not a login problem, even on the forced path.
                RefreshError::Transport(msg) => DatabaseError::ConnectionError(msg),
                other => DatabaseError::NotAuthenticated(format!(
                    "{other}; the session for {} was rejected and could not be refreshed",
                    self.endpoint
                )),
            })?;
        // Best-effort persist so sibling processes see the rotation too.
        let _ = save_auth_state(path, &refreshed);
        Ok(refreshed)
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
    /// Propagates the error from [`Self::select`] unchanged, so the variant reflects what
    /// actually happened: [`DatabaseError::ConnectionError`] when the backend could not
    /// be reached, [`DatabaseError::NotAuthenticated`] for a 401 that survived the forced
    /// refresh and retry, and [`DatabaseError::QueryError`] when the backend answered
    /// with some other error status.
    ///
    /// It deliberately does **not** rewrite an answered failure as a connection error.
    /// Doing so reported a self-hosted stack that answers `404` because its schema was
    /// never migrated as "could not reach the backend" — a cause the code had not
    /// established.
    pub async fn ping(&self) -> DatabaseResult<()> {
        let filters = QueryFilters::new();
        self.select(tables::INSTITUTIONS, "unitid", &filters, Some(1))
            .await
            .map(|_| ())
    }

    /// Query a table with filters, returning results as a JSON array.
    ///
    /// Sends `apikey: <anon key>` + `Authorization: Bearer <user JWT>` so RLS
    /// sees the request as `authenticated`. `select_cols` is comma-separated
    /// (use `"*"` for all).
    ///
    /// # Errors
    /// - [`DatabaseError::QueryError`] if `PostgREST` returns a non-success
    ///   status other than 401 (e.g. 400 for malformed filters, 404 when the table is
    ///   absent).
    /// - [`DatabaseError::NotAuthenticated`] if a 401 survives the forced refresh and
    ///   retry.
    /// - [`DatabaseError::ConnectionError`] if the backend could not be reached at all.
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
            return Err(self.classify_failure(response).await);
        }

        response
            .json::<serde_json::Value>()
            .await
            .map_err(|e| DatabaseError::ParseError(e.to_string()))
    }

    /// Turn a non-success `PostgREST` response into the right error variant.
    ///
    /// A 401 that survives the forced refresh-and-retry is an authentication failure, not
    /// a query failure: reporting it as `QueryError("PostgREST error (401)")` gave the
    /// user a bare status with no indication that `db login` was the fix.
    async fn classify_failure(&self, response: reqwest::Response) -> DatabaseError {
        let status = response.status().as_u16();
        let body = response.text().await.unwrap_or_default();
        if status == 401 {
            return DatabaseError::NotAuthenticated(format!(
                "{} rejected the session after a forced refresh (401): {body}",
                self.endpoint
            ));
        }
        DatabaseError::QueryError(format!("PostgREST error ({status}): {body}"))
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
            .map_err(|e| {
                DatabaseError::ConnectionError(format!("request to {} failed: {e}", self.endpoint))
            })
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
            .map_err(|e| {
                DatabaseError::ConnectionError(format!("request to {} failed: {e}", self.endpoint))
            })
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
    /// - [`DatabaseError::QueryError`] if Supabase answers with an error status other
    ///   than 401.
    /// - [`DatabaseError::NotAuthenticated`] if a 401 survives the forced refresh and
    ///   retry.
    /// - [`DatabaseError::ConnectionError`] if the backend could not be reached at all.
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
                return Err(self.classify_failure(response).await);
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

    // ---- forced re-authentication after a rejected token --------------------

    /// Session returned by the stub's happy auth route.
    const REFRESHED: &str = r#"{"access_token":"refreshed-token","refresh_token":"refresh-next","expires_in":3600,"user":{"email":"u@example.com"}}"#;

    /// Serve canned HTTP responses on an ephemeral port and return the base URL.
    ///
    /// `/auth/v1/token` (the `GoTrue` refresh) always gets a fresh session, so a client's
    /// forced refresh succeeds and its retry actually happens. Every other path gets
    /// `status_line`/`body`. Without the auth route the refresh fails first and the
    /// query response is never classified.
    ///
    /// Avoids a mock-HTTP dev-dependency: `tokio` is already a `database`-feature dep and
    /// these tests are already `#[tokio::test]`. The whole auth/query surface previously
    /// had no coverage at the HTTP boundary.
    async fn stub_server(status_line: &'static str, body: &'static str) -> String {
        stub_server_with_auth(("200 OK", REFRESHED), status_line, body).await
    }

    /// As [`stub_server`], but non-auth requests are answered from `responses` in order,
    /// reusing the last entry once exhausted — so a 401 can be followed by a 200 and the
    /// *successful* retry becomes expressible.
    async fn stub_server_seq(responses: Vec<(&'static str, &'static str)>) -> String {
        assert!(!responses.is_empty(), "stub needs at least one response");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        tokio::spawn(async move {
            let mut next = 0usize;
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0u8; 8192];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                    .await
                    .unwrap_or(0);
                let is_auth = String::from_utf8_lossy(&buf[..n]).contains("/auth/v1/token");
                let (line, payload) = if is_auth {
                    ("200 OK", REFRESHED)
                } else {
                    let entry = responses[next.min(responses.len() - 1)];
                    next += 1;
                    entry
                };
                let response = format!(
                    "HTTP/1.1 {line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });
        url
    }

    /// As [`stub_server`], but `auth` answers `/auth/v1/token`, so the *forced refresh
    /// itself* can be made to fail. Without this the auth route always succeeded and the
    /// refresh-rejected branch was unreachable — two tests took the same path.
    async fn stub_server_with_auth(
        auth: (&'static str, &'static str),
        status_line: &'static str,
        body: &'static str,
    ) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral port");
        let url = format!("http://{}", listener.local_addr().expect("local addr"));
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = [0u8; 8192];
                let n = tokio::io::AsyncReadExt::read(&mut stream, &mut buf)
                    .await
                    .unwrap_or(0);
                let is_auth = String::from_utf8_lossy(&buf[..n]).contains("/auth/v1/token");
                let (line, payload) = if is_auth { auth } else { (status_line, body) };
                let response = format!(
                    "HTTP/1.1 {line}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{payload}",
                    payload.len()
                );
                let _ = tokio::io::AsyncWriteExt::write_all(&mut stream, response.as_bytes()).await;
            }
        });
        url
    }

    fn session(access: &str, refresh: &str, expires_offset: i64) -> AuthState {
        AuthState {
            access_token: access.to_string(),
            refresh_token: refresh.to_string(),
            expires_at: chrono::Utc::now().timestamp() + expires_offset,
            user_email: Some("u@example.com".to_string()),
        }
    }

    #[tokio::test]
    async fn forced_refresh_adopts_a_newer_on_disk_token_without_spending_the_refresh() {
        // Another process (a `db login` in a second terminal) rotated the file. The
        // forced path must adopt that token rather than exchanging a refresh token —
        // and it must not need the network to do so, which this test proves by
        // pointing at an endpoint that would refuse to connect.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &session("newer-token", "refresh-b", 3600)).expect("save");

        let client = DbClient::new_with_session(
            "http://127.0.0.1:1", // nothing is listening
            "anon",
            session("rejected-token", "refresh-a", 3600),
            Some(path.clone()),
        )
        .expect("client builds");

        let token = client
            .reauthenticate(true)
            .await
            .expect("forced reauth adopts the on-disk token");
        assert_eq!(
            token, "newer-token",
            "a forced refresh must pick up a token another process wrote"
        );
    }

    #[tokio::test]
    async fn forced_refresh_reports_a_missing_auth_file_as_not_authenticated() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("gone.json");
        let client = DbClient::new_with_session(
            "http://127.0.0.1:1",
            "anon",
            session("tok", "refresh", 3600),
            Some(path.clone()),
        )
        .expect("client builds");

        let err = client
            .reauthenticate(true)
            .await
            .expect_err("no auth file means no session");
        match err {
            DatabaseError::NotAuthenticated(detail) => {
                assert!(
                    detail.contains(&path.display().to_string()),
                    "must name the file it looked for: {detail}"
                );
                // The remedy lives in next_steps, not in the detail, so one failure
                // states it once.
                let steps = DatabaseError::NotAuthenticated(detail)
                    .next_steps("e")
                    .join(" ");
                assert!(
                    steps.contains("db login"),
                    "must say what to do next: {steps}"
                );
            }
            other => panic!("expected NotAuthenticated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn forced_refresh_without_a_refresh_token_says_so() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        // Same access token as the rejected one, and no refresh token to fall back on.
        save_auth_state(&path, &session("rejected-token", "", 3600)).expect("save");
        let client = DbClient::new_with_session(
            "http://127.0.0.1:1",
            "anon",
            session("rejected-token", "", 3600),
            Some(path),
        )
        .expect("client builds");

        let err = client
            .reauthenticate(true)
            .await
            .expect_err("cannot refresh");
        match err {
            DatabaseError::NotAuthenticated(detail) => {
                assert!(detail.contains("no refresh token"), "got: {detail}");
                let steps = DatabaseError::NotAuthenticated(detail)
                    .next_steps("e")
                    .join(" ");
                assert!(steps.contains("db login"), "got: {steps}");
            }
            other => panic!("expected NotAuthenticated, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_rejected_refresh_is_not_reported_as_a_rejected_retry() {
        // The refresh itself is refused here. Distinct from
        // `a_401_surviving_the_retry_is_not_authenticated`, where the refresh succeeds
        // and the *retry* is refused — the two previously shared a stub that always
        // accepted the refresh, so they were the same test under two names.
        let url = stub_server_with_auth(
            ("401 Unauthorized", r#"{"error":"invalid_grant"}"#),
            "401 Unauthorized",
            r#"{"message":"JWT expired"}"#,
        )
        .await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &session("same-token", "refresh", 3600)).expect("save");

        let client = DbClient::new_with_session(
            &url,
            "anon",
            session("same-token", "refresh", 3600),
            Some(path.clone()),
        )
        .expect("client");

        let err = client
            .select("institutions", "*", &QueryFilters::default(), Some(1))
            .await
            .expect_err("a refused refresh must surface as an error");
        match err {
            DatabaseError::NotAuthenticated(detail) => {
                assert!(
                    detail.contains("could not be refreshed"),
                    "a refused refresh must not be reported as a refused retry: {detail}"
                );
                assert!(
                    detail.contains("invalid_grant"),
                    "must quote what the provider said: {detail}"
                );
                assert!(
                    !detail.contains("forced refresh"),
                    "that wording belongs to the surviving-401 case, not this one: {detail}"
                );
                assert!(
                    detail.contains(&url),
                    "must name the backend it talked to: {detail}"
                );
            }
            other => panic!("expected NotAuthenticated for a surviving 401, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_non_401_failure_is_still_a_query_error() {
        // Only 401 changes variant; a 400 must stay a QueryError so callers don't
        // suggest logging in for a malformed request.
        let url = stub_server(
            "400 Bad Request",
            r#"{"message":"column \"nope\" does not exist"}"#,
        )
        .await;
        let client = DbClient::new(&url, "anon", "tok".to_string()).expect("client");

        let err = client
            .select("institutions", "nope", &QueryFilters::default(), Some(1))
            .await
            .expect_err("400 is an error");
        match err {
            DatabaseError::QueryError(detail) => {
                assert!(detail.contains("400"), "got: {detail}");
                assert!(detail.contains("does not exist"), "got: {detail}");
            }
            other => panic!("expected QueryError for a 400, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn a_401_surviving_the_retry_is_not_authenticated() {
        // The defect this replaces: a stale-but-clock-valid token produced a bare
        // `PostgREST error (401)` classified as a QueryError, so nothing told the user
        // that `db login` was the fix. Here the stub's auth route lets the forced
        // refresh succeed, so the retry runs — and is refused again.
        let url = stub_server("401 Unauthorized", r#"{"message":"JWT expired"}"#).await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &session("same-token", "refresh", 3600)).expect("save");
        let client = DbClient::new_with_session(
            &url,
            "anon",
            session("same-token", "refresh", 3600),
            Some(path),
        )
        .expect("client");

        let err = client
            .select("institutions", "*", &QueryFilters::default(), Some(1))
            .await
            .expect_err("a 401 that survives the retry must be an error");
        match err {
            DatabaseError::NotAuthenticated(detail) => {
                assert!(
                    detail.contains("forced refresh"),
                    "must say the refresh was already tried: {detail}"
                );
                assert!(detail.contains("401"), "must report the status: {detail}");
                assert!(
                    detail.contains(&url),
                    "must name the backend it talked to: {detail}"
                );
                let steps = DatabaseError::NotAuthenticated(detail)
                    .next_steps(&url)
                    .join(" ");
                assert!(
                    steps.contains("db login"),
                    "must say what to do next: {steps}"
                );
            }
            other => panic!("expected NotAuthenticated for a surviving 401, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn an_unreachable_backend_at_startup_is_not_reported_as_a_login_problem() {
        // The mis-diagnosis this whole workstream exists to kill, at the one place it
        // mattered most: the MCP server builds its client once via `from_config`, so a
        // backend that is merely down used to be reported as "not signed in" — and
        // therefore "run `db login`" — for the entire process lifetime.
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        // Expired on purpose: that is what forces `from_config` to attempt a refresh,
        // which is where the transport failure happens.
        save_auth_state(&path, &session("tok", "refresh", -3600)).expect("save");

        let config = DatabaseConfig {
            enabled: true,
            // Port 1 is not listening, so the connection is refused without a timeout.
            endpoint: "http://127.0.0.1:1".to_string(),
            anon_key: "anon".to_string(),
            auth_file: path.display().to_string(),
            management_key: String::new(),
        };

        let err = DbClient::from_config(&config)
            .await
            .expect_err("an unreachable backend cannot yield a client");
        match err {
            DatabaseError::ConnectionError(detail) => {
                assert!(
                    detail.contains("127.0.0.1:1"),
                    "must name the backend it could not reach: {detail}"
                );
                let steps = DatabaseError::ConnectionError(detail).next_steps(&config.endpoint);
                let joined = steps.join(" ");
                assert!(
                    joined.contains("not a login problem"),
                    "remediation must not send the user to `db login`: {joined}"
                );
                assert!(
                    !joined.contains("db login"),
                    "an unreachable backend must never be told to log in: {joined}"
                );
            }
            other => {
                panic!("an unreachable backend must classify as ConnectionError, got {other:?}")
            }
        }
    }

    #[tokio::test]
    async fn a_401_that_the_forced_refresh_fixes_succeeds_and_persists_the_rotation() {
        // Every other new test here asserts a failure. This covers the path the retry
        // exists for — 401, forced refresh, retry succeeds — and the on-disk rotation
        // that lets sibling processes reuse the new session. Delete the `save_auth_state`
        // in `force_refresh_from_disk` and only this test notices.
        let url = stub_server_seq(vec![
            ("401 Unauthorized", r#"{"message":"JWT expired"}"#),
            ("200 OK", "[]"),
        ])
        .await;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("auth.json");
        save_auth_state(&path, &session("same-token", "refresh", 3600)).expect("save");
        let client = DbClient::new_with_session(
            &url,
            "anon",
            session("same-token", "refresh", 3600),
            Some(path.clone()),
        )
        .expect("client");

        let rows = client
            .select("institutions", "*", &QueryFilters::default(), Some(1))
            .await
            .expect("the retry after a forced refresh must succeed");
        assert!(rows.is_array(), "got {rows:?}");

        let on_disk = load_auth_state(&path).expect("auth file still present");
        assert_eq!(
            on_disk.access_token, "refreshed-token",
            "the rotated session must be persisted so sibling processes see it"
        );
    }
}
