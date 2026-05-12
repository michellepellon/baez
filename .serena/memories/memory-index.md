# Memory Index

## Project
- **project-overview** — What baez is, tech stack, commands, build/test/run instructions, configuration
- **architecture** — Entry points, module structure, data flow, vault layout, API endpoints, integration boundaries
- **design-intent** — Why the codebase is structured this way, trade-offs, conventions, technical debt, planned enhancements

## Modules
- **module-api** — ApiClient: Granola HTTP client with throttling, retry, pagination
- **module-model** — Serde data models for API responses, ProseMirror, Frontmatter (incl. `related`, `status`)
- **module-sync** — sync_all() + summarize_all_docs(), SummarizationContext, summarize_and_reconcile, caches
- **module-storage** — Vault paths, atomic writes, permissions, config file, frontmatter parsing, PeopleIndex, entity note CRUD
- **module-convert** — Transcript → Obsidian markdown, ProseMirror → markdown, wiki-links/tags. `to_markdown` signature with `related` + `status`.
- **module-summary** — Claude API summarization, entity extraction (ExtractedEntities), parse_summary_output, build_context_preamble
- **module-auth** — Granola token resolution chain (CLI → env → config → session file)
- **module-error** — Error enum with thiserror, structured exit codes (2-7)
- **module-util** — slugify, doc_slug, count_transcript_words, normalize_timestamp, retry_with_backoff, levenshtein_distance

## Features
- **knowledge-graph** — End-to-end flow for People/Concepts/Projects extraction: triage, preamble, Claude prompt, parse, reconcile (PeopleIndex + fuzzy match), entity note CRUD, meeting backlinks, design decisions

## Cross-Cutting
- **cross-cutting** — Error handling, retry, caches (sync + summary), triage, knowledge-graph reconciliation, security, testing, logging patterns, backward compatibility

## Development
- **suggested_commands** — Build, test, lint, format, install, run commands (incl. `summarize-all`)
- **task-completion** — What to run after completing a task (fmt, lint, test)
- **style-conventions** — Rust style, serde patterns, feature gating, testing conventions
