# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [0.1.0] - 2026-02-07

### Added
- Sync Granola meeting transcripts into an Obsidian vault as structured markdown
- Incremental sync with cache — only fetches documents that changed since last run
- AI-generated meeting summaries via Claude (Anthropic API) with Obsidian-native formatting
- Obsidian-flavored output: `[[wiki-links]]`, `#tags`, Dataview-compatible frontmatter
- Date-based file organization: `Vault/Granola/2025/01/2025-01-15_standup.md`
- Raw JSON archival of all API responses
- ProseMirror-to-markdown conversion for user notes
- Configurable summarization: model, context window, custom prompts
- macOS keychain storage for Anthropic API key
- Atomic file writes with restricted permissions
- Retry with exponential backoff for transient API errors
- `BAEZ_GRANOLA_TOKEN` and `BAEZ_ANTHROPIC_API_KEY` namespaced env vars
- Config file (`~/.config/baez/config.json`) for all settings including API keys
- Runtime warning when config file containing keys has loose permissions
- `--verbose` flag for diagnostic output
- `--dry-run` flag for sync (preview without writing)
- Progress bar during sync
- GitHub Actions CI (test + lint on Linux and macOS)

### Commands
- `baez sync` — Sync all documents (with optional AI summaries)
- `baez list` — List all documents
- `baez fetch <id>` — Fetch a specific document
- `baez open` — Open vault directory
- `baez fix-dates` — Fix file modification dates to match meeting dates
- `baez set-api-key <key>` — Store Anthropic API key in macOS keychain
- `baez set-config` — Configure summarization settings
- `baez summarize <doc-id>` — Summarize a single document

### Deprecated
- `BEARER_TOKEN` env var — use `BAEZ_GRANOLA_TOKEN` instead
