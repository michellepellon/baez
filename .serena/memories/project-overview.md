# Project Overview

**Baez** is a Rust CLI tool that syncs [Granola](https://granola.ai) meeting transcripts into an [Obsidian](https://obsidian.md) vault as enriched, read-only markdown with optional AI summaries powered by Claude (Anthropic API). Now also extracts a **knowledge graph** (People, Concepts, Projects) across meetings.

## What It Does
- Fetches all Granola transcripts via the Granola API (internal endpoints, not officially documented)
- Writes Obsidian-flavored markdown with `[[wiki-links]]`, `#tags`, and Dataview-compatible frontmatter
- Organizes files by date: `Vault/Granola/YYYY/MM/YYYY-MM-DD_slug.md`
- Preserves raw JSON API responses for archival in `.baez/raw/`
- Incremental sync — only fetches documents that changed since last run (via `.sync_cache.json`)
- Generates AI meeting summaries using Claude API, with Obsidian-native formatting
- **Extracts entities** — creates/enriches `People/`, `Concepts/`, `Projects/` notes interconnected via `[[wiki-links]]` and `related` frontmatter
- **Resumable summarization** — `.summary_cache.json` tracks which docs have been summarized; rerunning skips done docs
- Stores Anthropic API key in macOS keychain via `keyring` crate

## Tech Stack
- **Language:** Rust (edition 2021, MSRV 1.86)
- **CLI:** clap 4.5 with derive macros
- **HTTP:** reqwest 0.12 (blocking client, JSON + gzip)
- **Serialization:** serde + serde_json + serde_yaml
- **Time:** chrono 0.4
- **File I/O:** filetime (for setting mtime), atomic writes via temp+rename
- **Progress:** indicatif 0.17
- **Fuzzy matching:** strsim 0.11 (Levenshtein for person name reconciliation)
- **Testing:** wiremock 0.6 (async mocking), insta 1.34 (snapshots), assert_fs, tempfile
- **Optional:** keyring 2.3 (behind `summaries` feature flag)

## Feature Flags
- `default = ["summaries"]` — includes AI summarization + keychain support + knowledge graph
- `summaries` — gates `keyring` dependency and `summary.rs` module; CLI commands `set-api-key`, `set-config`, `summarize`, `summarize-all` are `#[cfg(feature = "summaries")]`. Knowledge-graph reconciliation in `sync.rs` is also gated.

## Commands
- `baez sync` — Main sync command (with `--force`, `--no-summarize`, `--dry-run`, `--verbose`)
- `baez list` — List all documents
- `baez fetch <id>` — Fetch a single document
- `baez open` — Open vault directory
- `baez fix-dates` — Fix file mtimes to match meeting creation dates
- `baez set-api-key <key>` — Store Anthropic key in macOS keychain
- `baez set-config` — Configure summarization (model, context window, prompt)
- `baez summarize <doc-id>` — Summarize a single document
- `baez summarize-all` — Batch-summarize all synced docs that lack summaries (reads local raw JSON, never hits Granola API). `--force` re-summarizes all, `--dry-run` previews.

## How to Build, Test, Run
- Build: `cargo build --release` or `just build`
- Test: `cargo test --lib` (unit), `cargo test --all-features --no-fail-fast` (all)
- Lint: `cargo clippy --all-features -- -D warnings && cargo clippy --no-default-features -- -D warnings`
- Format: `cargo fmt`
- Full CI: `just ci` (fmt + lint + test-all)
- Install: `cargo install --path .`

## Configuration
- Config file: `~/.config/baez/config.json` (vault, granola_token, anthropic_api_key)
- Env vars: `BAEZ_VAULT`, `BAEZ_GRANOLA_TOKEN`, `BAEZ_ANTHROPIC_API_KEY`, `ANTHROPIC_API_KEY`
- Granola auto-discovery: reads `~/Library/Application Support/Granola/supabase.json`

## Repository Conventions
- MIT license
- GitHub Actions CI: test on ubuntu+macos, lint on ubuntu
- Branching: main branch
- Keep a Changelog format
- Release binary optimized for size (LTO, strip, panic=abort)
