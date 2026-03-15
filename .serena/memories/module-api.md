# Module: api.rs — Granola API Client

## Purpose
Blocking HTTP client for the Granola API with throttling, retry, and auth.

## Key Symbols
- **`ApiClient`** (struct): Holds `reqwest::blocking::Client`, base_url, token, throttle range
  - `new(token, base_url)` → creates client with 30s timeout
  - `with_throttle(min, max)` / `disable_throttle()` → configure inter-request delay
  - `list_documents()` → `Vec<DocumentSummary>` via `POST /v2/get-documents`
  - `list_documents_with_notes()` → paginated (batch 100), includes panels, deduplication guard
  - `get_metadata(doc_id)` / `get_metadata_with_raw(doc_id)` → `DocumentMetadata`
  - `get_transcript(doc_id)` / `get_transcript_with_raw(doc_id)` → `RawTranscript`
  - `get_public_note(note_id)` → `PublicNote` (hardcoded `public-api.granola.ai`)
- **`ApiResponse<T>`** (struct): `{ raw: String, parsed: T }` — preserves raw JSON for archival
- **`is_retryable(err)`** (fn): Retries on network errors, 429, 5xx

## Patterns
- All internal API calls use POST with Granola desktop app headers
- `post_with_raw()` is the core method — retry wrapper via `util::retry_with_backoff`
- `truncate_str()` for safe UTF-8 truncation of error previews
- `MAX_RETRIES = 2`, `INITIAL_RETRY_DELAY = 1s`

## Dependencies
- `crate::{DocumentMetadata, DocumentSummary, Error, PublicNote, RawTranscript, Result}`
- `reqwest::blocking::Client`, `serde_json::json!`, `rand::Rng`
