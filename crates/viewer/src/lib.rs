pub mod commands;
pub mod wsl;

// Extraction/parse modules live in the shared `history-core` crate, re-exported
// under their original `crate::` paths.
pub use history_core::cli_args;
pub use history_core::export;
pub use history_core::models;
pub use history_core::providers;
pub use history_core::utils;

#[cfg(feature = "webui-server")]
pub mod server;

#[cfg(feature = "webui-server")]
const ALLOW_UNSAFE_NO_AUTH_FLAG: &str = "--allow-unsafe-no-auth";

#[cfg(feature = "webui-server")]
const MIN_CUSTOM_TOKEN_LENGTH: usize = 32;

#[cfg(feature = "webui-server")]
const AUTH_USER_FLAG: &str = "--auth-user";

#[cfg(feature = "webui-server")]
const AUTH_PASSWORD_HASH_FLAG: &str = "--auth-password-hash";

#[cfg(feature = "webui-server")]
const SECURE_COOKIES_FLAG: &str = "--secure-cookies";

#[cfg(feature = "webui-server")]
const PRINT_PASSWORD_HASH_FLAG: &str = "--print-password-hash";

#[cfg(test)]
pub mod test_utils;

pub fn run() {
    // Headless session export (issue #343): `--export <id|path> [--format html|json]
    // [--output <file>]`. Handled before any GUI/webview so it works over SSH/CI
    // with no display.
    {
        let args: Vec<String> = std::env::args().collect();
        if args
            .iter()
            .any(|a| a == "--export" || a.starts_with("--export="))
        {
            std::process::exit(export::run_export(&args));
        }
    }

    // Check for --serve flag (WebUI server mode)
    #[cfg(feature = "webui-server")]
    {
        let args: Vec<String> = std::env::args().collect();
        if args.iter().any(|a| a == "--serve") {
            run_server(&args);
            return;
        }
    }

    eprintln!(
        "usage: claude-code-history-viewer --serve [--port N] [--host H] [--token T | --no-auth]\n       \
         claude-code-history-viewer --export <session-id|/abs/path.jsonl> [--format html|json] [--output <file>]"
    );
    std::process::exit(2);
}

#[cfg(feature = "webui-server")]
fn run_server(args: &[String]) {
    use std::sync::Arc;

    match maybe_print_password_hash(args) {
        Ok(true) => return,
        Ok(false) => {}
        Err(message) => {
            eprintln!("{message}");
            std::process::exit(2);
        }
    }

    let port = crate::cli_args::extract_flag_value(args, "--port")
        .and_then(|v| v.parse::<u16>().ok())
        .unwrap_or(3727);
    let host = crate::cli_args::extract_flag_value(args, "--host")
        .unwrap_or_else(|| "0.0.0.0".to_string());
    let dist_dir = crate::cli_args::extract_flag_value(args, "--dist");
    let read_only = args.iter().any(|a| a == "--read-only");
    let base_path = crate::cli_args::extract_flag_value(args, "--base-path")
        .map(|value| {
            server::normalize_base_path(&value).unwrap_or_else(|error| {
                eprintln!("❌ Invalid --base-path: {error}");
                std::process::exit(2);
            })
        })
        .unwrap_or_else(|| "/".to_string());

    let resolved_auth = resolve_auth(args).unwrap_or_else(|message| {
        eprintln!("{message}");
        std::process::exit(2);
    });
    let allow_unsafe_no_auth = args.iter().any(|a| a == ALLOW_UNSAFE_NO_AUTH_FLAG);

    if let Err(message) =
        validate_auth_startup_options(&host, resolved_auth.auth.is_enabled(), allow_unsafe_no_auth)
    {
        eprintln!("{message}");
        std::process::exit(2);
    }

    if let Err(message) = validate_account_cookie_security(&host, &resolved_auth.startup) {
        eprintln!("{message}");
        std::process::exit(2);
    }

    let metadata = Arc::new(crate::commands::metadata::MetadataState::default());
    let (event_tx, _rx) =
        tokio::sync::broadcast::channel::<crate::commands::watcher::FileWatchEvent>(256);

    let state = Arc::new(server::state::AppState {
        metadata,
        start_time: std::time::Instant::now(),
        auth: resolved_auth.auth.clone(),
        read_only,
        event_tx,
    });

    // Print access info — resolve a routable IP when bound to 0.0.0.0
    let display_host = if host == "0.0.0.0" {
        get_local_ip().unwrap_or_else(|| host.clone())
    } else {
        host.clone()
    };
    let display_addr = format!("{display_host}:{port}");
    match &resolved_auth.startup {
        AuthStartup::Token { token, source } => {
            let preview: String = token.chars().take(8).collect();
            eprintln!("🔑 Auth token enabled: {preview}...");
            if is_weak_custom_token(token, *source) {
                eprintln!(
                    "⚠ Custom auth token is shorter than {MIN_CUSTOM_TOKEN_LENGTH} characters; use a strong random token for network access."
                );
            }
            eprintln!(
                "   Open in browser: http://{display_addr}{}",
                server_base_href(&base_path)
            );

            match source {
                AuthTokenSource::Generated => {
                    if let Some(path) = write_generated_token_file(token) {
                        eprintln!("   Generated token saved to: {}", path.to_string_lossy());
                        eprintln!("   First login: append '?token=<token-from-file>' to the URL");
                    } else {
                        eprintln!(
                            "⚠ Failed to persist generated token. Re-run with --token <value>."
                        );
                    }
                }
                AuthTokenSource::Cli | AuthTokenSource::Env => {
                    eprintln!("   First login: append '?token=<your-token>' to the URL");
                }
            }
        }
        AuthStartup::Account {
            username,
            source,
            secure_cookies,
        } => {
            eprintln!("🔐 Account auth enabled for user: {username}");
            eprintln!(
                "   Credentials source: {}",
                match source {
                    AccountAuthSource::Cli => "CLI flags",
                    AccountAuthSource::Env => "environment variables",
                }
            );
            if *secure_cookies {
                eprintln!("   Secure cookies enabled; serve behind HTTPS.");
            } else if !is_loopback_bind_host(&host) {
                eprintln!(
                    "⚠ Secure cookies are disabled. Add {SECURE_COOKIES_FLAG} when using HTTPS reverse proxy."
                );
            }
            eprintln!(
                "   Open in browser: http://{display_addr}{}",
                server_base_href(&base_path)
            );
        }
        AuthStartup::Disabled => {
            eprintln!("🔓 Authentication disabled (--no-auth)");
            if !is_loopback_bind_host(&host) {
                eprintln!(
                    "⚠ WARNING: --no-auth on a non-loopback host exposes your data to the network!"
                );
                eprintln!("  Anyone on your network can read your conversation history without authentication.");
            }
            eprintln!(
                "   Open in browser: http://{display_addr}{}",
                server_base_href(&base_path)
            );
        }
    }
    if read_only {
        eprintln!("🔒 Read-only mode enabled: mutating API endpoints will return 403");
    }

    let rt = tokio::runtime::Runtime::new().expect("Failed to create Tokio runtime");
    rt.block_on(async {
        // Start background file watcher (sends events to broadcast channel)
        let _watcher_handle = start_server_file_watcher(&state);

        server::start(state, &host, port, dist_dir.as_deref(), &base_path).await;
    });
}

#[cfg(feature = "webui-server")]
fn server_base_href(base_path: &str) -> String {
    if base_path == "/" {
        "/".to_string()
    } else {
        format!("{base_path}/")
    }
}

/// Detect the machine's LAN IP address by connecting a UDP socket to an
/// external address.  No actual traffic is sent — the OS just picks the
/// outbound interface, giving us the local IP.
#[cfg(feature = "webui-server")]
fn get_local_ip() -> Option<String> {
    let socket = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("8.8.8.8:80").ok()?;
    let addr = socket.local_addr().ok()?;
    Some(addr.ip().to_string())
}

#[cfg(feature = "webui-server")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AuthTokenSource {
    Cli,
    Env,
    Generated,
}

#[cfg(feature = "webui-server")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AccountAuthSource {
    Cli,
    Env,
}

#[cfg(feature = "webui-server")]
struct ResolvedAuth {
    auth: server::auth::AuthState,
    startup: AuthStartup,
}

#[cfg(feature = "webui-server")]
enum AuthStartup {
    Disabled,
    Token {
        token: String,
        source: AuthTokenSource,
    },
    Account {
        username: String,
        source: AccountAuthSource,
        secure_cookies: bool,
    },
}

#[cfg(feature = "webui-server")]
fn resolve_auth(args: &[String]) -> Result<ResolvedAuth, String> {
    if args.iter().any(|a| a == "--no-auth") {
        return Ok(ResolvedAuth {
            auth: server::auth::AuthState::Disabled,
            startup: AuthStartup::Disabled,
        });
    }

    let secure_cookies = secure_cookies_enabled(args);
    if let Some(account) = resolve_account_auth(args, secure_cookies)? {
        return Ok(account);
    }

    let Some((token, source)) = resolve_auth_token(args) else {
        return Ok(ResolvedAuth {
            auth: server::auth::AuthState::Disabled,
            startup: AuthStartup::Disabled,
        });
    };

    Ok(ResolvedAuth {
        auth: server::auth::AuthState::Token {
            token: token.clone(),
            secure_cookies,
        },
        startup: AuthStartup::Token { token, source },
    })
}

#[cfg(feature = "webui-server")]
fn resolve_account_auth(
    args: &[String],
    secure_cookies: bool,
) -> Result<Option<ResolvedAuth>, String> {
    let username_from_cli = require_non_empty_flag(args, AUTH_USER_FLAG)?;
    let hash_from_cli = require_non_empty_flag(args, AUTH_PASSWORD_HASH_FLAG)?;
    let username_from_env = non_empty_env("CCHV_AUTH_USERNAME");
    let hash_from_env = non_empty_env("CCHV_AUTH_PASSWORD_HASH");

    let username = username_from_cli
        .clone()
        .or(username_from_env)
        .unwrap_or_default();
    let password_hash = hash_from_cli.clone().or(hash_from_env).unwrap_or_default();

    if username.is_empty() && password_hash.is_empty() {
        return Ok(None);
    }
    if username.is_empty() {
        return Err(
            "Account auth is missing a username. Set --auth-user or CCHV_AUTH_USERNAME."
                .to_string(),
        );
    }
    if password_hash.is_empty() {
        return Err(
            "Account auth is missing a password hash. Set --auth-password-hash or CCHV_AUTH_PASSWORD_HASH."
                .to_string(),
        );
    }
    if !server::auth::password_hash_is_valid(&password_hash) {
        return Err("Account auth password hash must be a valid Argon2 PHC string.".to_string());
    }

    let source = if username_from_cli.is_some() || hash_from_cli.is_some() {
        AccountAuthSource::Cli
    } else {
        AccountAuthSource::Env
    };

    Ok(Some(ResolvedAuth {
        auth: server::auth::AuthState::Account(std::sync::Arc::new(
            server::auth::AccountAuth::new(username.clone(), password_hash, secure_cookies),
        )),
        startup: AuthStartup::Account {
            username,
            source,
            secure_cookies,
        },
    }))
}

#[cfg(feature = "webui-server")]
fn require_non_empty_flag(args: &[String], flag: &str) -> Result<Option<String>, String> {
    if let Some(value) = crate::cli_args::extract_flag_value(args, flag) {
        let trimmed = value.trim();
        if trimmed.is_empty() {
            return Err(format!("{flag} must not be empty"));
        }
        return Ok(Some(trimmed.to_string()));
    }
    if crate::cli_args::has_explicit_empty_flag(args, flag) {
        return Err(format!("{flag} must not be empty"));
    }
    Ok(None)
}

#[cfg(feature = "webui-server")]
fn non_empty_env(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(feature = "webui-server")]
fn secure_cookies_enabled(args: &[String]) -> bool {
    args.iter().any(|a| a == SECURE_COOKIES_FLAG)
        || non_empty_env("CCHV_SECURE_COOKIES")
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false)
}

#[cfg(feature = "webui-server")]
fn maybe_print_password_hash(args: &[String]) -> Result<bool, String> {
    let requested = args
        .iter()
        .any(|arg| arg == PRINT_PASSWORD_HASH_FLAG || arg.starts_with("--print-password-hash="));
    if !requested {
        return Ok(false);
    }

    let cli_value = crate::cli_args::extract_flag_value(args, PRINT_PASSWORD_HASH_FLAG)
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if cli_value.is_some() {
        eprintln!(
            "⚠ Passing the password on the command line exposes it in your shell history and process list. \
Prefer: CCHV_AUTH_PASSWORD=<password> {PRINT_PASSWORD_HASH_FLAG}"
        );
    }

    let password = cli_value
        .or_else(|| non_empty_env("CCHV_AUTH_PASSWORD"))
        .ok_or_else(|| {
            format!(
                "Set {PRINT_PASSWORD_HASH_FLAG} <password> or CCHV_AUTH_PASSWORD before generating a password hash."
            )
        })?;

    let hash = server::auth::hash_password_argon2id(&password)?;
    println!("{hash}");
    Ok(true)
}

#[cfg(feature = "webui-server")]
fn validate_auth_startup_options(
    host: &str,
    auth_enabled: bool,
    allow_unsafe_no_auth: bool,
) -> Result<(), String> {
    if auth_enabled || is_loopback_bind_host(host) || allow_unsafe_no_auth {
        return Ok(());
    }

    Err(format!(
        "Refusing to start with --no-auth on non-loopback host '{host}'. \
Use --host 127.0.0.1 for local-only access, enable token auth, or add \
{ALLOW_UNSAFE_NO_AUTH_FLAG} if you intentionally want unauthenticated network access."
    ))
}

/// Account auth issues a multi-day session bearer cookie. On a non-loopback bind
/// without secure cookies (i.e. plain HTTP), that cookie travels in cleartext and can
/// be sniffed and replayed to hijack the session. Refuse to start in that case rather
/// than only warning — mirroring the `--no-auth` guard. Loopback binds (local-only) and
/// `--secure-cookies` (HTTPS / TLS-terminating reverse proxy) are allowed.
#[cfg(feature = "webui-server")]
fn validate_account_cookie_security(host: &str, startup: &AuthStartup) -> Result<(), String> {
    if let AuthStartup::Account {
        secure_cookies: false,
        ..
    } = startup
    {
        if !is_loopback_bind_host(host) {
            return Err(format!(
                "Refusing to start account auth on non-loopback host '{host}' without secure cookies. \
The session cookie would be sent in cleartext and could be hijacked. \
Add {SECURE_COOKIES_FLAG} when serving over HTTPS (e.g. behind a TLS reverse proxy), \
or use --host 127.0.0.1 for local-only access."
            ));
        }
    }
    Ok(())
}

#[cfg(all(test, feature = "webui-server"))]
mod auth_startup_tests {
    use super::*;

    fn account(secure_cookies: bool) -> AuthStartup {
        AuthStartup::Account {
            username: "admin".to_string(),
            source: AccountAuthSource::Cli,
            secure_cookies,
        }
    }

    #[test]
    fn account_insecure_cookies_refused_on_non_loopback() {
        assert!(validate_account_cookie_security("0.0.0.0", &account(false)).is_err());
        assert!(validate_account_cookie_security("192.168.1.10", &account(false)).is_err());
    }

    #[test]
    fn account_insecure_cookies_allowed_on_loopback() {
        assert!(validate_account_cookie_security("127.0.0.1", &account(false)).is_ok());
        assert!(validate_account_cookie_security("localhost", &account(false)).is_ok());
    }

    #[test]
    fn account_secure_cookies_allowed_anywhere() {
        assert!(validate_account_cookie_security("0.0.0.0", &account(true)).is_ok());
    }

    #[test]
    fn non_account_modes_are_unaffected() {
        assert!(validate_account_cookie_security("0.0.0.0", &AuthStartup::Disabled).is_ok());
        assert!(validate_account_cookie_security(
            "0.0.0.0",
            &AuthStartup::Token {
                token: "x".to_string(),
                source: AuthTokenSource::Cli,
            },
        )
        .is_ok());
    }
}

#[cfg(feature = "webui-server")]
fn is_loopback_bind_host(host: &str) -> bool {
    let normalized = host
        .trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.');

    if normalized.eq_ignore_ascii_case("localhost") {
        return true;
    }

    normalized
        .parse::<std::net::IpAddr>()
        .map(|ip| ip.is_loopback())
        .unwrap_or(false)
}

#[cfg(feature = "webui-server")]
fn is_weak_custom_token(token: &str, source: AuthTokenSource) -> bool {
    !matches!(source, AuthTokenSource::Generated) && token.chars().count() < MIN_CUSTOM_TOKEN_LENGTH
}

/// Resolve the authentication token from CLI arguments or environment.
///
/// Priority:
/// - `--no-auth` → `None` (auth disabled)
/// - `--token <value>` → `Some(value)` (user-supplied via CLI)
/// - `CCHV_TOKEN` env var → `Some(value)` (user-supplied via env, e.g. systemd)
/// - otherwise → `Some(uuid-v4)` (auto-generated)
#[cfg(feature = "webui-server")]
fn resolve_auth_token(args: &[String]) -> Option<(String, AuthTokenSource)> {
    if args.iter().any(|a| a == "--no-auth") {
        return None;
    }
    if let Some(token) = crate::cli_args::extract_flag_value(args, "--token") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some((trimmed.to_string(), AuthTokenSource::Cli));
        }
        eprintln!("⚠ --token value is empty; falling back to auto-generated token");
    } else if crate::cli_args::has_explicit_empty_flag(args, "--token") {
        // `extract_flag_value` returns None for `--token=` and for a bare
        // `--token` at end-of-argv. Neither case should silently auto-generate
        // a token without warning the operator their config is broken.
        eprintln!("⚠ --token value is empty; falling back to auto-generated token");
    }
    if let Ok(token) = std::env::var("CCHV_TOKEN") {
        let trimmed = token.trim();
        if !trimmed.is_empty() {
            return Some((trimmed.to_string(), AuthTokenSource::Env));
        }
    }
    Some((uuid::Uuid::new_v4().to_string(), AuthTokenSource::Generated))
}

/// Persist auto-generated token to a local file instead of logging the full secret.
#[cfg(feature = "webui-server")]
fn write_generated_token_file(token: &str) -> Option<std::path::PathBuf> {
    let home = dirs::home_dir()?;
    let dir = home.join(".claude-history-viewer");
    std::fs::create_dir_all(&dir).ok()?;
    let path = dir.join("webui-token.txt");
    std::fs::write(&path, format!("{token}\n")).ok()?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Some(path)
}

/// Start a `notify`-based file watcher that pushes change events into the
/// broadcast channel on `state.event_tx`.
///
/// Returns the debouncer handle — it must be kept alive for the watcher to
/// continue running.  Returns `None` if the watched directory doesn't exist.
#[cfg(feature = "webui-server")]
fn start_server_file_watcher(
    state: &std::sync::Arc<server::state::AppState>,
) -> Option<notify_debouncer_mini::Debouncer<notify::RecommendedWatcher>> {
    let watch_paths = collect_watch_paths();
    if watch_paths.is_empty() {
        eprintln!("⚠ No supported provider directories found; real-time file watcher disabled");
        return None;
    }

    let tx = state.event_tx.clone();

    let mut debouncer = notify_debouncer_mini::new_debouncer(
        std::time::Duration::from_millis(500),
        move |result: Result<Vec<notify_debouncer_mini::DebouncedEvent>, notify::Error>| {
            if let Ok(events) = result {
                for event in events {
                    if let Some(watch_event) = crate::commands::watcher::to_file_watch_event(&event)
                    {
                        crate::commands::session::invalidate_search_cache();
                        // Ignore send errors (no active subscribers yet)
                        let _ = tx.send(watch_event);
                    }
                }
            }
        },
    )
    .ok()?;

    let mut watched_count = 0usize;
    for path in &watch_paths {
        match debouncer
            .watcher()
            .watch(path, notify::RecursiveMode::Recursive)
        {
            Ok(()) => {
                crate::commands::watcher::prime_watch_signatures(path);
                watched_count += 1;
                eprintln!("👁 File watcher active: {}", path.display());
            }
            Err(e) => {
                eprintln!("⚠ Failed to watch {}: {e}", path.display());
            }
        }
    }

    if watched_count == 0 {
        eprintln!("⚠ Real-time updates disabled (no watch path could be registered)");
        return None;
    }

    Some(debouncer)
}

/// Collect available provider directories to watch for live session file updates.
#[cfg(feature = "webui-server")]
fn collect_watch_paths() -> Vec<std::path::PathBuf> {
    use std::collections::HashSet;
    use std::path::PathBuf;

    let mut paths: Vec<PathBuf> = Vec::new();

    if let Some(home) = dirs::home_dir() {
        let claude_projects = home.join(".claude").join("projects");
        if claude_projects.is_dir() {
            paths.push(claude_projects);
        }

        // Load custom Claude paths from user-data.json
        let user_data_path = home.join(".claude-history-viewer").join("user-data.json");
        if let Ok(content) = std::fs::read_to_string(&user_data_path) {
            if let Ok(metadata) = serde_json::from_str::<serde_json::Value>(&content) {
                if let Some(custom_paths) = metadata
                    .get("settings")
                    .and_then(|s| s.get("customClaudePaths"))
                    .and_then(|v| v.as_array())
                {
                    for entry in custom_paths {
                        if let Some(path_str) = entry.get("path").and_then(|p| p.as_str()) {
                            let custom_base = PathBuf::from(path_str);
                            if let Ok(canonical_projects) =
                                crate::utils::validate_custom_claude_path(&custom_base)
                            {
                                paths.push(canonical_projects);
                            }
                        }
                    }
                }
            }
        }
    }

    if let Some(codex_base) = providers::codex::get_base_path() {
        let base = PathBuf::from(codex_base);
        let sessions = base.join("sessions");
        let archived_sessions = base.join("archived_sessions");
        if sessions.is_dir() {
            paths.push(sessions);
        }
        if archived_sessions.is_dir() {
            paths.push(archived_sessions);
        }
    }

    if let Some(kimi_base) = providers::kimi::get_base_path() {
        let sessions = PathBuf::from(kimi_base).join("sessions");
        if sessions.is_dir() {
            paths.push(sessions);
        }
    }

    if let Some(opencode_base) = providers::opencode::get_base_path() {
        let base = PathBuf::from(&opencode_base);
        let storage = base.join("storage");
        let session = storage.join("session");
        let message = storage.join("message");
        if session.is_dir() {
            paths.push(session);
        }
        if message.is_dir() {
            paths.push(message);
        }
        // Watch opencode.db for SQLite-based storage changes
        let db_path = base.join("opencode.db");
        if db_path.is_file() {
            paths.push(base);
        }
    }

    if let Some(codebuddy_base) = providers::codebuddy::get_base_path() {
        let codebuddy_projects = PathBuf::from(codebuddy_base);
        if codebuddy_projects.is_dir() {
            paths.push(codebuddy_projects);
        }
    }

    if let Some(cursor_agent_base) = providers::cursor_agent::get_base_path() {
        let cursor_agent_projects = PathBuf::from(cursor_agent_base);
        if cursor_agent_projects.is_dir() {
            paths.push(cursor_agent_projects);
        }
    }

    if let Some(continue_base) = providers::continue_dev::get_base_path() {
        let continue_sessions = PathBuf::from(continue_base);
        if continue_sessions.is_dir() {
            paths.push(continue_sessions);
        }
    }

    if let Some(pearai_base) = providers::pearai::get_base_path() {
        let pearai_sessions = PathBuf::from(pearai_base);
        if pearai_sessions.is_dir() {
            paths.push(pearai_sessions);
        }
    }

    if let Some(goose_base) = providers::goose::get_base_path() {
        let goose_sessions = PathBuf::from(goose_base);
        if goose_sessions.is_dir() {
            paths.push(goose_sessions);
        }
    }

    if let Some(llm_base) = providers::llm::get_base_path() {
        let llm_dir = PathBuf::from(llm_base);
        if llm_dir.is_dir() {
            paths.push(llm_dir);
        }
    }

    if let Some(amazon_q_base) = providers::amazon_q::get_base_path() {
        let amazon_q_dir = PathBuf::from(amazon_q_base);
        if amazon_q_dir.is_dir() {
            paths.push(amazon_q_dir);
        }
    }

    if let Some(oi_base) = providers::openinterpreter::get_base_path() {
        for sub in ["sessions", "archived_sessions"] {
            let dir = PathBuf::from(&oi_base).join(sub);
            if dir.is_dir() {
                paths.push(dir);
            }
        }
    }

    if let Some(pi_base) = providers::pi::get_base_path() {
        let pi_sessions = PathBuf::from(pi_base);
        if pi_sessions.is_dir() {
            paths.push(pi_sessions);
        }
    }

    if let Some(qwen_base) = providers::qwen::get_base_path() {
        let qwen_projects = PathBuf::from(qwen_base);
        if qwen_projects.is_dir() {
            paths.push(qwen_projects);
        }
    }

    if let Some(zed_base) = providers::zed::get_base_path() {
        let zed_dir = PathBuf::from(zed_base);
        if zed_dir.is_dir() {
            paths.push(zed_dir);
        }
    }

    if let Some(oh_base) = providers::openhands::get_base_path() {
        let oh_dir = PathBuf::from(oh_base);
        if oh_dir.is_dir() {
            paths.push(oh_dir);
        }
    }

    if let Some(trae_base) = providers::trae::get_base_path() {
        let trae_dir = PathBuf::from(trae_base);
        if trae_dir.is_dir() {
            paths.push(trae_dir);
        }
    }

    if let Some(copilot_base) = providers::copilot_cli::get_base_path() {
        let session_state = PathBuf::from(copilot_base).join("session-state");
        if session_state.is_dir() {
            paths.push(session_state);
        }
    }

    for vscode_base in providers::vscode::get_base_paths() {
        let ws_storage = vscode_base.join("workspaceStorage");
        if ws_storage.is_dir() {
            paths.push(ws_storage);
        }
    }

    let mut seen = HashSet::new();
    paths
        .into_iter()
        .filter(|p| seen.insert(p.clone()))
        .collect::<Vec<_>>()
}
