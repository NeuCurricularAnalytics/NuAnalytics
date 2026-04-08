//! Supabase database client.
//!
//! ## How authentication works
//!
//! Supabase `PostgREST` requires two separate headers on every request:
//!
//! ```text
//! apikey:        <project anon key>   — identifies the project, always the same
//! Authorization: Bearer <JWT>         — anon JWT for public reads,
//!                                       user JWT for authenticated writes
//! ```
//!
//! The `supabase-client-sdk` sets both headers to the same value, which is fine
//! for reads (anon key in both). For **writes with row-level security**, we need
//! `Authorization: Bearer <user JWT>` while keeping `apikey: <anon key>`. We
//! accomplish this by bypassing the SDK for write operations and making raw
//! `reqwest` calls with the headers set independently.
//!
//! This means:
//! - **Reads** (`select`): SDK with anon key — works for any public RLS policy
//! - **Writes** (`upsert_batch`): raw reqwest with split headers — requires the
//!   user to be logged in (`nuanalytics db login`)

use supabase_client_sdk::prelude::{Filterable, Modifiable, SupabaseClientQueryExt};
use supabase_client_sdk::{SupabaseClient, SupabaseConfig};

use super::auth::{auth_file_path, load_auth_state, AuthState};
use super::error::{DatabaseError, DatabaseResult};
use super::query::{FilterKind, QueryFilters};
use super::tables;
use crate::core::config::DatabaseConfig;

/// Batch size for upsert HTTP requests.
const WRITE_BATCH_SIZE: usize = 500;

/// Database client backed by Supabase.
///
/// Holds both an SDK client (for reads using the anon key) and a raw HTTP
/// client (for writes using split `apikey` / `Authorization` headers).
#[derive(Debug, Clone)]
pub struct DbClient {
    /// SDK client — always uses the anon key, suitable for public reads
    inner: SupabaseClient,
    /// Raw HTTP client for write operations that need split headers
    http: reqwest::Client,
    /// Supabase project URL (e.g. `https://xyz.supabase.co`)
    endpoint: String,
    /// Project anon key — goes in the `apikey` header on every request
    anon_key: String,
    /// Signed-in user JWT — goes in `Authorization: Bearer` on writes.
    /// `None` means the client operates in read-only mode.
    user_jwt: Option<String>,
}

impl DbClient {
    /// Create a new client from configuration, loading any saved session.
    ///
    /// If a valid OAuth session exists (from `nuanalytics db login`), the user JWT
    /// is stored and used in `Authorization: Bearer` on write requests. The anon key
    /// is always used for `apikey` on every request.
    ///
    /// # Errors
    ///
    /// - `DatabaseError::Disabled` if `config.enabled` is false.
    /// - `DatabaseError::NotConfigured` if endpoint or anon key are empty.
    /// - `DatabaseError::ConnectionError` if the SDK client fails to initialize.
    pub fn from_config(config: &DatabaseConfig) -> DatabaseResult<Self> {
        if !config.enabled {
            return Err(DatabaseError::Disabled);
        }
        let user_jwt = load_auth_state(&auth_file_path(config))
            .filter(AuthState::is_valid)
            .map(|s| s.access_token);
        Self::new(&config.endpoint, &config.anon_key, user_jwt)
    }

    /// Create a client with explicit credentials.
    ///
    /// `anon_key` is always used as the `apikey` header.
    /// `user_jwt` is used as `Authorization: Bearer` on writes when `Some`.
    ///
    /// # Errors
    ///
    /// - `DatabaseError::NotConfigured` if endpoint or anon key are empty.
    /// - `DatabaseError::ConnectionError` if the SDK client fails to initialize.
    pub fn new(endpoint: &str, anon_key: &str, user_jwt: Option<String>) -> DatabaseResult<Self> {
        if endpoint.is_empty() || anon_key.is_empty() {
            return Err(DatabaseError::NotConfigured);
        }
        let supabase_config = SupabaseConfig::new(endpoint, anon_key);
        let inner = SupabaseClient::new(supabase_config)
            .map_err(|e| DatabaseError::ConnectionError(e.to_string()))?;
        let http = reqwest::Client::new();
        Ok(Self {
            inner,
            http,
            endpoint: endpoint.to_string(),
            anon_key: anon_key.to_string(),
            user_jwt,
        })
    }

    /// Returns `true` if the client has a valid user JWT and can perform writes.
    #[must_use]
    pub const fn is_authenticated(&self) -> bool {
        self.user_jwt.is_some()
    }

    /// Check database connectivity with a minimal read query.
    ///
    /// # Errors
    ///
    /// Returns `DatabaseError::ConnectionError` if the ping fails.
    pub async fn ping(&self) -> DatabaseResult<()> {
        let response = self
            .inner
            .from(tables::INSTITUTIONS)
            .select("unitid")
            .limit(1)
            .execute()
            .await;

        if let Some(err) = response.error {
            return Err(DatabaseError::ConnectionError(err.to_string()));
        }
        Ok(())
    }

    /// Query a table with filters, returning results as a JSON array.
    ///
    /// Uses the anon key for both headers — suitable for any table with a public
    /// read RLS policy. `select_cols` is comma-separated (use `"*"` for all).
    ///
    /// # Errors
    ///
    /// - `DatabaseError::QueryError` if the query fails.
    /// - `DatabaseError::ParseError` if the response cannot be serialized.
    pub async fn select(
        &self,
        table: &str,
        select_cols: &str,
        filters: &QueryFilters,
        limit: Option<usize>,
    ) -> DatabaseResult<serde_json::Value> {
        let mut builder = self.inner.from(table).select(select_cols);

        for (kind, col, val) in &filters.entries {
            builder = match kind {
                FilterKind::Eq => builder.eq(col, val.as_str()),
                FilterKind::Ilike => builder.ilike(col, val.as_str()),
                FilterKind::StartsWith => builder.like(col, val.as_str()),
                FilterKind::Gte => builder.gte(col, val.as_str()),
                FilterKind::Lte => builder.lte(col, val.as_str()),
                FilterKind::In => {
                    // val is comma-separated (e.g. "167358,166629"); split for SDK's in_() call
                    let values: Vec<&str> = val.split(',').collect();
                    builder.in_(col, values)
                }
            };
        }

        if let Some(n) = limit {
            let n_i64 = i64::try_from(n).unwrap_or(i64::MAX);
            builder = builder.limit(n_i64);
        }

        let response = builder.execute().await;

        if let Some(err) = response.error {
            return Err(DatabaseError::QueryError(err.to_string()));
        }

        serde_json::to_value(&response.data).map_err(|e| DatabaseError::ParseError(e.to_string()))
    }

    /// Upsert a batch of records using split authentication headers.
    ///
    /// Requires the user to be signed in (`nuanalytics db login`). The request uses:
    /// - `apikey: <anon key>` — always the project anon key
    /// - `Authorization: Bearer <user JWT>` — the signed-in user's token
    ///
    /// Row-level security sees `auth.role() = 'authenticated'`, allowing writes
    /// that would be blocked for the anon role.
    ///
    /// `on_conflict` is the column(s) for upsert conflict resolution
    /// (e.g. `&["unitid"]` or `&["unitid", "cip_code", "award_level", "year"]`).
    ///
    /// `None`-valued fields are stripped so they don't overwrite existing DB data.
    ///
    /// # Errors
    ///
    /// - `DatabaseError::QueryError` if the user is not signed in.
    /// - `DatabaseError::ParseError` if records cannot be serialized.
    /// - `DatabaseError::QueryError` if the HTTP request fails or Supabase returns an error.
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

        let jwt = self.user_jwt.as_deref().ok_or_else(|| {
            DatabaseError::QueryError(
                "Write operations require authentication. Run `nuanalytics db login` first."
                    .to_string(),
            )
        })?;

        // Serialize records to JSON. Null fields are kept so that PostgREST receives
        // a uniform key set across the entire batch — PGRST102 is thrown when records
        // in the same request have different key sets. Null values map to NULL in the
        // database, which is correct for optional IPEDS fields.
        let json_records: Vec<serde_json::Value> = records
            .into_iter()
            .map(|r| serde_json::to_value(r).map_err(|e| DatabaseError::ParseError(e.to_string())))
            .collect::<DatabaseResult<Vec<_>>>()?;

        let conflict_param = on_conflict.join(",");
        let url = format!(
            "{}/rest/v1/{}?on_conflict={}",
            self.endpoint, table, conflict_param
        );

        for chunk in json_records.chunks(WRITE_BATCH_SIZE) {
            let response = self
                .http
                .post(&url)
                // apikey identifies the project — must be the anon key, not a user JWT
                .header("apikey", &self.anon_key)
                // Authorization carries the user JWT so RLS sees auth.role() = 'authenticated'
                .header("Authorization", format!("Bearer {jwt}"))
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
