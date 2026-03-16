//! Core sync logic for fetching and storing documents.
//!
//! Handles incremental update detection, cache management, and progress reporting.

use crate::{
    api::ApiClient,
    convert::to_markdown,
    storage::{read_frontmatter, set_file_time, write_atomic, Paths},
    util::count_transcript_words,
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

#[cfg(feature = "summaries")]
#[derive(Serialize, Deserialize)]
pub(crate) struct SummaryCacheEntry {
    pub summarized_at: DateTime<Utc>,
    pub model: String,
}

#[cfg(feature = "summaries")]
pub(crate) fn load_summary_cache(
    cache_path: &std::path::Path,
) -> HashMap<String, SummaryCacheEntry> {
    if !cache_path.exists() {
        return HashMap::new();
    }

    std::fs::read_to_string(cache_path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(feature = "summaries")]
pub(crate) fn save_summary_cache(
    cache_path: &std::path::Path,
    cache: &HashMap<String, SummaryCacheEntry>,
    tmp_dir: &std::path::Path,
) -> Result<()> {
    let json = serde_json::to_string_pretty(cache)?;
    write_atomic(cache_path, json.as_bytes(), tmp_dir)?;
    Ok(())
}

#[cfg(feature = "summaries")]
pub(crate) struct SummarizationContext<'a> {
    pub config: &'a crate::summary::SummaryConfig,
    pub api_key: &'a str,
    pub client: &'a reqwest::blocking::Client,
    pub context_preamble: &'a mut String,
    pub paths: &'a Paths,
    pub people_index: &'a mut crate::storage::PeopleIndex,
    pub summary_cache: &'a mut HashMap<String, SummaryCacheEntry>,
    pub summary_cache_path: &'a std::path::Path,
    pub dry_run: bool,
}

/// Summarize a document and reconcile extracted entities.
///
/// Returns the summary text and extracted entities on success.
/// Updates the summary cache after successful summarization.
#[cfg(feature = "summaries")]
pub(crate) fn summarize_and_reconcile(
    ctx: &mut SummarizationContext,
    doc_id: &str,
    transcript: &crate::model::RawTranscript,
    meta: &crate::model::DocumentMetadata,
    slug: &str,
    attendee_names: &[String],
) -> Result<(Option<String>, Option<crate::summary::ExtractedEntities>)> {
    let word_count = crate::util::count_transcript_words(transcript);
    if word_count < 20 {
        return Ok((None, None));
    }

    let input = crate::summary::format_transcript_for_llm(transcript, meta);
    let (summary_text, extracted_entities) = match crate::summary::summarize_transcript(
        &input,
        ctx.api_key,
        ctx.config,
        ctx.client,
        ctx.context_preamble,
    ) {
        Ok(raw_summary) => {
            let (clean_md, entities) = crate::summary::parse_summary_output(&raw_summary);
            (Some(clean_md), entities)
        }
        Err(e) => {
            eprintln!("Warning: Failed to summarize {}: {}", doc_id, e);
            return Ok((None, None));
        }
    };

    // Entity reconciliation
    if let Some(ref entities) = extracted_entities {
        if !ctx.dry_run {
            let date = meta.created_at.format("%Y-%m-%d").to_string();
            let meeting_slug = format!("{}_{}", date, slug);

            // People
            for person in &entities.people {
                let match_result = ctx.people_index.find_match(&person.name, attendee_names);
                if let Some((canonical, existing_path)) = match_result {
                    let alias_refs: Vec<&str> = person.aliases.iter().map(|s| s.as_str()).collect();
                    if let Err(e) = crate::storage::enrich_person_note(
                        &existing_path,
                        &alias_refs,
                        &person.context,
                        &meeting_slug,
                        &date,
                        &ctx.paths.tmp_dir,
                    ) {
                        eprintln!("Warning: Failed to enrich People/{}: {}", canonical, e);
                    }
                } else if person.name.contains(' ') {
                    let people_dir = ctx.paths.vault_dir.join("People");
                    let alias_refs: Vec<&str> = person.aliases.iter().map(|s| s.as_str()).collect();
                    if let Err(e) = crate::storage::create_person_note(
                        &people_dir,
                        &person.name,
                        person.role.as_deref(),
                        person.company.as_deref(),
                        &alias_refs,
                        &person.context,
                        &meeting_slug,
                        &date,
                        &ctx.paths.tmp_dir,
                    ) {
                        eprintln!("Warning: Failed to create People/{}: {}", person.name, e);
                    } else {
                        ctx.people_index
                            .add_person(&person.name, &people_dir, &alias_refs);
                    }
                }
            }

            // Concepts
            let concepts_dir = ctx.paths.vault_dir.join("Concepts");
            for concept in &entities.concepts {
                let existing = crate::storage::find_entity_file(&concepts_dir, &concept.name);
                if let Some(existing_path) = existing {
                    if let Err(e) = crate::storage::enrich_concept_note(
                        &existing_path,
                        &meeting_slug,
                        &date,
                        &ctx.paths.tmp_dir,
                    ) {
                        eprintln!("Warning: Failed to enrich Concepts/{}: {}", concept.name, e);
                    }
                } else if let Err(e) = crate::storage::create_concept_note(
                    &concepts_dir,
                    &concept.name,
                    &concept.description,
                    &meeting_slug,
                    &date,
                    &ctx.paths.tmp_dir,
                ) {
                    eprintln!("Warning: Failed to create Concepts/{}: {}", concept.name, e);
                } else {
                    *ctx.context_preamble =
                        crate::summary::build_context_preamble(&ctx.paths.vault_dir);
                }
            }

            // Projects
            let projects_dir = ctx.paths.vault_dir.join("Projects");
            for project in &entities.projects {
                let existing = crate::storage::find_entity_file(&projects_dir, &project.name);
                if let Some(existing_path) = existing {
                    if let Err(e) = crate::storage::enrich_project_note(
                        &existing_path,
                        &project.description,
                        &meeting_slug,
                        &ctx.paths.tmp_dir,
                    ) {
                        eprintln!("Warning: Failed to enrich Projects/{}: {}", project.name, e);
                    }
                } else if let Err(e) = crate::storage::create_project_note(
                    &projects_dir,
                    &project.name,
                    &project.description,
                    &meeting_slug,
                    &date,
                    &ctx.paths.tmp_dir,
                ) {
                    eprintln!("Warning: Failed to create Projects/{}: {}", project.name, e);
                } else {
                    *ctx.context_preamble =
                        crate::summary::build_context_preamble(&ctx.paths.vault_dir);
                }
            }
        }
    }

    // Update summary cache
    if summary_text.is_some() && !ctx.dry_run {
        ctx.summary_cache.insert(
            doc_id.to_string(),
            SummaryCacheEntry {
                summarized_at: Utc::now(),
                model: ctx.config.model.clone(),
            },
        );
        save_summary_cache(
            ctx.summary_cache_path,
            ctx.summary_cache,
            &ctx.paths.tmp_dir,
        )?;
    }

    Ok((summary_text, extracted_entities))
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

    // Create entity directories only when summarization is enabled AND an API key is available
    #[cfg(feature = "summaries")]
    let entity_dirs_ready = summarize_state.is_some();
    #[cfg(feature = "summaries")]
    if entity_dirs_ready {
        let _ = std::fs::create_dir_all(paths.vault_dir.join("People"));
        let _ = std::fs::create_dir_all(paths.vault_dir.join("Concepts"));
        let _ = std::fs::create_dir_all(paths.vault_dir.join("Projects"));
    }
    #[cfg(not(feature = "summaries"))]
    let _entity_dirs_ready = false;

    // Build PeopleIndex and context preamble once
    #[cfg(feature = "summaries")]
    let mut people_index = if entity_dirs_ready {
        crate::storage::PeopleIndex::build(&paths.vault_dir.join("People"))
    } else {
        crate::storage::PeopleIndex::build(&std::path::PathBuf::new())
    };

    #[cfg(feature = "summaries")]
    let mut context_preamble = if entity_dirs_ready {
        crate::summary::build_context_preamble(&paths.vault_dir)
    } else {
        String::new()
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

    // Load the summary cache (tracks which docs have been summarized)
    #[cfg(feature = "summaries")]
    let summary_cache_path = paths.baez_dir.join(".summary_cache.json");
    #[cfg(feature = "summaries")]
    let mut summary_cache = if summarize_state.is_some() {
        load_summary_cache(&summary_cache_path)
    } else {
        HashMap::new()
    };

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
    #[cfg(feature = "summaries")]
    let mut people_count = 0u32;
    #[cfg(feature = "summaries")]
    let mut concepts_count = 0u32;
    #[cfg(feature = "summaries")]
    let mut projects_count = 0u32;
    #[cfg(feature = "summaries")]
    let mut summarize_only = 0u32;

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
            // Summarize-only path: doc is sync-cached but may need summarization
            #[cfg(feature = "summaries")]
            if !dry_run {
                if let Some((ref config, ref key, ref claude_client)) = summarize_state {
                    if !summary_cache.contains_key(&doc_summary.id) {
                        // Try to load raw files from cache entry
                        if let Some(cache_entry) = cache.get(&doc_summary.id) {
                            let transcript_path = paths
                                .raw_dir
                                .join(format!("{}_transcript.json", cache_entry.filename));
                            let metadata_path = paths
                                .raw_dir
                                .join(format!("{}_metadata.json", cache_entry.filename));

                            let raw_ok = (|| -> std::result::Result<
                                (crate::model::RawTranscript, crate::model::DocumentMetadata),
                                Box<dyn std::error::Error>,
                            > {
                                let t_str = std::fs::read_to_string(&transcript_path)?;
                                let t: crate::model::RawTranscript = serde_json::from_str(&t_str)?;
                                let m_str = std::fs::read_to_string(&metadata_path)?;
                                let mut m: crate::model::DocumentMetadata =
                                    serde_json::from_str(&m_str)?;
                                m.created_at = doc_summary.created_at;
                                Ok((t, m))
                            })();

                            match raw_ok {
                                Ok((transcript, meta)) => {
                                    let slug = crate::util::doc_slug(
                                        meta.title.as_deref(),
                                        &doc_summary.id,
                                    );
                                    let attendee_names: Vec<String> =
                                        if let Some(ref rich) = meta.attendees {
                                            rich.iter().filter_map(|a| a.name.clone()).collect()
                                        } else {
                                            meta.participants.clone()
                                        };

                                    let mut ctx = SummarizationContext {
                                        config,
                                        api_key: key,
                                        client: claude_client,
                                        context_preamble: &mut context_preamble,
                                        paths,
                                        people_index: &mut people_index,
                                        summary_cache: &mut summary_cache,
                                        summary_cache_path: &summary_cache_path,
                                        dry_run,
                                    };

                                    match summarize_and_reconcile(
                                        &mut ctx,
                                        &doc_summary.id,
                                        &transcript,
                                        &meta,
                                        &slug,
                                        &attendee_names,
                                    ) {
                                        Ok((Some(summary_text), extracted_entities)) => {
                                            // Update the existing markdown file with summary
                                            let doc_path = paths.doc_path(&meta.created_at, &slug);
                                            if doc_path.exists() {
                                                match std::fs::read_to_string(&doc_path) {
                                                    Ok(existing_md) => {
                                                        let updated = crate::summary::update_summary_in_markdown(
                                                            &existing_md,
                                                            &summary_text,
                                                        );
                                                        if let Err(e) = write_atomic(
                                                            &doc_path,
                                                            updated.as_bytes(),
                                                            &paths.tmp_dir,
                                                        ) {
                                                            eprintln!(
                                                                "Warning: Failed to update summary in {}: {}",
                                                                doc_path.display(),
                                                                e
                                                            );
                                                        }
                                                    }
                                                    Err(e) => {
                                                        eprintln!(
                                                            "Warning: Failed to read {}: {}",
                                                            doc_path.display(),
                                                            e
                                                        );
                                                    }
                                                }
                                            }

                                            // Merge frontmatter related links
                                            if let Some(ref entities) = extracted_entities {
                                                let mut related = Vec::new();
                                                for p in &entities.people {
                                                    related.push(format!("[[{}]]", p.name));
                                                }
                                                for c in &entities.concepts {
                                                    related.push(format!("[[{}]]", c.name));
                                                }
                                                for pr in &entities.projects {
                                                    related.push(format!("[[{}]]", pr.name));
                                                }
                                                if let Err(e) =
                                                    crate::storage::merge_frontmatter_related(
                                                        &doc_path,
                                                        &related,
                                                        &paths.tmp_dir,
                                                    )
                                                {
                                                    eprintln!(
                                                        "Warning: Failed to merge related links in {}: {}",
                                                        doc_path.display(),
                                                        e
                                                    );
                                                }

                                                people_count += entities.people.len() as u32;
                                                concepts_count += entities.concepts.len() as u32;
                                                projects_count += entities.projects.len() as u32;
                                            }

                                            summarize_only += 1;
                                            summarized += 1;
                                        }
                                        Ok((None, _)) => {
                                            // No summary produced (e.g. too short)
                                        }
                                        Err(e) => {
                                            eprintln!(
                                                "Warning: Catch-up summarization failed for {}: {}",
                                                doc_summary.id, e
                                            );
                                        }
                                    }
                                }
                                Err(e) => {
                                    eprintln!(
                                        "Warning: Cannot load raw files for catch-up summary of {}: {}",
                                        doc_summary.id, e
                                    );
                                }
                            }
                        }
                    }
                }
            }

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

        // Triage: check if transcript has enough content
        let word_count = count_transcript_words(&transcript);
        let status = if word_count < 20 {
            "stub"
        } else {
            "substantive"
        };

        // The metadata endpoint sometimes omits created_at; prefer the
        // summary's value which the list endpoint always provides.
        meta.created_at = doc_summary.created_at;

        // Extract user notes from panels (my_notes -> notes field -> last_viewed_panel fallback)
        let notes_md = doc_summary
            .user_notes()
            .as_ref()
            .map(crate::convert::prosemirror_to_markdown)
            .filter(|s| !s.is_empty());

        // AI summarization + entity extraction (only for substantive transcripts)
        #[cfg(feature = "summaries")]
        let (summary_text, extracted_entities): (
            Option<String>,
            Option<crate::summary::ExtractedEntities>,
        ) = if status == "substantive" {
            if let Some((ref config, ref key, ref claude_client)) = summarize_state {
                let slug_for_summary =
                    crate::util::doc_slug(meta.title.as_deref(), &doc_summary.id);
                let attendee_names: Vec<String> = if let Some(ref rich) = meta.attendees {
                    rich.iter().filter_map(|a| a.name.clone()).collect()
                } else {
                    meta.participants.clone()
                };

                let mut ctx = SummarizationContext {
                    config,
                    api_key: key,
                    client: claude_client,
                    context_preamble: &mut context_preamble,
                    paths,
                    people_index: &mut people_index,
                    summary_cache: &mut summary_cache,
                    summary_cache_path: &summary_cache_path,
                    dry_run,
                };

                match summarize_and_reconcile(
                    &mut ctx,
                    &doc_summary.id,
                    &transcript,
                    &meta,
                    &slug_for_summary,
                    &attendee_names,
                ) {
                    Ok((summary, entities)) => {
                        if summary.is_some() {
                            summarized += 1;
                        }
                        if let Some(ref ents) = entities {
                            people_count += ents.people.len() as u32;
                            concepts_count += ents.concepts.len() as u32;
                            projects_count += ents.projects.len() as u32;
                        }
                        (summary, entities)
                    }
                    Err(e) => {
                        eprintln!("Warning: Failed to summarize {}: {}", doc_summary.id, e);
                        (None, None)
                    }
                }
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        #[cfg(not(feature = "summaries"))]
        let summary_text: Option<String> = None;
        #[cfg(not(feature = "summaries"))]
        let _extracted_entities: Option<()> = None;

        // Build related list from extracted entities
        #[cfg(feature = "summaries")]
        let related: Vec<String> = match &extracted_entities {
            Some(entities) => {
                let mut r = Vec::new();
                for p in &entities.people {
                    r.push(format!("[[{}]]", p.name));
                }
                for c in &entities.concepts {
                    r.push(format!("[[{}]]", c.name));
                }
                for p in &entities.projects {
                    r.push(format!("[[{}]]", p.name));
                }
                r
            }
            None => vec![],
        };
        #[cfg(not(feature = "summaries"))]
        let related: Vec<String> = vec![];

        // Convert to markdown
        let md = to_markdown(
            &transcript,
            &meta,
            &doc_summary.id,
            notes_md.as_deref(),
            summary_text.as_deref(),
            related,
            Some(status),
        )?;

        let full_md = format!("---\n{}---\n\n{}", md.frontmatter_yaml, md.body);

        // Compute filename and date-based path
        let slug = crate::util::doc_slug(meta.title.as_deref(), &doc_summary.id);
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
        "synced {} docs ({} new/updated, {} skipped, {} summarized, {} catch-up summarized, {} people, {} concepts, {} projects)",
        docs.len(),
        synced,
        skipped,
        summarized,
        summarize_only,
        people_count,
        concepts_count,
        projects_count
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

    #[cfg(feature = "summaries")]
    #[test]
    fn test_summary_cache_roundtrip() {
        use super::{load_summary_cache, save_summary_cache, SummaryCacheEntry};
        use chrono::Utc;
        use std::collections::HashMap;

        let temp = TempDir::new().unwrap();
        let cache_path = temp.path().join(".summary_cache.json");
        let tmp_dir = temp.path().join("tmp");
        std::fs::create_dir_all(&tmp_dir).unwrap();

        let mut cache = HashMap::new();
        cache.insert(
            "doc-123".to_string(),
            SummaryCacheEntry {
                summarized_at: Utc::now(),
                model: "claude-sonnet-4-20250514".to_string(),
            },
        );

        save_summary_cache(&cache_path, &cache, &tmp_dir).unwrap();
        let loaded = load_summary_cache(&cache_path);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded["doc-123"].model, "claude-sonnet-4-20250514");
    }

    #[cfg(feature = "summaries")]
    #[test]
    fn test_summary_cache_load_missing_file() {
        use super::load_summary_cache;

        let temp = TempDir::new().unwrap();
        let cache_path = temp.path().join("nonexistent.json");
        let loaded = load_summary_cache(&cache_path);
        assert!(loaded.is_empty());
    }

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
