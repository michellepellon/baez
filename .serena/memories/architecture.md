# Architecture

## Entry Points
- **`src/main.rs`**: CLI entrypoint. Parses `Cli` via clap, dispatches commands. Contains `run()` → match on `Commands` enum. Error handling: prints `[E{code}] {msg}` and exits with structured codes.
- **Binary:** `baez` (defined in `Cargo.toml [[bin]]`)
- **Library:** `baez` (defined in `Cargo.toml [lib]`, path `src/lib.rs`)

## Module Structure

```
src/
├── main.rs      — CLI entrypoint, command dispatch
├── lib.rs       — Module re-exports (public API surface)
├── cli.rs       — clap definitions: Cli struct, Commands enum, flag parsers
├── model.rs     — Serde data models for Granola API + Obsidian frontmatter
├── api.rs       — ApiClient: blocking HTTP client for Granola API
├── auth.rs      — Token resolution chain (CLI → env → config → session file)
├── sync.rs      — sync_all() + summarize_all_docs(), caches, SummarizationContext, summarize_and_reconcile
├── storage.rs   — Vault paths, atomic writes, config file, frontmatter parsing, PeopleIndex, entity note CRUD
├── convert.rs   — Transcript → Obsidian markdown conversion, ProseMirror → markdown
├── summary.rs   — Claude API summarization + ExtractedEntities parsing (feature-gated)
├── error.rs     — Error enum with thiserror, exit codes
└── util.rs      — slugify, doc_slug, count_transcript_words, normalize_timestamp, retry_with_backoff, levenshtein_distance
```

## Data Flow

1. **Auth:** `resolve_token()` in `auth.rs` resolves Granola token from CLI flag → `BAEZ_GRANOLA_TOKEN` → `BEARER_TOKEN` (deprecated) → config file → Granola session file
2. **API Client:** `ApiClient` in `api.rs` uses blocking reqwest with Bearer token auth. Mimics Granola desktop app headers (`User-Agent: Granola/5.354.0`). Random throttle (100-300ms) between requests.
3. **Sync loop** (`sync_all()` in `sync.rs`):
   - `client.list_documents_with_notes()` — paginated fetch of all docs with notes/panels
   - Loads both `.sync_cache.json` (timestamp-based skip) and `.summary_cache.json` (summarization-done skip)
   - For each doc: compare `updated_at` against sync cache → if unchanged, take **summarize-only path** if summary cache lacks entry and raw files exist locally
   - Otherwise: fetch metadata + transcript via API (keeping raw JSON)
   - Triage: `count_transcript_words(&transcript)` < 20 → `status = "stub"` (skip summary), else `"substantive"`
   - Extract user notes from ProseMirror `notes` field (with `last_viewed_panel.content` fallback)
   - If substantive + key available: `summarize_and_reconcile()` summarizes via Claude, parses entity JSON block, creates/enriches entity notes, updates summary cache
   - `to_markdown()` converts to Obsidian-flavored markdown with frontmatter (incl. `related` list of `[[wiki-links]]` to entities, `status` field)
   - `write_atomic()` writes files via temp+rename with 0o600 permissions
   - `set_file_time()` sets mtime to meeting creation date
   - Update sync cache atomically
4. **Batch summarization** (`summarize_all_docs()` in `sync.rs`):
   - Reads only from local raw JSON + sync cache — **never hits the Granola API**
   - Backfills `.summary_cache.json` by scanning existing markdown for `## Summary` if cache is empty
   - For each unsummarized doc: parse local raw files → `summarize_and_reconcile()` → update existing markdown's `## Summary` section + `related` frontmatter
5. **Output:** Markdown files in `Vault/Granola/YYYY/MM/`, raw JSON in `.baez/raw/`, entity notes in `Vault/People|Concepts|Projects/`

## Key APIs Used
- **Granola internal API** (`api.granola.ai`):
  - `POST /v2/get-documents` (list, with pagination + panels)
  - `POST /v1/get-document-metadata`
  - `POST /v1/get-document-transcript`
- **Granola public API** (`public-api.granola.ai`):
  - `GET /v1/notes/{id}` (PublicNote with summary_text)
- **Anthropic Messages API** (`api.anthropic.com/v1/messages`): Claude summarization

## Integration Boundaries
- **Granola API**: HTTP/JSON, Bearer token auth, no official docs
- **Anthropic API**: HTTP/JSON, x-api-key auth, Messages API v2023-06-01
- **Obsidian Vault**: Filesystem, markdown with YAML frontmatter, `[[wiki-links]]`, Dataview compatibility
- **macOS Keychain**: via `keyring` crate (Anthropic API key storage)
- **Config file**: `~/.config/baez/config.json` (JSON, XDG_CONFIG_HOME respected)

## Vault Layout
```
Vault/
├── People/                 — Entity notes (auto-created when summarization runs)
├── Concepts/
├── Projects/
└── Granola/
    ├── YYYY/MM/YYYY-MM-DD_slug.md
    └── .baez/
        ├── raw/                 (transcript + metadata JSON)
        ├── summaries/           (legacy summary files dir)
        ├── tmp/                 (atomic write temp dir)
        ├── summary_config.json
        ├── .sync_cache.json     (incremental sync state)
        └── .summary_cache.json  (summarization progress)
```
Note: `People/`, `Concepts/`, `Projects/` are at the **vault root**, not under `Granola/` — they are first-class Obsidian notes the user navigates directly.

## Configuration
- `SummaryConfig` in `summary.rs`: model (default: `claude-opus-4-6`), max_input_chars (600K), max_tokens (8192), custom_prompt, temperature
- Stored at `.baez/summary_config.json` in vault
- `Paths` in `storage.rs`: vault_dir, granola_dir, baez_dir, raw_dir, summaries_dir, tmp_dir
