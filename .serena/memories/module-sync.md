# Module: sync.rs — Core Sync Logic

## Purpose
Orchestrates incremental sync: fetches documents, detects changes via cache, writes markdown + raw JSON.

## Key Symbols
- **`sync_all(client, paths, force, summarize, verbose, dry_run)`** → `Result<()>`
  - Main sync function, called from `main.rs`
  - Uses `list_documents_with_notes()` for paginated fetch
  - Cache: `HashMap<String, CacheEntry>` at `.baez/.sync_cache.json`
  - For each doc: compare `updated_at` against cache, skip if unchanged (unless `--force`)
  - Extracts user notes via `doc_summary.user_notes()` → `prosemirror_to_markdown()`
  - Optionally summarizes via Claude API (if `summarize_state` is Some)
  - Writes: markdown file, transcript JSON, metadata JSON (all via `write_atomic`)
  - Sets file mtime to meeting `created_at`
  - Handles filename changes: removes old files when slug changes
  - Progress bar via indicatif
- **`fix_dates(paths)`** → `Result<()>`: Walk all .md files, set mtime from frontmatter
- **`CacheEntry`** (struct): `{ filename: String, updated_at: DateTime<Utc> }`

## Critical Detail
- Cache stores `doc_summary.updated_at` (from list endpoint), NOT `meta.updated_at` (from metadata endpoint) — they can differ. The comparison must use the same source.
- `meta.created_at` is overridden with `doc_summary.created_at` because the metadata endpoint sometimes omits it.

## Summarization Integration
- Feature-gated with `#[cfg(feature = "summaries")]`
- Loads `SummaryConfig`, resolves API key, builds Claude client
- Non-fatal: failures print warning, continue sync
