//! Vault-based storage layer with atomic writes.
//!
//! Handles paths, directory permissions, config file access, and frontmatter parsing.

use crate::{Error, Frontmatter, Result};
use chrono::{DateTime, Datelike, Utc};
use filetime::FileTime;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};

/// Vault layout:
///
/// ```text
/// Vault/                          <- user's Obsidian vault root
/// └── Granola/                    <- baez's subfolder
///     ├── 2025/
///     │   ├── 01/
///     │   │   ├── 2025-01-15_standup.md
///     │   │   └── 2025-01-16_planning.md
///     │   └── 02/
///     │       └── ...
///     └── .baez/                  <- hidden internal state
///         ├── raw/                <- raw JSON API responses
///         ├── summaries/          <- AI-generated summary files
///         ├── tmp/                <- atomic write temp dir
///         └── .sync_cache.json    <- sync cache
/// ```
pub struct Paths {
    /// Obsidian vault root directory
    pub vault_dir: PathBuf,
    /// Granola subfolder inside the vault (vault_dir/Granola)
    pub granola_dir: PathBuf,
    /// Hidden internal state directory (granola_dir/.baez)
    pub baez_dir: PathBuf,
    /// Raw JSON API responses (baez_dir/raw)
    pub raw_dir: PathBuf,
    /// AI-generated summary files (baez_dir/summaries)
    pub summaries_dir: PathBuf,
    /// Atomic write temp directory (baez_dir/tmp)
    pub tmp_dir: PathBuf,
}

impl Paths {
    /// Create a new Paths from a vault directory.
    ///
    /// `vault_override` takes precedence. If None, resolution falls back to:
    /// 1. `BAEZ_VAULT` env var
    /// 2. Config file at `~/.config/baez/config.json`
    /// 3. Default: `~/.local/share/baez` (for backward compat / testing)
    pub fn new(vault_override: Option<PathBuf>) -> Result<Self> {
        let vault_dir = if let Some(dir) = vault_override {
            dir
        } else if let Ok(vault_env) = env::var("BAEZ_VAULT") {
            PathBuf::from(vault_env)
        } else if let Some(config_vault) = Self::vault_from_config()? {
            config_vault
        } else {
            // Fallback: XDG data home
            let base = if let Ok(xdg_data) = env::var("XDG_DATA_HOME") {
                PathBuf::from(xdg_data)
            } else {
                let home = env::var("HOME").map_err(|_| {
                    Error::Filesystem(std::io::Error::new(
                        std::io::ErrorKind::NotFound,
                        "Could not determine home directory (HOME not set)",
                    ))
                })?;
                PathBuf::from(home).join(".local").join("share")
            };
            base.join("baez")
        };

        let granola_dir = vault_dir.join("Granola");
        let baez_dir = granola_dir.join(".baez");

        Ok(Paths {
            raw_dir: baez_dir.join("raw"),
            summaries_dir: baez_dir.join("summaries"),
            tmp_dir: baez_dir.join("tmp"),
            baez_dir,
            granola_dir,
            vault_dir,
        })
    }

    /// Try to read vault path from config file at ~/.config/baez/config.json
    fn vault_from_config() -> Result<Option<PathBuf>> {
        Ok(read_config_field("vault")?.map(PathBuf::from))
    }

    /// Compute the output path for a document based on its creation date and slug.
    /// Returns: `granola_dir/YYYY/MM/YYYY-MM-DD_slug.md`
    pub fn doc_path(&self, created_at: &DateTime<Utc>, slug: &str) -> PathBuf {
        let year = format!("{:04}", created_at.year());
        let month = format!("{:02}", created_at.month());
        let date = created_at.format("%Y-%m-%d").to_string();
        self.granola_dir
            .join(&year)
            .join(&month)
            .join(format!("{}_{}.md", date, slug))
    }

    /// Create all directories in the vault layout, setting restricted permissions on internal dirs.
    pub fn ensure_dirs(&self) -> Result<()> {
        // Vault-visible directories: normal permissions (Obsidian needs to read them)
        fs::create_dir_all(&self.granola_dir)?;

        // Internal .baez directories: restricted permissions
        for dir in &[
            &self.baez_dir,
            &self.raw_dir,
            &self.summaries_dir,
            &self.tmp_dir,
        ] {
            fs::create_dir_all(dir)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = fs::Permissions::from_mode(0o700);
                fs::set_permissions(dir, perms)?;
            }
        }
        Ok(())
    }
}

/// Resolve the path to `~/.config/baez/config.json`, respecting `XDG_CONFIG_HOME`.
fn config_file_path() -> Option<PathBuf> {
    let config_dir = if let Ok(xdg_config) = env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config)
    } else {
        let home = env::var("HOME").unwrap_or_default();
        if home.is_empty() {
            return None;
        }
        PathBuf::from(home).join(".config")
    };
    let path = config_dir.join("baez").join("config.json");
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Read a string field from `~/.config/baez/config.json`.
///
/// Returns `Ok(None)` if the config file doesn't exist or the field is absent.
pub fn read_config_field(field: &str) -> Result<Option<String>> {
    let config_path = match config_file_path() {
        Some(p) => p,
        None => return Ok(None),
    };

    let content = fs::read_to_string(&config_path)?;
    let value: serde_json::Value = serde_json::from_str(&content)?;
    Ok(value
        .get(field)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string()))
}

/// Warn if `~/.config/baez/config.json` contains secrets and has loose permissions.
pub fn warn_config_permissions() {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        let config_path = match config_file_path() {
            Some(p) => p,
            None => return,
        };

        let content = match fs::read_to_string(&config_path) {
            Ok(c) => c,
            Err(_) => return,
        };
        let has_secrets =
            content.contains("granola_token") || content.contains("anthropic_api_key");
        if !has_secrets {
            return;
        }

        if let Ok(metadata) = fs::metadata(&config_path) {
            let mode = metadata.permissions().mode();
            if mode & 0o077 != 0 {
                eprintln!(
                    "Warning: {} contains API keys and is accessible by other users (mode {:o}). \
                     Fix with: chmod 600 {}",
                    config_path.display(),
                    mode & 0o777,
                    config_path.display()
                );
            }
        }
    }
}

/// Write `content` to `path` atomically via a temp file in `tmp_dir`.
///
/// Sets file permissions to `0o600` (owner-only) on Unix.
pub fn write_atomic(path: &Path, content: &[u8], tmp_dir: &Path) -> Result<()> {
    use rand::Rng;

    // Create temp file
    let random: u32 = rand::thread_rng().gen();
    let tmp_path = tmp_dir.join(format!("{:x}.part", random));

    // Write to temp
    fs::write(&tmp_path, content)?;

    // Set permissions
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(&tmp_path, perms)?;
    }

    // Atomic rename
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::rename(&tmp_path, path)?;

    Ok(())
}

/// Set file modification time to match a given datetime
pub fn set_file_time(path: &Path, datetime: &DateTime<Utc>) -> Result<()> {
    let timestamp = datetime.timestamp();
    let filetime = FileTime::from_unix_time(timestamp, 0);
    filetime::set_file_mtime(path, filetime).map_err(|e| {
        Error::Filesystem(std::io::Error::other(format!(
            "Failed to set file time: {}",
            e
        )))
    })
}

/// Parse YAML frontmatter from a markdown file, returning `None` if absent.
pub fn read_frontmatter(md_path: &Path) -> Result<Option<Frontmatter>> {
    if !md_path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(md_path)?;

    // Look for YAML frontmatter (--- ... ---)
    if !content.starts_with("---\n") {
        return Ok(None);
    }

    if content.len() < 4 {
        return Ok(None);
    }
    let rest = &content[4..];
    if let Some(end_pos) = rest.find("\n---\n") {
        let yaml = &rest[..end_pos];
        let fm: Frontmatter = serde_yaml::from_str(yaml).map_err(|e| {
            Error::Filesystem(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse frontmatter: {}", e),
            ))
        })?;
        Ok(Some(fm))
    } else {
        Ok(None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_paths_new_with_override() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        assert_eq!(paths.vault_dir, temp.path());
        assert_eq!(paths.granola_dir, temp.path().join("Granola"));
        assert_eq!(paths.baez_dir, temp.path().join("Granola").join(".baez"));
        assert_eq!(
            paths.raw_dir,
            temp.path().join("Granola").join(".baez").join("raw")
        );
    }

    #[test]
    fn test_ensure_dirs_creates_structure() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        assert!(paths.granola_dir.exists());
        assert!(paths.baez_dir.exists());
        assert!(paths.raw_dir.exists());
        assert!(paths.summaries_dir.exists());
        assert!(paths.tmp_dir.exists());
    }

    #[test]
    #[cfg(unix)]
    fn test_ensure_dirs_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        // .baez dirs should have restricted permissions
        let perms = fs::metadata(&paths.raw_dir).unwrap().permissions();
        assert_eq!(
            perms.mode() & 0o777,
            0o700,
            "raw_dir should have 0o700 permissions"
        );
    }

    #[test]
    fn test_doc_path_generates_date_folders() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();

        let created_at: DateTime<Utc> = "2025-01-15T10:00:00Z".parse().unwrap();
        let path = paths.doc_path(&created_at, "standup");

        assert_eq!(
            path,
            temp.path()
                .join("Granola")
                .join("2025")
                .join("01")
                .join("2025-01-15_standup.md")
        );
    }
}

#[cfg(test)]
mod write_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_atomic_creates_file() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("test.txt");
        write_atomic(&target, b"hello", &paths.tmp_dir).unwrap();

        assert!(target.exists());
        assert_eq!(fs::read_to_string(&target).unwrap(), "hello");
    }

    #[test]
    #[cfg(unix)]
    fn test_write_atomic_sets_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("test.txt");
        write_atomic(&target, b"hello", &paths.tmp_dir).unwrap();

        let perms = fs::metadata(&target).unwrap().permissions();
        assert_eq!(perms.mode() & 0o777, 0o600);
    }
}

#[cfg(test)]
mod frontmatter_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_frontmatter_valid() {
        let temp = TempDir::new().unwrap();
        let md_path = temp.path().join("test.md");

        let content = r#"---
doc_id: "doc123"
source: "granola"
created_at: "2025-10-28T15:04:05Z"
title: "Test"
attendees: []
tags: []
generator: "baez"
---

# Test Meeting
"#;
        fs::write(&md_path, content).unwrap();

        let fm = read_frontmatter(&md_path).unwrap();
        assert!(fm.is_some());
        assert_eq!(fm.unwrap().doc_id, "doc123");
    }

    #[test]
    fn test_read_frontmatter_missing_file() {
        let temp = TempDir::new().unwrap();
        let md_path = temp.path().join("missing.md");

        let fm = read_frontmatter(&md_path).unwrap();
        assert!(fm.is_none());
    }

    #[test]
    fn test_read_frontmatter_no_yaml() {
        let temp = TempDir::new().unwrap();
        let md_path = temp.path().join("test.md");
        fs::write(&md_path, "# Just content").unwrap();

        let fm = read_frontmatter(&md_path).unwrap();
        assert!(fm.is_none());
    }

    #[test]
    fn test_read_frontmatter_backward_compat() {
        let temp = TempDir::new().unwrap();
        let md_path = temp.path().join("test.md");

        // Old format with participants/labels/created_at should still parse
        let content = r#"---
doc_id: "doc123"
source: "granola"
created_at: "2025-10-28T15:04:05Z"
title: "Test"
participants: ["Alice"]
labels: ["planning"]
generator: "muesli 1.0"
---

# Test Meeting
"#;
        fs::write(&md_path, content).unwrap();

        let fm = read_frontmatter(&md_path).unwrap();
        assert!(fm.is_some());
        let fm = fm.unwrap();
        assert_eq!(fm.doc_id, "doc123");
        assert_eq!(fm.attendees, vec!["Alice"]);
        assert_eq!(fm.tags, vec!["planning"]);
    }
}
