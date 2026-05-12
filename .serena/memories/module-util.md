# Module: util.rs — Utilities

## Purpose
Shared utility functions for slugging, timestamps, retry logic, triage, and fuzzy string matching.

## Key Symbols
- **`slugify(text)`** → `String`: URL-safe slug via `slug` crate, empty → `"untitled"`
- **`doc_slug(title: Option<&str>, doc_id: &str)`** → `String`: Calls `slugify`, but when the result is `"untitled"`, appends the first 8 alphanumeric chars of `doc_id` to disambiguate (e.g. `untitled-abc12345`). Used everywhere a filename slug is built to prevent collisions across untitled meetings.
- **`count_transcript_words(transcript)`** → `usize`: Sums whitespace-separated word counts across all transcript entries. Used for triage: < 20 → `status = "stub"`, skip summary.
- **`normalize_timestamp(ts)`** → `Option<String>`: ISO 8601 datetime → `HH:MM:SS`, or `HH:MM:SS.sss` → `HH:MM:SS`
- **`normalize_timestamp_legacy(ts: &TimestampValue)`** → `Option<String>`: Handles both `Seconds(f64)` and `String` variants (unused in main flow)
- **`retry_with_backoff(max_retries, initial_delay, operation, should_retry)`** → `Result<T, E>`: Generic retry with exponential backoff, used by both `api.rs` and `summary.rs`
- **`levenshtein_distance(a, b)`** → `usize`: Thin wrapper over `strsim::levenshtein`. Used by `PeopleIndex::find_match` for fuzzy person matching (threshold 2, skipped for names ≤ 5 chars).

## Test Modules
- `tests`, `retry_tests`, `timestamp_tests`, `triage_tests`, `levenshtein_tests`
