# Architecture

## Entry Points
- **`src/main.rs`**: CLI entrypoint. Parses `Cli` via clap, dispatches commands. Contains `run()` → match on `Commands` enum. Error handling: prints `[E{code}] {msg}` and exits with structured codes.
- **Binary:** `baez` (defined in `Cargo.toml [[bin]]`)
- **Library:** `baez` (defined in `Cargo.toml [lib]`, path `src/lib.rs`)

## Module Structure

```
src/
├── main.rs      — CLI entrypoint, command dispatch, watch loop (planned)
├── lib.rs       — Module re-exports (public API surface)
├── cli.rs       — clap definitions: Cli struct, Commands enum, flag parsers
├── model.rs     — Serde data models for Granola API + Obsidian frontmatter
├── api.rs       — ApiClient: blocking HTTP client for Granola API
├── auth.rs      — Token resolution chain (CLI → env → config → session file)
├── sync.rs      — Core sync_all() orchestration + fix_dates()
├── storage.rs   — Vault paths, atomic writes, config file, frontmatter parsing
├── convert.rs   — Transcript → Obsidian markdown conversion, ProseMirror → markdown
├── summary.rs   — Claude API summarization (feature-gated behind "summaries")
├── error.rs     — Error enum with thiserror, exit codes
└── util.rs      — slugify, timestamp normalization, retry_with_backoff
```

## Data Flow

1. **Auth:** `resolve_token()` in `auth.rs` resolves Granola token from CLI flag → `BAEZ_GRANOLA_TOKEN` → `BEARER_TOKEN` (deprecated) → config file → Granola session file
2. **API Client:** `ApiClient` in `api.rs` uses blocking reqwest with Bearer token auth. Mimics Granola desktop app headers (`User-Agent: Granola/5.354.0`). Random throttle (100-300ms) between requests.
3. **Sync loop** (`sync_all()` in `sync.rs`):
   - `client.list_documents_with_notes()` — paginated fetch of all docs with notes/panels
   - For each doc: compare `updated_at` against cache → skip if unchanged
   - Fetch metadata + transcript via separate API calls (keeping raw JSON)
   - Extract user notes from ProseMirror `notes` field (with `last_viewed_panel.content` fallback)
   - Optionally summarize via Claude API
   - `to_markdown()` converts to Obsidian-flavored markdown with frontmatter
   - `write_atomic()` writes files via temp+rename with 0o600 permissions
   - `set_file_time()` sets mtime to meeting creation date
   - Update sync cache atomically
4. **Output:** Markdown files in `Vault/Granola/YYYY/MM/`, raw JSON in `.baez/raw/`

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

## Configuration
- `SummaryConfig` in `summary.rs`: model (default: claude-opus-4-6), max_input_chars (600K), max_tokens (4096), custom_prompt, temperature
- Stored at `.baez/summary_config.json` in vault
- `Paths` in `storage.rs`: vault_dir, granola_dir, baez_dir, raw_dir, summaries_dir, tmp_dir
