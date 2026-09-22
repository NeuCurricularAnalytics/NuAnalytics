//! Database management CLI commands.
//!
//! Handles `nuanalytics db` subcommands:
//! - `login` / `logout` / `whoami` — Supabase authentication (OAuth or password)
//! - `status` — connectivity check
//! - `ipeds-import` — IPEDS CSV ingestion
//! - `import` — degree report → normalized program tables

use std::fmt::Write as _;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use nu_analytics::config::{Config, ConfigSources};
use nu_analytics::database::import::{execute_import, ImportOptions, ImportOutcome, ImportResult};
use nu_analytics::database::{
    auth_file_path, bootstrap, clear_auth_state, doctor, ipeds, load_auth_state, save_auth_state,
    sign_in_with_password, validate, AuthState, DatabaseError, DbClient, SignInError,
};

use crate::args::DbSubcommand;

const OAUTH_TIMEOUT_SECS: u64 = 120;
const SUPABASE_MGMT_API_BASE: &str = "https://api.supabase.com/v1/projects";

/// Run the `db` subcommand, dispatching to the appropriate handler.
pub fn run(subcommand: DbSubcommand, config: &Config, sources: &ConfigSources) {
    match subcommand {
        DbSubcommand::Login { provider, email } => {
            run_login(config, provider.as_deref(), email.as_deref());
        }
        DbSubcommand::Logout => run_logout(config),
        DbSubcommand::Whoami => run_whoami(config),
        DbSubcommand::ExecSql { file } => run_exec_sql(config, &file),
        DbSubcommand::Status => run_status(config, sources),
        DbSubcommand::Bootstrap { print } => run_bootstrap(config, print),
        DbSubcommand::Validate { file, year } => run_validate(config, &file, year),
        DbSubcommand::Doctor => run_doctor(config, sources),
        DbSubcommand::IpedsImport {
            dir,
            institutions,
            completions,
            year,
        } => run_ipeds_import(config, dir.as_deref(), institutions, completions, year),
        DbSubcommand::Import {
            files,
            variant,
            unitid,
            institution,
            cip,
            catalog,
            degree_id,
            force,
            replace,
            skip_existing,
            dry_run,
            jobs,
        } => {
            let opts = ImportOptions {
                variant: variant.unwrap_or_else(|| "full".to_string()),
                unitid,
                institution,
                cip_code: cip,
                catalog_year: catalog,
                degree_id,
                force,
                replace,
                skip_existing,
                dry_run,
            };
            run_import(config, &files, &opts, jobs);
        }
    }
}

// ============================================================================
// Shared helpers
// ============================================================================

/// Build a single-threaded Tokio runtime, printing an error and returning `None` on failure.
pub(super) fn make_runtime() -> Option<tokio::runtime::Runtime> {
    match tokio::runtime::Runtime::new() {
        Ok(rt) => Some(rt),
        Err(e) => {
            eprintln!("✗ Failed to create async runtime: {e}");
            None
        }
    }
}

// ============================================================================
// Validate — compare stored data against the survey file it came from
// ============================================================================

/// Compare a local IPEDS file against the backend and print the disagreements.
fn run_validate(config: &Config, file: &std::path::Path, year: u16) {
    let Some(rt) = make_runtime() else { return };
    // The exit happens here rather than inside the async block so the runtime shuts
    // down normally.
    let code = match rt.block_on(do_validate(config, file, year)) {
        Ok(verdict) => verdict.exit_code(),
        Err(e) => {
            eprintln!("✗ {e}");
            1
        }
    };
    drop(rt);
    if code != 0 {
        std::process::exit(code);
    }
}

/// Read the file, fetch the backend, and report both checks.
///
/// # Errors
/// Returns a message when the file cannot be read or the backend cannot be reached.
/// A *disagreement* is not an error — it is the result, and it sets the exit code.
async fn do_validate(
    config: &Config,
    file: &std::path::Path,
    year: u16,
) -> Result<validate::Verdict, String> {
    let client = DbClient::from_config(&config.database)
        .await
        .map_err(|e| format!("{e}"))?;

    let kind = validate::survey_kind_of(file).map_err(|e| format!("{e}"))?;
    println!("File:     {}", file.display());
    println!("Survey:   {}", kind.label());
    println!("Backend:  {}", config.database.endpoint);
    println!();

    if kind == validate::SurveyKind::Completions {
        return validate_completions(&client, file, year).await;
    }

    let from_file = validate::read_institutions(file, year).map_err(|e| format!("{e}"))?;
    let from_db = validate::fetch_institutions(&client)
        .await
        .map_err(|e| format!("{e}"))?;

    let report = validate::diff_institutions(&from_file, &from_db);
    print_coverage(&report.coverage);
    println!();
    print_columns(&report);

    let provenance = validate::read_carnegie_candidates(file).map_err(|e| format!("{e}"))?;
    let wrong_source = report_provenance(&from_db, &provenance);

    let verdict = validate::institutions_verdict(&report, wrong_source);
    announce(&verdict, file);
    Ok(verdict)
}

/// Compare a completions file: exact row count, then values over a bounded sample.
async fn validate_completions(
    client: &DbClient,
    file: &std::path::Path,
    year: u16,
) -> Result<validate::Verdict, String> {
    let (rows_in_file, dropped_from_file, from_file) =
        validate::read_completions(file, year).map_err(|e| format!("{e}"))?;
    let unitids: std::collections::BTreeSet<i32> = from_file.keys().map(|k| k.unitid).collect();

    // `QueryFilters::in_list` drops an empty list, so an empty sample would fetch the
    // whole year unfiltered and then report it as rows the file does not contain.
    if unitids.is_empty() {
        return Err(format!(
            "no rows in {} could be keyed — the file is missing UNITID, CIPCODE, AWLEVEL \
             or MAJORNUM, so there is nothing to compare",
            file.display()
        ));
    }

    let year_filter = nu_analytics::database::QueryFilters::new().eq("year", Some(year));
    let rows_in_db = client
        .count_rows_filtered(nu_analytics::database::tables::COMPLETIONS, &year_filter)
        .await
        .map_err(|e| format!("{e}"))?;
    let (dropped_from_db, from_db) = validate::fetch_completions(client, year, &unitids)
        .await
        .map_err(|e| format!("{e}"))?;

    let report = validate::diff_completions(
        (rows_in_file, rows_in_db),
        unitids.len(),
        (dropped_from_file, dropped_from_db),
        &from_file,
        &from_db,
    );

    println!("Rows for {year}");
    let marker = if report.counts_agree() { "✓" } else { "✗" };
    println!("  {marker} in file    {}", report.rows_in_file);
    println!("  {marker} in backend {}", report.rows_in_db);
    println!();
    println!(
        "Values  (sample: {} institutions, {} rows — the count above covers every row)",
        report.sampled_institutions, report.coverage.in_both
    );
    if report.coverage.missing_from_db > 0 {
        println!(
            "  ✗ {} sampled row(s) absent from the backend",
            report.coverage.missing_from_db
        );
    }
    if report.coverage.absent_from_file > 0 {
        println!(
            "  ✗ {} stored row(s) for sampled institutions are not in this file",
            report.coverage.absent_from_file
        );
    }
    if report.dropped_from_file > 0 || report.dropped_from_db > 0 {
        println!(
            "  ! {} file row(s) and {} stored row(s) could not be keyed — not checked",
            report.dropped_from_file, report.dropped_from_db
        );
    }
    for column in &report.columns {
        if column.mismatched == 0 {
            continue;
        }
        println!(
            "  ✗ {:<24} {} of {} differ",
            column.column, column.mismatched, column.compared
        );
        for m in &column.examples {
            println!("      {}: file={} db={}", m.key, m.in_file, m.in_db);
        }
    }
    if report.failing_columns() == 0 && report.coverage.in_both > 0 {
        println!(
            "  ✓ all {} columns match across the sample",
            report.columns.len()
        );
    }

    let verdict = validate::completions_verdict(&report);
    announce(&verdict, file);
    Ok(verdict)
}

/// Print the verdict. The exit code is set by the caller, not here.
fn announce(verdict: &validate::Verdict, file: &std::path::Path) {
    println!();
    match verdict {
        validate::Verdict::Clean => println!("✓ backend matches {}", file.display()),
        validate::Verdict::Inconclusive(reasons) => {
            println!("✗ inconclusive — nothing was actually checked:");
            for reason in reasons {
                println!("  - {reason}");
            }
        }
        validate::Verdict::Failed(reasons) => {
            println!("✗ mismatch:");
            for reason in reasons {
                println!("  - {reason}");
            }
        }
    }
}

/// Print how many rows each side had./// Print how many rows each side had.
fn print_coverage(coverage: &validate::Coverage) {
    println!("Rows");
    println!("  compared            {}", coverage.in_both);
    let marker = if coverage.missing_from_db == 0 {
        "✓"
    } else {
        "✗"
    };
    println!(
        "  {marker} in file, not stored {}",
        coverage.missing_from_db
    );
    // Not a fault: `institutions` accumulates across survey years, so rows from earlier
    // years are expected to outlive the file being checked.
    println!(
        "  · stored, not in file {}  (institutions accumulate across survey years — \
         not a fault on its own)",
        coverage.absent_from_file
    );
}

/// Print the per-column comparison, worst first.
fn print_columns(report: &validate::FidelityReport) {
    println!("Columns");
    for column in &report.columns {
        if column.mismatched == 0 {
            println!("  ✓ {:<16} {} compared", column.column, column.compared);
            continue;
        }
        println!(
            "  ✗ {:<16} {} of {} differ ({:.1}%)",
            column.column,
            column.mismatched,
            column.compared,
            column.mismatch_rate()
        );
        for m in &column.examples {
            println!("      unitid {}: file={} db={}", m.key, m.in_file, m.in_db);
        }
    }
}

/// Print which source column the stored data agrees with. Returns `true` if it is the
/// wrong one.
fn report_provenance(
    from_db: &std::collections::BTreeMap<i32, nu_analytics::database::Institution>,
    candidates: &[validate::CandidateValues],
) -> bool {
    if candidates.len() < 2 {
        // One vintage in the file means nothing to confuse it with — but say so, or the
        // reader cannot tell the check from a skipped one.
        println!();
        println!("Provenance  · one Carnegie vintage in this file — nothing to attribute");
        return false;
    }
    let stored: std::collections::BTreeMap<i32, Option<i32>> = from_db
        .iter()
        .map(|(unitid, inst)| (*unitid, inst.carnegie_class))
        .collect();
    let report = validate::diff_provenance("carnegie_class", &stored, candidates);

    println!();
    println!(
        "Provenance  (carnegie_class — this file carries {} vintages)",
        candidates.len()
    );
    for c in &report.candidates {
        let marker = if c.is_exact() { "✓" } else { " " };
        println!(
            "  {marker} {:<10} {} of {} rows agree",
            c.source_column, c.agreements, c.compared
        );
    }
    match (report.sole_exact_match(), report.expected.as_deref()) {
        (Some(actual), Some(expected)) if actual.source_column != expected => {
            println!(
                "  ✗ stored data came from {}, but the importer reads {expected} first",
                actual.source_column
            );
            println!("    Re-import this year to correct it.");
            true
        }
        (Some(actual), _) => {
            println!(
                "  ✓ stored data came from {}, as intended",
                actual.source_column
            );
            false
        }
        (None, _) => {
            // Either several candidates are identical in this file, or none matches
            // exactly — often because a *different* discrepancy is also in play. Neither
            // establishes a wrong source, so neither is reported as one. But if one
            // candidate clearly leads the expected column, say so: that is an
            // observation, and withholding it would hide the only evidence available.
            println!("  · no single column matches exactly — cannot attribute a source");
            if let Some((best, expected)) = report.best_beats_expected() {
                println!(
                    "  ! {} agrees with {} rows against {}'s {} — the stored data looks",
                    best.source_column,
                    best.agreements,
                    expected.source_column,
                    expected.agreements
                );
                println!(
                    "    like it came from {}. Re-import to settle it.",
                    best.source_column
                );
            }
            false
        }
    }
}

// ============================================================================
// Bootstrap — apply or emit the schema and seed files
// ============================================================================

/// Apply the bootstrap files, or write them to stdout with `--print`.
fn run_bootstrap(config: &Config, print: bool) {
    if print {
        // Deliberately unconditional: this path touches no backend, so it must work with
        // no config, no session, and no network — which is exactly the situation someone
        // setting a stack up for the first time is in.
        print!("{}", bootstrap::concatenated());
        return;
    }

    let alternative = "`nuanalytics db bootstrap --print | psql \"$DATABASE_URL\"`";
    let Some((management_key, project_ref)) =
        management_api_credentials(config, "db bootstrap", alternative)
    else {
        std::process::exit(1);
    };

    let Some(rt) = make_runtime() else { return };

    println!(
        "Applying {} files to project {project_ref}",
        bootstrap::SCHEMA_FILES.len()
    );
    for (index, file) in bootstrap::SCHEMA_FILES.iter().enumerate() {
        let step = index + 1;
        let total = bootstrap::SCHEMA_FILES.len();
        println!("  [{step}/{total}] {} — {}", file.path, file.purpose);

        if let Err(e) = rt.block_on(do_exec_sql(&management_key, &project_ref, file.sql)) {
            eprintln!("✗ {} failed: {e}", file.path);
            // Stops rather than continuing: every later file depends on an earlier one,
            // so carrying on would bury this error under a cascade of missing-relation
            // failures that all share this cause.
            eprintln!(
                "  Stopped at step {step} of {total}. Fix this, then re-run — the files are \
                 idempotent, so the steps already applied will not be repeated."
            );
            std::process::exit(1);
        }
    }
    println!("✓ Schema and seed data applied");
    println!("  Verify with: nuanalytics db doctor");
}

// ============================================================================
// Login — OAuth PKCE flow and password grant
// ============================================================================

/// OAuth provider used when neither `--provider` nor `--email` is given.
const DEFAULT_OAUTH_PROVIDER: &str = "github";

/// Validate database config, build an async runtime, and run the requested login flow.
///
/// `provider` and `email` are mutually exclusive at the arg parser, so at most one is
/// `Some`; `email` selects the password grant and anything else selects OAuth.
fn run_login(config: &Config, provider: Option<&str>, email: Option<&str>) {
    if config.database.endpoint.is_empty() || config.database.anon_key.is_empty() {
        // Uses the shared steps rather than its own copy, which omitted the caveat that
        // `config set` writes to the home config while a project-local nuanalytics.toml
        // outranks it — the documented trap.
        eprintln!("✗ {}", DatabaseError::NotConfigured);
        report_db_error(&DatabaseError::NotConfigured, &config.database.endpoint);
        std::process::exit(1);
    }

    let Some(rt) = make_runtime() else { return };

    let outcome = email.map_or_else(
        || {
            rt.block_on(do_oauth_login(
                config,
                provider.unwrap_or(DEFAULT_OAUTH_PROVIDER),
            ))
        },
        |address| rt.block_on(do_password_login(config, address)),
    );

    // Exits non-zero like every other `db` subcommand. Login is the first step of setting
    // a deployment up, so a script that chains `db login && db import` must not carry on
    // against a backend it never signed in to.
    if let Err(e) = outcome {
        eprintln!("✗ Login failed: {e}");
        std::process::exit(1);
    }
}

/// Sign in with an email address and a password read from the terminal.
///
/// The password is prompted for rather than accepted as an argument: an argument would
/// persist in shell history and be visible in `ps` output for the life of the process.
async fn do_password_login(config: &Config, email: &str) -> Result<(), String> {
    println!("Signing in to {} as {email}", config.database.endpoint);
    let password = prompt_password()?;
    if password.is_empty() {
        return Err("no password entered".to_string());
    }

    let state = sign_in_with_password(
        &config.database.endpoint,
        &config.database.anon_key,
        email,
        &password,
    )
    .await
    .map_err(|e| describe_sign_in_error(&e, &config.database.endpoint))?;

    persist_session(config, &state)
}

/// Read a password from the terminal without echoing it.
///
/// # Errors
/// Fails when there is no terminal to prompt on, which is what happens in a pipeline or
/// CI job — the message says so rather than reporting it as a credential problem.
fn prompt_password() -> Result<String, String> {
    rpassword::prompt_password("Password: ").map_err(|e| {
        let mut msg = format!("cannot read a password from this terminal ({e}).");
        msg.push_str(" `--email` needs an interactive terminal; use OAuth for unattended sign-in.");
        msg
    })
}

/// Render a [`SignInError`] with the backend named and a next step that fits the variant.
///
/// Rejections quote `GoTrue`'s own message and add no cause of our own: "Invalid login
/// credentials" and "Email not confirmed" want different things from the user, and only
/// the backend knows which applies.
fn describe_sign_in_error(error: &SignInError, endpoint: &str) -> String {
    match error {
        SignInError::Transport(msg) => {
            format!("{endpoint} could not be reached: {msg}")
        }
        SignInError::Rejected { status, detail } => {
            let mut msg = format!("{endpoint} refused the sign-in: {detail} (HTTP {status})");
            msg.push_str(
                "\n  The account must already exist on the backend — `--email` never creates one.",
            );
            msg.push_str(
                "\n  If the address and password are right, check that email/password sign-in",
            );
            msg.push_str("\n  is enabled on the stack (GoTrue `GOTRUE_EXTERNAL_EMAIL_ENABLED`).");
            msg
        }
        SignInError::Malformed(msg) => format!(
            "{endpoint} answered the sign-in with something this client could not read: {msg}"
        ),
    }
}

/// Write a freshly obtained session to the configured auth file and report where it went.
fn persist_session(config: &Config, state: &AuthState) -> Result<(), String> {
    let auth_path = auth_file_path(&config.database);
    save_auth_state(&auth_path, state)?;

    let email = state.user_email.as_deref().unwrap_or("(no email)");
    println!("✓ Signed in as {email}");
    println!("  Session saved to {}", auth_path.display());
    Ok(())
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
        accept_oauth_callback(listener, &config.database.endpoint),
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

    persist_session(config, &state)
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
async fn accept_oauth_callback(
    listener: tokio::net::TcpListener,
    endpoint: &str,
) -> Result<String, String> {
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
    let failure = CallbackFailure::from_request_line(request);

    let (status, body) = if code.is_some() {
        ("200 OK", CALLBACK_SUCCESS_HTML.to_string())
    } else {
        ("400 Bad Request", failure.error_page(endpoint))
    };

    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await.ok();

    code.ok_or_else(|| failure.terminal_message(endpoint))
}

/// What the auth provider reported on a failed OAuth callback.
///
/// `GoTrue` puts the actionable text in `error_description`; `error` on its own is a coarse
/// class such as `server_error`. Discarding the description used to collapse every
/// failure into one invented cause, which sent two people to audit provider settings for
/// what was actually a malformed `auth.users` row.
#[derive(Debug, Default)]
struct CallbackFailure {
    /// OAuth error class, e.g. `server_error`, `access_denied`.
    error: Option<String>,
    /// Provider's human-readable explanation — the part worth reading.
    description: Option<String>,
    /// Provider-specific code, when sent.
    code: Option<String>,
}

impl CallbackFailure {
    fn from_request_line(request_line: &str) -> Self {
        Self {
            error: extract_query_param(request_line, "error"),
            description: extract_query_param(request_line, "error_description"),
            code: extract_query_param(request_line, "error_code"),
        }
    }

    /// The provider's own words, most specific part first. `None` when it said nothing.
    fn provider_text(&self) -> Option<String> {
        let mut parts: Vec<&str> = Vec::new();
        if let Some(ref d) = self.description {
            parts.push(d);
        }
        if let Some(ref e) = self.error {
            if Some(e) != self.description.as_ref() {
                parts.push(e);
            }
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" — "))
        }
    }

    /// Terminal message: which backend, what failed, what to do. Asserts no cause.
    fn terminal_message(&self, endpoint: &str) -> String {
        let backend = nu_analytics::config::endpoint_label(endpoint);
        let mut out = String::from("OAuth callback returned no authorization code.\n");
        let _ = writeln!(out, "  backend: {backend}");
        // The closing advice differs by branch: calling "nothing was sent" the provider's
        // own message would itself be false, and with an error_code present the line
        // immediately above is the code, not the provider text.
        let provider_text = self.provider_text();
        if let Some(ref text) = provider_text {
            let _ = writeln!(out, "  provider reported: {text}");
        } else {
            out.push_str("  provider reported: nothing — no error or error_description was sent\n");
        }
        if let Some(ref code) = self.code {
            let _ = writeln!(out, "  error_code: {code}");
        }
        if provider_text.is_some() {
            out.push_str(
                "The `provider reported` line is the provider's own message. Read it before \
                 changing provider settings; re-run `nuanalytics db login` to retry.",
            );
        } else {
            out.push_str(
                "The callback carried no error detail. Re-run `nuanalytics db login`; if it \
                 fails again, read the full redirect URL from the browser's address bar.",
            );
        }
        out
    }

    /// Browser page carrying the same detail, so it survives closing the tab.
    fn error_page(&self, endpoint: &str) -> String {
        use nu_analytics::core::report::visualization::escape_html;

        let detail = self.provider_text().map_or_else(
            || "The provider sent no error description.".to_string(),
            |text| format!("Provider reported: {}", escape_html(&text)),
        );
        let code = self.code.as_ref().map_or_else(String::new, |c| {
            format!("<p class=\"meta\">error_code: {}</p>", escape_html(c))
        });
        let backend = escape_html(nu_analytics::config::endpoint_label(endpoint));
        format!(
            r#"<!DOCTYPE html>
<html><head><title>NuAnalytics — Sign In Failed</title>
<style>body{{font-family:sans-serif;display:flex;justify-content:center;align-items:center;height:100vh;margin:0;background:#fff5f5}}
.box{{text-align:left;max-width:40rem;padding:2rem;background:#fff;border-radius:8px;box-shadow:0 2px 12px rgba(0,0,0,.1)}}
h1{{color:#c53030}}p{{color:#555}}
.detail{{background:#f7f7f7;padding:.75rem;border-radius:4px;font-family:monospace;white-space:pre-wrap;word-break:break-word}}
.meta{{color:#777;font-size:.9em}}</style></head>
<body><div class="box"><h1>✗ Sign in failed</h1>
<p class="meta">backend: {backend}</p>
<p class="detail">{detail}</p>
{code}
<p>This is the provider's own message. Return to the terminal, which shows the same
detail, and re-run <code>nuanalytics db login</code> to retry.</p></div></body></html>"#
        )
    }
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

// ============================================================================
// Logout
// ============================================================================

fn run_logout(config: &Config) {
    let path = auth_file_path(&config.database);
    match load_auth_state(&path) {
        Some(state) if state.is_valid() => {
            let email = state.user_email.as_deref().unwrap_or("unknown");
            clear_auth_state(&path);
            println!("✓ Local session cleared ({email})");
            // Stated rather than implied: "Signed out" read as revocation, and the token
            // is still accepted by the backend until it expires.
            println!(
                "  Note: this removed the local token only. It stays valid at {} until it",
                endpoint_or_unset(&config.database)
            );
            println!("  expires; nothing was revoked server-side.");
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
            println!("Signed in as:  {email}");
            println!("Backend:       {}", endpoint_or_unset(&config.database));
            println!("Token expires: {expires}");
            println!("Auth file:     {}", path.display());
        }
        Some(_) => println!(
            "Session expired for {}. Run `nuanalytics db login` to sign in again.",
            endpoint_or_unset(&config.database)
        ),
        None => println!(
            "Not signed in to {}. Run `nuanalytics db login`.",
            endpoint_or_unset(&config.database)
        ),
    }
}

// ============================================================================
// exec-sql
// ============================================================================

/// Suggest a Supabase project ref from a `*.supabase.co` endpoint.
///
/// Used **only** to fill in a hint when `database.project_ref` is unset — never to decide
/// where a request goes. Deriving the ref from the host was the original defect: the first
/// subdomain label gave `Some("db")` for `https://db.example.edu` and
/// `Some("localhost:8000")` for a local stack, and that meaningless ref was then sent to
/// `api.supabase.com`. Host matching is also wrong in the other direction — a real cloud
/// project served through a custom domain has no `.supabase.co` in its URL.
fn suggest_project_ref(endpoint: &str) -> Option<&str> {
    let host = endpoint
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()?;
    // Anchored to the real cloud host, and only when there is exactly one label in front.
    let label = host.strip_suffix(".supabase.co")?;
    if label.is_empty() || label.contains('.') || label.contains(':') {
        None
    } else {
        Some(label)
    }
}

/// Read a SQL file from disk and execute it against the Supabase Management API.
/// Resolve the two settings the Supabase Management API needs, or explain what is missing.
///
/// Returns `(management_key, project_ref)`. The project ref is checked **first and
/// explicitly**: the Management API is a cloud-only path, and a self-hosted user was once
/// told to go create a Supabase Personal Access Token before ever being told the command
/// does not apply to their deployment. `project_ref` is never inferred from the hostname —
/// a cloud project on a custom domain has no `.supabase.co` in its URL, and a self-hosted
/// host would otherwise yield a meaningless ref.
///
/// `command` names the caller and `self_hosted_alternative` is the advice to give when
/// there is no project ref, which differs by caller: applying one file is `psql -f`, while
/// applying the whole bootstrap is a pipe.
fn management_api_credentials(
    config: &Config,
    command: &str,
    self_hosted_alternative: &str,
) -> Option<(String, String)> {
    if config.database.project_ref.is_empty() {
        eprintln!("✗ `{command}` targets the Supabase Management API, which needs");
        eprintln!("  `database.project_ref` to be set explicitly:");
        eprintln!();
        if let Some(suggestion) = suggest_project_ref(&config.database.endpoint) {
            eprintln!("    nuanalytics config set database.project_ref {suggestion}");
            eprintln!(
                "  (suggested from the endpoint {}; confirm it in the Supabase dashboard)",
                config.database.endpoint
            );
        } else {
            eprintln!("    nuanalytics config set database.project_ref <ref>");
            eprintln!(
                "  The endpoint {} is not a Supabase-cloud URL. If this is a",
                endpoint_or_unset(&config.database)
            );
            eprintln!("  self-hosted stack there is no project ref and no Management API —");
            eprintln!("  apply SQL directly instead: {self_hosted_alternative}");
        }
        return None;
    }

    if config.database.management_key.is_empty() {
        eprintln!("✗ `database.management_key` is not set.");
        eprintln!("  1. Go to https://app.supabase.com/account/tokens");
        eprintln!("  2. Create a Personal Access Token");
        eprintln!("  3. Run: nuanalytics config set database.management_key <token>");
        return None;
    }

    Some((
        config.database.management_key.clone(),
        config.database.project_ref.clone(),
    ))
}

fn run_exec_sql(config: &Config, file: &std::path::Path) {
    let alternative = format!("`psql -f {}`", file.display());
    let Some((management_key, project_ref)) =
        management_api_credentials(config, "db exec-sql", &alternative)
    else {
        std::process::exit(1);
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

    match rt.block_on(do_exec_sql(&management_key, &project_ref, &sql)) {
        Ok(msg) => println!("✓ {msg}"),
        Err(e) => {
            eprintln!("✗ SQL execution failed: {e}");
            std::process::exit(1);
        }
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

fn run_status(config: &Config, sources: &ConfigSources) {
    print_config_line("endpoint", &config.database.endpoint);
    // Which tier supplied the endpoint. Without this, a stale home config is
    // indistinguishable from a correct one — and the home file differs by build profile
    // (`config.toml` for release, `dconfig.toml` for debug), so the same machine can
    // report two different backends depending on which binary is run.
    //
    // Passed in from `main` rather than re-loaded: a second `load_with_sources()` would
    // re-run the loader's create-and-save side effects, and would not see the
    // `--db-endpoint` override that `main` applied — so it could name a config file that
    // did not supply the value being printed on the line above.
    for line in sources.describe() {
        println!("  {line}");
    }
    print_config_line(
        "anon key",
        if config.database.anon_key.is_empty() {
            ""
        } else {
            "set"
        },
    );

    let auth_path = auth_file_path(&config.database);
    let had_auth_file = load_auth_state(&auth_path).is_some();
    if !had_auth_file {
        println!("auth file:     {} ✗ (missing)", auth_path.display());
    }

    let Some(rt) = make_runtime() else { return };

    let probe = rt.block_on(async {
        let client = DbClient::from_config(&config.database).await?;
        client.ping().await
    });

    // Rendered after the ping, not before: the ping refreshes and rewrites the session,
    // so reading `expires_at` first reported a long-expired token immediately above a
    // successful authenticated read.
    if had_auth_file {
        match load_auth_state(&auth_path) {
            Some(state) => println!(
                "auth file:     {} ✓ ({})",
                auth_path.display(),
                format_session_descriptor(&state)
            ),
            None => println!(
                "auth file:     {} ✗ (removed during probe)",
                auth_path.display()
            ),
        }
    }

    match probe {
        Ok(()) => println!("ping:          ✓ authenticated read succeeded"),
        Err(e) => {
            eprintln!("ping:          ✗ {e}");
            report_db_error(&e, &config.database.endpoint);
            std::process::exit(1);
        }
    }
}

/// Run `db doctor`: print a per-check deployment report and exit 1 on a hard failure.
///
/// Presentation only — the checks and their ordering live in `core::database::doctor` so
/// they can be tested without a terminal.
fn run_doctor(config: &Config, sources: &ConfigSources) {
    println!("Backend:  {}", endpoint_or_unset(&config.database));
    for line in sources.describe() {
        println!("  {line}");
    }
    println!();

    let Some(rt) = make_runtime() else { return };
    let report = rt.block_on(doctor::diagnose(&config.database));

    for check in &report.checks {
        println!(
            "{} {:<20} {}",
            check.outcome.marker(),
            check.name,
            check.outcome.detail()
        );
    }

    let (pass, warn, fail, skipped) = report.tally();
    println!();
    println!("{pass} passed, {warn} warning(s), {fail} failed, {skipped} skipped");

    if report.has_failures() {
        std::process::exit(1);
    }
}

/// Print a database error's remediation to stderr.
///
/// Every caller routes through here so the wording comes from
/// [`DatabaseError::next_steps`] rather than being guessed per call site. Three sites
/// previously printed "Configure the database and run `nuanalytics db login` first" for
/// *any* failure, which tells a user with an unreachable backend to log in.
pub fn report_db_error(error: &DatabaseError, endpoint: &str) {
    for (i, line) in error.next_steps(endpoint).into_iter().enumerate() {
        if i == 0 {
            eprintln!("→ {line}");
        } else {
            eprintln!("  {line}");
        }
    }
}

/// The configured endpoint, or an explicit marker when blank.
///
/// Printed by every command that reports state: with cloud and self-hosted both
/// supported, "which backend is this?" is the first question in any support exchange.
fn endpoint_or_unset(db: &nu_analytics::config::DatabaseConfig) -> &str {
    db.endpoint_label()
}

/// Print a config field as `label:    value ✓/✗`. Empty values become `(unset)` and a ✗.
fn print_config_line(label: &str, value: &str) {
    if value.is_empty() {
        println!("{label:14} (unset) ✗");
    } else {
        println!("{label:14} {value} ✓");
    }
}

/// Format an auth state into the human-readable expiry/email blurb shown
/// next to the auth-file path in `db status` output.
fn format_session_descriptor(state: &AuthState) -> String {
    let email = state.user_email.as_deref().unwrap_or("(no email)");
    let remaining = state.expires_at - chrono::Utc::now().timestamp();
    if remaining > 60 {
        format!("expires in {} min as {email}", remaining / 60)
    } else if remaining > 0 {
        format!("expires in {remaining}s as {email}")
    } else {
        format!("expired {} min ago as {email}", -remaining / 60)
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
    let Some(rt) = make_runtime() else { return };

    let client = match rt.block_on(DbClient::from_config(&config.database)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ Database not available: {e}");
            report_db_error(&e, &config.database.endpoint);
            return;
        }
    };

    let (inst_path, comp_path) =
        resolve_ipeds_paths(dir, institutions_path, completions_path, year);

    let mut institutions_failed = false;
    let mut completions_ok = false;
    let mut failures: Vec<String> = Vec::new();

    if let Some(path) = inst_path {
        println!("Importing institutions from {} ...", path.display());
        match rt.block_on(ipeds::ingest_institutions(&client, &path, year)) {
            Ok(stats) => println!(
                "  ✓ {} read, {} upserted, {} skipped",
                stats.rows_read, stats.rows_upserted, stats.rows_skipped
            ),
            Err(e) => {
                eprintln!("  ✗ Institutions import failed: {e}");
                institutions_failed = true;
                failures.push(format!("institutions ({}): {e}", path.display()));
            }
        }
    } else {
        println!("  ℹ Skipping institutions (no file provided or found)");
    }

    if let Some(path) = comp_path {
        println!("Importing completions from {} ...", path.display());
        println!("  (all CIP codes stored; query with CIP filter for CS vs all-programs)");
        match rt.block_on(ipeds::ingest_completions(&client, &path, year)) {
            Ok(stats) => {
                println!(
                    "  ✓ {} rows read, {} with a usable UNITID, {} upserted, {} skipped",
                    stats.rows_read, stats.rows_filtered, stats.rows_upserted, stats.rows_skipped
                );
                completions_ok = true;
            }
            Err(e) => {
                eprintln!("  ✗ Completions import failed: {e}");
                failures.push(format!("completions ({}): {e}", path.display()));
            }
        }
    } else {
        println!("  ℹ Skipping completions (no file provided or found)");
    }

    if failures.is_empty() {
        return;
    }

    // Exits non-zero so a batch loop over several years stops reporting success on a
    // half-finished import — the original symptom was 494 completions rows referencing
    // unitids that never made it into `institutions`.
    eprintln!();
    eprintln!("✗ IPEDS import for {year} did not complete:");
    for failure in &failures {
        eprintln!("  - {failure}");
    }
    if institutions_failed && completions_ok {
        eprintln!(
            "  Completions were written while institutions were not, so some rows now \
             reference unitids that are absent from `institutions`. Re-run the \
             institutions import for {year} before relying on joins."
        );
    }
    std::process::exit(1);
}

// ============================================================================
// Degree import
// ============================================================================

/// Run `db import`: load degree report(s) into the normalized program tables.
///
/// Expands any directory arguments to their report files, connects to the
/// database (printing login guidance if unauthenticated), then imports each
/// file. A single file prints a detailed outcome and exits non-zero on a
/// blocked result; a batch isolates per-file failures to `import_failures.log`
/// and prints a final summary.
fn run_import(config: &Config, files: &[std::path::PathBuf], opts: &ImportOptions, jobs: usize) {
    // `--jobs` is accepted for forward-compatibility; v1 always runs
    // sequentially (no worker pool yet — the heavy lifting is one network
    // round-trip per file, so a pool buys little for the common batch sizes).
    let _ = jobs;

    let inputs = collect_import_inputs(files);
    if inputs.is_empty() {
        eprintln!("✗ No degree report files to import.");
        eprintln!("  Pass one or more *.json report files, or a directory containing them.");
        std::process::exit(1);
    }

    let Some(rt) = make_runtime() else { return };

    let client = match rt.block_on(DbClient::from_config(&config.database)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("✗ Database not available: {e}");
            report_db_error(&e, &config.database.endpoint);
            std::process::exit(1);
        }
    };

    let dry = if opts.dry_run { " (dry-run)" } else { "" };

    if inputs.len() == 1 {
        run_import_single(&rt, &client, &inputs[0], opts, dry);
    } else {
        run_import_batch(&rt, &client, &inputs, opts, dry);
    }
}

/// Import a single file with full diagnostics; exit non-zero when the import is
/// blocked (rejected / ambiguous / needs-confirmation) or a transport error
/// occurs.
fn run_import_single(
    rt: &tokio::runtime::Runtime,
    client: &DbClient,
    path: &std::path::Path,
    opts: &ImportOptions,
    dry: &str,
) {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("✗ Cannot read {}: {e}", path.display());
            std::process::exit(1);
        }
    };

    match rt.block_on(execute_import(client, &text, opts)) {
        Ok(outcome) => {
            print_import_outcome(path, &outcome, dry);
            if is_blocked(&outcome.result) {
                std::process::exit(1);
            }
        }
        Err(e) => {
            eprintln!("✗ Import failed for {}: {e}", path.display());
            std::process::exit(1);
        }
    }
}

/// Running tally of batch outcomes by class.
#[derive(Default)]
struct ImportTally {
    created: usize,
    updated: usize,
    skipped: usize,
    needs_confirmation: usize,
    ambiguous: usize,
    rejected: usize,
    errors: usize,
}

/// Import many files with per-file isolation: a read/parse/transport error on
/// one file is logged and the run continues. Failures are written to
/// `import_failures.log`; a one-line progress is printed per file plus a final
/// summary.
fn run_import_batch(
    rt: &tokio::runtime::Runtime,
    client: &DbClient,
    inputs: &[std::path::PathBuf],
    opts: &ImportOptions,
    dry: &str,
) {
    let total = inputs.len();
    println!("Importing {total} report(s){dry}…");

    let mut tally = ImportTally::default();
    let mut failures: Vec<(std::path::PathBuf, String)> = Vec::new();

    for (idx, path) in inputs.iter().enumerate() {
        let label = format!("[{}/{total}] {}", idx + 1, path.display());
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) => {
                tally.errors += 1;
                failures.push((path.clone(), format!("read error: {e}")));
                println!("  ✗ {label}: read error: {e}");
                continue;
            }
        };
        match rt.block_on(execute_import(client, &text, opts)) {
            Ok(outcome) => {
                tally_outcome(&mut tally, &outcome.result);
                println!(
                    "  {} {label}: {}",
                    outcome_marker(&outcome.result),
                    result_label(&outcome.result)
                );
            }
            Err(e) => {
                tally.errors += 1;
                failures.push((path.clone(), e.to_string()));
                println!("  ✗ {label}: {e}");
            }
        }
    }

    print_import_summary(&tally, total, dry);
    write_import_failures(&failures);
}

/// Expand `files` into a flat list of report files. A directory is expanded to
/// its `*_report.json` files, falling back to `*.json` when none match. A plain
/// file path is taken verbatim. Missing paths are warned about and skipped.
fn collect_import_inputs(files: &[std::path::PathBuf]) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    for path in files {
        if path.is_dir() {
            let expanded = collect_dir_reports(path);
            if expanded.is_empty() {
                eprintln!("⚠ No *.json report files found in {}", path.display());
            }
            out.extend(expanded);
        } else if path.exists() {
            out.push(path.clone());
        } else {
            eprintln!("⚠ Skipping missing path: {}", path.display());
        }
    }
    out
}

/// List the report files directly under `dir`: `*_report.json` if any exist,
/// otherwise every `*.json`. Results are sorted for a deterministic order.
fn collect_dir_reports(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let json: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .is_some_and(|e| e.eq_ignore_ascii_case("json"))
        })
        .collect();
    let mut reports: Vec<std::path::PathBuf> = json
        .iter()
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_report.json"))
        })
        .cloned()
        .collect();
    if reports.is_empty() {
        reports = json;
    }
    reports.sort();
    reports
}

/// Whether a result requires the user to act before the import can proceed.
const fn is_blocked(result: &ImportResult) -> bool {
    matches!(
        result,
        ImportResult::NeedsConfirmation(_)
            | ImportResult::InstitutionAmbiguous(_)
            | ImportResult::Rejected(_)
    )
}

/// The `✓`/`⚠`/`✗` status marker for a result class.
const fn outcome_marker(result: &ImportResult) -> &'static str {
    match result {
        ImportResult::Created | ImportResult::Updated => "✓",
        ImportResult::Skipped => "ℹ",
        ImportResult::NeedsConfirmation(_) | ImportResult::InstitutionAmbiguous(_) => "⚠",
        ImportResult::Rejected(_) => "✗",
    }
}

/// A short human label for a result class.
const fn result_label(result: &ImportResult) -> &'static str {
    match result {
        ImportResult::Created => "created",
        ImportResult::Updated => "updated",
        ImportResult::Skipped => "skipped",
        ImportResult::NeedsConfirmation(_) => "needs confirmation",
        ImportResult::InstitutionAmbiguous(_) => "institution ambiguous",
        ImportResult::Rejected(_) => "rejected",
    }
}

/// Increment the batch tally for one result.
const fn tally_outcome(tally: &mut ImportTally, result: &ImportResult) {
    match result {
        ImportResult::Created => tally.created += 1,
        ImportResult::Updated => tally.updated += 1,
        ImportResult::Skipped => tally.skipped += 1,
        ImportResult::NeedsConfirmation(_) => tally.needs_confirmation += 1,
        ImportResult::InstitutionAmbiguous(_) => tally.ambiguous += 1,
        ImportResult::Rejected(_) => tally.rejected += 1,
    }
}

/// Print the detailed outcome of a single import (result, resolved unit id,
/// row counts, conversion warnings, messages, and guidance when blocked).
fn print_import_outcome(path: &std::path::Path, outcome: &ImportOutcome, dry: &str) {
    println!(
        "{} {}: {}{dry}",
        outcome_marker(&outcome.result),
        path.display(),
        result_label(&outcome.result)
    );
    let institution = match (&outcome.institution, outcome.resolved_unitid) {
        (Some(name), Some(u)) => format!("{name} (unitid {u})"),
        (Some(name), None) => format!("{name} (unresolved)"),
        (None, Some(u)) => format!("(unitid {u})"),
        (None, None) => "(unresolved)".to_string(),
    };
    println!("  institution:    {institution}");
    println!("  program_key:    {}", outcome.program_key);
    println!("  variant:        {}", outcome.variant);
    // On the ambiguous path nothing is built (resolution stops early), so the
    // analysis/row counts would be meaningless — skip them; the candidate list
    // printed below is the actionable part.
    if !matches!(outcome.result, ImportResult::InstitutionAmbiguous(_)) {
        if outcome.run_written {
            match (outcome.variations_run, outcome.sample_type.as_deref()) {
                (Some(v), Some(s)) => println!(
                    "  analysis:       {v} variations ({s}), {} sample plans",
                    outcome.plans_written
                ),
                (Some(v), None) => {
                    println!(
                        "  analysis:       {v} variations, {} sample plans",
                        outcome.plans_written
                    );
                }
                _ => println!("  analysis:       {} sample plans", outcome.plans_written),
            }
        } else {
            println!("  analysis:       none (no metrics in upload)");
        }
        println!(
            "  rows:           {} courses, {} requirements, {} course-metrics",
            outcome.courses_written, outcome.requirements_written, outcome.course_metrics_written
        );
    }

    if !outcome.conversion_warnings.is_empty() {
        println!("  conversion warnings:");
        for w in &outcome.conversion_warnings {
            println!("    • {w}");
        }
    }
    if !outcome.messages.is_empty() {
        println!("  messages:");
        for m in &outcome.messages {
            println!("    • {m}");
        }
    }

    print_blocked_guidance(&outcome.result);
}

/// Print actionable guidance for a blocked result (rejected / ambiguous /
/// needs-confirmation); no-op for create/update/skip.
fn print_blocked_guidance(result: &ImportResult) {
    match result {
        ImportResult::Rejected(errors) => {
            eprintln!("  ✗ report rejected:");
            for e in errors {
                eprintln!("    • {e}");
            }
        }
        ImportResult::InstitutionAmbiguous(candidates) => {
            eprintln!(
                "  ⚠ institution name matched multiple institutions — re-run with --unitid <N>:"
            );
            for (unitid, name) in candidates {
                eprintln!("    • {unitid}  {name}");
            }
        }
        ImportResult::NeedsConfirmation(reason) => {
            eprintln!("  ⚠ {reason}");
            eprintln!("    re-run with --replace (unverified) or --force (verified) to overwrite");
        }
        ImportResult::Created | ImportResult::Updated | ImportResult::Skipped => {}
    }
}

/// Print the final batch summary line(s).
fn print_import_summary(tally: &ImportTally, total: usize, dry: &str) {
    println!(
        "✓ imported {created} created, {updated} updated, {skipped} skipped of {total}{dry}",
        created = tally.created,
        updated = tally.updated,
        skipped = tally.skipped,
    );
    let attention = tally.needs_confirmation + tally.ambiguous + tally.rejected + tally.errors;
    if attention > 0 {
        println!(
            "  ⚠ {} needs-confirmation, {} ambiguous, {} rejected, {} error(s)",
            tally.needs_confirmation, tally.ambiguous, tally.rejected, tally.errors
        );
    }
}

/// Write batch failures to `import_failures.log` in the current directory
/// (one `path<TAB>reason` line each). No-op when there are no failures.
fn write_import_failures(failures: &[(std::path::PathBuf, String)]) {
    use std::fmt::Write as _;

    if failures.is_empty() {
        return;
    }
    let mut body = String::new();
    for (path, reason) in failures {
        let _ = writeln!(body, "{}\t{reason}", path.display());
    }
    let log = std::path::Path::new("import_failures.log");
    if std::fs::write(log, body).is_ok() {
        println!("  failures listed in {}", log.display());
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
    // Matching is case-insensitive: IPEDS ships `hd2022.csv` and `hd2025.csv` lowercase
    // but `HD2023.csv` and `HD2024.csv` uppercase, so an exact-case lookup silently
    // skipped whole years and the import reported "no file provided or found".
    let entries = directory_names(dir);

    for candidate in candidates {
        // Fast path for an exact, correctly-cased name.
        let direct = dir.join(candidate);
        if direct.exists() {
            return Some(direct);
        }

        let wanted = candidate.to_lowercase();
        let matched = if let Some((prefix, suffix)) = wanted.split_once('*') {
            find_name(&entries, |name| {
                name.starts_with(prefix) && name.ends_with(suffix)
            })
        } else {
            find_name(&entries, |name| name == wanted)
        };
        if let Some(name) = matched {
            return Some(dir.join(name));
        }
    }
    None
}

/// File names in `dir`, sorted, for deterministic matching.
///
/// Sorted because `read_dir` order is arbitrary: with both `HD2023.csv` and `hd2023.csv`
/// present, an unsorted scan picks a different file from run to run.
fn directory_names(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort_unstable();
    names
}

/// First name whose lower-cased form satisfies `matches`.
fn find_name(names: &[String], matches: impl Fn(&str) -> bool) -> Option<&String> {
    names.iter().find(|name| matches(&name.to_lowercase()))
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

    // --- bootstrap ----------------------------------------------------------

    #[test]
    fn bootstrap_print_needs_nothing_configured() {
        // The whole point of --print is that it works before any of this is set up:
        // no endpoint, no session, no network. Anything that reads config here would
        // defeat it, so this pins that the output depends only on the embedded files.
        let sql = bootstrap::concatenated();
        assert!(sql.starts_with("--"), "must open with a SQL comment");
        for file in &bootstrap::SCHEMA_FILES {
            assert!(
                sql.contains(file.sql),
                "{} must be emitted verbatim, not summarised",
                file.path
            );
        }
    }

    #[test]
    fn bootstrap_emits_the_files_in_dependency_order() {
        // Out of order, a seed file hits a table that does not exist yet. This asserts
        // the emitted stream, not just the constant, because the stream is what a user
        // pipes into psql.
        let sql = bootstrap::concatenated();
        let at = |needle: &str| sql.find(needle).expect("file is present");
        assert!(at("docs/database/schema.sql") < at("docs/database/cip-seed.sql"));
        assert!(
            at("docs/database/programs-schema.sql") < at("docs/database/program-lookup-seed.sql")
        );
    }

    // --- describe_sign_in_error ---------------------------------------------

    #[test]
    fn describe_sign_in_error_quotes_the_backend_and_invents_no_cause() {
        // The rule this whole workstream enforces: report what the backend said, name the
        // backend, and do not assert a cause the code has not established.
        let rendered = describe_sign_in_error(
            &SignInError::Rejected {
                status: 400,
                detail: "Email not confirmed".to_string(),
            },
            "https://db.example.edu",
        );
        assert!(rendered.contains("https://db.example.edu"), "{rendered}");
        assert!(rendered.contains("Email not confirmed"), "{rendered}");
        assert!(
            !rendered.to_lowercase().contains("wrong password"),
            "must not narrow a refusal to a cause GoTrue did not state: {rendered}"
        );
    }

    #[test]
    fn describe_sign_in_error_says_unreachable_rather_than_refused() {
        // A host that is down must not be reported as a credential problem — the same
        // confusion `RefreshError` was split up to prevent.
        let rendered = describe_sign_in_error(
            &SignInError::Transport("connection refused".to_string()),
            "https://db.example.edu",
        );
        assert!(
            rendered.contains("could not be reached"),
            "expected an unreachability message, got {rendered}"
        );
        assert!(
            !rendered.contains("refused the sign-in"),
            "a transport failure is not a refusal: {rendered}"
        );
    }

    #[test]
    fn describe_sign_in_error_names_the_endpoint_for_every_variant() {
        // Whatever went wrong, the message has to say which backend it was talking to;
        // the MCP server's failures were unusable before this became the rule.
        let variants = [
            SignInError::Transport("down".to_string()),
            SignInError::Rejected {
                status: 403,
                detail: "nope".to_string(),
            },
            SignInError::Malformed("bad json".to_string()),
        ];
        for variant in &variants {
            let rendered = describe_sign_in_error(variant, "https://db.example.edu");
            assert!(
                rendered.contains("https://db.example.edu"),
                "{variant:?} rendered without the endpoint: {rendered}"
            );
        }
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

    // --- suggest_project_ref -----------------------------------------------

    #[test]
    fn test_suggest_project_ref_only_fires_for_supabase_cloud_hosts() {
        // A suggestion, never a decision: deriving the ref from the host is what sent
        // `db` and `localhost:8000` to api.supabase.com as project refs.
        assert_eq!(
            suggest_project_ref("https://abcdefgh.supabase.co"),
            Some("abcdefgh")
        );
        assert_eq!(
            suggest_project_ref("https://abcdefgh.supabase.co/"),
            Some("abcdefgh"),
            "a trailing slash must not defeat the suggestion"
        );

        // Every one of these previously produced a bogus ref.
        for endpoint in [
            "https://db.example.edu",
            "http://localhost:8000",
            "https://nu.lionelle.com",
            "https://api.db.example.com",
            "https://.supabase.co",
            "https://",
            "",
        ] {
            assert_eq!(
                suggest_project_ref(endpoint),
                None,
                "{endpoint} is not a Supabase-cloud project URL"
            );
        }
    }

    #[test]
    fn test_suggest_project_ref_rejects_a_nested_supabase_subdomain() {
        // `a.b.supabase.co` has no single project label, so there is nothing to suggest.
        assert_eq!(suggest_project_ref("https://a.b.supabase.co"), None);
    }

    // ---- OAuth callback failure reporting -----------------------------------

    /// The exact callback that cost hours during the self-hosted cutover: `GoTrue` put a
    /// malformed-`auth.users`-row error in `error_description`, and it was discarded.
    const REAL_FAILURE: &str = "GET /callback?error=server_error&error_description=sql%3A+Scan+error+on+column+index+3%2C+name+%22confirmation_token%22%3A+converting+NULL+to+string+is+unsupported HTTP/1.1";

    #[test]
    fn test_callback_failure_keeps_the_provider_description() {
        let f = CallbackFailure::from_request_line(REAL_FAILURE);
        assert_eq!(f.error.as_deref(), Some("server_error"));
        let description = f.description.as_deref().expect("description is parsed");
        assert!(
            description.contains("confirmation_token"),
            "the provider's actual explanation must survive parsing, got: {description}"
        );
        assert!(
            description.contains("converting NULL to string is unsupported"),
            "got: {description}"
        );
    }

    #[test]
    fn test_terminal_message_names_backend_and_quotes_the_provider() {
        let f = CallbackFailure::from_request_line(REAL_FAILURE);
        let msg = f.terminal_message("https://nu.example.com");

        // (a) which backend
        assert!(
            msg.contains("https://nu.example.com"),
            "message must name the backend: {msg}"
        );
        // (b) what actually failed — the provider's own words, not a summary
        assert!(
            msg.contains("confirmation_token"),
            "message must carry the provider's explanation: {msg}"
        );
        // (c) what to do next
        assert!(
            msg.contains("db login"),
            "message must say what to do next: {msg}"
        );
        // And must NOT assert a cause the code cannot know.
        assert!(
            !msg.to_lowercase().contains("provider is enabled"),
            "message must not invent a cause: {msg}"
        );
    }

    #[test]
    fn test_terminal_message_says_so_when_the_provider_sent_nothing() {
        let f = CallbackFailure::from_request_line("GET /callback HTTP/1.1");
        assert!(
            f.provider_text().is_none(),
            "an empty callback yields no provider text"
        );
        let msg = f.terminal_message("https://nu.example.com");
        assert!(
            msg.contains("nothing"),
            "an empty callback must report that the provider said nothing, not guess: {msg}"
        );
        assert!(!msg.to_lowercase().contains("provider is enabled"), "{msg}");
    }

    #[test]
    fn test_terminal_message_handles_an_unconfigured_endpoint() {
        let f = CallbackFailure::from_request_line(REAL_FAILURE);
        let msg = f.terminal_message("");
        assert!(
            msg.contains("no endpoint configured"),
            "an empty endpoint must be stated, not printed as a blank: {msg}"
        );
    }

    #[test]
    fn test_error_page_carries_the_description_and_escapes_it() {
        let f = CallbackFailure::from_request_line(REAL_FAILURE);
        let page = f.error_page("https://nu.example.com");
        assert!(
            page.contains("confirmation_token"),
            "the browser page must show the provider's explanation — it is the page the \
             user was previously forced to read out of the address bar"
        );
        assert!(
            page.contains("https://nu.example.com"),
            "page names the backend"
        );
        assert!(
            !page.contains("provider is enabled in your Supabase project"),
            "the page must not invent a cause"
        );
        // The description reached us through a URL, so it must not be able to inject markup.
        assert!(
            f.description
                .as_deref()
                .expect("has description")
                .contains('"'),
            "fixture should contain a quote character to make the escaping meaningful"
        );
        assert!(
            page.contains("&quot;confirmation_token&quot;"),
            "provider text must be HTML-escaped in the page"
        );
    }

    #[test]
    fn test_error_page_escapes_injected_markup() {
        let line =
            "GET /callback?error=x&error_description=%3Cscript%3Ealert(1)%3C%2Fscript%3E HTTP/1.1";
        let page = CallbackFailure::from_request_line(line).error_page("https://nu.example.com");
        assert!(
            !page.contains("<script>"),
            "a callback URL must not be able to inject script into the page"
        );
        assert!(page.contains("&lt;script&gt;"), "markup must be escaped");
    }

    #[test]
    fn test_error_code_is_reported_when_present() {
        let line = "GET /callback?error=server_error&error_code=unexpected_failure&error_description=boom HTTP/1.1";
        let f = CallbackFailure::from_request_line(line);
        assert_eq!(f.code.as_deref(), Some("unexpected_failure"));
        assert!(f.terminal_message("e").contains("unexpected_failure"));
        assert!(f.error_page("e").contains("unexpected_failure"));
    }

    // ---- IPEDS filename casing ---------------------------------------------

    fn touch(dir: &std::path::Path, name: &str) {
        std::fs::write(dir.join(name), b"UNITID\n").expect("write fixture");
    }

    #[test]
    fn test_auto_detect_file_matches_lowercase_ipeds_names() {
        // IPEDS ships hd2022.csv and hd2025.csv lowercase but HD2023.csv and HD2024.csv
        // uppercase; an exact-case lookup silently skipped the lowercase years and the
        // import reported "no file provided or found".
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "hd2022.csv");

        let found = auto_detect_file(dir.path(), &["HD2022.csv", "HD2022.zip", "HD*.csv"])
            .expect("a lowercase IPEDS file must be found");
        assert_eq!(found.file_name().unwrap(), "hd2022.csv");
    }

    #[test]
    fn test_auto_detect_file_matches_mixed_case_zip_and_glob() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "Hd2024.ZIP");
        let found = auto_detect_file(dir.path(), &["HD2024.csv", "HD2024.zip"])
            .expect("case-insensitive zip match");
        assert_eq!(found.file_name().unwrap(), "Hd2024.ZIP");

        let glob_dir = tempfile::tempdir().expect("tempdir");
        touch(glob_dir.path(), "c2023_a.csv");
        let found = auto_detect_file(glob_dir.path(), &["C2023_A.csv", "C*_A.csv"])
            .expect("case-insensitive glob match");
        assert_eq!(found.file_name().unwrap(), "c2023_a.csv");
    }

    #[test]
    fn test_auto_detect_file_prefers_an_exact_name_over_the_glob() {
        // The year-specific candidate must win, or a directory holding several years
        // imports the wrong one.
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "HD2019.csv");
        touch(dir.path(), "hd2024.csv");

        let found =
            auto_detect_file(dir.path(), &["HD2024.csv", "HD2024.zip", "HD*.csv"]).expect("found");
        assert_eq!(
            found.file_name().unwrap(),
            "hd2024.csv",
            "the requested year must win over an earlier glob match"
        );
    }

    #[test]
    fn test_auto_detect_file_is_deterministic_across_case_variants() {
        // read_dir order is arbitrary; with both cases present the same file must be
        // chosen every time rather than alternating between runs.
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "HD2022.csv");
        touch(dir.path(), "hd2022.csv");

        let first = auto_detect_file(dir.path(), &["HD*.csv"]).expect("found");
        for _ in 0..20 {
            assert_eq!(
                auto_detect_file(dir.path(), &["HD*.csv"]).expect("found"),
                first,
                "case-variant selection must not vary between calls"
            );
        }
    }

    #[test]
    fn test_auto_detect_file_still_returns_none_when_nothing_matches() {
        let dir = tempfile::tempdir().expect("tempdir");
        touch(dir.path(), "readme.txt");
        assert!(auto_detect_file(dir.path(), &["HD2022.csv", "HD*.csv"]).is_none());
    }
}
