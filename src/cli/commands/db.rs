//! Database management CLI commands.
//!
//! Handles `nuanalytics db` subcommands:
//! - `login` / `logout` / `whoami` — Supabase OAuth authentication
//! - `status` — connectivity check
//! - `ipeds-import` — IPEDS CSV ingestion

use tokio::io::{AsyncReadExt, AsyncWriteExt};

use nu_analytics::config::Config;
use nu_analytics::database::{
    auth_file_path, clear_auth_state, ipeds, load_auth_state, save_auth_state, AuthState, DbClient,
};

use crate::args::DbSubcommand;

const OAUTH_TIMEOUT_SECS: u64 = 120;
const SUPABASE_MGMT_API_BASE: &str = "https://api.supabase.com/v1/projects";

/// Run the `db` subcommand, dispatching to the appropriate handler.
pub fn run(subcommand: DbSubcommand, config: &Config) {
    match subcommand {
        DbSubcommand::Login { provider } => run_login(config, &provider),
        DbSubcommand::Logout => run_logout(config),
        DbSubcommand::Whoami => run_whoami(config),
        DbSubcommand::ExecSql { file } => run_exec_sql(config, &file),
        DbSubcommand::Status => run_status(config),
        DbSubcommand::IpedsImport {
            dir,
            institutions,
            completions,
            year,
        } => run_ipeds_import(config, dir.as_deref(), institutions, completions, year),
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Build a single-threaded Tokio runtime, printing an error and returning `None` on failure.
fn make_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => Some(rt),
        Err(e) => {
            eprintln!("✗ Failed to create async runtime: {e}");
            None
        }
    }
}

// ============================================================================
// Login — OAuth PKCE flow
// ============================================================================

/// Validate database config, build an async runtime, and run the OAuth login flow.
fn run_login(config: &Config, provider: &str) {
    if config.database.endpoint.is_empty() || config.database.anon_key.is_empty() {
        eprintln!("✗ Database not configured.");
        eprintln!("  Set `database.endpoint` and `database.anon_key` in your config, then re-run.");
        eprintln!("  nuanalytics config set database.endpoint https://your-project.supabase.co");
        eprintln!("  nuanalytics config set database.anon_key <anon-key>");
        return;
    }

    let Some(rt) = make_runtime() else { return };

    if let Err(e) = rt.block_on(do_oauth_login(config, provider)) {
        eprintln!("✗ Login failed: {e}");
    }
}

/// Carry out the full OAuth 2.0 PKCE flow:
///
/// 1. Generate a PKCE verifier/challenge pair.
/// 2. Bind a local callback server on a random port.
/// 3. Build the Supabase OAuth URL and open the user's browser.
/// 4. Wait up to 2 minutes for the browser to redirect back with a code.
/// 5. Exchange the code + verifier for a session.
/// 6. Save the session to disk.
async fn do_oauth_login(config: &Config, provider: &str) -> Result<(), String> {
    use supabase_client_sdk::supabase_client_auth::AuthClient;

    // In WSL2, ports bound to 127.0.0.1 are NOT forwarded to the Windows host —
    // only ports on 0.0.0.0 are. We therefore bind to 0.0.0.0 under WSL so the
    // Windows browser can reach the listener via localhost port forwarding.
    // The redirect URL still uses 127.0.0.1 (what the browser connects to).
    let bind_addr = if detect_wsl() {
        "0.0.0.0:0"
    } else {
        "127.0.0.1:0"
    };
    let listener = tokio::net::TcpListener::bind(bind_addr)
        .await
        .map_err(|e| format!("Cannot bind callback port: {e}"))?;
    let port = listener
        .local_addr()
        .map_err(|e| format!("Cannot get local addr: {e}"))?
        .port();
    let callback_url = format!("http://127.0.0.1:{port}/callback");

    let pkce = AuthClient::generate_pkce_pair();
    let verifier = pkce.verifier.as_str().to_string();

    let oauth_provider = parse_provider(provider);
    let auth_client = AuthClient::new(&config.database.endpoint, &config.database.anon_key)
        .map_err(|e| format!("Cannot create auth client: {e}"))?;

    // get_oauth_sign_in_url gives us the base URL; we append PKCE params manually
    let base_url = auth_client
        .get_oauth_sign_in_url(oauth_provider, Some(&callback_url), None)
        .map_err(|e| format!("Cannot build OAuth URL: {e}"))?;

    let auth_url = format!(
        "{}&code_challenge={}&code_challenge_method=S256",
        base_url,
        pkce.challenge.as_str()
    );

    println!("Opening browser to authenticate with {provider}...");
    if let Err(e) = open_browser(&auth_url) {
        eprintln!("  Could not open browser automatically: {e}");
    }
    println!("If the browser did not open, visit:");
    println!("  {auth_url}");
    println!("Waiting for OAuth callback ({OAUTH_TIMEOUT_SECS}s timeout)...");

    let code = tokio::time::timeout(
        tokio::time::Duration::from_secs(OAUTH_TIMEOUT_SECS),
        accept_oauth_callback(listener),
    )
    .await
    .map_err(|_| format!("Timed out waiting for browser callback ({OAUTH_TIMEOUT_SECS}s)"))??;

    let session = auth_client
        .exchange_code_for_session(&code, Some(&verifier))
        .await
        .map_err(|e| format!("Token exchange failed: {e}"))?;

    let expires_at = session
        .expires_at
        .unwrap_or_else(|| chrono::Utc::now().timestamp() + session.expires_in);

    let state = AuthState {
        access_token: session.access_token,
        refresh_token: session.refresh_token,
        expires_at,
        user_email: session.user.email,
    };

    let auth_path = auth_file_path(&config.database);
    save_auth_state(&auth_path, &state)?;

    let email = state.user_email.as_deref().unwrap_or("(no email)");
    println!("✓ Signed in as {email}");
    println!("  Session saved to {}", auth_path.display());

    Ok(())
}

/// Parse a provider name string into the SDK's `OAuthProvider` enum.
///
/// Unknown names become `OAuthProvider::Custom`, so this never fails.
fn parse_provider(name: &str) -> supabase_client_sdk::supabase_client_auth::OAuthProvider {
    use supabase_client_sdk::supabase_client_auth::OAuthProvider;
    match name.to_lowercase().as_str() {
        "github" => OAuthProvider::GitHub,
        "google" => OAuthProvider::Google,
        "gitlab" => OAuthProvider::GitLab,
        "discord" => OAuthProvider::Discord,
        "azure" => OAuthProvider::Azure,
        "bitbucket" => OAuthProvider::Bitbucket,
        "linkedin" => OAuthProvider::LinkedIn,
        "twitter" => OAuthProvider::Twitter,
        other => OAuthProvider::Custom(other.to_string()),
    }
}

/// Returns `true` when running inside a WSL (Windows Subsystem for Linux) environment.
///
/// Checks for environment variables set by the WSL kernel. Used to decide whether to
/// open the Windows browser and bind on all interfaces for port forwarding.
fn detect_wsl() -> bool {
    std::env::var_os("WSL_DISTRO_NAME").is_some() || std::env::var_os("WSL_INTEROP").is_some()
}

/// Escape a URL for embedding inside a `PowerShell` single-quoted string literal.
///
/// In `PowerShell`, single-quoted strings are verbatim except that a literal `'`
/// must be written as `''`. This prevents `&` in OAuth URLs from being
/// interpreted as the `PowerShell` call operator.
fn ps_single_quote_escape(url: &str) -> String {
    url.replace('\'', "''")
}

/// Open a URL in the system default browser.
///
/// On WSL and native Windows, uses `powershell.exe Start-Process` rather than
/// `cmd.exe /C start`: `PowerShell` single-quoted strings keep `&` in OAuth URLs
/// from being treated as a command separator. On plain Linux, stderr from
/// `xdg-open` is suppressed to avoid D-Bus noise on headless systems.
fn open_browser(url: &str) -> Result<(), String> {
    // `PowerShell`'s single-quoted strings are verbatim, so '&' in OAuth URLs is
    // never interpreted as the call operator or a cmd.exe command separator.
    let ps_open = || {
        let ps_cmd = format!("Start-Process -FilePath '{}'", ps_single_quote_escape(url));
        std::process::Command::new("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", &ps_cmd])
            .stderr(std::process::Stdio::null())
            .spawn()
    };

    let result = if detect_wsl() {
        ps_open()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else if cfg!(target_os = "windows") {
        ps_open()
    } else {
        // Plain Linux — suppress D-Bus noise that xdg-open prints on headless systems
        std::process::Command::new("xdg-open")
            .arg(url)
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    result.map(|_| ()).map_err(|e| e.to_string())
}

/// Accept one HTTP request on the listener, extract the OAuth `code` parameter,
/// and return a response page to the browser.
async fn accept_oauth_callback(listener: tokio::net::TcpListener) -> Result<String, String> {
    let (mut stream, _) = listener
        .accept()
        .await
        .map_err(|e| format!("Callback accept failed: {e}"))?;

    let mut buf = [0u8; 4096];
    let n = stream
        .read(&mut buf)
        .await
        .map_err(|e| format!("Callback read failed: {e}"))?;
    let request = std::str::from_utf8(&buf[..n])
        .unwrap_or("")
        .lines()
        .next()
        .unwrap_or("");

    // Request line: "GET /callback?code=XXX&... HTTP/1.1"
    let code = extract_query_param(request, "code");
    let error = extract_query_param(request, "error");

    let (status, body) = if code.is_some() {
        ("200 OK", CALLBACK_SUCCESS_HTML)
    } else {
        ("400 Bad Request", CALLBACK_ERROR_HTML)
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.ok();

    code.ok_or_else(|| {
        error.map_or_else(
            || "No code in OAuth callback — check the provider is enabled in Supabase".to_string(),
            |e| format!("OAuth error: {e}"),
        )
    })
}

/// Extract the first value of a named query parameter from an HTTP request line.
///
/// Handles `+` and `%XX` decoding via `form_urlencoded` — the same parser
/// browsers use for `application/x-www-form-urlencoded` request bodies.
fn extract_query_param(request_line: &str, name: &str) -> Option<String> {
    // Slice out the query string between '?' and the trailing ' HTTP/...'
    let qs_start = request_line.find('?')?;
    let qs_end = request_line.rfind(' ').unwrap_or(request_line.len());
    let qs = &request_line[qs_start + 1..qs_end];

    form_urlencoded::parse(qs.as_bytes()).find_map(|(k, v)| (k == name).then(|| v.into_owned()))
}

const CALLBACK_SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>NuAnalytics — Signed In</title>
<style>body{font-family:sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#f0f9f4}
.box{text-align:center;padding:2rem;background:#fff;border-radius:8px;box-shadow:0 2px 12px rgba(0,0,0,.1)}
h1{color:#16803d}p{color:#555}</style></head>
<body><div class="box"><h1>✓ Signed in successfully</h1>
<p>You can close this tab and return to the terminal.</p></div></body></html>"#;

const CALLBACK_ERROR_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>NuAnalytics — Sign In Failed</title>
<style>body{font-family:sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#fff5f5}
.box{text-align:center;padding:2rem;background:#fff;border-radius:8px;box-shadow:0 2px 12px rgba(0,0,0,.1)}
h1{color:#c53030}p{color:#555}</style></head>
<body><div class="box"><h1>✗ Sign in failed</h1>
<p>Check that the provider is enabled in your Supabase project, then try again.</p></div></body></html>"#;

// ============================================================================
// Logout
// ============================================================================

fn run_logout(config: &Config) {
    let path = auth_file_path(&config.database);
    match load_auth_state(&path) {
        Some(state) if state.is_valid() => {
            let email = state.user_email.as_deref().unwrap_or("unknown");
            clear_auth_state(&path);
            println!("✓ Signed out ({email})");
        }
        Some(_) => {
            clear_auth_state(&path);
            println!("✓ Cleared expired session");
        }
        None => println!("  No active session found"),
    }
}

// ============================================================================
// Whoami
// ============================================================================

fn run_whoami(config: &Config) {
    let path = auth_file_path(&config.database);
    match load_auth_state(&path) {
        Some(state) if state.is_valid() => {
            let email = state.user_email.as_deref().unwrap_or("(email not stored)");
            let expires = chrono::DateTime::from_timestamp(state.expires_at, 0).map_or_else(
                || "unknown".to_string(),
                |dt| dt.format("%Y-%m-%d %H:%M UTC").to_string(),
            );
            println!("Signed in as: {email}");
            println!("Token expires: {expires}");
            println!("Auth file:    {}", path.display());
        }
        Some(_) => println!("Session expired. Run `nuanalytics db login` to sign in again."),
        None => println!("Not signed in. Run `nuanalytics db login`."),
    }
}

// ============================================================================
// exec-sql
// ============================================================================

/// Extract the Supabase project reference from the configured endpoint URL.
///
/// Parses the leading subdomain from a Supabase URL. Returns `None` if the
/// URL is empty or has no subdomain before the first `.`.
///
/// `"https://abcdefgh.supabase.co"` → `Some("abcdefgh")`
fn extract_project_ref(endpoint: &str) -> Option<String> {
    let host = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    let subdomain = host.split('.').next()?;
    if subdomain.is_empty() {
        None
    } else {
        Some(subdomain.to_string())
    }
}

/// Read a SQL file from disk and execute it against the Supabase Management API.
fn run_exec_sql(config: &Config, file: &std::path::Path) {
    if config.database.management_key.is_empty() {
        eprintln!("✗ `database.management_key` is not set.");
        eprintln!("  1. Go to https://app.supabase.com/account/tokens");
        eprintln!("  2. Create a Personal Access Token");
        eprintln!("  3. Run: nuanalytics config set database.management_key <token>");
        return;
    }

    let Some(project_ref) = extract_project_ref(&config.database.endpoint) else {
        eprintln!(
            "✗ Cannot determine project ref from endpoint: {}",
            config.database.endpoint
        );
        return;
    };

    let sql = match std::fs::read_to_string(file) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("✗ Cannot read {}: {e}", file.display());
            return;
        }
    };

    println!(
        "Executing {} ({} bytes) against project {}...",
        file.display(),
        sql.len(),
        project_ref
    );

    let Some(rt) = make_runtime() else { return };

    match rt.block_on(do_exec_sql(
        &config.database.management_key,
        &project_ref,
        &sql,
    )) {
        Ok(msg) => println!("✓ {msg}"),
        Err(e) => eprintln!("✗ SQL execution failed: {e}"),
    }
}

/// Execute arbitrary SQL via the Supabase Management API.
///
/// Posts to `{SUPABASE_MGMT_API_BASE}/{project_ref}/database/query`
/// using the provided Personal Access Token for authorization. Returns a
/// human-readable success message or the API's error string on failure.
///
/// # Errors
///
/// Returns an error if the HTTP request fails or the API returns a non-success status.
async fn do_exec_sql(management_key: &str, project_ref: &str, sql: &str) -> Result<String, String> {
    let url = format!("{SUPABASE_MGMT_API_BASE}/{project_ref}/database/query");

    let body = serde_json::json!({ "query": sql });

    let response = reqwest::Client::new()
        .post(&url)
        .header("Authorization", format!("Bearer {management_key}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("HTTP request failed: {e}"))?;

    let status = response.status();
    let json: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse response: {e}"))?;

    if status.is_success() {
        let rows = json.as_array().map_or(0, Vec::len);
        if rows > 0 {
            Ok(format!("Done — {rows} rows returned"))
        } else {
            Ok("Done".to_string())
        }
    } else {
        let msg = json
            .get("message")
            .or_else(|| json.get("error"))
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| json.as_str().unwrap_or("Unknown error"));
        Err(msg.to_string())
    }
}

// ============================================================================
// Status
// ============================================================================

fn run_status(config: &Config) {
    let client = match DbClient::from_config(&config.database) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ Database not available: {e}");
            return;
        }
    };

    // Show access level: read-only (anon key) vs read-write (user JWT)
    if client.is_authenticated() {
        let email = load_auth_state(&auth_file_path(&config.database))
            .and_then(|s| s.user_email)
            .unwrap_or_else(|| "authenticated user".to_string());
        println!("Auth: read-write  (signed in as {email})");
    } else {
        println!("Auth: read-only   (not signed in — run `nuanalytics db login` for write access)");
    }

    let Some(rt) = make_runtime() else { return };

    match rt.block_on(client.ping()) {
        Ok(()) => println!("✓ Database connection successful"),
        Err(e) => eprintln!("✗ Database ping failed: {e}"),
    }
}

// ============================================================================
// IPEDS Import
// ============================================================================

/// Resolve IPEDS file paths, preferring auto-detection from `dir` over explicit paths.
///
/// When `dir` is provided, searches for standard IPEDS filenames for the given year.
/// Falls back to the explicitly supplied paths when `dir` is `None`.
fn resolve_ipeds_paths(
    dir: Option<&std::path::Path>,
    institutions_path: Option<std::path::PathBuf>,
    completions_path: Option<std::path::PathBuf>,
    year: u16,
) -> (Option<std::path::PathBuf>, Option<std::path::PathBuf>) {
    dir.map_or((institutions_path, completions_path), |d| {
        (
            auto_detect_file(
                d,
                &[
                    &format!("HD{year}.csv"),
                    &format!("HD{year}.zip"),
                    "HD*.csv",
                ],
            ),
            auto_detect_file(
                d,
                &[
                    &format!("C{year}_A.csv"),
                    &format!("C{year}_A.zip"),
                    "C*_A.csv",
                ],
            ),
        )
    })
}

fn run_ipeds_import(
    config: &Config,
    dir: Option<&std::path::Path>,
    institutions_path: Option<std::path::PathBuf>,
    completions_path: Option<std::path::PathBuf>,
    year: u16,
) {
    let client = match DbClient::from_config(&config.database) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ Database not available: {e}");
            eprintln!("  Configure the database and run `nuanalytics db login` for write access.");
            return;
        }
    };

    let Some(rt) = make_runtime() else { return };

    let (inst_path, comp_path) =
        resolve_ipeds_paths(dir, institutions_path, completions_path, year);

    if let Some(path) = inst_path {
        println!("Importing institutions from {} ...", path.display());
        match rt.block_on(ipeds::ingest_institutions(&client, &path, year)) {
            Ok(stats) => println!(
                "  ✓ {} read, {} upserted, {} skipped",
                stats.rows_read, stats.rows_upserted, stats.rows_skipped
            ),
            Err(e) => eprintln!("  ✗ Institutions import failed: {e}"),
        }
    } else {
        println!("  ℹ Skipping institutions (no file provided or found)");
    }

    if let Some(path) = comp_path {
        println!("Importing completions from {} ...", path.display());
        println!("  (all CIP codes stored; query with CIP filter for CS vs all-programs)");
        match rt.block_on(ipeds::ingest_completions(&client, &path, year)) {
            Ok(stats) => println!(
                "  ✓ {} rows read, {} matched CS CIP codes, {} upserted, {} skipped",
                stats.rows_read, stats.rows_filtered, stats.rows_upserted, stats.rows_skipped
            ),
            Err(e) => eprintln!("  ✗ Completions import failed: {e}"),
        }
    } else {
        println!("  ℹ Skipping completions (no file provided or found)");
    }
}

// ============================================================================
// Helpers
// ============================================================================

/// Auto-detect a file in a directory by trying names in order.
///
/// Supports simple glob-style patterns with a single `*` wildcard
/// (e.g. `"HD*.csv"` matches `"HD2023.csv"`). Returns the first match, or `None`.
fn auto_detect_file(dir: &std::path::Path, candidates: &[&str]) -> Option<std::path::PathBuf> {
    for candidate in candidates {
        let path = dir.join(candidate);
        if path.exists() {
            return Some(path);
        }

        if candidate.contains('*') {
            let prefix = candidate.split('*').next().unwrap_or("");
            let suffix = candidate.split('*').next_back().unwrap_or("");
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let name = entry.file_name();
                    let name_str = name.to_string_lossy();
                    if name_str.starts_with(prefix) && name_str.ends_with(suffix) {
                        return Some(entry.path());
                    }
                }
            }
        }
    }
    None
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // --- extract_query_param -----------------------------------------------

    #[test]
    fn test_extract_query_param_basic() {
        let line = "GET /callback?code=abc123&state=xyz HTTP/1.1";
        assert_eq!(
            extract_query_param(line, "code"),
            Some("abc123".to_string())
        );
        assert_eq!(extract_query_param(line, "state"), Some("xyz".to_string()));
    }

    #[test]
    fn test_extract_query_param_missing() {
        let line = "GET /callback?code=abc HTTP/1.1";
        assert_eq!(extract_query_param(line, "error"), None);
    }

    #[test]
    fn test_extract_query_param_no_qs() {
        let line = "GET /callback HTTP/1.1";
        assert_eq!(extract_query_param(line, "code"), None);
    }

    #[test]
    fn test_extract_query_param_percent_encoded() {
        let line = "GET /callback?code=abc%2B123&state=x%20y HTTP/1.1";
        assert_eq!(
            extract_query_param(line, "code"),
            Some("abc+123".to_string())
        );
        assert_eq!(extract_query_param(line, "state"), Some("x y".to_string()));
    }

    #[test]
    fn test_extract_query_param_decodes_plus_as_space() {
        // form_urlencoded decodes literal '+' as a space character — this
        // pins that behavior, which was previously covered by the now-deleted
        // test_percent_decode_plus_as_space.
        let line = "GET /callback?state=hello+world&code=a+b HTTP/1.1";
        assert_eq!(
            extract_query_param(line, "state"),
            Some("hello world".to_string())
        );
        assert_eq!(extract_query_param(line, "code"), Some("a b".to_string()));
    }

    // --- parse_provider ----------------------------------------------------

    #[test]
    fn test_parse_provider_known() {
        use supabase_client_sdk::supabase_client_auth::OAuthProvider;
        assert!(matches!(parse_provider("github"), OAuthProvider::GitHub));
        assert!(matches!(parse_provider("GITHUB"), OAuthProvider::GitHub));
        assert!(matches!(parse_provider("google"), OAuthProvider::Google));
        assert!(matches!(parse_provider("gitlab"), OAuthProvider::GitLab));
    }

    #[test]
    fn test_parse_provider_remaining_known() {
        use supabase_client_sdk::supabase_client_auth::OAuthProvider;
        assert!(matches!(parse_provider("discord"), OAuthProvider::Discord));
        assert!(matches!(parse_provider("azure"), OAuthProvider::Azure));
        assert!(matches!(
            parse_provider("bitbucket"),
            OAuthProvider::Bitbucket
        ));
        assert!(matches!(
            parse_provider("linkedin"),
            OAuthProvider::LinkedIn
        ));
        assert!(matches!(parse_provider("twitter"), OAuthProvider::Twitter));
    }

    #[test]
    fn test_parse_provider_custom() {
        use supabase_client_sdk::supabase_client_auth::OAuthProvider;
        assert!(matches!(parse_provider("myidp"), OAuthProvider::Custom(_)));
    }

    // --- ps_single_quote_escape --------------------------------------------

    #[test]
    fn test_ps_single_quote_escape_no_quotes() {
        // Typical OAuth URL — nothing to escape
        let url =
            "https://example.supabase.co/auth/v1/authorize?provider=github&code_challenge=abc";
        assert_eq!(ps_single_quote_escape(url), url);
    }

    #[test]
    fn test_ps_single_quote_escape_single_quote() {
        assert_eq!(ps_single_quote_escape("it's"), "it''s");
    }

    #[test]
    fn test_ps_single_quote_escape_multiple_quotes() {
        assert_eq!(ps_single_quote_escape("a'b'c"), "a''b''c");
    }

    #[test]
    fn test_ps_single_quote_escape_leading_trailing() {
        assert_eq!(ps_single_quote_escape("'hello'"), "''hello''");
    }

    #[test]
    fn test_ps_single_quote_escape_empty() {
        assert_eq!(ps_single_quote_escape(""), "");
    }

    // --- detect_wsl --------------------------------------------------------

    // std::env::set_var/remove_var are unsafe in Rust 1.81+ because they are not
    // thread-safe. This test manipulates env vars to exercise detect_wsl(); run the
    // test suite with --test-threads=1 if parallel test runners become a problem.
    #[allow(unsafe_code)]
    #[test]
    fn test_detect_wsl_via_env() {
        // Save current values so we can restore them after the test.
        let had_distro = std::env::var_os("WSL_DISTRO_NAME");
        let had_interop = std::env::var_os("WSL_INTEROP");

        // SAFETY: single-threaded test context; see module-level comment above.
        unsafe {
            std::env::remove_var("WSL_DISTRO_NAME");
            std::env::remove_var("WSL_INTEROP");
        }
        assert!(!detect_wsl(), "should be false when neither var is set");

        unsafe { std::env::set_var("WSL_DISTRO_NAME", "Ubuntu-22.04") };
        assert!(detect_wsl(), "WSL_DISTRO_NAME should trigger WSL detection");

        unsafe {
            std::env::remove_var("WSL_DISTRO_NAME");
            std::env::set_var("WSL_INTEROP", "/run/WSL/1_interop");
        }
        assert!(detect_wsl(), "WSL_INTEROP should trigger WSL detection");

        // Restore originals
        unsafe {
            std::env::remove_var("WSL_INTEROP");
            if let Some(v) = had_distro {
                std::env::set_var("WSL_DISTRO_NAME", v);
            }
            if let Some(v) = had_interop {
                std::env::set_var("WSL_INTEROP", v);
            }
        }
    }

    // --- auto_detect_file --------------------------------------------------

    #[test]
    fn test_auto_detect_file_exact_match() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("HD2023.csv");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"")
            .unwrap();
        let result = auto_detect_file(dir.path(), &["HD2023.csv"]);
        assert_eq!(result, Some(file_path));
    }

    #[test]
    fn test_auto_detect_file_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let result = auto_detect_file(dir.path(), &["HD2023.csv", "HD*.csv"]);
        assert_eq!(result, None);
    }

    #[test]
    fn test_auto_detect_file_glob_match() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("HD2023.csv");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"")
            .unwrap();
        // Pattern "HD*.csv" — no exact match; must fall through to glob branch
        let result = auto_detect_file(dir.path(), &["HD*.csv"]);
        assert_eq!(result, Some(file_path));
    }

    #[test]
    fn test_auto_detect_file_glob_suffix_mismatch() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("HD2023.csv");
        std::fs::File::create(&file_path)
            .unwrap()
            .write_all(b"")
            .unwrap();
        // Pattern "C*.csv" should NOT match "HD2023.csv"
        let result = auto_detect_file(dir.path(), &["C*.csv"]);
        assert_eq!(result, None);
    }

    // --- extract_project_ref -----------------------------------------------

    #[test]
    fn test_extract_project_ref_supabase_url() {
        assert_eq!(
            extract_project_ref("https://abcdefgh.supabase.co"),
            Some("abcdefgh".to_string())
        );
    }

    #[test]
    fn test_extract_project_ref_http_scheme() {
        assert_eq!(
            extract_project_ref("http://myproject.example.com"),
            Some("myproject".to_string())
        );
    }

    #[test]
    fn test_extract_project_ref_empty() {
        assert_eq!(extract_project_ref(""), None);
    }

    #[test]
    fn test_extract_project_ref_scheme_only() {
        assert_eq!(extract_project_ref("https://"), None);
    }

    #[test]
    fn test_extract_project_ref_dot_start() {
        assert_eq!(extract_project_ref("https://.supabase.co"), None);
    }

    #[test]
    fn test_extract_project_ref_multiple_subdomains() {
        // Only the first segment is returned
        assert_eq!(
            extract_project_ref("https://api.db.example.com"),
            Some("api".to_string())
        );
    }
}
