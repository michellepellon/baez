# Cross-Cutting Concerns

## Error Handling
- Central `Error` enum in `error.rs` with `thiserror` derive
- Each variant has a stable exit code (2-7)
- API errors distinguished: `Auth`, `Network`, `Api` (with status), `Parse`, `Filesystem`, `Summarization`
- `is_retryable()` in `api.rs`: retries network errors, 429, 5xx
- Summarization failures during sync are non-fatal (warning printed, sync continues)
- Entity creation/enrichment failures during reconciliation are non-fatal (warning printed, continue)
- `parse_summary_output` failures are non-fatal — markdown is preserved, entities defaulted to None
- `main.rs` top-level: catches all errors, prints formatted message, exits with code

## Retry Logic
- Generic `retry_with_backoff()` in `util.rs`, used by both API client and Claude client
- Max 2 retries, exponential backoff (1s/2s for API, 2s/4s for Claude)
- Caller provides `should_retry` predicate

## Caches
- **`.sync_cache.json`** (`HashMap<String, CacheEntry>`): incremental sync state, keyed by doc_id, stores `filename` + `doc_summary.updated_at`
- **`.summary_cache.json`** (`HashMap<String, SummaryCacheEntry>`): per-doc summarization done-set, keyed by doc_id, stores `summarized_at` + `model`
- Backfill: when summary cache is empty + not `--force`, `summarize_all_docs` scans existing markdown files for `\n## Summary\n` and pre-populates the cache (records `model: "unknown (backfilled)"`)
- Both caches persisted via atomic write

## Security
- Atomic file writes via temp+rename (prevents partial writes)
- Files written with 0o600 (owner-only read/write)
- `.baez/` directories: 0o700
- Config file permission warning for loose permissions with secrets
- macOS keychain for Anthropic API key (most secure option)
- Granola token auto-discovery from local session file (no network exposure)

## Triage
- `count_transcript_words(&transcript)` < 20 → classify as `"stub"`, skip Claude entirely
- Stubs still get markdown + frontmatter (with `status: stub`); just no summary or entities
- Prevents wasting tokens/money on placeholder meetings

## Knowledge Graph Reconciliation
- Three entity directories at vault root: `People/`, `Concepts/`, `Projects/`
- `PeopleIndex` (in-memory) built once per run, mutated as new people are added
- `context_preamble` rebuilt after every new concept/project creation so within-run dedup works
- Cross-cuts `sync.rs` (orchestration), `summary.rs` (parsing, preamble), `storage.rs` (note CRUD, index, merge)

## Testing
- **Unit tests**: Inline `#[cfg(test)]` modules in every source file (often multiple modules per file: `tests`, `metadata_tests`, `frontmatter_tests`, `triage_tests`, `levenshtein_tests`, `people_index_tests`, `entity_note_tests`, etc.)
- **Integration tests**: `tests/api_integration.rs` (wiremock HTTP mocking, 6 tests), `tests/workflow_integration.rs` (15 tests covering file I/O, ProseMirror conversion, vault layout, entity note creation, triage, related/status in markdown)
- **Snapshot tests**: insta for full markdown output verification
- **Test fixtures**: `tempfile::TempDir` for isolated filesystem tests
- **Async bridge**: Integration tests use `tokio::test` + `spawn_blocking` because wiremock is async but ApiClient is blocking
- Both feature sets tested: `--all-features` and `--no-default-features`
- Summary-parsing tests gated behind `summaries` feature

## Logging & Observability
- `--verbose` flag prints token source, API key source, and diagnostic info to stderr
- Progress bar (indicatif) during sync and batch summarization
- Warnings printed to stderr for non-fatal issues (missing API key, permission issues, failed summaries, failed entity enrichment, malformed entity JSON)
- No structured logging framework — all output via `println!`/`eprintln!`
- Sync stats line at end: `synced N docs (X new/updated, Y skipped, Z summarized, W catch-up summarized, P people, C concepts, Pr projects)`

## Backward Compatibility
- Frontmatter field aliases (`created_at` → `created`, `participants` → `attendees`, etc.)
- `BEARER_TOKEN` env var accepted but deprecated
- Old flat file layout still handled (filename change detection in sync cache)
- Legacy transcript types (`Segment`, `Monologue`) preserved
- Unknown JSON fields silently ignored in all API models
- Summary cache backfill heals vaults that were summarized before the cache existed
- `related` and `status` use `skip_serializing_if` so old docs don't get noisy frontmatter when not applicable
