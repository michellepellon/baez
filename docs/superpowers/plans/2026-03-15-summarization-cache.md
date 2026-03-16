# Summarization Cache Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a summarization cache so `baez sync` can resume summarization for previously-synced docs, and add a standalone `baez summarize-all` command that processes unsummarized docs from local files without hitting the Granola API.

**Architecture:** A separate `.summary_cache.json` tracks which docs have been summarized and with which model. The sync loop gains a "summarize-only" path for cached-but-unsummarized docs. A new `summarize-all` CLI command iterates the sync cache inventory and processes gaps. Both paths share a `summarize_and_reconcile` function via a `SummarizationContext` struct.

**Tech Stack:** Rust, serde, chrono, clap (derive), indicatif, reqwest (blocking)

**Spec:** `docs/superpowers/specs/2026-03-15-summarization-cache-design.md`

---

## File Structure

| File | Action | Responsibility |
|------|--------|---------------|
| `src/sync.rs` | Modify | Add `SummaryCacheEntry`, cache I/O, `SummarizationContext`, `summarize_and_reconcile`, modify sync loop, add `summarize_all_docs` |
| `src/storage.rs` | Modify | Add `merge_frontmatter_related()` helper |
| `src/cli.rs` | Modify | Add `SummarizeAll` command variant |
| `src/main.rs` | Modify | Wire up `SummarizeAll` command |
| `src/lib.rs` | Modify | Export new public items from `sync` module |

---

## Chunk 1: Summary Cache Types, I/O, and Frontmatter Helper

### Task 1: Add SummaryCacheEntry and cache I/O functions

**Files:**
- Modify: `src/sync.rs:17-44` (after existing CacheEntry and cache I/O)

- [ ] **Step 1: Add SummaryCacheEntry struct and load/save functions**

Add after line 44 in `src/sync.rs` (after `save_cache`):

```rust
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
```

- [ ] **Step 2: Add unit tests for summary cache I/O**

Add to the `mod tests` block at the bottom of `src/sync.rs`:

```rust
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
```

- [ ] **Step 3: Run tests to verify**

Run: `cargo test --features summaries test_summary_cache`
Expected: Both tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/sync.rs
git commit -m "feat: add SummaryCacheEntry and cache I/O functions"
```

### Task 2: Add merge_frontmatter_related helper

**Files:**
- Modify: `src/storage.rs` (add after `read_frontmatter` function)

This helper reads a markdown file, finds the `related:` section in frontmatter, and merges new wiki-links using string manipulation (no YAML round-trip, to preserve field order and formatting).

- [ ] **Step 1: Write failing test**

Add to the `mod tests` block in `src/storage.rs`:

```rust
#[test]
fn test_merge_frontmatter_related_adds_new_links() {
    let temp = tempfile::TempDir::new().unwrap();
    let md_path = temp.path().join("test.md");
    let tmp_dir = temp.path().join("tmp");
    std::fs::create_dir_all(&tmp_dir).unwrap();

    std::fs::write(
        &md_path,
        "---\ndoc_id: doc123\nsource: granola\ncreated: 2025-10-28T15:04:05Z\ngenerator: baez\nrelated:\n  - \"[[Alice]]\"\n---\n\n# Meeting\n",
    ).unwrap();

    let new_links = vec!["[[Alice]]".to_string(), "[[Bob]]".to_string(), "[[API Design]]".to_string()];
    merge_frontmatter_related(&md_path, &new_links, &tmp_dir).unwrap();

    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("[[Alice]]"));
    assert!(content.contains("[[Bob]]"));
    assert!(content.contains("[[API Design]]"));
    // Should not duplicate Alice
    let alice_count = content.matches("[[Alice]]").count();
    assert_eq!(alice_count, 1, "Alice should appear exactly once in related");
    // Verify frontmatter structure is preserved
    assert!(content.starts_with("---\n"));
    assert!(content.contains("\n---\n"));
}

#[test]
fn test_merge_frontmatter_related_no_existing_related() {
    let temp = tempfile::TempDir::new().unwrap();
    let md_path = temp.path().join("test.md");
    let tmp_dir = temp.path().join("tmp");
    std::fs::create_dir_all(&tmp_dir).unwrap();

    std::fs::write(
        &md_path,
        "---\ndoc_id: doc123\nsource: granola\ncreated: 2025-10-28T15:04:05Z\ngenerator: baez\n---\n\n# Meeting\n",
    ).unwrap();

    let new_links = vec!["[[Bob]]".to_string()];
    merge_frontmatter_related(&md_path, &new_links, &tmp_dir).unwrap();

    let content = std::fs::read_to_string(&md_path).unwrap();
    assert!(content.contains("[[Bob]]"));
    // Field order should be preserved — generator still before ---
    assert!(content.contains("generator: baez\nrelated:"));
}

#[test]
fn test_merge_frontmatter_related_no_frontmatter() {
    let temp = tempfile::TempDir::new().unwrap();
    let md_path = temp.path().join("test.md");
    let tmp_dir = temp.path().join("tmp");
    std::fs::create_dir_all(&tmp_dir).unwrap();

    let original = "# Meeting\n\nNo frontmatter here.\n";
    std::fs::write(&md_path, original).unwrap();

    let new_links = vec!["[[Bob]]".to_string()];
    merge_frontmatter_related(&md_path, &new_links, &tmp_dir).unwrap();

    // File should be unchanged
    let content = std::fs::read_to_string(&md_path).unwrap();
    assert_eq!(content, original);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test test_merge_frontmatter_related`
Expected: FAIL — `merge_frontmatter_related` not found

- [ ] **Step 3: Implement merge_frontmatter_related**

Uses string-based insertion to preserve field order and formatting (avoids serde_yaml round-trip which reorders fields and changes quoting).

Add to `src/storage.rs` (near the other frontmatter functions):

```rust
/// Merge new `related` wiki-links into an existing markdown file's frontmatter.
/// Performs a set union — preserves existing links, adds new ones, deduplicates.
/// Uses string manipulation to preserve field order and formatting.
pub fn merge_frontmatter_related(
    md_path: &Path,
    new_links: &[String],
    tmp_dir: &Path,
) -> Result<()> {
    if new_links.is_empty() {
        return Ok(());
    }

    let content = fs::read_to_string(md_path)?;

    // Find frontmatter boundaries (between first and second "---")
    let Some(fm_start) = content.find("---\n") else {
        return Ok(());
    };
    let fm_content_start = fm_start + 4;
    let Some(fm_end_offset) = content[fm_content_start..].find("\n---") else {
        return Ok(());
    };
    let fm_end = fm_content_start + fm_end_offset;
    let fm_str = &content[fm_content_start..fm_end];

    // Collect existing related links from the frontmatter text
    let mut existing_links: Vec<String> = Vec::new();
    let mut related_section_start: Option<usize> = None;
    let mut related_section_end: usize = fm_str.len();

    if let Some(rel_offset) = fm_str.find("\nrelated:") {
        related_section_start = Some(rel_offset + 1); // skip leading \n
        // Parse the list items following "related:"
        let after_key = &fm_str[rel_offset + "\nrelated:".len()..];
        for line in after_key.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- ") {
                let value = trimmed[2..].trim().trim_matches('"').trim_matches('\'');
                existing_links.push(value.to_string());
            } else if !trimmed.is_empty() {
                // Hit the next YAML key — mark end of related section
                let line_start = fm_str.len() - after_key.len()
                    + (line.as_ptr() as usize - after_key.as_ptr() as usize);
                related_section_end = line_start;
                break;
            }
        }
    } else if fm_str.starts_with("related:") {
        related_section_start = Some(0);
        let after_key = &fm_str["related:".len()..];
        for line in after_key.lines() {
            let trimmed = line.trim();
            if trimmed.starts_with("- ") {
                let value = trimmed[2..].trim().trim_matches('"').trim_matches('\'');
                existing_links.push(value.to_string());
            } else if !trimmed.is_empty() {
                let line_start = fm_str.len() - after_key.len()
                    + (line.as_ptr() as usize - after_key.as_ptr() as usize);
                related_section_end = line_start;
                break;
            }
        }
    }

    // Merge: add new links not already present
    let mut merged = existing_links.clone();
    for link in new_links {
        if !merged.contains(link) {
            merged.push(link.clone());
        }
    }

    // If nothing new to add, skip the write
    if merged.len() == existing_links.len() {
        return Ok(());
    }

    // Build the new related: block
    let mut related_block = String::from("related:\n");
    for link in &merged {
        related_block.push_str(&format!("  - \"{}\"\n", link));
    }

    // Reconstruct frontmatter
    let new_fm = if let Some(start) = related_section_start {
        // Replace existing related section
        let mut result = String::new();
        result.push_str(&fm_str[..start]);
        result.push_str(&related_block);
        if related_section_end < fm_str.len() {
            result.push_str(&fm_str[related_section_end..]);
        }
        result
    } else {
        // No existing related section — append before closing ---
        format!("{}\n{}", fm_str, related_block)
    };

    // Reconstruct file
    let mut result = String::new();
    result.push_str(&content[..fm_content_start]);
    result.push_str(&new_fm);
    result.push_str(&content[fm_end..]);

    write_atomic(md_path, result.as_bytes(), tmp_dir)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test test_merge_frontmatter_related`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/storage.rs
git commit -m "feat: add merge_frontmatter_related helper for frontmatter updates"
```

---

## Chunk 2: SummarizationContext and summarize_and_reconcile

### Task 3: Add SummarizationContext struct and summarize_and_reconcile function

**Files:**
- Modify: `src/sync.rs`

This extracts the summarization + entity reconciliation logic from the sync loop into a reusable function.

- [ ] **Step 1: Add SummarizationContext struct**

Add after the summary cache I/O functions in `src/sync.rs`:

```rust
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
```

- [ ] **Step 2: Add summarize_and_reconcile function**

This function is extracted from the sync loop body (lines 207-446 of current sync.rs). It handles: Claude API call, summary parsing, entity extraction, entity note reconciliation, and summary cache update.

```rust
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
                    let alias_refs: Vec<&str> =
                        person.aliases.iter().map(|s| s.as_str()).collect();
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
                    let alias_refs: Vec<&str> =
                        person.aliases.iter().map(|s| s.as_str()).collect();
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
                        eprintln!(
                            "Warning: Failed to enrich Concepts/{}: {}",
                            concept.name, e
                        );
                    }
                } else if let Err(e) = crate::storage::create_concept_note(
                    &concepts_dir,
                    &concept.name,
                    &concept.description,
                    &meeting_slug,
                    &date,
                    &ctx.paths.tmp_dir,
                ) {
                    eprintln!(
                        "Warning: Failed to create Concepts/{}: {}",
                        concept.name, e
                    );
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
                        eprintln!(
                            "Warning: Failed to enrich Projects/{}: {}",
                            project.name, e
                        );
                    }
                } else if let Err(e) = crate::storage::create_project_note(
                    &projects_dir,
                    &project.name,
                    &project.description,
                    &meeting_slug,
                    &date,
                    &ctx.paths.tmp_dir,
                ) {
                    eprintln!(
                        "Warning: Failed to create Projects/{}: {}",
                        project.name, e
                    );
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
        save_summary_cache(ctx.summary_cache_path, ctx.summary_cache, &ctx.paths.tmp_dir)?;
    }

    Ok((summary_text, extracted_entities))
}
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --features summaries`
Expected: Compiles with no errors (function is unused for now, that's fine)

- [ ] **Step 4: Commit**

```bash
git add src/sync.rs
git commit -m "feat: add SummarizationContext and summarize_and_reconcile function"
```

---

## Chunk 3: Refactor Sync Loop to Use Shared Function and Summary Cache

### Task 4: Refactor sync_all to use summarize_and_reconcile and add summary-only path

**Files:**
- Modify: `src/sync.rs:50-486` (the `sync_all` function)

This is the largest change. The sync loop is refactored to:
1. Load the summary cache alongside the sync cache
2. Use `summarize_and_reconcile` instead of inline summarization code
3. Add a "summarize-only" path for docs that are cached but not yet summarized

- [ ] **Step 1: Add summary cache loading and new counters**

In `sync_all`, after the sync cache loading (line 135), add:

```rust
#[cfg(feature = "summaries")]
let summary_cache_path = paths.baez_dir.join(".summary_cache.json");
#[cfg(feature = "summaries")]
let mut summary_cache = if summarize_state.is_some() {
    load_summary_cache(&summary_cache_path)
} else {
    HashMap::new()
};
```

Add a new counter after the existing counters (line 154):

```rust
#[cfg(feature = "summaries")]
let mut summarize_only = 0u32;
```

- [ ] **Step 2: Add summarize-only path for skipped docs**

Replace the current skip block (lines 167-171):

```rust
if !should_update {
    skipped += 1;
    pb.inc(1);
    continue;
}
```

With:

```rust
if !should_update {
    // Check if this doc needs summarization even though sync is current
    #[cfg(feature = "summaries")]
    if let Some((ref config, ref key, ref claude_client)) = summarize_state {
        if !summary_cache.contains_key(&doc_summary.id) {
            // Look up filename from sync cache to find raw files
            if let Some(cache_entry) = cache.get(&doc_summary.id) {
                let transcript_path = paths
                    .raw_dir
                    .join(format!("{}_transcript.json", cache_entry.filename));
                let metadata_path = paths
                    .raw_dir
                    .join(format!("{}_metadata.json", cache_entry.filename));

                if transcript_path.exists() && metadata_path.exists() {
                    match (
                        std::fs::read_to_string(&transcript_path)
                            .ok()
                            .and_then(|s| serde_json::from_str::<crate::model::RawTranscript>(&s).ok()),
                        std::fs::read_to_string(&metadata_path)
                            .ok()
                            .and_then(|s| serde_json::from_str::<crate::model::DocumentMetadata>(&s).ok()),
                    ) {
                        (Some(transcript), Some(mut meta)) => {
                            meta.created_at = doc_summary.created_at;
                            let slug = crate::util::doc_slug(meta.title.as_deref(), &doc_summary.id);
                            let date = meta.created_at.format("%Y-%m-%d").to_string();
                            let base_filename = format!("{}_{}", date, slug);

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

                            if let Ok((Some(summary), entities)) =
                                summarize_and_reconcile(&mut ctx, &doc_summary.id, &transcript, &meta, &slug, &attendee_names)
                            {
                                // Update existing markdown with summary
                                if !dry_run {
                                    let doc_path = paths.doc_path(&meta.created_at, &slug);
                                    if doc_path.exists() {
                                        if let Ok(content) = std::fs::read_to_string(&doc_path) {
                                            let updated = crate::summary::update_summary_in_markdown(&content, &summary);
                                            let _ = write_atomic(&doc_path, updated.as_bytes(), &paths.tmp_dir);

                                            // Merge related links from entities
                                            if let Some(ref ent) = entities {
                                                let mut related = Vec::new();
                                                for p in &ent.people { related.push(format!("[[{}]]", p.name)); }
                                                for c in &ent.concepts { related.push(format!("[[{}]]", c.name)); }
                                                for p in &ent.projects { related.push(format!("[[{}]]", p.name)); }
                                                let _ = crate::storage::merge_frontmatter_related(&doc_path, &related, &paths.tmp_dir);
                                            }
                                        }
                                    }
                                }
                                summarize_only += 1;
                            }
                        }
                        _ => {
                            eprintln!(
                                "Warning: Could not read raw files for {}, skipping summarization",
                                doc_summary.id
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
```

- [ ] **Step 3: Replace inline summarization in the sync path with summarize_and_reconcile**

Replace the current summarization block (lines 207-446, everything from "AI summarization" through the entity reconciliation ending) with:

```rust
// AI summarization + entity extraction
#[cfg(feature = "summaries")]
let (summary_text, extracted_entities): (
    Option<String>,
    Option<crate::summary::ExtractedEntities>,
) = if let Some((ref config, ref key, ref claude_client)) = summarize_state {
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

    match summarize_and_reconcile(&mut ctx, &doc_summary.id, &transcript, &meta, &slug, &attendee_names) {
        Ok((s, e)) => {
            if s.is_some() {
                summarized += 1;
            }
            if let Some(ref ent) = e {
                people_count += ent.people.len() as u32;
                concepts_count += ent.concepts.len() as u32;
                projects_count += ent.projects.len() as u32;
            }
            (s, e)
        }
        Err(e) => {
            eprintln!("Warning: Summarization error for {}: {}", doc_summary.id, e);
            (None, None)
        }
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
        for p in &entities.people { r.push(format!("[[{}]]", p.name)); }
        for c in &entities.concepts { r.push(format!("[[{}]]", c.name)); }
        for p in &entities.projects { r.push(format!("[[{}]]", p.name)); }
        r
    }
    None => vec![],
};
#[cfg(not(feature = "summaries"))]
let related: Vec<String> = vec![];
```

Note: the entity reconciliation is now handled inside `summarize_and_reconcile`, so remove the separate entity reconciliation block (lines 337-446 of the original).

- [ ] **Step 4: Update stats message to include summarize_only counter**

Replace the stats_msg format (lines 466-475) with:

```rust
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
```

Entity counters are restored by inspecting the returned `ExtractedEntities` after each `summarize_and_reconcile` call (see Step 3).

```rust
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
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check --features summaries`
Expected: Compiles with no errors

- [ ] **Step 6: Run existing tests**

Run: `cargo test --features summaries`
Expected: All existing tests PASS

- [ ] **Step 7: Commit**

```bash
git add src/sync.rs
git commit -m "refactor: use summarize_and_reconcile in sync loop, add summary-only path"
```

---

## Chunk 4: CLI Command and Standalone summarize-all

### Task 5: Add SummarizeAll CLI command

**Files:**
- Modify: `src/cli.rs:57-124`

- [ ] **Step 1: Add SummarizeAll variant to Commands enum**

Add after the existing `Summarize` variant (line 123), still inside the enum:

```rust
/// Batch-summarize all synced documents that haven't been summarized yet
#[cfg(feature = "summaries")]
SummarizeAll {
    /// Force re-summarization of all documents
    #[arg(long)]
    force: bool,
    /// Preview what would be summarized without making changes
    #[arg(long)]
    dry_run: bool,
},
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --features summaries`
Expected: Compiles (warning about unused variant is OK)

- [ ] **Step 3: Commit**

```bash
git add src/cli.rs
git commit -m "feat: add SummarizeAll CLI command variant"
```

### Task 6: Implement summarize_all_docs and wire up in main.rs

**Files:**
- Modify: `src/sync.rs` (add `summarize_all_docs` function)
- Modify: `src/main.rs` (wire up command)
- Modify: `src/lib.rs` (export new function)

- [ ] **Step 1: Add summarize_all_docs function to sync.rs**

Add after `summarize_and_reconcile`, before the `fix_dates` function:

```rust
/// Batch-summarize all synced documents that haven't been summarized yet.
///
/// Reads transcripts from local raw JSON files — does NOT hit the Granola API.
/// Uses the sync cache as the inventory of known documents.
#[cfg(feature = "summaries")]
pub fn summarize_all_docs(paths: &Paths, force: bool, verbose: bool, dry_run: bool) -> Result<()> {
    use indicatif::{ProgressBar, ProgressStyle};

    let config_path = paths.baez_dir.join("summary_config.json");
    let config = crate::summary::SummaryConfig::load(&config_path)?;

    let api_key = match crate::summary::get_api_key_verbose(verbose) {
        Some(key) => key,
        None => {
            eprintln!("Error: No Anthropic API key found. Set BAEZ_ANTHROPIC_API_KEY, ANTHROPIC_API_KEY, add anthropic_api_key to ~/.config/baez/config.json, or run `baez set-api-key`.");
            return Ok(());
        }
    };

    let claude_client = crate::summary::build_claude_client()?;
    println!("Batch summarization (model: {})", config.model);

    // Create entity directories
    let _ = std::fs::create_dir_all(paths.vault_dir.join("People"));
    let _ = std::fs::create_dir_all(paths.vault_dir.join("Concepts"));
    let _ = std::fs::create_dir_all(paths.vault_dir.join("Projects"));

    let mut people_index =
        crate::storage::PeopleIndex::build(&paths.vault_dir.join("People"));
    let mut context_preamble = crate::summary::build_context_preamble(&paths.vault_dir);

    // Load caches
    let sync_cache_path = paths.baez_dir.join(".sync_cache.json");
    let sync_cache = load_cache(&sync_cache_path);

    let summary_cache_path = paths.baez_dir.join(".summary_cache.json");
    let mut summary_cache = load_summary_cache(&summary_cache_path);

    if sync_cache.is_empty() {
        println!("No synced documents found. Run `baez sync` first.");
        return Ok(());
    }

    // Collect docs to process
    let to_process: Vec<(&String, &CacheEntry)> = sync_cache
        .iter()
        .filter(|(doc_id, _)| force || !summary_cache.contains_key(*doc_id))
        .collect();

    if to_process.is_empty() {
        println!("All {} documents already summarized.", sync_cache.len());
        return Ok(());
    }

    println!(
        "{} documents to summarize ({} already done, {} total)",
        to_process.len(),
        sync_cache.len() - to_process.len(),
        sync_cache.len(),
    );

    if dry_run {
        println!("Dry run — no files will be written");
    }

    let pb = ProgressBar::new(to_process.len() as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{bar:40}] {pos}/{len} docs")
            .unwrap()
            .progress_chars("##-"),
    );

    let mut summarized = 0u32;
    let mut skipped_stubs = 0u32;
    let mut skipped_missing = 0u32;

    for (doc_id, cache_entry) in &to_process {
        let transcript_path = paths
            .raw_dir
            .join(format!("{}_transcript.json", cache_entry.filename));
        let metadata_path = paths
            .raw_dir
            .join(format!("{}_metadata.json", cache_entry.filename));

        if !transcript_path.exists() || !metadata_path.exists() {
            eprintln!(
                "Warning: Raw files missing for {} ({}), skipping",
                doc_id, cache_entry.filename
            );
            skipped_missing += 1;
            pb.inc(1);
            continue;
        }

        let transcript = match std::fs::read_to_string(&transcript_path)
            .ok()
            .and_then(|s| serde_json::from_str::<crate::model::RawTranscript>(&s).ok())
        {
            Some(t) => t,
            None => {
                eprintln!("Warning: Could not parse transcript for {}, skipping", doc_id);
                skipped_missing += 1;
                pb.inc(1);
                continue;
            }
        };

        let meta = match std::fs::read_to_string(&metadata_path)
            .ok()
            .and_then(|s| serde_json::from_str::<crate::model::DocumentMetadata>(&s).ok())
        {
            Some(m) => m,
            None => {
                eprintln!("Warning: Could not parse metadata for {}, skipping", doc_id);
                skipped_missing += 1;
                pb.inc(1);
                continue;
            }
        };

        // Check word count
        let word_count = crate::util::count_transcript_words(&transcript);
        if word_count < 20 {
            skipped_stubs += 1;
            pb.inc(1);
            continue;
        }

        let slug = crate::util::doc_slug(meta.title.as_deref(), doc_id);

        let attendee_names: Vec<String> = if let Some(ref rich) = meta.attendees {
            rich.iter().filter_map(|a| a.name.clone()).collect()
        } else {
            meta.participants.clone()
        };

        if dry_run {
            let title = meta.title.as_deref().unwrap_or("(untitled)");
            let date = meta.created_at.format("%Y-%m-%d");
            println!("  would summarize: {} — {}", date, title);
            pb.inc(1);
            continue;
        }

        let mut ctx = SummarizationContext {
            config: &config,
            api_key: &api_key,
            client: &claude_client,
            context_preamble: &mut context_preamble,
            paths,
            people_index: &mut people_index,
            summary_cache: &mut summary_cache,
            summary_cache_path: &summary_cache_path,
            dry_run,
        };

        match summarize_and_reconcile(&mut ctx, doc_id, &transcript, &meta, &slug, &attendee_names) {
            Ok((Some(summary), entities)) => {
                // Update existing markdown with summary
                let doc_path = paths.doc_path(&meta.created_at, &slug);
                if doc_path.exists() {
                    if let Ok(content) = std::fs::read_to_string(&doc_path) {
                        let updated =
                            crate::summary::update_summary_in_markdown(&content, &summary);
                        let _ = write_atomic(&doc_path, updated.as_bytes(), &paths.tmp_dir);

                        // Merge related links
                        if let Some(ref ent) = entities {
                            let mut related = Vec::new();
                            for p in &ent.people { related.push(format!("[[{}]]", p.name)); }
                            for c in &ent.concepts { related.push(format!("[[{}]]", c.name)); }
                            for p in &ent.projects { related.push(format!("[[{}]]", p.name)); }
                            let _ = crate::storage::merge_frontmatter_related(
                                &doc_path, &related, &paths.tmp_dir,
                            );
                        }
                    }
                } else {
                    eprintln!(
                        "Warning: Markdown file not found at {}, summary not written (metadata created_at may differ from sync)",
                        doc_path.display()
                    );
                }
                summarized += 1;
            }
            Ok((None, _)) => {
                // Summarization returned None (stub or failure handled inside)
            }
            Err(e) => {
                eprintln!("Warning: Failed to summarize {}: {}", doc_id, e);
            }
        }

        pb.inc(1);
    }

    let stats_msg = format!(
        "summarized {} docs ({} stubs skipped, {} missing files skipped, {} already done)",
        summarized,
        skipped_stubs,
        skipped_missing,
        sync_cache.len() - to_process.len(),
    );
    pb.finish_with_message(stats_msg);

    Ok(())
}
```

- [ ] **Step 2: Export summarize_all_docs from lib.rs**

In `src/lib.rs`, add to the `#[cfg(feature = "summaries")]` pub use block (line 36):

```rust
pub use sync::summarize_all_docs;
```

And update the non-gated sync export to keep `sync_all` (line 32 is fine as-is).

- [ ] **Step 3: Wire up SummarizeAll in main.rs**

Add a new match arm in `src/main.rs` after the existing `Summarize` arm (line 249), before the closing `}`:

```rust
#[cfg(feature = "summaries")]
Some(baez::cli::Commands::SummarizeAll { force, dry_run }) => {
    let paths = Paths::new(cli.vault)?;
    paths.ensure_dirs()?;
    baez::sync::summarize_all_docs(&paths, force, cli.verbose, dry_run)?;
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --features summaries`
Expected: Compiles with no errors

- [ ] **Step 5: Run all tests**

Run: `cargo test --features summaries`
Expected: All tests PASS

- [ ] **Step 6: Test CLI help**

Run: `cargo run --features summaries -- summarize-all --help`
Expected: Shows help for the `summarize-all` command with `--force` and `--dry-run` flags

- [ ] **Step 7: Commit**

```bash
git add src/sync.rs src/main.rs src/lib.rs
git commit -m "feat: add baez summarize-all command for batch summarization"
```

---

## Chunk 5: Integration Verification

### Task 7: End-to-end dry-run verification

- [ ] **Step 1: Run summarize-all --dry-run against real vault**

Run: `cargo run --features summaries -- summarize-all --dry-run`
Expected: Lists unsummarized docs without making changes. Should show ~86 docs to summarize and ~59 already done.

- [ ] **Step 2: Run sync --dry-run to verify summary-only path**

Run: `cargo run --features summaries -- sync --dry-run`
Expected: Shows 0 new/updated docs (all cached), mentions catch-up summarization count.

- [ ] **Step 3: Verify cargo fmt and clippy**

Run: `cargo fmt --check && cargo clippy --features summaries -- -D warnings`
Expected: No formatting or lint issues

- [ ] **Step 4: Fix any issues found, then final commit**

```bash
cargo fmt
git add -A
git commit -m "style: apply formatting and fix clippy warnings"
```
