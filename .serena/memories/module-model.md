# Module: model.rs — Data Models

## Purpose
Serde data models for Granola API responses and Obsidian frontmatter. Tolerant parsing with `#[serde(default)]` on most fields.

## Key Types

### API Response Models
- **`DocumentSummary`**: Lightweight doc listing (`id`, `title`, `created_at`, `updated_at`, `notes: Option<Value>`, `last_viewed_panel: Option<Value>`)
  - `user_notes()` method: extracts ProseMirror doc from `notes`, falls back to `last_viewed_panel.content`
- **`DocumentMetadata`**: Rich metadata (`id: Option<String>`, `title`, `created_at` (defaults to `Utc::now`), `updated_at`, `participants`, `duration_seconds`, `labels`, `creator: Option<Attendee>`, `attendees: Option<Vec<Attendee>>`)
- **`RawTranscript`**: `#[serde(transparent)]` wrapper around `Vec<TranscriptEntry>` (`entries` field)
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
- **`Frontmatter`**: Dataview-compatible YAML
  - `doc_id`, `source`, `date: Option<String>` (YYYY-MM-DD)
  - `created: DateTime<Utc>` (alias `created_at`)
  - `updated: Option<DateTime<Utc>>` (alias `remote_updated_at`)
  - `title: Option<String>`
  - `attendees: Vec<String>` (alias `participants`)
  - `duration_minutes: Option<u64>` (alias `duration_seconds`)
  - `tags: Vec<String>` (alias `labels`)
  - **`related: Vec<String>`** — `[[wiki-links]]` to extracted entities, `skip_serializing_if = "Vec::is_empty"`
  - **`status: Option<String>`** — `"substantive"` or `"stub"`, `skip_serializing_if = "Option::is_none"`
  - `generator: String`
- **`MarkdownOutput`**: see `convert.rs`

### Legacy Models
- **`Segment`**, **`Monologue`**, **`Block`**, **`TimestampValue`** — kept for backward compat, not used in current flow

## Patterns
- All API models use `#[serde(default)]` liberally for forward/backward compat
- Unknown JSON fields are silently ignored (no `deny_unknown_fields`)
- `notes` and `last_viewed_panel` stored as `serde_json::Value` to tolerate malformed structures
- `Frontmatter` uses `skip_serializing_if` for `related` (empty Vec) and `status` (None) so old docs don't grow noisy frontmatter

## Test Modules
- `tests`, `metadata_tests`, `transcript_tests`, `frontmatter_tests`, `public_note_tests`, `prosemirror_tests`
