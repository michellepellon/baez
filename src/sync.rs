//! Core sync logic for fetching and storing documents.
//!
//! Handles incremental update detection, cache management, and progress reporting.

use crate::{
    api::ApiClient,
    convert::to_markdown,
    storage::{read_frontmatter, set_file_time, write_atomic, Paths},
    util::slugify,
    Result,
};
use chrono::{DateTime, Utc};
use indicatif::{ProgressBar, ProgressStyle};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Serialize, Deserialize)]
struct CacheEntry {
    filename: String,
    updated_at: DateTime<Utc>,
}

/// Load the sync cache (doc_id -> metadata)
fn load_cache(cache_path: &std::path::Path) -> HashMap<String, CacheEntry> {
    if !cache_path.exists() {
        return HashMap::new();
    }

    std::fs::read_to_string(cache_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Save the sync cache atomically
fn save_cache(
    cache_path: &std::path::Path,
    cache: &HashMap<String, CacheEntry>,
    tmp_dir: &std::path::Path,
) -> Result<()> {
    let json = serde_json::to_string_pretty(cache)?;
    write_atomic(cache_path, json.as_bytes(), tmp_dir)?;
    Ok(())
}

/// Sync all documents from the Granola API into the vault.
///
/// Fetches the document list, compares against a local cache, and writes
/// markdown + raw JSON for any new or updated documents.
pub fn sync_all(
    client: &ApiClient,
    paths: &Paths,
    force: bool,
    summarize: bool,
    verbose: bool,
    dry_run: bool,
) -> Result<()> {
    paths.ensure_dirs()?;

    // Set up summarization state if enabled
    #[cfg(not(feature = "summaries"))]
    {
        let _ = summarize;
        let _ = verbose;
    }
    #[cfg(feature = "summaries")]
    let summarize_state: Option<(
        crate::summary::SummaryConfig,
        String,
        reqwest::blocking::Client,
    )> = if summarize {
        let config_path = paths.baez_dir.join("summary_config.json");
        let config = crate::summary::SummaryConfig::load(&config_path)?;
        match crate::summary::get_api_key_verbose(verbose) {
            Some(key) => {
                let claude_client = crate::summary::build_claude_client()?;
                println!("Summarization enabled (model: {})", config.model);
                Some((config, key, claude_client))
            }
            None => {
                eprintln!("Warning: No Anthropic API key found. Set BAEZ_ANTHROPIC_API_KEY, ANTHROPIC_API_KEY, add anthropic_api_key to ~/.config/baez/config.json, or run `baez set-api-key`. Skipping summarization.");
                None
            }
        }
    } else {
        None
    };

    if dry_run {
        println!("Dry run — no files will be written");
    }
    if force {
        println!("Force sync enabled — ignoring cache timestamps");
    }
    println!("Fetching document list...");
    let docs = client.list_documents_with_notes()?;

    // Diagnostic: count notes availability
    let with_user = docs.iter().filter(|d| d.user_notes().is_some()).count();
    println!(
        "Notes: {} with user notes (of {} total)",
        with_user,
        docs.len(),
    );

    // Load the sync cache
    let cache_path = paths.baez_dir.join(".sync_cache.json");
    let mut cache = load_cache(&cache_path);

    let pb = ProgressBar::new(docs.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} docs")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut synced = 0;
    let mut skipped = 0;
    #[cfg(feature = "summaries")]
    let mut summarized = 0u32;

    for doc_summary in &docs {
        // Check cache for quick timestamp comparison (--force bypasses cache)
        let should_update = if force {
            true
        } else if let Some(cache_entry) = cache.get(&doc_summary.id) {
            let remote_ts = doc_summary.updated_at.unwrap_or(doc_summary.created_at);
            remote_ts > cache_entry.updated_at
        } else {
            true
        };

        if !should_update {
            skipped += 1;
            pb.inc(1);
            continue;
        }

        if dry_run {
            let title = doc_summary.title.as_deref().unwrap_or("(untitled)");
            let date = doc_summary.created_at.format("%Y-%m-%d");
            println!("  would sync: {} — {}", date, title);
            synced += 1;
            pb.inc(1);
            continue;
        }

        // Fetch metadata and transcript from API, keeping raw responses
        let meta_resp = client.get_metadata_with_raw(&doc_summary.id)?;
        let transcript_resp = client.get_transcript_with_raw(&doc_summary.id)?;
        let mut meta = meta_resp.parsed;
        let transcript = transcript_resp.parsed;

        // The metadata endpoint sometimes omits created_at; prefer the
        // summary's value which the list endpoint always provides.
        meta.created_at = doc_summary.created_at;

        // Extract user notes from panels (my_notes -> notes field -> last_viewed_panel fallback)
        let notes_md = doc_summary
            .user_notes()
            .as_ref()
            .map(crate::convert::prosemirror_to_markdown)
            .filter(|s| !s.is_empty());

        // AI summarization via Claude API
        #[cfg(feature = "summaries")]
        let summary_text: Option<String> =
            if let Some((ref config, ref key, ref claude_client)) = summarize_state {
                let input = crate::summary::format_transcript_for_llm(&transcript, &meta);
                match crate::summary::summarize_transcript(&input, key, config, claude_client, "") {
                    Ok(s) => {
                        summarized += 1;
                        Some(s)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to summarize {}: {}", doc_summary.id, e);
                        None
                    }
                }
            } else {
                None
            };

        #[cfg(not(feature = "summaries"))]
        let summary_text: Option<String> = None;

        // Convert to markdown
        let md = to_markdown(
            &transcript,
            &meta,
            &doc_summary.id,
            notes_md.as_deref(),
            summary_text.as_deref(),
            vec![],
            None,
        )?;

        let full_md = format!("---\n{}---\n\n{}", md.frontmatter_yaml, md.body);

        // Compute filename and date-based path
        let slug = slugify(meta.title.as_deref().unwrap_or("untitled"));
        let doc_path = paths.doc_path(&meta.created_at, &slug);
        let date = meta.created_at.format("%Y-%m-%d").to_string();
        let base_filename = format!("{}_{}", date, slug);

        // If filename changed in cache, remove old files
        if let Some(old_entry) = cache.get(&doc_summary.id) {
            if old_entry.filename != base_filename {
                // Old files could be in flat transcripts_dir or date-based paths
                let old_path = paths.granola_dir.join(format!("{}.md", old_entry.filename));
                if old_path.exists() {
                    std::fs::remove_file(&old_path)?;
                }
                // Clean up all raw file variants
                for suffix in &["", "_transcript", "_metadata"] {
                    let old_json = paths
                        .raw_dir
                        .join(format!("{}{}.json", old_entry.filename, suffix));
                    if old_json.exists() {
                        std::fs::remove_file(&old_json)?;
                    }
                }
            }
        }

        // Write files: save verbatim API responses as raw JSON
        let transcript_json_path = paths
            .raw_dir
            .join(format!("{}_transcript.json", base_filename));
        let metadata_json_path = paths
            .raw_dir
            .join(format!("{}_metadata.json", base_filename));

        write_atomic(
            &transcript_json_path,
            transcript_resp.raw.as_bytes(),
            &paths.tmp_dir,
        )?;
        write_atomic(
            &metadata_json_path,
            meta_resp.raw.as_bytes(),
            &paths.tmp_dir,
        )?;
        write_atomic(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;

        // Remove legacy single .json file if it exists
        let legacy_json = paths.raw_dir.join(format!("{}.json", base_filename));
        if legacy_json.exists() {
            std::fs::remove_file(&legacy_json)?;
        }

        // Set file modification time to meeting creation date
        set_file_time(&transcript_json_path, &meta.created_at)?;
        set_file_time(&metadata_json_path, &meta.created_at)?;
        set_file_time(&doc_path, &meta.created_at)?;

        // Update cache - CRITICAL: store the same timestamp we compare against
        // (doc_summary.updated_at, NOT meta.updated_at - they can differ!)
        let stored_ts = doc_summary.updated_at.unwrap_or(doc_summary.created_at);

        cache.insert(
            doc_summary.id.clone(),
            CacheEntry {
                filename: base_filename.clone(),
                updated_at: stored_ts,
            },
        );
        save_cache(&cache_path, &cache, &paths.tmp_dir)?;

        synced += 1;
        pb.inc(1);
    }

    #[cfg(feature = "summaries")]
    let stats_msg = format!(
        "synced {} docs ({} new/updated, {} skipped, {} summarized)",
        docs.len(),
        synced,
        skipped,
        summarized
    );
    #[cfg(not(feature = "summaries"))]
    let stats_msg = format!(
        "synced {} docs ({} new/updated, {} skipped)",
        docs.len(),
        synced,
        skipped
    );
    pb.finish_with_message(stats_msg);

    Ok(())
}

/// Fix file modification dates for all existing files to match meeting creation dates.
/// Walks the date-based directory tree recursively.
pub fn fix_dates(paths: &Paths) -> Result<()> {
    println!("Fixing file modification dates...");

    let mut fixed = 0;
    let mut failed = 0;

    // Walk the granola_dir recursively for .md files
    walk_md_files(&paths.granola_dir, &mut |path| {
        let frontmatter = match read_frontmatter(&path)? {
            Some(fm) => fm,
            None => {
                eprintln!("Warning: Skipping {} (no frontmatter)", path.display());
                failed += 1;
                return Ok(());
            }
        };

        // Set the file time
        match set_file_time(&path, &frontmatter.created_at()) {
            Ok(_) => {
                // Also fix corresponding JSON files if they exist
                let filename = path.file_stem().unwrap().to_str().unwrap();
                for suffix in &["_transcript", "_metadata", ""] {
                    let json_path = paths.raw_dir.join(format!("{}{}.json", filename, suffix));
                    if json_path.exists() {
                        if let Err(e) = set_file_time(&json_path, &frontmatter.created_at()) {
                            eprintln!(
                                "Warning: Failed to set time for {}: {}",
                                json_path.display(),
                                e
                            );
                        }
                    }
                }
                fixed += 1;
            }
            Err(e) => {
                eprintln!("Warning: Failed to set time for {}: {}", path.display(), e);
                failed += 1;
            }
        }
        Ok(())
    })?;

    println!("Fixed dates for {} files", fixed);
    if failed > 0 {
        println!("{} files failed", failed);
    }

    Ok(())
}

/// Recursively walk a directory tree, calling `f` on every .md file found.
fn walk_md_files(
    dir: &std::path::Path,
    f: &mut dyn FnMut(std::path::PathBuf) -> Result<()>,
) -> Result<()> {
    use std::fs;

    if !dir.exists() {
        return Ok(());
    }

    let entries = fs::read_dir(dir).map_err(crate::Error::Filesystem)?;

    for entry in entries {
        let entry = entry.map_err(crate::Error::Filesystem)?;
        let path = entry.path();

        // Skip hidden directories (like .baez)
        if path.is_dir() {
            if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
                if name.starts_with('.') {
                    continue;
                }
            }
            walk_md_files(&path, f)?;
        } else if path.extension().and_then(|s| s.to_str()) == Some("md") {
            f(path)?;
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::storage::Paths;
    use tempfile::TempDir;

    #[test]
    fn test_sync_creates_directory_structure() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();

        paths.ensure_dirs().unwrap();

        assert!(
            paths.raw_dir.exists(),
            "raw_dir should exist at {}",
            paths.raw_dir.display()
        );
        assert!(
            paths.baez_dir.exists(),
            "baez_dir should exist at {}",
            paths.baez_dir.display()
        );
    }
}
