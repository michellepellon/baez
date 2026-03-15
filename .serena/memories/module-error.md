# Module: error.rs — Error Types

## Purpose
Structured error types with stable exit codes for CLI reporting.

## Key Symbols
- **`Error`** (enum, `#[derive(thiserror::Error)]`):
  - `Auth(String)` → exit code 2
  - `Network(reqwest::Error)` → exit code 3
  - `Api { endpoint, status, message }` → exit code 4
  - `Parse(serde_json::Error)` → exit code 5
  - `Filesystem(std::io::Error)` → exit code 6
  - `Summarization(String)` → exit code 7
- **`Result<T>`** = `std::result::Result<T, Error>`

## Error Display
`main.rs` prints: `baez: [E{code}] {message}` and exits with the code.
