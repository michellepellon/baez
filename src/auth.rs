//! Granola public API key discovery and storage.
//!
//! Resolution order: CLI flag → `BAEZ_GRANOLA_API_KEY` env var → config file →
//! macOS keychain.

use crate::{Error, Result};
use std::env;

/// macOS keychain service and account names. The account name mirrors the
/// `granola_api_key` field used in `~/.config/baez/config.json`.
const KEYCHAIN_SERVICE: &str = "baez";
const KEYCHAIN_ACCOUNT: &str = "granola_api_key";

/// Resolve the Granola public API key from the first available source.
///
/// Precedence: CLI flag → `BAEZ_GRANOLA_API_KEY` → config file → keychain.
pub fn resolve_api_key(cli_key: Option<String>, verbose: bool) -> Result<String> {
    if let Some(key) = cli_key {
        if verbose {
            eprintln!("[verbose] Granola API key: --api-key flag");
        }
        return Ok(key);
    }

    if let Ok(key) = env::var("BAEZ_GRANOLA_API_KEY") {
        if verbose {
            eprintln!("[verbose] Granola API key: BAEZ_GRANOLA_API_KEY env var");
        }
        return Ok(key);
    }

    if let Some(key) = api_key_from_config()? {
        if verbose {
            eprintln!("[verbose] Granola API key: config file (granola_api_key)");
        }
        return Ok(key);
    }

    if let Some(key) = api_key_from_keychain() {
        if verbose {
            eprintln!("[verbose] Granola API key: macOS keychain");
        }
        return Ok(key);
    }

    Err(Error::Auth(
        "No Granola API key found. Generate one in Granola Desktop \
         (Settings → Connectors → API keys), then run `baez set-granola-api-key <key>`, \
         set BAEZ_GRANOLA_API_KEY, or add granola_api_key to ~/.config/baez/config.json."
            .into(),
    ))
}

fn api_key_from_config() -> Result<Option<String>> {
    crate::storage::read_config_field("granola_api_key")
}

/// Retrieve the Granola API key from the macOS keychain. Returns `None` if the
/// keychain entry is absent or the keychain is unavailable.
pub fn api_key_from_keychain() -> Option<String> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT).ok()?;
    entry.get_password().ok()
}

/// Store the Granola API key in the macOS keychain.
pub fn set_api_key_in_keychain(api_key: &str) -> Result<()> {
    let entry = keyring::Entry::new(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
        .map_err(|e| Error::Auth(format!("Failed to open keychain entry: {}", e)))?;
    entry
        .set_password(api_key)
        .map_err(|e| Error::Auth(format!("Failed to set keychain entry: {}", e)))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_api_key_cli_precedence() {
        let key = resolve_api_key(Some("cli_key".into()), false).unwrap();
        assert_eq!(key, "cli_key");
    }

    /// Tests env var resolution. Combined into one test because env vars are
    /// process-global and parallel tests would race.
    #[test]
    fn test_resolve_api_key_env_var() {
        env::remove_var("BAEZ_GRANOLA_API_KEY");

        env::set_var("BAEZ_GRANOLA_API_KEY", "env_key");
        let key = resolve_api_key(None, false).unwrap();
        assert_eq!(key, "env_key");

        env::remove_var("BAEZ_GRANOLA_API_KEY");
    }
}
