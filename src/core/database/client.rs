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
//! [`auth::load_and_refresh`], which exchanges the saved refresh token
//! for a fresh access token whenever the current one is within the
//! [`AuthState::is_expired`] 60s safety buffer.

use super::auth::{auth_file_path, load_and_refresh, load_auth_state, save_auth_state, AuthState};
use super::error::{DatabaseError, DatabaseResult};
use super::query::{FilterKind, QueryFilters};
use super::tables;
use crate::core::config::DatabaseConfig;
use std::path::PathBuf;

/// Batch size for upsert HTTP requests.
const WRITE_BATCH_SIZE: usize = 500;

/// Relative path under which `PostgREST` exposes the tables.
const REST_API_PREFIX: &str = "/rest/v1";

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
    /// Signed-in user JWT — goes in `Authorization: Bearer` on every request
    user_jwt: String,
    /// Path to the on-disk auth file, when known. Used to persist refreshed
    /// tokens back to disk so the next process startup also sees a valid
    /// session. `None` when the client was constructed with an explicit JWT
    /// (e.g. in tests).
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
        Self::new_with_auth_path(
            &config.endpoint,
            &config.anon_key,
            state.access_token,
            Some(auth_path),
        )
    }

    /// Create a client with explicit credentials. Useful for tests and for
    /// callers that have already loaded a session manually.
    ///
    /// # Errors
    ///
    /// - [`DatabaseError::NotConfigured`] if endpoint or anon key are empty.
    /// - [`DatabaseError::NotAuthenticated`] if `user_jwt` is empty.
    pub fn new(endpoint: &str, anon_key: &str, user_jwt: String) -> DatabaseResult<Self> {
        Self::new_with_auth_path(endpoint, anon_key, user_jwt, None)
    }

    fn new_with_auth_path(
        endpoint: &str,
        anon_key: &str,
        user_jwt: String,
        auth_path: Option<PathBuf>,
    ) -> DatabaseResult<Self> {
        if endpoint.is_empty() || anon_key.is_empty() {
            return Err(DatabaseError::NotConfigured);
        }
        if user_jwt.is_empty() {
            return Err(DatabaseError::NotAuthenticated(
                "empty user JWT".to_string(),
            ));
        }
        Ok(Self {
            http: reqwest::Client::new(),
            endpoint: endpoint.to_string(),
            anon_key: anon_key.to_string(),
            user_jwt,
            auth_path,
        })
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
        let response = self
            .http
            .get(&url)
            .header("apikey", &self.anon_key)
            .header("Authorization", format!("Bearer {}", self.user_jwt))
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| DatabaseError::QueryError(format!("HTTP request failed: {e}")))?;

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
            let response = self
                .http
                .post(&url)
                .header("apikey", &self.anon_key)
                .header("Authorization", format!("Bearer {}", self.user_jwt))
                .header("Content-Type", "application/json")
                .header("Prefer", "resolution=merge-duplicates")
                .json(chunk)
                .send()
                .await
                .map_err(|e| DatabaseError::QueryError(format!("HTTP request failed: {e}")))?;

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
        assert_eq!(client.user_jwt, "jwt");
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
}
