# Module: util.rs — Utilities

## Purpose
Shared utility functions for slugging, timestamps, and retry logic.

## Key Symbols
- **`slugify(text)`** → `String`: URL-safe slug via `slug` crate, empty → `"untitled"`
- **`normalize_timestamp(ts)`** → `Option<String>`: ISO 8601 datetime → `HH:MM:SS`, or `HH:MM:SS.sss` → `HH:MM:SS`
- **`normalize_timestamp_legacy(ts: &TimestampValue)`** → `Option<String>`: Handles both `Seconds(f64)` and `String` variants
- **`retry_with_backoff(max_retries, initial_delay, operation, should_retry)`** → `Result<T, E>`: Generic retry with exponential backoff, used by both `api.rs` and `summary.rs`
