# Cross-Cutting Concerns

## Error Handling
- Central `Error` enum in `error.rs` with `thiserror` derive
- Each variant has a stable exit code (2-7)
- API errors distinguished: `Auth`, `Network`, `Api` (with status), `Parse`, `Filesystem`, `Summarization`
- `is_retryable()` in `api.rs`: retries network errors, 429, 5xx
- Summarization failures during sync are non-fatal (warning printed, sync continues)
- `main.rs` top-level: catches all errors, prints formatted message, exits with code

## Retry Logic
- Generic `retry_with_backoff()` in `util.rs`, used by both API client and Claude client
- Max 2 retries, exponential backoff (1s/2s for API, 2s/4s for Claude)
- Caller provides `should_retry` predicate

## Security
- Atomic file writes via temp+rename (prevents partial writes)
- Files written with 0o600 (owner-only read/write)
- `.baez/` directories: 0o700
- Config file permission warning for loose permissions with secrets
- macOS keychain for Anthropic API key (most secure option)
- Granola token auto-discovery from local session file (no network exposure)

## Testing
- **Unit tests**: Inline `#[cfg(test)]` modules in every source file
- **Integration tests**: `tests/api_integration.rs` (wiremock HTTP mocking), `tests/workflow_integration.rs` (file I/O, ProseMirror conversion, vault layout)
- **Snapshot tests**: insta for full markdown output verification
- **Test fixtures**: `tempfile::TempDir` for isolated filesystem tests
- **Async bridge**: Integration tests use `tokio::test` + `spawn_blocking` because wiremock is async but ApiClient is blocking
- Both feature sets tested: `--all-features` and `--no-default-features`

## Logging & Observability
- `--verbose` flag prints token source, API key source, and diagnostic info to stderr
- Progress bar (indicatif) during sync
- Warnings printed to stderr for non-fatal issues (missing API key, permission issues, failed summaries)
- No structured logging framework — all output via `println!`/`eprintln!`

## Backward Compatibility
- Frontmatter field aliases (`created_at` → `created`, `participants` → `attendees`, etc.)
- `BEARER_TOKEN` env var accepted but deprecated
- Old flat file layout still handled (filename change detection in sync cache)
- Legacy transcript types (`Segment`, `Monologue`) preserved
- Unknown JSON fields silently ignored in all API models
