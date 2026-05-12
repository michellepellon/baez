//! Granola token discovery and refresh.
//!
//! Resolution order: CLI flag → env var → config file → Granola session file.
//! Only the Granola session file source supports refresh (it has a `refresh_token`
//! alongside the access_token).

use crate::{Error, Result};
use chrono::Utc;
use reqwest::blocking::Client;
use serde::Deserialize;
use serde_json::json;
use std::cell::RefCell;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

const REFRESH_MAX_RETRIES: u32 = 2;
const REFRESH_INITIAL_RETRY_DELAY: Duration = Duration::from_secs(2);

/// Pinned auth domain for WorkOS User Management. Hardcoded (rather than read from
/// the JWT `iss` field) so a tampered supabase.json cannot redirect the refresh_token
/// POST to an attacker-controlled host.
pub(crate) const GRANOLA_AUTH_DOMAIN: &str = "https://auth.granola.ai";

/// Pinned WorkOS client_id for Granola's tenant. Non-secret tenant identifier;
/// rotation would require a baez code update.
pub(crate) const GRANOLA_CLIENT_ID: &str = "client_01JZJ0XBDAT8PHJWQY09Y0VD61";

/// Authentication credentials for the Granola API. Static tokens come from CLI flags,
/// env vars, or the config file; Refreshable credentials come from the Granola
/// session file and can be refreshed via WorkOS when expired.
pub enum Credentials {
    Static(String),
    Refreshable(RefCell<RefreshableState>),
}

/// Mutable state for refreshable credentials. Lives inside a `RefCell` so the
/// `&self` API can update tokens after a successful refresh.
pub struct RefreshableState {
    pub access_token: String,
    pub refresh_token: String,
    pub supabase_path: PathBuf,
    pub http_client: Client,
    pub verbose: bool,
    /// WorkOS auth domain. Defaults to `GRANOLA_AUTH_DOMAIN`; overridable for tests.
    pub auth_domain: String,
}

#[derive(Deserialize)]
struct WorkosRefreshResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

impl Credentials {
    /// Return the current access token. Does not check expiry or trigger refresh —
    /// refresh happens via `refresh_with_cas()` on a 401 response.
    pub fn access_token(&self) -> String {
        match self {
            Credentials::Static(t) => t.clone(),
            Credentials::Refreshable(state) => state.borrow().access_token.clone(),
        }
    }

    /// Attempt to refresh credentials. Returns `Ok(true)` if tokens were updated
    /// (either by adopting a value another process wrote, or by performing the
    /// WorkOS refresh ourselves) and `Ok(false)` for `Static` credentials.
    pub fn refresh_with_cas(&self) -> Result<bool> {
        match self {
            Credentials::Static(_) => Ok(false),
            Credentials::Refreshable(state_cell) => {
                let (supabase_path, in_mem_refresh, verbose) = {
                    let state = state_cell.borrow();
                    (
                        state.supabase_path.clone(),
                        state.refresh_token.clone(),
                        state.verbose,
                    )
                };

                // CAS step: re-read supabase.json. If its refresh_token differs from
                // our in-memory value, another process refreshed; adopt without HTTP.
                if let Some(fresh) = parse_session_credentials(&supabase_path)? {
                    if fresh.refresh_token != in_mem_refresh {
                        if verbose {
                            eprintln!(
                                "[verbose] Adopted refreshed tokens from supabase.json (another process refreshed)"
                            );
                        }
                        let mut state = state_cell.borrow_mut();
                        state.access_token = fresh.access_token;
                        state.refresh_token = fresh.refresh_token;
                        return Ok(true);
                    }
                }

                // CAS miss: perform the refresh ourselves.
                if verbose {
                    eprintln!(
                        "[verbose] Granola access token rejected (401), attempting refresh..."
                    );
                }

                let (http_client, auth_domain) = {
                    let state = state_cell.borrow();
                    (state.http_client.clone(), state.auth_domain.clone())
                };
                let response = workos_refresh(&http_client, &auth_domain, &in_mem_refresh)?;
                let new_access = response.access_token;
                let new_refresh = response.refresh_token.unwrap_or(in_mem_refresh);
                let now_ms = Utc::now().timestamp_millis();

                // Write back to supabase.json first; in-memory update next. On write
                // failure, log a warning and continue — the next baez run will retry.
                let tmp_dir = supabase_path
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("."));
                if let Err(e) = write_session_back(
                    &supabase_path,
                    &new_access,
                    &new_refresh,
                    response.expires_in,
                    now_ms,
                    &tmp_dir,
                ) {
                    eprintln!(
                        "Warning: refresh succeeded but failed to update supabase.json: {}",
                        e
                    );
                }

                {
                    let mut state = state_cell.borrow_mut();
                    state.access_token = new_access;
                    state.refresh_token = new_refresh;
                }

                if verbose {
                    eprintln!("[verbose] Refresh succeeded");
                }
                Ok(true)
            }
        }
    }
}

/// POST to WorkOS user_management/authenticate with the refresh_token grant.
/// Retries transient failures via `util::retry_with_backoff`.
fn workos_refresh(
    client: &Client,
    auth_domain: &str,
    refresh_token: &str,
) -> Result<WorkosRefreshResponse> {
    let url = format!("{}/user_management/authenticate", auth_domain);

    crate::util::retry_with_backoff(
        REFRESH_MAX_RETRIES,
        REFRESH_INITIAL_RETRY_DELAY,
        || {
            let body = json!({
                "grant_type": "refresh_token",
                "client_id": GRANOLA_CLIENT_ID,
                "refresh_token": refresh_token,
            });

            let response = client.post(&url).json(&body).send()?;
            let status = response.status();
            if !status.is_success() {
                let message = response.text().unwrap_or_default();
                return Err(Error::Api {
                    endpoint: "/user_management/authenticate".into(),
                    status: status.as_u16(),
                    message,
                });
            }
            response
                .json::<WorkosRefreshResponse>()
                .map_err(Error::from)
        },
        is_retryable_refresh,
    )
    .map_err(|e| match e {
        Error::Api {
            status, message, ..
        } => Error::Auth(format!("refresh failed: HTTP {}: {}", status, message)),
        Error::Network(re) => Error::Auth(format!("refresh failed: network: {}", re)),
        Error::Parse(pe) => Error::Auth(format!("refresh failed: response parse: {}", pe)),
        other => other,
    })
}

/// Update supabase.json in place after a successful refresh. Preserves session_id,
/// user_info, and any unknown fields inside workos_tokens. Preserves the file's
/// existing mode (does not force 0o600 on a file Granola owns).
fn write_session_back(
    path: &Path,
    new_access: &str,
    new_refresh: &str,
    new_expires_in: Option<u64>,
    obtained_at_ms: i64,
    tmp_dir: &Path,
) -> Result<()> {
    let content = fs::read_to_string(path)?;
    let mut top: serde_json::Value = serde_json::from_str(&content)?;

    let workos_str = top
        .get("workos_tokens")
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Auth("supabase.json missing workos_tokens field".into()))?
        .to_string();
    let mut workos: serde_json::Value = serde_json::from_str(&workos_str)?;

    if let Some(obj) = workos.as_object_mut() {
        obj.insert("access_token".to_string(), json!(new_access));
        obj.insert("refresh_token".to_string(), json!(new_refresh));
        if let Some(exp) = new_expires_in {
            obj.insert("expires_in".to_string(), json!(exp));
        }
        obj.insert("obtained_at".to_string(), json!(obtained_at_ms));
    } else {
        return Err(Error::Auth(
            "supabase.json workos_tokens is not a JSON object".into(),
        ));
    }

    let new_workos_str = serde_json::to_string(&workos)?;
    top["workos_tokens"] = json!(new_workos_str);

    let serialized = serde_json::to_string(&top)?;
    crate::storage::write_atomic_preserve_mode(path, serialized.as_bytes(), tmp_dir)
}

/// Retry predicate for WorkOS refresh requests. 5xx and network errors are
/// transient; 4xx is terminal (the refresh_token is dead and won't be revived
/// by retrying).
fn is_retryable_refresh(err: &Error) -> bool {
    match err {
        Error::Network(_) => true,
        Error::Api { status, .. } => *status >= 500,
        _ => false,
    }
}

/// Resolve credentials from the first available source.
///
/// Precedence: CLI flag → `BAEZ_GRANOLA_TOKEN` → `BEARER_TOKEN` (deprecated) →
/// config file → Granola session file.
///
/// All sources except the session file return `Credentials::Static` (no refresh
/// possible — the user supplied a single token deliberately).
pub fn resolve_credentials(cli_token: Option<String>, verbose: bool) -> Result<Credentials> {
    if let Some(token) = cli_token {
        if verbose {
            eprintln!("[verbose] Granola token: --token flag");
        }
        return Ok(Credentials::Static(token));
    }

    if let Ok(token) = env::var("BAEZ_GRANOLA_TOKEN") {
        if verbose {
            eprintln!("[verbose] Granola token: BAEZ_GRANOLA_TOKEN env var");
        }
        return Ok(Credentials::Static(token));
    }

    if let Ok(token) = env::var("BEARER_TOKEN") {
        eprintln!("Warning: BEARER_TOKEN is deprecated, use BAEZ_GRANOLA_TOKEN instead");
        return Ok(Credentials::Static(token));
    }

    if let Some(token) = token_from_config()? {
        if verbose {
            eprintln!("[verbose] Granola token: config file (granola_token)");
        }
        return Ok(Credentials::Static(token));
    }

    if let Some(mut state) = try_session_credentials()? {
        if verbose {
            eprintln!(
                "[verbose] Granola token: Granola session file (auto-discovery, refreshable)"
            );
        }
        state.verbose = verbose;
        return Ok(Credentials::Refreshable(RefCell::new(state)));
    }

    Err(Error::Auth(
        "No Granola token found. Provide via --token, BAEZ_GRANOLA_TOKEN env var, \
         granola_token in ~/.config/baez/config.json, or log in to Granola"
            .into(),
    ))
}

/// Backward-compat wrapper. New code should use `resolve_credentials`.
pub fn resolve_token(cli_token: Option<String>, verbose: bool) -> Result<String> {
    Ok(resolve_credentials(cli_token, verbose)?.access_token())
}

fn token_from_config() -> Result<Option<String>> {
    crate::storage::read_config_field("granola_token")
}

fn try_session_credentials() -> Result<Option<RefreshableState>> {
    let home = env::var("HOME").map_err(|_| Error::Auth("HOME not set".into()))?;
    let path = PathBuf::from(home).join("Library/Application Support/Granola/supabase.json");

    parse_session_credentials(&path)
}

/// Parse the Granola session file at `path` into a `RefreshableState`. Returns
/// `None` if the file is absent or lacks the required fields.
fn parse_session_credentials(path: &Path) -> Result<Option<RefreshableState>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    let Some(workos_str) = json.get("workos_tokens").and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    let workos: serde_json::Value = serde_json::from_str(workos_str)?;

    let Some(access_token) = workos.get("access_token").and_then(|v| v.as_str()) else {
        return Ok(None);
    };
    let Some(refresh_token) = workos.get("refresh_token").and_then(|v| v.as_str()) else {
        return Ok(None);
    };

    let http_client = Client::builder().timeout(Duration::from_secs(30)).build()?;

    Ok(Some(RefreshableState {
        access_token: access_token.to_string(),
        refresh_token: refresh_token.to_string(),
        supabase_path: path.to_path_buf(),
        http_client,
        verbose: false,
        auth_domain: GRANOLA_AUTH_DOMAIN.to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_credentials_cli_returns_static() {
        let creds = resolve_credentials(Some("cli_token".into()), false).unwrap();
        assert!(matches!(creds, Credentials::Static(_)));
        assert_eq!(creds.access_token(), "cli_token");
    }

    /// Test both BAEZ_GRANOLA_TOKEN and legacy BEARER_TOKEN env vars.
    /// Combined into one test because env vars are process-global and
    /// parallel tests would race.
    #[test]
    fn test_resolve_credentials_env_vars_return_static() {
        env::remove_var("BAEZ_GRANOLA_TOKEN");
        env::remove_var("BEARER_TOKEN");

        env::set_var("BAEZ_GRANOLA_TOKEN", "new_env_token");
        env::set_var("BEARER_TOKEN", "legacy_env_token");
        let creds = resolve_credentials(None, false).unwrap();
        assert!(matches!(creds, Credentials::Static(_)));
        assert_eq!(creds.access_token(), "new_env_token");

        env::remove_var("BAEZ_GRANOLA_TOKEN");
        let creds = resolve_credentials(None, false).unwrap();
        assert!(matches!(creds, Credentials::Static(_)));
        assert_eq!(creds.access_token(), "legacy_env_token");

        env::remove_var("BEARER_TOKEN");
    }

    #[test]
    fn test_parse_session_credentials_valid() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("supabase.json");

        let content = r#"{
            "workos_tokens": "{\"access_token\": \"test_at\", \"refresh_token\": \"test_rt\"}"
        }"#;
        fs::write(&session_path, content).unwrap();

        let state = parse_session_credentials(&session_path).unwrap();
        let state = state.expect("expected Some(RefreshableState)");
        assert_eq!(state.access_token, "test_at");
        assert_eq!(state.refresh_token, "test_rt");
        assert_eq!(state.supabase_path, session_path);
    }

    #[test]
    fn test_parse_session_credentials_missing_file() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("missing.json");

        let result = parse_session_credentials(&session_path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_session_credentials_missing_refresh_token() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("supabase.json");

        let content = r#"{
            "workos_tokens": "{\"access_token\": \"test_at\"}"
        }"#;
        fs::write(&session_path, content).unwrap();

        let result = parse_session_credentials(&session_path).unwrap();
        assert!(result.is_none(), "missing refresh_token should yield None");
    }

    #[test]
    fn test_credentials_refresh_with_cas_static_returns_false() {
        let creds = Credentials::Static("token".into());
        assert!(!creds.refresh_with_cas().unwrap());
    }

    #[test]
    fn test_resolve_token_wrapper_returns_access_token() {
        // Backward-compat: resolve_token should still work via the wrapper.
        let token = resolve_token(Some("cli_token".into()), false).unwrap();
        assert_eq!(token, "cli_token");
    }

    #[test]
    fn test_refresh_with_cas_adopts_file_when_refresh_token_differs() {
        // Simulates the case where another process (Granola desktop or another
        // baez instance) refreshed the token between our load and our refresh call.
        // We should adopt the file's tokens without making an HTTP call.
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("supabase.json");

        let content = r#"{
            "workos_tokens": "{\"access_token\": \"new_at\", \"refresh_token\": \"new_rt\"}"
        }"#;
        fs::write(&session_path, content).unwrap();

        let state = RefreshableState {
            access_token: "old_at".to_string(),
            refresh_token: "old_rt".to_string(),
            supabase_path: session_path.clone(),
            http_client: Client::new(),
            verbose: false,
            auth_domain: GRANOLA_AUTH_DOMAIN.to_string(),
        };
        let creds = Credentials::Refreshable(RefCell::new(state));

        let refreshed = creds.refresh_with_cas().unwrap();

        assert!(refreshed, "CAS adoption should report a refresh occurred");
        assert_eq!(creds.access_token(), "new_at");
    }

    #[test]
    fn test_is_retryable_refresh_classifies_correctly() {
        // 5xx is retryable
        assert!(is_retryable_refresh(&Error::Api {
            endpoint: "/auth".into(),
            status: 500,
            message: "".into(),
        }));
        assert!(is_retryable_refresh(&Error::Api {
            endpoint: "/auth".into(),
            status: 502,
            message: "".into(),
        }));

        // 4xx is terminal (refresh_token is dead, no point retrying)
        assert!(!is_retryable_refresh(&Error::Api {
            endpoint: "/auth".into(),
            status: 400,
            message: "".into(),
        }));
        assert!(!is_retryable_refresh(&Error::Api {
            endpoint: "/auth".into(),
            status: 401,
            message: "".into(),
        }));
        assert!(!is_retryable_refresh(&Error::Api {
            endpoint: "/auth".into(),
            status: 422,
            message: "".into(),
        }));

        // Non-Api errors not retryable by this predicate (parse, filesystem, etc.)
        assert!(!is_retryable_refresh(&Error::Auth("bad".into())));
    }

    #[test]
    fn test_write_session_back_preserves_session_id_and_user_info() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("supabase.json");

        let workos_str = serde_json::json!({
            "access_token": "old_at",
            "refresh_token": "old_rt",
            "expires_in": 21599_u64,
            "obtained_at": 1700000000000_i64,
            "token_type": "Bearer",
            "session_id": "session_xyz",
            "unknown_field": "preserved"
        })
        .to_string();
        let original = serde_json::json!({
            "workos_tokens": workos_str,
            "session_id": "session_xyz",
            "user_info": "{\"name\":\"Michelle\"}"
        })
        .to_string();
        fs::write(&session_path, &original).unwrap();

        write_session_back(
            &session_path,
            "new_at",
            "new_rt",
            Some(21599),
            1800000000000_i64,
            temp.path(),
        )
        .unwrap();

        let content = fs::read_to_string(&session_path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&content).unwrap();

        assert_eq!(json["session_id"].as_str().unwrap(), "session_xyz");
        assert!(json["user_info"].as_str().unwrap().contains("Michelle"));

        let workos: serde_json::Value =
            serde_json::from_str(json["workos_tokens"].as_str().unwrap()).unwrap();
        assert_eq!(workos["access_token"].as_str().unwrap(), "new_at");
        assert_eq!(workos["refresh_token"].as_str().unwrap(), "new_rt");
        assert_eq!(workos["obtained_at"].as_i64().unwrap(), 1800000000000_i64);
        assert_eq!(workos["unknown_field"].as_str().unwrap(), "preserved");
        assert_eq!(workos["token_type"].as_str().unwrap(), "Bearer");
    }
}
