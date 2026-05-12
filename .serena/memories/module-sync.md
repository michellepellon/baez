# Module: sync.rs — Core Sync Logic + Batch Summarization

## Purpose
Orchestrates incremental sync, summarization, and entity reconciliation. Also provides a standalone batch-summarization path that never hits the Granola API.

## Key Symbols
- **`CacheEntry`** (struct): `{ filename: String, updated_at: DateTime<Utc> }` — sync cache value
- **`SummaryCacheEntry`** (struct, feature-gated): `{ summarized_at: DateTime<Utc>, model: String }` — tracks per-doc summarization
- **`SummarizationContext<'a>`** (struct, feature-gated, `pub(crate)`): Bundle of mutable references passed into `summarize_and_reconcile`:
  - `config`, `api_key`, `client` (Claude bits)
  - `context_preamble: &mut String` (concepts/projects list, rebuilt after creating new ones)
  - `paths: &Paths`
  - `people_index: &mut PeopleIndex`
  - `summary_cache: &mut HashMap<String, SummaryCacheEntry>`
  - `summary_cache_path: &Path`
  - `dry_run: bool`
- **`load_cache(path)` / `save_cache(path, cache, tmp_dir)`** — sync cache JSON I/O
- **`load_summary_cache(path)` / `save_summary_cache(path, cache, tmp_dir)`** — summary cache JSON I/O (feature-gated)
- **`summarize_and_reconcile(ctx, doc_id, transcript, meta, slug, attendee_names)`** → `Result<(Option<String>, Option<ExtractedEntities>)>` (feature-gated, `pub(crate)`):
  - Word-count gate (< 20 → returns `(None, None)`)
  - Calls `summarize_transcript()` then `parse_summary_output()`
  - For each entity: fuzzy-matches via `PeopleIndex`/`find_entity_file` → enriches or creates
  - Updates `summary_cache` and persists (skipped in dry_run)
  - Concept/project creation triggers `build_context_preamble()` refresh so subsequent docs see new entities
- **`sync_all(client, paths, force, summarize, verbose, dry_run)`** → `Result<()>`
  - Sets up `summarize_state` (config, key, claude_client) if `summarize && key available`
  - Creates `People/`, `Concepts/`, `Projects/` at vault root when entity dirs ready
  - Builds `PeopleIndex` and `context_preamble` once before the loop
  - Loads both sync cache and summary cache
  - For each doc: if cache says skip, take **summarize-only path** (load raw files, summarize, update existing markdown). Else fetch fresh, triage, summarize, write.
  - Critical detail unchanged: stores `doc_summary.updated_at` in cache (NOT `meta.updated_at`)
- **`summarize_all_docs(paths, force, verbose, dry_run)`** → `Result<()>` (feature-gated):
  - Reads from `sync_cache` as inventory; parses local raw transcript + metadata JSON
  - **Backfill block** (only when `summary_cache` is empty + !force): scans existing markdown for `\n## Summary\n`, populates cache with `model: "unknown (backfilled)"`
  - Triages by word count, calls `summarize_and_reconcile()`, then updates markdown summary section + `merge_frontmatter_related()`
  - Reports stats: summarized / stubs skipped / missing files skipped / already done
- **`fix_dates(paths)`** → `Result<()>`: Walk all .md files under granola_dir, set mtime from frontmatter `created` field
- **`walk_md_files(dir)`** — recursive helper

## Sync-only-path Quirk
When sync cache shows a doc is up-to-date but summary cache lacks an entry, sync_all does a catch-up summarization using the local raw files (saved from a prior sync). This means the sync command itself can heal old un-summarized docs — `summarize-all` is the fully detached equivalent for users who never want to hit Granola.

## Cache Locations
- `.baez/.sync_cache.json` (sync state)
- `.baez/.summary_cache.json` (summarization state)

## Critical Details
- Cache stores `doc_summary.updated_at` (from list endpoint), NOT `meta.updated_at` (from metadata endpoint) — they can differ.
- `meta.created_at` is overridden with `doc_summary.created_at` because the metadata endpoint sometimes omits it. In `summarize_all_docs` the same fix is applied from the cache filename (`YYYY-MM-DD` prefix).
- Entity directory creation is gated on `entity_dirs_ready` (summarize feature ON + key present) so the no-key path doesn't pollute the vault.

## Summarization Integration
- Feature-gated with `#[cfg(feature = "summaries")]`
- Non-fatal: failures print warning, continue sync
