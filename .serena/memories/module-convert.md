# Module: convert.rs — Markdown Conversion

## Purpose
Converts raw transcript data to Obsidian-flavored Markdown with wiki-links, tags, Dataview frontmatter, and ProseMirror → markdown conversion.

## Key Symbols
- **`to_markdown(raw, meta, doc_id, notes, summary_text)`** → `Result<MarkdownOutput>`
  - Builds `Frontmatter` struct from metadata
  - Generates body: title heading, metadata line (date/duration/participants with wiki-links), tags line, Participants section (rich attendees), Summary section, Notes section, separator, transcript entries
  - Transcript entries: `**[[Speaker]] (HH:MM:SS):** text`
  - Empty transcript: `_No transcript content available._`
- **`MarkdownOutput`** (struct): `{ frontmatter_yaml: String, body: String }`
- **`prosemirror_to_markdown(doc)`** → `String`
  - Converts ProseMirror doc to markdown
  - Handles: headings, paragraphs, bulletList, listItem, text
  - Inline marks: bold (**), italic (*), bold+italic (***)
- **`format_attendee_line(attendee)`** (private) — `[[Name]], Title, Company`
- **`format_tags(labels)`** (private) — always includes `#granola`, labels become `#meeting/label-name`

## Body Section Order
1. `# Title`
2. `_Date | Duration | Participants: [[wiki-links]]_`
3. Tags line (`#granola #meeting/...`)
4. `## Participants` (if rich attendee data)
5. `## Summary` (if summary_text provided and non-empty)
6. `## Notes` (if notes provided and non-empty)
7. `---` separator
8. Transcript entries

## Snapshot Test
- `src/snapshots/baez__convert__snapshot_tests__markdown_output_snapshot.snap` — full markdown output snapshot via insta
