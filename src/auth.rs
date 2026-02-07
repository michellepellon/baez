//! Granola token discovery with multi-level precedence chain.
//!
//! Resolution order: CLI flag → env var → config file → Granola session file.

use crate::{Error, Result};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Resolve the Granola API token from the first available source.
///
/// Precedence: CLI flag → `BAEZ_GRANOLA_TOKEN` → `BEARER_TOKEN` (deprecated) →
/// config file → Granola session file.
pub fn resolve_token(cli_token: Option<String>, verbose: bool) -> Result<String> {
    // 1. CLI flag (explicit override)
    if let Some(token) = cli_token {
        if verbose {
            eprintln!("[verbose] Granola token: --token flag");
        }
        return Ok(token);
    }

    // 2. BAEZ_GRANOLA_TOKEN env var (preferred)
    if let Ok(token) = env::var("BAEZ_GRANOLA_TOKEN") {
        if verbose {
            eprintln!("[verbose] Granola token: BAEZ_GRANOLA_TOKEN env var");
        }
        return Ok(token);
    }

    // 3. BEARER_TOKEN env var (backward compat, deprecated)
    if let Ok(token) = env::var("BEARER_TOKEN") {
        eprintln!("Warning: BEARER_TOKEN is deprecated, use BAEZ_GRANOLA_TOKEN instead");
        return Ok(token);
    }

    // 4. Config file (~/.config/baez/config.json → granola_token)
    if let Some(token) = token_from_config()? {
        if verbose {
            eprintln!("[verbose] Granola token: config file (granola_token)");
        }
        return Ok(token);
    }

    // 5. Granola session file (auto-discovery)
    if let Some(token) = try_session_file()? {
        if verbose {
            eprintln!("[verbose] Granola token: Granola session file (auto-discovery)");
        }
        return Ok(token);
    }

    Err(Error::Auth(
        "No Granola token found. Provide via --token, BAEZ_GRANOLA_TOKEN env var, \
         granola_token in ~/.config/baez/config.json, or log in to Granola"
            .into(),
    ))
}

fn token_from_config() -> Result<Option<String>> {
    crate::storage::read_config_field("granola_token")
}

fn try_session_file() -> Result<Option<String>> {
    let home = env::var("HOME").map_err(|_| Error::Auth("HOME not set".into()))?;
    let path = PathBuf::from(home).join("Library/Application Support/Granola/supabase.json");

    parse_session_file(&path)
}

fn parse_session_file(path: &Path) -> Result<Option<String>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;
    let json: serde_json::Value = serde_json::from_str(&content)?;

    // Parse workos_tokens (which is a stringified JSON)
    if let Some(workos_str) = json.get("workos_tokens").and_then(|v| v.as_str()) {
        let workos: serde_json::Value = serde_json::from_str(workos_str)?;
        if let Some(access_token) = workos.get("access_token").and_then(|v| v.as_str()) {
            return Ok(Some(access_token.to_string()));
        }
    }

    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_resolve_token_cli_precedence() {
        let token = resolve_token(Some("cli_token".into()), false).unwrap();
        assert_eq!(token, "cli_token");
    }

    /// Test both BAEZ_GRANOLA_TOKEN and legacy BEARER_TOKEN env vars.
    /// Combined into one test because env vars are process-global and
    /// parallel tests would race.
    #[test]
    fn test_resolve_token_env_vars() {
        // Ensure clean state
        env::remove_var("BAEZ_GRANOLA_TOKEN");
        env::remove_var("BEARER_TOKEN");

        // BAEZ_GRANOLA_TOKEN takes precedence
        env::set_var("BAEZ_GRANOLA_TOKEN", "new_env_token");
        env::set_var("BEARER_TOKEN", "legacy_env_token");
        let token = resolve_token(None, false).unwrap();
        assert_eq!(token, "new_env_token");

        // Legacy BEARER_TOKEN works when BAEZ_GRANOLA_TOKEN is absent
        env::remove_var("BAEZ_GRANOLA_TOKEN");
        let token = resolve_token(None, false).unwrap();
        assert_eq!(token, "legacy_env_token");

        // Cleanup
        env::remove_var("BEARER_TOKEN");
    }

    #[test]
    fn test_parse_session_file_valid() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("supabase.json");

        let content = r#"{
            "workos_tokens": "{\"access_token\": \"test_token_123\"}"
        }"#;
        fs::write(&session_path, content).unwrap();

        let token = parse_session_file(&session_path).unwrap();
        assert_eq!(token, Some("test_token_123".into()));
    }

    #[test]
    fn test_parse_session_file_missing() {
        let temp = TempDir::new().unwrap();
        let session_path = temp.path().join("missing.json");

        let token = parse_session_file(&session_path).unwrap();
        assert!(token.is_none());
    }
}
