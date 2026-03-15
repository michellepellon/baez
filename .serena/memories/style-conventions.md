# Code Style and Conventions

## Rust Style
- Edition 2021, MSRV 1.86
- Standard Rust naming: snake_case functions/variables, CamelCase types, SCREAMING_SNAKE constants
- `//!` module-level doc comments on every file (purpose + one-line description)
- `///` doc comments on public functions/structs only
- No inline comments unless the logic is non-obvious
- `use` imports grouped: `crate::` first, then external crates, then std

## Error Handling
- Use `?` operator for propagation
- `thiserror` derive for the `Error` enum
- Never `unwrap()` in production code (ok in tests)
- Map errors at boundaries (e.g., `map_err` when converting between error types)

## Serde Conventions
- All API models: `#[serde(default)]` on optional/collection fields
- Never `#[serde(deny_unknown_fields)]` — forward compat
- Use `#[serde(rename = "...")]` for JSON field name mapping
- Use `#[serde(alias = "...")]` for backward compat with old formats

## Feature Gating
- `#[cfg(feature = "summaries")]` on modules, types, functions, and match arms
- Pair with `#[cfg(not(feature = "summaries"))]` for default behavior (e.g., `let _ = summarize;`)
- Feature-gated code should compile cleanly when disabled

## Testing
- Inline `#[cfg(test)] mod tests { ... }` in each source file
- Multiple test modules per file for logical grouping (e.g., `tests`, `metadata_tests`, `frontmatter_tests`)
- Use `tempfile::TempDir` for filesystem isolation
- Use `wiremock` for HTTP mocking (async tests with `#[tokio::test]`)
- Use `insta` for snapshot testing of complex outputs
- Test both feature configurations
