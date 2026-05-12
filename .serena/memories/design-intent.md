# Design Intent & Conventions

## Why This Architecture
- **Single-user CLI**: No concurrency concerns, blocking HTTP is simpler than async
- **Obsidian-first**: Output designed for Obsidian's specific features (wiki-links, Dataview, daily notes, graph view)
- **Defensive parsing**: All API models use `#[serde(default)]` and `Option<>` because the Granola API has no public docs and responses vary
- **Raw archival**: Both parsed and raw JSON stored so data is never lost even if parsing changes
- **Incremental by default**: Sync cache avoids re-fetching unchanged documents
- **Resumable summarization**: A separate `.summary_cache.json` decouples "have we summarized this?" from "have we synced this?" so an interrupted sync can resume summarization without re-fetching from Granola
- **Knowledge graph as a side effect of summarization**: Claude returns a JSON entity block alongside the markdown summary, parsed by `parse_summary_output()`. Entities are reconciled (matched against existing notes via fuzzy matching) or created in `People/`, `Concepts/`, `Projects/`.

## Trade-offs
- **Blocking HTTP over async**: Simpler code, acceptable for a CLI tool that runs serially
- **Granola internal API**: Uses undocumented endpoints with desktop app headers — fragile but necessary (no official API for transcripts)
- **Feature-gated summarization**: Keeps the core sync lean; users who don't want AI can compile without keyring dependency. Entity reconciliation is also gated since it depends on Claude output.
- **No database**: Simple JSON cache files instead of SQLite — sufficient for the scale
- **Atomic writes via rename**: Not truly atomic on all filesystems but good enough for single-user
- **Entity JSON in HTML comment**: The `<!-- baez-entities ... -->` block keeps machine-readable data inline with the summary but invisible in rendered Obsidian — avoids needing a sidecar file while remaining structured.
- **Fuzzy name matching (Levenshtein, threshold 2)**: Catches typos and minor variations but skipped for names ≤ 5 chars to avoid false positives. Ambiguous matches return None rather than guessing.

## Conventions
- **Naming**: Snake case throughout, Rust standard
- **Module comments**: `//!` doc comments at top of each file describing purpose
- **Error handling**: `?` propagation, `thiserror` for enum variants, structured exit codes
- **Feature gating**: `#[cfg(feature = "summaries")]` on types, functions, and CLI commands. Knowledge-graph reconciliation in `sync.rs` is also gated.
- **Testing**: Inline test modules, each file has its own tests (often multiple modules per file for logical grouping)
- **File organization**: One concern per file, clear module boundaries
- **No doc comments on private functions**: Only `///` on public API
- **Tolerant parsing**: Never fail on unknown API fields, always default missing fields
- **Triage before LLM call**: `count_transcript_words(&transcript)` < 20 → classify as `"stub"`, do not summarize. Avoids burning API budget on placeholder meetings.

## Technical Debt / Simplifications
- `summary.rs` retry logic duplicates the pattern from `api.rs` but with string-matching for status codes instead of structured error matching
- The `Monologue`/`Segment`/`Block` types in `model.rs` appear unused (legacy, kept for backward compat)
- `TimestampValue` enum handles seconds vs string but it's only used via `normalize_timestamp_legacy()` which also appears unused in the main flow
- No structured logging — all diagnostics via print macros
- `summarize_and_reconcile()` is feature-gated `pub(crate)`; it bundles summarization + entity creation in a single transaction-like function. The `sync_all` summarize-only path duplicates some of its post-processing (markdown update + `merge_frontmatter_related`) — refactor candidate.
- `summarize_all_docs()` has a backfill-from-existing-docs block (~80 lines) that runs only when summary cache is empty + no `--force`. Heuristic-based (looks for `\n## Summary\n` substring) — works but is fragile.

## Planned Enhancements
1. Backlink-aware updates (`<!-- baez-managed-above -->` marker) — see `docs/superpowers/`
2. Daily notes linking (`Daily Notes/YYYY-MM-DD.md`)
3. Watch mode (`--watch --interval`)
