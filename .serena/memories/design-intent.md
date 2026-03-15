# Design Intent & Conventions

## Why This Architecture
- **Single-user CLI**: No concurrency concerns, blocking HTTP is simpler than async
- **Obsidian-first**: Output designed for Obsidian's specific features (wiki-links, Dataview, daily notes)
- **Defensive parsing**: All API models use `#[serde(default)]` and `Option<>` because the Granola API has no public docs and responses vary
- **Raw archival**: Both parsed and raw JSON stored so data is never lost even if parsing changes
- **Incremental by default**: Sync cache avoids re-fetching unchanged documents

## Trade-offs
- **Blocking HTTP over async**: Simpler code, acceptable for a CLI tool that runs serially
- **Granola internal API**: Uses undocumented endpoints with desktop app headers — fragile but necessary (no official API for transcripts)
- **Feature-gated summarization**: Keeps the core sync lean; users who don't want AI can compile without keyring dependency
- **No database**: Simple JSON cache file instead of SQLite — sufficient for the scale
- **Atomic writes via rename**: Not truly atomic on all filesystems but good enough for single-user

## Conventions
- **Naming**: Snake case throughout, Rust standard
- **Module comments**: `//!` doc comments at top of each file describing purpose
- **Error handling**: `?` propagation, `thiserror` for enum variants, structured exit codes
- **Feature gating**: `#[cfg(feature = "summaries")]` on types, functions, and CLI commands
- **Testing**: Inline test modules, each file has its own tests
- **File organization**: One concern per file, clear module boundaries
- **No doc comments on private functions**: Only `///` on public API
- **Tolerant parsing**: Never fail on unknown API fields, always default missing fields

## Technical Debt / Simplifications
- `summary.rs` retry logic duplicates the pattern from `api.rs` but with string-matching for status codes instead of structured error matching
- The `Monologue`/`Segment`/`Block` types in `model.rs` appear unused (legacy, kept for backward compat)
- `TimestampValue` enum handles seconds vs string but it's only used via `normalize_timestamp_legacy()` which also appears unused in the main flow
- No structured logging — all diagnostics via print macros
- Watch mode and other vault enhancements are planned but not yet implemented (see docs/superpowers/)

## Planned Enhancements (from design docs)
1. Backlink-aware updates (`<!-- baez-managed-above -->` marker)
2. Dataview inline fields in summary prompt
3. Daily notes linking (`Daily Notes/YYYY-MM-DD.md`)
4. Watch mode (`--watch --interval`)
