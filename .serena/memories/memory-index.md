# Memory Index

## Project
- **project-overview** — What baez is, tech stack, commands, build/test/run instructions, configuration
- **architecture** — Entry points, module structure, data flow, API endpoints, integration boundaries
- **design-intent** — Why the codebase is structured this way, trade-offs, conventions, planned enhancements

## Modules
- **module-api** — ApiClient: Granola HTTP client with throttling, retry, pagination
- **module-model** — Serde data models for API responses, ProseMirror, frontmatter
- **module-sync** — sync_all() orchestration, cache management, incremental updates
- **module-storage** — Vault paths, atomic writes, permissions, config file, frontmatter parsing
- **module-convert** — Transcript → Obsidian markdown, ProseMirror → markdown, wiki-links/tags
- **module-summary** — Claude API summarization, chunking, prompt, API key resolution
- **module-auth** — Granola token resolution chain (CLI → env → config → session file)
- **module-error** — Error enum with thiserror, structured exit codes (2-7)
- **module-util** — slugify, timestamp normalization, generic retry_with_backoff

## Cross-Cutting
- **cross-cutting** — Error handling, retry, security, testing, logging patterns

## Development
- **suggested_commands** — Build, test, lint, format, install, run commands
- **task-completion** — What to run after completing a task (fmt, lint, test)
- **style-conventions** — Rust style, serde patterns, feature gating, testing conventions
