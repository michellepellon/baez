//! Vault-based storage layer with atomic writes.
//!
//! Handles paths, directory permissions, config file access, and frontmatter parsing.

use crate::{Error, Frontmatter, Result};
use crate::util::levenshtein_distance;
use chrono::{DateTime, Datelike, Utc};
use filetime::FileTime;
use std::collections::HashMap;
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

/// In-memory index of People/ notes for fuzzy name matching.
pub struct PeopleIndex {
    /// Maps lowercase canonical name → (original_case_name, file_path)
    entries: HashMap<String, (String, PathBuf)>,
    /// Maps lowercase alias → lowercase canonical name
    aliases: HashMap<String, String>,
}

impl PeopleIndex {
    /// Build the index by scanning the People/ directory.
    pub fn build(people_dir: &Path) -> Self {
        let mut entries = HashMap::new();
        let mut aliases = HashMap::new();

        if !people_dir.is_dir() {
            return PeopleIndex { entries, aliases };
        }

        let dir_entries = match fs::read_dir(people_dir) {
            Ok(e) => e,
            Err(_) => return PeopleIndex { entries, aliases },
        };

        for entry in dir_entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            let canonical = match path.file_stem().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let canonical_lower = canonical.to_lowercase();

            // Try to read aliases from frontmatter
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(alias_list) = parse_aliases_from_frontmatter(&content) {
                    for alias in alias_list {
                        aliases.insert(alias.to_lowercase(), canonical_lower.clone());
                    }
                }
            }

            entries.insert(canonical_lower, (canonical, path));
        }

        PeopleIndex { entries, aliases }
    }

    /// Register a newly created person in the index (no filesystem I/O).
    pub fn add_person(&mut self, name: &str, people_dir: &Path, new_aliases: &[&str]) {
        let lower = name.to_lowercase();
        let path = people_dir.join(format!("{}.md", name));
        self.entries.insert(lower.clone(), (name.to_string(), path));
        for alias in new_aliases {
            self.aliases.insert(alias.to_lowercase(), lower.clone());
        }
    }

    /// Find a matching person for the given name.
    ///
    /// Returns `Some((canonical_name, file_path))` on match, `None` on no match or ambiguity.
    pub fn find_match(&self, name: &str, attendees: &[String]) -> Option<(String, PathBuf)> {
        let lower = name.to_lowercase();

        // 1. Exact match
        if let Some((canonical, path)) = self.entries.get(&lower) {
            return Some((canonical.clone(), path.clone()));
        }

        // 2. Alias match
        if let Some(canonical_lower) = self.aliases.get(&lower) {
            if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                return Some((canonical.clone(), path.clone()));
            }
        }

        // 3. Attendee disambiguation (first-name-only)
        if !name.contains(' ') {
            let matches: Vec<&String> = attendees
                .iter()
                .filter(|a| a.to_lowercase().starts_with(&lower))
                .collect();

            if matches.len() == 1 {
                let full_name_lower = matches[0].to_lowercase();
                // Re-try exact and alias match with the full name
                if let Some((canonical, path)) = self.entries.get(&full_name_lower) {
                    return Some((canonical.clone(), path.clone()));
                }
                if let Some(canonical_lower) = self.aliases.get(&full_name_lower) {
                    if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                        return Some((canonical.clone(), path.clone()));
                    }
                }
            }
        }

        // 4. Fuzzy match (Levenshtein)
        if lower.len() <= 5 {
            // Short names require exact match (too many false positives)
            return None;
        }

        let threshold = 2;
        let mut best_match: Option<(String, PathBuf, usize)> = None;
        let mut ambiguous = false;

        // Check against entry names
        for (entry_lower, (canonical, path)) in &self.entries {
            let dist = levenshtein_distance(&lower, entry_lower);
            if dist <= threshold {
                match &best_match {
                    Some((_, _, best_dist)) if dist < *best_dist => {
                        best_match = Some((canonical.clone(), path.clone(), dist));
                        ambiguous = false;
                    }
                    Some((_, _, best_dist)) if dist == *best_dist => {
                        ambiguous = true;
                    }
                    None => {
                        best_match = Some((canonical.clone(), path.clone(), dist));
                    }
                    _ => {}
                }
            }
        }

        // Check against aliases
        for (alias_lower, canonical_lower) in &self.aliases {
            let dist = levenshtein_distance(&lower, alias_lower);
            if dist <= threshold {
                if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                    match &best_match {
                        Some((_, _, best_dist)) if dist < *best_dist => {
                            best_match = Some((canonical.clone(), path.clone(), dist));
                            ambiguous = false;
                        }
                        Some((_, _, best_dist)) if dist == *best_dist => {
                            ambiguous = true;
                        }
                        None => {
                            best_match = Some((canonical.clone(), path.clone(), dist));
                        }
                        _ => {}
                    }
                }
            }
        }

        if ambiguous {
            return None;
        }

        best_match.map(|(name, path, _)| (name, path))
    }
}

/// Read entity note frontmatter as flexible JSON Value + body text.
/// Returns None if file doesn't exist or has no frontmatter.
pub fn read_entity_frontmatter(path: &Path) -> Result<Option<(serde_json::Value, String)>> {
    if !path.exists() {
        return Ok(None);
    }
    let content = fs::read_to_string(path)?;
    if !content.starts_with("---\n") {
        return Ok(None);
    }
    let rest = &content[4..];
    let Some(end_pos) = rest.find("\n---") else {
        return Ok(None);
    };
    let yaml = &rest[..end_pos];
    let body_start = end_pos + 4;
    let body = if body_start < rest.len() {
        let skip = if rest.as_bytes().get(body_start) == Some(&b'\n') { 1 } else { 0 };
        rest[body_start + skip..].to_string()
    } else {
        String::new()
    };
    let value: serde_json::Value = serde_yaml::from_str(yaml).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to parse entity frontmatter: {}", e),
        ))
    })?;
    Ok(Some((value, body)))
}

/// Find an entity file by name, case-insensitive.
pub fn find_entity_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let lower = name.to_lowercase();
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            let stem = path.file_stem()?.to_str()?;
            if stem.to_lowercase() == lower {
                return Some(path);
            }
        }
        None
    })
}

/// Create a new People note.
#[allow(clippy::too_many_arguments)]
pub fn create_person_note(
    people_dir: &Path, name: &str, role: Option<&str>, company: Option<&str>,
    aliases: &[&str], context: &str, meeting_slug: &str, date: &str, tmp_dir: &Path,
) -> Result<()> {
    let alias_yaml = if aliases.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", aliases.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", "))
    };
    let company_str = company.unwrap_or("Unknown");
    let role_str = role.unwrap_or("Unknown");
    let mut ctx_lines = String::new();
    if company.is_some() {
        ctx_lines.push_str(&format!("- **Company:** {}\n", company_str));
    }
    if role.is_some() {
        ctx_lines.push_str(&format!("- **Role:** {}\n", role_str));
    }
    let mut fm_extra = String::new();
    if company.is_some() {
        fm_extra.push_str(&format!("company: \"{}\"\n", company_str));
    }
    if role.is_some() {
        fm_extra.push_str(&format!("role: \"{}\"\n", role_str));
    }
    let content = format!(
        "---\ntitle: \"{name}\"\ndate: \"{date}\"\ntags: [people]\naliases: {alias_yaml}\ntype: person\n{fm_extra}last-contact: \"{date}\"\nstatus: active\nrelated:\n  - \"[[{meeting_slug}]]\"\n---\n\n# {name}\n\n## Context\n{ctx_lines}\n## Notes\n- From [[{meeting_slug}]]: {context}\n",
        name=name, date=date, alias_yaml=alias_yaml, fm_extra=fm_extra,
        meeting_slug=meeting_slug, context=context, ctx_lines=ctx_lines,
    );
    let path = people_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing People note with a new meeting reference.
pub fn enrich_person_note(
    path: &Path, new_aliases: &[&str], context: &str,
    meeting_slug: &str, date: &str, tmp_dir: &Path,
) -> Result<()> {
    let Some((mut fm, body)) = read_entity_frontmatter(path)? else {
        return Ok(());
    };
    // Update related (no duplicates)
    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        if !related.iter().any(|v| v.as_str() == Some(&meeting_ref)) {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else {
        fm["related"] = serde_json::json!([meeting_ref]);
    }
    // Update last-contact if newer
    if let Some(existing_date) = fm.get("last-contact").and_then(|v| v.as_str()) {
        if date > existing_date { fm["last-contact"] = serde_json::Value::String(date.to_string()); }
    } else {
        fm["last-contact"] = serde_json::Value::String(date.to_string());
    }
    // Merge aliases
    if !new_aliases.is_empty() {
        let existing: Vec<String> = fm.get("aliases").and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();
        let mut merged = existing;
        for alias in new_aliases {
            if !merged.iter().any(|a| a.to_lowercase() == alias.to_lowercase()) {
                merged.push(alias.to_string());
            }
        }
        fm["aliases"] = serde_json::json!(merged);
    }
    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Failed to serialize: {}", e)))
    })?;
    // Append to ## Notes section (or create it)
    let notes_bullet = format!("- From [[{}]]: {}", meeting_slug, context);
    let updated_body = if body.contains("## Notes") {
        let pos = body.find("## Notes").unwrap();
        let after = &body[pos + 8..];
        let next = after.find("\n## ");
        let insert_pos = match next { Some(p) => pos + 8 + p, None => body.len() };
        let mut new_body = body[..insert_pos].to_string();
        if !new_body.ends_with('\n') { new_body.push('\n'); }
        new_body.push_str(&notes_bullet);
        new_body.push('\n');
        new_body.push_str(&body[insert_pos..]);
        new_body
    } else {
        format!("{}\n## Notes\n{}\n", body.trim_end(), notes_bullet)
    };
    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}

/// Create a new Concept note.
pub fn create_concept_note(
    concepts_dir: &Path, name: &str, description: &str,
    meeting_slug: &str, date: &str, tmp_dir: &Path,
) -> Result<()> {
    let content = format!(
        "---\ntitle: \"{name}\"\ndate: \"{date}\"\ntags: [concept]\ntype: concept\nstatus: active\nrelated:\n  - \"[[{meeting_slug}]]\"\n---\n\n# {name}\n\n## What is it?\n{description}\n\n## Sources\n- [[{meeting_slug}]] — extracted {date}\n",
        name=name, date=date, description=description, meeting_slug=meeting_slug,
    );
    let path = concepts_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing Concept note with a new source reference.
pub fn enrich_concept_note(path: &Path, meeting_slug: &str, date: &str, tmp_dir: &Path) -> Result<()> {
    let Some((mut fm, body)) = read_entity_frontmatter(path)? else { return Ok(()); };
    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        if !related.iter().any(|v| v.as_str() == Some(&meeting_ref)) {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else { fm["related"] = serde_json::json!([meeting_ref]); }
    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Failed to serialize: {}", e)))
    })?;
    let source_line = format!("- [[{}]] — extracted {}", meeting_slug, date);
    let updated_body = if body.contains("## Sources") {
        let pos = body.find("## Sources").unwrap();
        let after = &body[pos + 10..];
        let next = after.find("\n## ");
        let insert_pos = match next { Some(p) => pos + 10 + p, None => body.len() };
        let mut new_body = body[..insert_pos].to_string();
        if !new_body.ends_with('\n') { new_body.push('\n'); }
        new_body.push_str(&source_line); new_body.push('\n');
        new_body.push_str(&body[insert_pos..]);
        new_body
    } else {
        format!("{}\n## Sources\n{}\n", body.trim_end(), source_line)
    };
    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}

/// Create a new Project note.
pub fn create_project_note(
    projects_dir: &Path, name: &str, description: &str,
    meeting_slug: &str, date: &str, tmp_dir: &Path,
) -> Result<()> {
    let content = format!(
        "---\ntitle: \"{name}\"\ndate: \"{date}\"\ntags: [project]\ntype: project\nstatus: active\nrelated:\n  - \"[[{meeting_slug}]]\"\n---\n\n# {name}\n\nMentioned in [[{meeting_slug}]]: {description}\n",
        name=name, date=date, description=description, meeting_slug=meeting_slug,
    );
    let path = projects_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing Project note with a new mention.
pub fn enrich_project_note(path: &Path, description: &str, meeting_slug: &str, tmp_dir: &Path) -> Result<()> {
    let Some((mut fm, body)) = read_entity_frontmatter(path)? else { return Ok(()); };
    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        if !related.iter().any(|v| v.as_str() == Some(&meeting_ref)) {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else { fm["related"] = serde_json::json!([meeting_ref]); }
    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Failed to serialize: {}", e)))
    })?;
    let mention_line = format!("\nMentioned in [[{}]]: {}", meeting_slug, description);
    let updated_body = format!("{}{}\n", body.trim_end(), mention_line);
    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}

/// Extract the `aliases` array from YAML frontmatter in a markdown file.
fn parse_aliases_from_frontmatter(content: &str) -> Option<Vec<String>> {
    if !content.starts_with("---\n") {
        return None;
    }
    let rest = &content[4..];
    let end_pos = rest.find("\n---")?;
    let yaml = &rest[..end_pos];

    let value: serde_json::Value = serde_yaml::from_str(yaml).ok()?;
    let aliases = value.get("aliases")?.as_array()?;
    Some(
        aliases
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
    )
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

#[cfg(test)]
mod people_index_tests {
    use super::*;
    use tempfile::TempDir;

    fn write_person(dir: &Path, name: &str, aliases: &[&str]) {
        let alias_yaml = if aliases.is_empty() {
            "aliases: []".to_string()
        } else {
            format!("aliases: [{}]", aliases.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", "))
        };
        let content = format!(
            "---\ntitle: \"{}\"\n{}\ntype: person\n---\n\n# {}\n",
            name, alias_yaml, name
        );
        fs::write(dir.join(format!("{}.md", name)), content).unwrap();
    }

    #[test]
    fn test_build_empty_dir() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        let index = PeopleIndex::build(&people_dir);
        assert!(index.find_match("Alice", &[]).is_none());
    }

    #[test]
    fn test_exact_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Alice Smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_case_insensitive_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("alice smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_alias_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Dennis Crowley", &["Dens", "DC"]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Dens", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Dennis Crowley");
    }

    #[test]
    fn test_attendee_disambiguation() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let attendees = vec!["Alice Smith".to_string(), "Bob Jones".to_string()];
        let result = index.find_match("Alice", &attendees);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_attendee_disambiguation_ambiguous() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        write_person(&people_dir, "Alice Jones", &[]);
        let index = PeopleIndex::build(&people_dir);
        let attendees = vec!["Alice Smith".to_string(), "Alice Jones".to_string()];
        let result = index.find_match("Alice", &attendees);
        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_match_within_threshold() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Alce Smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_fuzzy_match_beyond_threshold() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Bob Johnson", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_short_name_requires_exact() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Bob", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Rob", &[]);
        assert!(result.is_none());
        let result = index.find_match("Bob", &[]);
        assert!(result.is_some());
    }

    #[test]
    fn test_no_match_returns_none() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Totally Different Person", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_add_person_updates_index() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        let mut index = PeopleIndex::build(&people_dir);
        assert!(index.find_match("New Person", &[]).is_none());
        index.add_person("New Person", &people_dir, &["NP"]);
        let result = index.find_match("New Person", &[]);
        assert!(result.is_some());
        let result = index.find_match("NP", &[]);
        assert!(result.is_some());
    }
}

#[cfg(test)]
mod entity_note_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_entity_frontmatter_valid() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.md");
        fs::write(&path, "---\ntitle: \"Test\"\ntype: person\nrelated: []\n---\n\n# Test\n\nBody content.\n").unwrap();
        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_some());
        let (fm, body) = result.unwrap();
        assert_eq!(fm["title"].as_str(), Some("Test"));
        assert!(body.contains("Body content."));
    }

    #[test]
    fn test_read_entity_frontmatter_missing() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("missing.md");
        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_entity_frontmatter_no_yaml() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("plain.md");
        fs::write(&path, "# Just content\n").unwrap();
        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_create_person_note() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        create_person_note(&people_dir, "Alice Smith", Some("Engineer"), Some("Acme Corp"), &["Alice"], "Led API discussion", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let path = people_dir.join("Alice Smith.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"Alice Smith\""));
        assert!(content.contains("company: \"Acme Corp\""));
        assert!(content.contains("role: \"Engineer\""));
        assert!(content.contains("[[2025-01-15_standup]]"));
        assert!(content.contains("Led API discussion"));
    }

    #[test]
    fn test_enrich_person_note_adds_related() {
        let temp = TempDir::new().unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let path = temp.path().join("Alice Smith.md");
        let initial = "---\ntitle: \"Alice Smith\"\ntype: person\nrelated:\n  - \"[[2025-01-10_meeting]]\"\nlast-contact: \"2025-01-10\"\n---\n\n# Alice Smith\n\n## Notes\n- From [[2025-01-10_meeting]]: Initial context\n";
        fs::write(&path, initial).unwrap();
        enrich_person_note(&path, &["New Alias"], "Discussed migration", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[[2025-01-10_meeting]]"));
        assert!(content.contains("[[2025-01-15_standup]]"));
        assert!(content.contains("Discussed migration"));
        assert!(content.contains("last-contact: 2025-01-15") || content.contains("last-contact: \"2025-01-15\""));
    }

    #[test]
    fn test_enrich_person_note_no_duplicate_related() {
        let temp = TempDir::new().unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let path = temp.path().join("test.md");
        let initial = "---\ntitle: \"Alice\"\ntype: person\nrelated:\n  - \"[[2025-01-15_standup]]\"\nlast-contact: \"2025-01-15\"\n---\n\n# Alice\n\n## Notes\n- From [[2025-01-15_standup]]: Initial note\n";
        fs::write(&path, &initial).unwrap();
        enrich_person_note(&path, &[], "Again", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        // related has 1 entry (no dup), Notes has original + new bullet = 2 mentions, total = 3
        let count = content.matches("2025-01-15_standup").count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_enrich_person_note_missing_notes_section() {
        let temp = TempDir::new().unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let path = temp.path().join("test.md");
        let initial = "---\ntitle: \"Alice\"\ntype: person\nrelated: []\n---\n\n# Alice\n\n## Context\n- Engineer\n";
        fs::write(&path, &initial).unwrap();
        enrich_person_note(&path, &[], "New context", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("## Notes\n"));
        assert!(content.contains("New context"));
    }

    #[test]
    fn test_create_concept_note() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Concepts");
        fs::create_dir_all(&dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        create_concept_note(&dir, "API-First Design", "Building APIs before UIs", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let path = dir.join("API-First Design.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"API-First Design\""));
        assert!(content.contains("Building APIs before UIs"));
        assert!(content.contains("[[2025-01-15_standup]]"));
    }

    #[test]
    fn test_create_project_note() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Projects");
        fs::create_dir_all(&dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        create_project_note(&dir, "Project Atlas", "Internal migration tool", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();
        let path = dir.join("Project Atlas.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"Project Atlas\""));
        assert!(content.contains("Internal migration tool"));
    }

    #[test]
    fn test_find_entity_file_case_insensitive() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Concepts");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("API-First Design.md"), "# test").unwrap();
        let result = find_entity_file(&dir, "api-first design");
        assert!(result.is_some());
    }

    #[test]
    fn test_find_entity_file_no_match() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Concepts");
        fs::create_dir_all(&dir).unwrap();
        let result = find_entity_file(&dir, "Nonexistent");
        assert!(result.is_none());
    }
}
