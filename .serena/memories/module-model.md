# Module: model.rs — Data Models

## Purpose
Serde data models for Granola API responses and Obsidian frontmatter. Tolerant parsing with `#[serde(default)]` on most fields.

## Key Types

### API Response Models
- **`DocumentSummary`**: Lightweight doc listing (`id`, `title`, `created_at`, `updated_at`, `notes: Option<Value>`, `last_viewed_panel: Option<Value>`)
  - `user_notes()` method: extracts ProseMirror doc from `notes`, falls back to `last_viewed_panel.content`
- **`DocumentMetadata`**: Rich metadata (`id`, `title`, `created_at` (defaults to Utc::now), `updated_at`, `participants`, `duration_seconds`, `labels`, `creator: Option<Attendee>`, `attendees: Option<Vec<Attendee>>`)
- **`RawTranscript`**: `#[serde(transparent)]` wrapper around `Vec<TranscriptEntry>`
- **`TranscriptEntry`**: `{ document_id, start (start_timestamp), end (end_timestamp), text, source, id, is_final, speaker }`
- **`PublicNote`**: `{ id, title, summary_text }` — from public API

### Attendee Models (rich metadata)
- **`Attendee`**: `{ name, email, details: Option<PersonDetails> }`
- **`PersonDetails`**: `{ person: Option<PersonInfo>, company: Option<CompanyInfo> }`
- **`PersonInfo`**: `{ name: Option<PersonName>, employment: Option<Employment>, linkedin: Option<LinkedIn> }`

### ProseMirror Models
- **`ProseMirrorDoc`**: `{ node_type ("type"), content: Option<Vec<ProseMirrorNode>> }`
- **`ProseMirrorNode`**: `{ node_type, content, text, attrs, marks }`
- **`ProseMirrorMark`**: `{ mark_type ("type") }` — bold, italic

### Obsidian Models
- **`Frontmatter`**: Dataview-compatible YAML (`doc_id`, `source`, `date`, `created`, `updated`, `title`, `attendees`, `duration_minutes`, `tags`, `generator`)
  - Backward compat aliases: `created_at` → `created`, `participants` → `attendees`, `labels` → `tags`, `duration_seconds` → `duration_minutes`

### Legacy Models
- **`Segment`**, **`Monologue`**, **`Block`**, **`TimestampValue`** — kept for backward compat

## Patterns
- All API models use `#[serde(default)]` liberally for forward/backward compat
- Unknown JSON fields are silently ignored (no `deny_unknown_fields`)
- `notes` and `last_viewed_panel` stored as `serde_json::Value` to tolerate malformed structures
