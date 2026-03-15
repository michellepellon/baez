# Knowledge Graph Extraction Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enhance baez's sync pipeline to produce structured meeting summaries with 10 extraction categories, create/update People/Concepts/Projects entity notes, and add `related`/`status` frontmatter fields.

**Architecture:** Single Claude API call per meeting produces markdown summary + JSON entity block. Baez parses the JSON to create/update vault entity notes during `sync_all()`. Fuzzy name matching resolves transcript names against existing People/ notes.

**Tech Stack:** Rust 1.86, strsim 0.11 (new dep), serde/serde_json/serde_yaml, chrono. Tests use tempfile, insta.

**Spec:** `docs/superpowers/specs/2026-03-14-knowledge-graph-extraction-design.md`

---

## File Structure

| File | Changes |
|------|---------|
| `src/model.rs` | Add `related: Vec<String>` and `status: Option<String>` to `Frontmatter` |
| `src/util.rs` | Add `count_transcript_words()`, `levenshtein_distance()` |
| `src/summary.rs` | Replace `DEFAULT_SUMMARY_PROMPT`, add `CHUNK_SUMMARY_PROMPT`, add `ExtractedEntities` structs, add `parse_summary_output()`, add `build_context_preamble()`, increase `max_tokens`, modify `summarize_transcript()` for chunk vs final prompts |
| `src/convert.rs` | Update `to_markdown()` signature (+`related`, +`status`), change summary insertion to raw (no heading wrapper) |
| `src/storage.rs` | Add `PeopleIndex`, `read_entity_frontmatter()`, entity create/enrich functions |
| `src/sync.rs` | Restructure `sync_all()` loop with triage, entity parsing, entity reconciliation |
| `src/lib.rs` | Re-export new public types |
| `src/main.rs` | Update `to_markdown()` call sites in Fetch handler, fix `summarize` command to strip entity JSON |
| `Cargo.toml` | Add `strsim = "0.11"` |

---

## Chunk 1: Frontmatter Enhancements + Triage

### Task 1: Add `related` and `status` fields to `Frontmatter`

**Files:**
- Modify: `src/model.rs:442-465` (Frontmatter struct)

- [ ] **Step 1: Write the failing tests**

Add to the existing `frontmatter_tests` module in `src/model.rs`:

```rust
    #[test]
    fn test_frontmatter_with_related_and_status() {
        let fm = Frontmatter {
            doc_id: "doc123".into(),
            source: "granola".into(),
            date: Some("2025-10-28".into()),
            created: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated: None,
            title: Some("Test".into()),
            attendees: vec![],
            duration_minutes: None,
            tags: vec![],
            generator: "baez".into(),
            related: vec!["[[Alice Smith]]".into(), "[[API Design]]".into()],
            status: Some("substantive".into()),
        };

        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("related:"));
        assert!(yaml.contains("[[Alice Smith]]"));
        assert!(yaml.contains("status: substantive"));

        let parsed: Frontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.related.len(), 2);
        assert_eq!(parsed.status.as_deref(), Some("substantive"));
    }

    #[test]
    fn test_frontmatter_empty_related_not_serialized() {
        let fm = Frontmatter {
            doc_id: "doc123".into(),
            source: "granola".into(),
            date: None,
            created: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated: None,
            title: None,
            attendees: vec![],
            duration_minutes: None,
            tags: vec![],
            generator: "baez".into(),
            related: vec![],
            status: None,
        };

        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(!yaml.contains("related:"));
        assert!(!yaml.contains("status:"));
    }

    #[test]
    fn test_frontmatter_backward_compat_no_related_status() {
        let yaml = r#"
doc_id: doc123
source: granola
created: 2025-10-28T15:04:05Z
generator: baez
"#;
        let parsed: Frontmatter = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.related.is_empty());
        assert!(parsed.status.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib frontmatter_tests -- --nocapture`
Expected: FAIL — `related` and `status` fields not found on `Frontmatter`

- [ ] **Step 3: Add fields to `Frontmatter` struct**

In `src/model.rs`, add these two fields to the `Frontmatter` struct before the `generator` field:

```rust
    /// Wiki-linked entity references for Obsidian graph connectivity
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    /// Transcript quality: "substantive" or "stub"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib frontmatter_tests -- --nocapture`
Expected: PASS

- [ ] **Step 5: Commit**

```bash
git add src/model.rs
git commit -m "feat: add related and status fields to Frontmatter"
```

### Task 2: Update `to_markdown()` signature and summary insertion

**Files:**
- Modify: `src/convert.rs:50-56` (to_markdown signature)
- Modify: `src/convert.rs:80-92` (frontmatter construction)
- Modify: `src/convert.rs:142-148` (summary insertion)

- [ ] **Step 1: Update `to_markdown()` signature and frontmatter construction**

In `src/convert.rs`, change the `to_markdown` function signature from:

```rust
pub fn to_markdown(
    raw: &RawTranscript,
    meta: &DocumentMetadata,
    doc_id: &str,
    notes: Option<&str>,
    summary_text: Option<&str>,
) -> Result<MarkdownOutput> {
```

to:

```rust
pub fn to_markdown(
    raw: &RawTranscript,
    meta: &DocumentMetadata,
    doc_id: &str,
    notes: Option<&str>,
    summary_text: Option<&str>,
    related: Vec<String>,
    status: Option<&str>,
) -> Result<MarkdownOutput> {
```

Then update the frontmatter construction (around line 81) from:

```rust
    let frontmatter = Frontmatter {
        doc_id: doc_id.to_string(),
        source: "granola".into(),
        date: Some(date_str),
        created: meta.created_at,
        updated: meta.updated_at,
        title: meta.title.clone(),
        attendees: attendee_names.clone(),
        duration_minutes,
        tags,
        generator: "baez".into(),
    };
```

to:

```rust
    let frontmatter = Frontmatter {
        doc_id: doc_id.to_string(),
        source: "granola".into(),
        date: Some(date_str),
        created: meta.created_at,
        updated: meta.updated_at,
        title: meta.title.clone(),
        attendees: attendee_names.clone(),
        duration_minutes,
        tags,
        generator: "baez".into(),
        related,
        status: status.map(|s| s.to_string()),
    };
```

- [ ] **Step 2: Change summary insertion to raw (no heading wrapper)**

In `src/convert.rs`, change the summary insertion block (around line 142) from:

```rust
    // AI-generated summary section
    if let Some(summary) = summary_text {
        if !summary.is_empty() {
            body.push_str("## Summary\n\n");
            body.push_str(summary);
            body.push_str("\n\n");
        }
    }
```

to:

```rust
    // AI-generated summary sections (inserted as-is, Claude's output includes headings)
    if let Some(summary) = summary_text {
        if !summary.is_empty() {
            body.push_str(summary);
            if !summary.ends_with('\n') {
                body.push('\n');
            }
            body.push('\n');
        }
    }
```

- [ ] **Step 3: Fix all callers of `to_markdown()`**

Every call to `to_markdown()` needs two new arguments. There are call sites in:

In `src/sync.rs` (around line 189), change:
```rust
        let md = to_markdown(
            &transcript,
            &meta,
            &doc_summary.id,
            notes_md.as_deref(),
            summary_text.as_deref(),
        )?;
```
to:
```rust
        let md = to_markdown(
            &transcript,
            &meta,
            &doc_summary.id,
            notes_md.as_deref(),
            summary_text.as_deref(),
            vec![],
            None,
        )?;
```

In `src/main.rs` (around line 70), change:
```rust
            let md = baez::convert::to_markdown(&transcript, &meta, id, None, None)?;
```
to:
```rust
            let md = baez::convert::to_markdown(&transcript, &meta, id, None, None, vec![], None)?;
```

- [ ] **Step 4: Fix all tests that call `to_markdown()`**

In `src/convert.rs` tests, update every `to_markdown()` call to add `, vec![], None` as the last two arguments. There are calls in:
- `test_to_markdown_entries`
- `test_wiki_links_in_participants`
- `test_tags_from_labels`
- `test_dataview_frontmatter_format`
- `test_to_markdown_with_rich_attendees_wiki_links`
- `test_to_markdown_empty_transcript`
- `test_to_markdown_with_summary_and_notes`
- `test_markdown_output_snapshot`

In `tests/workflow_integration.rs`, update every `baez::to_markdown()` call similarly:
- `test_notes_and_summary_in_markdown`
- `test_wiki_links_in_transcript`
- `test_dataview_frontmatter`
- `test_tags_in_body`
- `test_empty_last_viewed_panel_no_notes_section`
- `test_empty_summary_text_no_summary_section`

- [ ] **Step 5: Update the `test_to_markdown_with_summary_and_notes` assertion**

The summary insertion no longer adds `## Summary\n\n` wrapper. Update the assertion in `src/convert.rs` from:

```rust
        assert!(output
            .body
            .contains("## Summary\n\nWe discussed project priorities."));
```

to:

```rust
        assert!(output
            .body
            .contains("We discussed project priorities."));
```

And in `tests/workflow_integration.rs` `test_notes_and_summary_in_markdown`, change:

```rust
        assert!(output
            .body
            .contains("## Summary\n\nDiscussed deployment timeline and code review process."),);
```

to:

```rust
        assert!(output
            .body
            .contains("Discussed deployment timeline and code review process."));
```

Note: The `summary_pos` / `notes_pos` ordering assertions in these tests that look for `"## Summary"` will need to change since the summary text is now inserted raw. If the test passes `"We discussed project priorities."` as summary_text (no `## Summary` heading), then `find("## Summary")` will return `None`. Update these tests to find the summary text directly instead of the heading.

For `test_to_markdown_with_summary_and_notes` in `convert.rs`, replace the ordering assertions:
```rust
        let summary_pos = output.body.find("## Summary").unwrap();
        let notes_pos = output.body.find("## Notes").unwrap();
        let separator_pos = output.body.find("---\n").unwrap();
        assert!(summary_pos < notes_pos);
        assert!(notes_pos < separator_pos);
```
with:
```rust
        let summary_pos = output.body.find("We discussed project priorities.").unwrap();
        let notes_pos = output.body.find("## Notes").unwrap();
        let separator_pos = output.body.find("---\n").unwrap();
        assert!(summary_pos < notes_pos);
        assert!(notes_pos < separator_pos);
```

For `test_notes_and_summary_in_markdown` in `tests/workflow_integration.rs`, similarly replace:
```rust
        let summary_pos = output.body.find("## Summary").unwrap();
```
with:
```rust
        let summary_pos = output.body.find("Discussed deployment timeline").unwrap();
```

- [ ] **Step 6: Update insta snapshot**

Run: `cargo test --lib snapshot_tests`
Then: `cargo insta review` to accept the updated snapshot (frontmatter now includes the potential for `related`/`status` fields, and summary no longer has the `## Summary` wrapper added by baez).

- [ ] **Step 7: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 8: Commit**

```bash
git add src/convert.rs src/sync.rs src/main.rs src/snapshots/ tests/workflow_integration.rs
git commit -m "feat: update to_markdown() with related/status params, raw summary insertion"
```

### Task 3: Add `count_transcript_words()` triage function

**Files:**
- Modify: `src/util.rs` (add function + tests)

- [ ] **Step 1: Write the failing tests**

Add a new test module at the end of `src/util.rs`:

```rust
#[cfg(test)]
mod triage_tests {
    use super::*;
    use crate::model::{RawTranscript, TranscriptEntry};

    fn make_entry(text: &str) -> TranscriptEntry {
        TranscriptEntry {
            document_id: None,
            speaker: None,
            start: None,
            end: None,
            text: text.into(),
            source: None,
            id: None,
            is_final: None,
        }
    }

    #[test]
    fn test_empty_transcript() {
        let t = RawTranscript { entries: vec![] };
        assert_eq!(count_transcript_words(&t), 0);
    }

    #[test]
    fn test_stub_transcript() {
        let t = RawTranscript {
            entries: vec![make_entry("hello world")],
        };
        assert_eq!(count_transcript_words(&t), 2);
    }

    #[test]
    fn test_substantive_transcript() {
        let words: String = (0..25).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
        let t = RawTranscript {
            entries: vec![make_entry(&words)],
        };
        assert_eq!(count_transcript_words(&t), 25);
    }

    #[test]
    fn test_whitespace_only_entries() {
        let t = RawTranscript {
            entries: vec![make_entry("   "), make_entry(""), make_entry("\n\t")],
        };
        assert_eq!(count_transcript_words(&t), 0);
    }

    #[test]
    fn test_multiple_entries_summed() {
        let t = RawTranscript {
            entries: vec![
                make_entry("one two three"),
                make_entry("four five"),
            ],
        };
        assert_eq!(count_transcript_words(&t), 5);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib triage_tests -- --nocapture`
Expected: FAIL — `count_transcript_words` not found

- [ ] **Step 3: Implement `count_transcript_words()`**

Add this function in `src/util.rs` after the `slugify` function:

```rust
/// Count the total number of words across all transcript entries.
/// Used for triage: transcripts with < 20 words are classified as stubs.
pub fn count_transcript_words(transcript: &crate::model::RawTranscript) -> usize {
    transcript
        .entries
        .iter()
        .map(|e| e.text.split_whitespace().count())
        .sum()
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib triage_tests -- --nocapture`
Expected: All 5 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/util.rs
git commit -m "feat: add count_transcript_words() for triage logic"
```

---

## Chunk 2: Summary Parsing + Enhanced Prompt

### Task 4: Add `ExtractedEntities` structs and `parse_summary_output()`

**Files:**
- Modify: `src/summary.rs` (add structs and parsing function)

- [ ] **Step 1: Write the failing tests**

Add a new test module in `src/summary.rs` (inside the existing `#[cfg(test)] mod tests` block or as a new module):

```rust
#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn test_parse_valid_markdown_and_json() {
        let input = r#"## Summary
- Point one
- Point two

## People
| [[Alice Smith]] | Engineer |

<!-- baez-entities
{
  "people": [{"name": "Alice Smith", "role": "Engineer", "company": "Acme", "aliases": ["Alice"], "context": "Led discussion"}],
  "concepts": [{"name": "API Design", "description": "Building APIs first", "existing": false}],
  "projects": []
}
-->"#;

        let (markdown, entities) = parse_summary_output(input);

        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("## People"));
        assert!(!markdown.contains("baez-entities"));
        assert!(!markdown.contains("-->"));

        let entities = entities.unwrap();
        assert_eq!(entities.people.len(), 1);
        assert_eq!(entities.people[0].name, "Alice Smith");
        assert_eq!(entities.people[0].role.as_deref(), Some("Engineer"));
        assert_eq!(entities.people[0].aliases, vec!["Alice"]);
        assert_eq!(entities.concepts.len(), 1);
        assert_eq!(entities.concepts[0].name, "API Design");
        assert!(!entities.concepts[0].existing);
        assert!(entities.projects.is_empty());
    }

    #[test]
    fn test_parse_no_json_block() {
        let input = "## Summary\n- Just markdown\n\n## Notes\nSome notes.";
        let (markdown, entities) = parse_summary_output(input);
        assert_eq!(markdown, input);
        assert!(entities.is_none());
    }

    #[test]
    fn test_parse_malformed_json() {
        let input = "## Summary\n\n<!-- baez-entities\n{invalid json\n-->";
        let (markdown, entities) = parse_summary_output(input);
        assert!(markdown.contains("## Summary"));
        assert!(entities.is_none());
    }

    #[test]
    fn test_parse_empty_entities() {
        let input = r#"## Summary

<!-- baez-entities
{"people": [], "concepts": [], "projects": []}
-->"#;
        let (markdown, entities) = parse_summary_output(input);
        assert!(markdown.contains("## Summary"));
        let entities = entities.unwrap();
        assert!(entities.people.is_empty());
        assert!(entities.concepts.is_empty());
        assert!(entities.projects.is_empty());
    }

    #[test]
    fn test_parse_strips_trailing_whitespace() {
        let input = "## Summary\n- Point\n\n<!-- baez-entities\n{\"people\": [], \"concepts\": [], \"projects\": []}\n-->\n";
        let (markdown, _) = parse_summary_output(input);
        assert!(!markdown.contains("baez-entities"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib parse_tests -- --nocapture`
Expected: FAIL — `ExtractedEntities` and `parse_summary_output` not found

- [ ] **Step 3: Add entity structs**

Add these structs in `src/summary.rs` after the `SummaryConfig` impl block:

```rust
/// Entities extracted from a Claude summary for vault knowledge graph updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntities {
    #[serde(default)]
    pub people: Vec<PersonEntity>,
    #[serde(default)]
    pub concepts: Vec<ConceptEntity>,
    #[serde(default)]
    pub projects: Vec<ProjectEntity>,
}

/// A person mentioned in a meeting transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonEntity {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: String,
}

/// A reusable concept or insight extracted from a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptEntity {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub existing: bool,
}

/// A project mentioned in a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntity {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub existing: bool,
}
```

- [ ] **Step 4: Implement `parse_summary_output()`**

Add this function in `src/summary.rs` after the entity structs:

```rust
const ENTITY_MARKER_START: &str = "<!-- baez-entities";
const ENTITY_MARKER_END: &str = "-->";

/// Separate markdown summary from the JSON entity block.
///
/// Returns (clean_markdown, optional_entities). Parsing failures are non-fatal:
/// if the JSON block is missing or malformed, returns the full text as markdown
/// with `None` for entities.
pub fn parse_summary_output(raw: &str) -> (String, Option<ExtractedEntities>) {
    let Some(marker_start) = raw.find(ENTITY_MARKER_START) else {
        return (raw.to_string(), None);
    };

    let markdown = raw[..marker_start].trim_end().to_string();

    let json_start = marker_start + ENTITY_MARKER_START.len();
    let Some(marker_end) = raw[json_start..].find(ENTITY_MARKER_END) else {
        eprintln!("Warning: Found baez-entities marker but no closing -->");
        return (raw.to_string(), None);
    };

    let json_str = raw[json_start..json_start + marker_end].trim();

    match serde_json::from_str::<ExtractedEntities>(json_str) {
        Ok(entities) => (markdown, Some(entities)),
        Err(e) => {
            eprintln!("Warning: Failed to parse baez-entities JSON: {}", e);
            (markdown, None)
        }
    }
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib parse_tests -- --nocapture`
Expected: All 5 tests PASS

- [ ] **Step 6: Commit**

```bash
git add src/summary.rs
git commit -m "feat: add ExtractedEntities structs and parse_summary_output()"
```

### Task 5: Replace `DEFAULT_SUMMARY_PROMPT` and add chunking support

**Files:**
- Modify: `src/summary.rs:11-37` (replace prompt constant)
- Modify: `src/summary.rs:54-63` (SummaryConfig default)
- Modify: `src/summary.rs:139-160` (summarize_transcript)

- [ ] **Step 1: Replace `DEFAULT_SUMMARY_PROMPT`**

In `src/summary.rs`, replace lines 11-37 (the entire `DEFAULT_SUMMARY_PROMPT` constant) with:

```rust
const DEFAULT_SUMMARY_PROMPT: &str = r#"You are an expert meeting summarizer producing Obsidian-optimized markdown.

Given the transcript below, produce a structured summary with these sections:

## Summary
3–7 bullet points capturing the meeting's essence.

## Key Decisions
Bulleted list of decisions made, each wrapped in a Dataview inline field:
- [decision:: Approve Q2 budget for infrastructure migration]
- [decision:: Defer mobile app to Q3]
If no decisions were made, write "None."

## Action Items
Bulleted checklist. Each item uses Dataview inline fields for owner and action, with optional due date and priority:
- [ ] [owner:: [[Alice Smith]]] [action:: Deploy staging environment by Friday] *(due: 2025-03-20, priority: high)*
- [ ] [owner:: [[Bob Chen]]] [action:: Update API documentation] *(priority: medium)*
Owner names must be [[wiki-links]]. Due dates and priorities are optional — only include if mentioned.

## Discussion Highlights
Group by topic using ### subheadings. Use [[wiki-links]] for people's names.

## Open Questions
Bulleted list of unresolved items.

## People
Table of all people mentioned with their role/context in this meeting:
| Person | Role/Context |
|--------|-------------|
| [[Alice Smith]] | Engineering Manager, led API discussion |

## Project Ideas
- [[Project Name]] — what was discussed, potential next steps

## Blog Ideas
- **Idea title** — why it's worth writing about, angle to take

## Concepts
- [[Concept Name]] — brief description and why it matters

## Ideas
- Idea description — enough context to act on later

Rules:
- Use [[wiki-links]] for all person names (e.g. [[Alice Smith]]).
- Use `- [ ]` checkboxes for action items.
- Use markdown headers (##, ###) for sections.
- Preserve important names, dates, and numbers accurately.
- Only use information from the transcript; label any inferences as "(inferred)".
- Be explicit when something is unclear, missing, or not specified.
- Ignore small talk; focus on substance.
- Use Dataview inline field syntax [field:: value] exactly as shown in the examples above.
- If a section has no items, write "None." under the heading — do not omit the section.

After the markdown summary, output a machine-readable JSON entity block for automated processing.
Wrap it in an HTML comment so it does not render in Obsidian:

<!-- baez-entities
{
  "people": [
    {"name": "Full Name", "role": "their role if known", "company": "their company if known", "aliases": ["nickname", "abbreviation"], "context": "one-line context from this meeting"}
  ],
  "concepts": [
    {"name": "Concept Name", "description": "one-line description", "existing": true}
  ],
  "projects": [
    {"name": "Project Name", "description": "one-line description", "existing": false}
  ]
}
-->

Rules for the entity block:
- Include ALL people mentioned by name (full names only — skip first-name-only references unless you can infer the full name from context).
- For concepts: set "existing" to true if the concept appears in the provided existing concepts list, false otherwise.
- For projects: set "existing" to true if the project appears in the provided existing projects list, false otherwise.
- Only include genuinely reusable concepts, not trivial topics.
- The JSON must be valid. Use null for missing optional fields (role, company)."#;
```

- [ ] **Step 2: Add `CHUNK_SUMMARY_PROMPT`**

Add this constant right after `DEFAULT_SUMMARY_PROMPT`:

```rust
/// Simplified prompt for intermediate chunk summaries (no entity extraction).
const CHUNK_SUMMARY_PROMPT: &str = r#"You are an expert meeting summarizer. Summarize the following transcript chunk into a concise narrative. Focus on key points, decisions, and action items. Use [[wiki-links]] for person names. This is a partial transcript — do not state conclusions about the overall meeting."#;
```

- [ ] **Step 3: Increase `max_tokens` default**

In `src/summary.rs`, in the `Default` impl for `SummaryConfig`, change:

```rust
            max_tokens: 4096,
```

to:

```rust
            max_tokens: 8192,
```

- [ ] **Step 4: Modify `summarize_transcript()` for chunk vs final prompts**

In `src/summary.rs`, modify the `summarize_transcript` function. The current implementation (around line 139) is:

```rust
pub fn summarize_transcript(
    transcript_text: &str,
    api_key: &str,
    config: &SummaryConfig,
    client: &reqwest::blocking::Client,
) -> Result<String> {
    let chunks = chunk_transcript(transcript_text, config.max_input_chars);

    if chunks.len() > 1 {
        let mut chunk_summaries = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            println!("Summarizing chunk {}/{}...", i + 1, chunks.len());
            let summary = call_claude_api(client, chunk, api_key, config)?;
            chunk_summaries.push(summary);
        }
        // Combine summaries into a single final summary
        let combined = chunk_summaries.join("\n\n---\n\n");
        call_claude_api(client, &combined, api_key, config)
    } else {
        call_claude_api(client, &chunks[0], api_key, config)
    }
}
```

Replace it with:

```rust
pub fn summarize_transcript(
    transcript_text: &str,
    api_key: &str,
    config: &SummaryConfig,
    client: &reqwest::blocking::Client,
    context_preamble: &str,
) -> Result<String> {
    let chunks = chunk_transcript(transcript_text, config.max_input_chars);

    if chunks.len() > 1 {
        // Multi-chunk: use simplified prompt for intermediate summaries
        let chunk_config = SummaryConfig {
            custom_prompt: Some(CHUNK_SUMMARY_PROMPT.to_string()),
            ..config.clone()
        };
        let mut chunk_summaries = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            println!("Summarizing chunk {}/{}...", i + 1, chunks.len());
            let summary = call_claude_api(client, chunk, api_key, &chunk_config)?;
            chunk_summaries.push(summary);
        }
        // Final pass: combine chunk summaries with full prompt + context preamble
        let combined = format!(
            "{}\n\nCombined meeting summary from {} chunks:\n\n{}",
            context_preamble,
            chunks.len(),
            chunk_summaries.join("\n\n---\n\n")
        );
        call_claude_api(client, &combined, api_key, config)
    } else {
        // Single chunk: full prompt with context preamble
        let input = format!("{}\n\n{}", context_preamble, &chunks[0]);
        call_claude_api(client, &input, api_key, config)
    }
}
```

- [ ] **Step 5: Fix callers of `summarize_transcript()`**

The function now takes an extra `context_preamble: &str` parameter. Update callers:

In `src/sync.rs` (around line 171), change:
```rust
                match crate::summary::summarize_transcript(&input, key, config, claude_client) {
```
to:
```rust
                match crate::summary::summarize_transcript(&input, key, config, claude_client, "") {
```

In `src/main.rs` (around line 238), change:
```rust
            let summary =
                baez::summary::summarize_transcript(&input, &api_key, &config, &claude_client)?;
```
to:
```rust
            let raw_summary =
                baez::summary::summarize_transcript(&input, &api_key, &config, &claude_client, "")?;
            // Strip the JSON entity block before writing to vault
            let (summary, _entities) = baez::summary::parse_summary_output(&raw_summary);
```

(Empty string for context preamble — the `summarize` command operates on a single doc without vault-wide context. The entity block is stripped so it doesn't appear in the vault file.)

- [ ] **Step 6: Run tests**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS. The existing `test_summary_prompt_format` asserts on "Summary", "Action Items", "Key Decisions", "Open Questions", "[[wiki-links]]" — all present in the new prompt.

- [ ] **Step 7: Run linter**

Run: `cargo clippy --all-features -- -D warnings && cargo clippy --no-default-features -- -D warnings`
Expected: No warnings

- [ ] **Step 8: Commit**

```bash
git add src/summary.rs src/sync.rs src/main.rs
git commit -m "feat: enhanced summary prompt with 10 sections, entity JSON block, chunk support"
```

### Task 6: Add `build_context_preamble()`

**Files:**
- Modify: `src/summary.rs` (add function + tests)

- [ ] **Step 1: Write the failing tests**

Add to the `tests` module in `src/summary.rs`:

```rust
    #[test]
    fn test_build_context_preamble_empty() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = build_context_preamble(temp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_context_preamble_with_concepts_and_projects() {
        let temp = tempfile::TempDir::new().unwrap();
        let concepts_dir = temp.path().join("Concepts");
        let projects_dir = temp.path().join("Projects");
        std::fs::create_dir_all(&concepts_dir).unwrap();
        std::fs::create_dir_all(&projects_dir).unwrap();

        std::fs::write(concepts_dir.join("API Design.md"), "# API Design").unwrap();
        std::fs::write(concepts_dir.join("Conway's Law.md"), "# Conway's Law").unwrap();
        std::fs::write(projects_dir.join("Project Atlas.md"), "# Project Atlas").unwrap();

        let result = build_context_preamble(temp.path());
        assert!(result.contains("Existing concepts"));
        assert!(result.contains("- API Design"));
        assert!(result.contains("- Conway's Law"));
        assert!(result.contains("Existing projects"));
        assert!(result.contains("- Project Atlas"));
    }

    #[test]
    fn test_build_context_preamble_ignores_non_md_files() {
        let temp = tempfile::TempDir::new().unwrap();
        let concepts_dir = temp.path().join("Concepts");
        std::fs::create_dir_all(&concepts_dir).unwrap();

        std::fs::write(concepts_dir.join("Real Concept.md"), "").unwrap();
        std::fs::write(concepts_dir.join(".DS_Store"), "").unwrap();
        std::fs::write(concepts_dir.join("notes.txt"), "").unwrap();

        let result = build_context_preamble(temp.path());
        assert!(result.contains("- Real Concept"));
        assert!(!result.contains(".DS_Store"));
        assert!(!result.contains("notes"));
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib test_build_context_preamble -- --nocapture`
Expected: FAIL — `build_context_preamble` not found

- [ ] **Step 3: Implement `build_context_preamble()`**

Add in `src/summary.rs`:

```rust
/// Scan Concepts/ and Projects/ directories for existing entity names.
/// Returns a prompt preamble string listing them for Claude to reference.
pub fn build_context_preamble(vault_dir: &std::path::Path) -> String {
    let mut sections = Vec::new();

    if let Some(names) = scan_entity_dir(&vault_dir.join("Concepts")) {
        if !names.is_empty() {
            let mut section = String::from(
                "Existing concepts in the vault (reference by exact name if relevant, only propose new ones if genuinely distinct):\n"
            );
            for name in &names {
                section.push_str(&format!("- {}\n", name));
            }
            sections.push(section);
        }
    }

    if let Some(names) = scan_entity_dir(&vault_dir.join("Projects")) {
        if !names.is_empty() {
            let mut section = String::from(
                "Existing projects in the vault (reference by exact name if relevant):\n"
            );
            for name in &names {
                section.push_str(&format!("- {}\n", name));
            }
            sections.push(section);
        }
    }

    sections.join("\n")
}

/// Scan a directory for .md files and return their filename stems.
fn scan_entity_dir(dir: &std::path::Path) -> Option<Vec<String>> {
    if !dir.is_dir() {
        return None;
    }

    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    names.sort();
    Some(names)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib test_build_context_preamble -- --nocapture`
Expected: All 3 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/summary.rs
git commit -m "feat: add build_context_preamble() for concept/project deduplication"
```

---

## Chunk 3: Fuzzy Name Matching

### Task 7: Add `strsim` dependency and `levenshtein_distance()`

**Files:**
- Modify: `Cargo.toml`
- Modify: `src/util.rs`

- [ ] **Step 1: Add `strsim` dependency**

In `Cargo.toml`, add after the `filetime` line in `[dependencies]`:

```toml
strsim = "0.11"
```

- [ ] **Step 2: Write the failing tests**

Add a new test module at the end of `src/util.rs`:

```rust
#[cfg(test)]
mod levenshtein_tests {
    use super::*;

    #[test]
    fn test_identical_strings() {
        assert_eq!(levenshtein_distance("alice", "alice"), 0);
    }

    #[test]
    fn test_one_edit() {
        assert_eq!(levenshtein_distance("alice", "alce"), 1);
    }

    #[test]
    fn test_two_edits() {
        assert_eq!(levenshtein_distance("alice", "alise"), 1);
        assert_eq!(levenshtein_distance("smith", "smyth"), 1);
    }

    #[test]
    fn test_completely_different() {
        assert!(levenshtein_distance("alice", "bob") > 2);
    }
}
```

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --lib levenshtein_tests -- --nocapture`
Expected: FAIL — `levenshtein_distance` not found

- [ ] **Step 4: Implement `levenshtein_distance()`**

Add in `src/util.rs`:

```rust
/// Compute Levenshtein edit distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    strsim::levenshtein(a, b)
}
```

- [ ] **Step 5: Run tests to verify they pass**

Run: `cargo test --lib levenshtein_tests -- --nocapture`
Expected: All 4 tests PASS

- [ ] **Step 6: Verify compilation**

Run: `cargo check`
Expected: Compiles (strsim resolves)

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock src/util.rs
git commit -m "feat: add strsim dependency and levenshtein_distance() wrapper"
```

### Task 8: Add `PeopleIndex` with fuzzy matching

**Files:**
- Modify: `src/storage.rs` (add struct, build, find_match)

- [ ] **Step 1: Write the failing tests**

Add a new test module at the end of `src/storage.rs`:

```rust
#[cfg(test)]
mod people_index_tests {
    use super::*;
    use tempfile::TempDir;

    fn write_person(dir: &Path, name: &str, aliases: &[&str]) {
        let alias_yaml = if aliases.is_empty() {
            "aliases: []".to_string()
        } else {
            format!("aliases: [{}]", aliases.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", "))
        };
        let content = format!(
            "---\ntitle: \"{}\"\n{}\ntype: person\n---\n\n# {}\n",
            name, alias_yaml, name
        );
        fs::write(dir.join(format!("{}.md", name)), content).unwrap();
    }

    #[test]
    fn test_build_empty_dir() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();

        let index = PeopleIndex::build(&people_dir);
        assert!(index.find_match("Alice", &[]).is_none());
    }

    #[test]
    fn test_exact_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Alice Smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_case_insensitive_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("alice smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_alias_match() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Dennis Crowley", &["Dens", "DC"]);

        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Dens", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Dennis Crowley");
    }

    #[test]
    fn test_attendee_disambiguation() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        let attendees = vec!["Alice Smith".to_string(), "Bob Jones".to_string()];
        let result = index.find_match("Alice", &attendees);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_attendee_disambiguation_ambiguous() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);
        write_person(&people_dir, "Alice Jones", &[]);

        let index = PeopleIndex::build(&people_dir);
        let attendees = vec!["Alice Smith".to_string(), "Alice Jones".to_string()];
        let result = index.find_match("Alice", &attendees);
        assert!(result.is_none());
    }

    #[test]
    fn test_fuzzy_match_within_threshold() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        // "Alce Smith" is 1 edit from "Alice Smith" (missing 'i')
        let result = index.find_match("Alce Smith", &[]);
        assert!(result.is_some());
        assert_eq!(result.unwrap().0, "Alice Smith");
    }

    #[test]
    fn test_fuzzy_match_beyond_threshold() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Bob Johnson", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_short_name_requires_exact() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Bob", &[]);

        let index = PeopleIndex::build(&people_dir);
        // "Rob" is 1 edit from "Bob" but name is ≤ 5 chars, requires exact
        let result = index.find_match("Rob", &[]);
        assert!(result.is_none());
        // Exact match works
        let result = index.find_match("Bob", &[]);
        assert!(result.is_some());
    }

    #[test]
    fn test_no_match_returns_none() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        write_person(&people_dir, "Alice Smith", &[]);

        let index = PeopleIndex::build(&people_dir);
        let result = index.find_match("Totally Different Person", &[]);
        assert!(result.is_none());
    }

    #[test]
    fn test_add_person_updates_index() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();

        let mut index = PeopleIndex::build(&people_dir);
        assert!(index.find_match("New Person", &[]).is_none());

        index.add_person("New Person", &people_dir, &["NP"]);
        let result = index.find_match("New Person", &[]);
        assert!(result.is_some());
        // Also findable by alias
        let result = index.find_match("NP", &[]);
        assert!(result.is_some());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib people_index_tests -- --nocapture`
Expected: FAIL — `PeopleIndex` not found

- [ ] **Step 3: Implement `PeopleIndex`**

Add in `src/storage.rs` (add `use std::collections::HashMap;` to imports if not present):

```rust
use crate::util::levenshtein_distance;

/// In-memory index of People/ notes for fuzzy name matching.
pub struct PeopleIndex {
    /// Maps lowercase canonical name → (original_case_name, file_path)
    entries: HashMap<String, (String, PathBuf)>,
    /// Maps lowercase alias → lowercase canonical name
    aliases: HashMap<String, String>,
}

impl PeopleIndex {
    /// Build the index by scanning the People/ directory.
    pub fn build(people_dir: &Path) -> Self {
        let mut entries = HashMap::new();
        let mut aliases = HashMap::new();

        if !people_dir.is_dir() {
            return PeopleIndex { entries, aliases };
        }

        let dir_entries = match fs::read_dir(people_dir) {
            Ok(e) => e,
            Err(_) => return PeopleIndex { entries, aliases },
        };

        for entry in dir_entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }

            let canonical = match path.file_stem().and_then(|s| s.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let canonical_lower = canonical.to_lowercase();

            // Try to read aliases from frontmatter
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(alias_list) = parse_aliases_from_frontmatter(&content) {
                    for alias in alias_list {
                        aliases.insert(alias.to_lowercase(), canonical_lower.clone());
                    }
                }
            }

            entries.insert(canonical_lower, (canonical, path));
        }

        PeopleIndex { entries, aliases }
    }

    /// Register a newly created person in the index (no filesystem I/O).
    pub fn add_person(&mut self, name: &str, people_dir: &Path, new_aliases: &[&str]) {
        let lower = name.to_lowercase();
        let path = people_dir.join(format!("{}.md", name));
        self.entries.insert(lower.clone(), (name.to_string(), path));
        for alias in new_aliases {
            self.aliases.insert(alias.to_lowercase(), lower.clone());
        }
    }

    /// Find a matching person for the given name.
    ///
    /// Returns `Some((canonical_name, file_path))` on match, `None` on no match or ambiguity.
    pub fn find_match(&self, name: &str, attendees: &[String]) -> Option<(String, PathBuf)> {
        let lower = name.to_lowercase();

        // 1. Exact match
        if let Some((canonical, path)) = self.entries.get(&lower) {
            return Some((canonical.clone(), path.clone()));
        }

        // 2. Alias match
        if let Some(canonical_lower) = self.aliases.get(&lower) {
            if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                return Some((canonical.clone(), path.clone()));
            }
        }

        // 3. Attendee disambiguation (first-name-only)
        if !name.contains(' ') {
            let matches: Vec<&String> = attendees
                .iter()
                .filter(|a| a.to_lowercase().starts_with(&lower))
                .collect();

            if matches.len() == 1 {
                let full_name_lower = matches[0].to_lowercase();
                // Re-try exact and alias match with the full name
                if let Some((canonical, path)) = self.entries.get(&full_name_lower) {
                    return Some((canonical.clone(), path.clone()));
                }
                if let Some(canonical_lower) = self.aliases.get(&full_name_lower) {
                    if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                        return Some((canonical.clone(), path.clone()));
                    }
                }
            }
        }

        // 4. Fuzzy match (Levenshtein)
        if lower.len() <= 5 {
            // Short names require exact match (too many false positives)
            return None;
        }

        let threshold = 2;
        let mut best_match: Option<(String, PathBuf, usize)> = None;
        let mut ambiguous = false;

        // Check against entry names
        for (entry_lower, (canonical, path)) in &self.entries {
            let dist = levenshtein_distance(&lower, entry_lower);
            if dist <= threshold {
                match &best_match {
                    Some((_, _, best_dist)) if dist < *best_dist => {
                        best_match = Some((canonical.clone(), path.clone(), dist));
                        ambiguous = false;
                    }
                    Some((_, _, best_dist)) if dist == *best_dist => {
                        ambiguous = true;
                    }
                    None => {
                        best_match = Some((canonical.clone(), path.clone(), dist));
                    }
                    _ => {}
                }
            }
        }

        // Check against aliases
        for (alias_lower, canonical_lower) in &self.aliases {
            let dist = levenshtein_distance(&lower, alias_lower);
            if dist <= threshold {
                if let Some((canonical, path)) = self.entries.get(canonical_lower) {
                    match &best_match {
                        Some((_, _, best_dist)) if dist < *best_dist => {
                            best_match = Some((canonical.clone(), path.clone(), dist));
                            ambiguous = false;
                        }
                        Some((_, _, best_dist)) if dist == *best_dist => {
                            ambiguous = true;
                        }
                        None => {
                            best_match = Some((canonical.clone(), path.clone(), dist));
                        }
                        _ => {}
                    }
                }
            }
        }

        if ambiguous {
            return None;
        }

        best_match.map(|(name, path, _)| (name, path))
    }
}

/// Extract the `aliases` array from YAML frontmatter in a markdown file.
fn parse_aliases_from_frontmatter(content: &str) -> Option<Vec<String>> {
    if !content.starts_with("---\n") {
        return None;
    }
    let rest = &content[4..];
    let end_pos = rest.find("\n---")?;
    let yaml = &rest[..end_pos];

    let value: serde_json::Value = serde_yaml::from_str(yaml).ok()?;
    let aliases = value.get("aliases")?.as_array()?;
    Some(
        aliases
            .iter()
            .filter_map(|v| v.as_str().map(|s| s.to_string()))
            .collect(),
    )
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib people_index_tests -- --nocapture`
Expected: All 11 tests PASS

- [ ] **Step 5: Run linter**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 6: Commit**

```bash
git add src/storage.rs
git commit -m "feat: add PeopleIndex with fuzzy name matching and alias support"
```

---

## Chunk 4: Entity Note Creation

### Task 9: Add `read_entity_frontmatter()` and entity create/enrich functions

**Files:**
- Modify: `src/storage.rs` (add functions + tests)

- [ ] **Step 1: Write the failing tests**

Add a new test module at the end of `src/storage.rs`:

```rust
#[cfg(test)]
mod entity_note_tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_read_entity_frontmatter_valid() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("test.md");
        fs::write(&path, "---\ntitle: \"Test\"\ntype: person\nrelated: []\n---\n\n# Test\n\nBody content.\n").unwrap();

        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_some());
        let (fm, body) = result.unwrap();
        assert_eq!(fm["title"].as_str(), Some("Test"));
        assert!(body.contains("Body content."));
    }

    #[test]
    fn test_read_entity_frontmatter_missing() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("missing.md");
        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_read_entity_frontmatter_no_yaml() {
        let temp = TempDir::new().unwrap();
        let path = temp.path().join("plain.md");
        fs::write(&path, "# Just content\n").unwrap();
        let result = read_entity_frontmatter(&path).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_create_person_note() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();

        create_person_note(
            &people_dir,
            "Alice Smith",
            Some("Engineer"),
            Some("Acme Corp"),
            &["Alice"],
            "Led API discussion",
            "2025-01-15_standup",
            "2025-01-15",
            &tmp_dir,
        ).unwrap();

        let path = people_dir.join("Alice Smith.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"Alice Smith\""));
        assert!(content.contains("company: \"Acme Corp\""));
        assert!(content.contains("role: \"Engineer\""));
        assert!(content.contains("[[2025-01-15_standup]]"));
        assert!(content.contains("Led API discussion"));
    }

    #[test]
    fn test_enrich_person_note_adds_related() {
        let temp = TempDir::new().unwrap();
        let people_dir = temp.path().join("People");
        fs::create_dir_all(&people_dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();

        // Create initial note
        let initial = "---\ntitle: \"Alice Smith\"\ntype: person\nrelated:\n  - \"[[2025-01-10_meeting]]\"\nlast-contact: \"2025-01-10\"\n---\n\n# Alice Smith\n\n## Notes\n- From [[2025-01-10_meeting]]: Initial context\n";
        let path = people_dir.join("Alice Smith.md");
        fs::write(&path, initial).unwrap();

        enrich_person_note(
            &path,
            &["New Alias"],
            "Discussed migration",
            "2025-01-15_standup",
            "2025-01-15",
            &tmp_dir,
        ).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("[[2025-01-10_meeting]]"));
        assert!(content.contains("[[2025-01-15_standup]]"));
        assert!(content.contains("Discussed migration"));
        assert!(content.contains("last-contact: \"2025-01-15\""));
    }

    #[test]
    fn test_enrich_person_note_no_duplicate_related() {
        let temp = TempDir::new().unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let path = temp.path().join("test.md");

        let initial = "---\ntitle: \"Alice\"\ntype: person\nrelated:\n  - \"[[2025-01-15_standup]]\"\nlast-contact: \"2025-01-15\"\n---\n\n# Alice\n\n## Notes\n- Existing\n";
        fs::write(&path, &initial).unwrap();

        enrich_person_note(&path, &[], "Again", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        // related should NOT have a duplicate entry (stays at 1)
        // But Notes section now has 2 bullets (original + new), each containing the slug
        // Total occurrences: 1 (related) + 1 (original note) + 1 (new note) = 3
        let count = content.matches("2025-01-15_standup").count();
        assert_eq!(count, 3);
    }

    #[test]
    fn test_enrich_person_note_missing_notes_section() {
        let temp = TempDir::new().unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();
        let path = temp.path().join("test.md");

        let initial = "---\ntitle: \"Alice\"\ntype: person\nrelated: []\n---\n\n# Alice\n\n## Context\n- Engineer\n";
        fs::write(&path, &initial).unwrap();

        enrich_person_note(&path, &[], "New context", "2025-01-15_standup", "2025-01-15", &tmp_dir).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("## Notes\n"));
        assert!(content.contains("New context"));
    }

    #[test]
    fn test_create_concept_note() {
        let temp = TempDir::new().unwrap();
        let concepts_dir = temp.path().join("Concepts");
        fs::create_dir_all(&concepts_dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();

        create_concept_note(
            &concepts_dir,
            "API-First Design",
            "Building APIs before UIs",
            "2025-01-15_standup",
            "2025-01-15",
            &tmp_dir,
        ).unwrap();

        let path = concepts_dir.join("API-First Design.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"API-First Design\""));
        assert!(content.contains("Building APIs before UIs"));
        assert!(content.contains("[[2025-01-15_standup]]"));
    }

    #[test]
    fn test_create_project_note() {
        let temp = TempDir::new().unwrap();
        let projects_dir = temp.path().join("Projects");
        fs::create_dir_all(&projects_dir).unwrap();
        let tmp_dir = temp.path().join("tmp");
        fs::create_dir_all(&tmp_dir).unwrap();

        create_project_note(
            &projects_dir,
            "Project Atlas",
            "Internal migration tool",
            "2025-01-15_standup",
            "2025-01-15",
            &tmp_dir,
        ).unwrap();

        let path = projects_dir.join("Project Atlas.md");
        assert!(path.exists());
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("title: \"Project Atlas\""));
        assert!(content.contains("Internal migration tool"));
    }

    #[test]
    fn test_find_entity_file_case_insensitive() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Concepts");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("API-First Design.md"), "# test").unwrap();

        // Case-insensitive match
        let result = find_entity_file(&dir, "api-first design");
        assert!(result.is_some());
        assert!(result.unwrap().ends_with("API-First Design.md"));
    }

    #[test]
    fn test_find_entity_file_no_match() {
        let temp = TempDir::new().unwrap();
        let dir = temp.path().join("Concepts");
        fs::create_dir_all(&dir).unwrap();

        let result = find_entity_file(&dir, "Nonexistent");
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib entity_note_tests -- --nocapture`
Expected: FAIL — functions not found

- [ ] **Step 3: Implement entity note functions**

Add in `src/storage.rs`:

```rust
/// Read entity note frontmatter as flexible JSON Value + body text.
/// Returns None if file doesn't exist or has no frontmatter.
pub fn read_entity_frontmatter(path: &Path) -> Result<Option<(serde_json::Value, String)>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = fs::read_to_string(path)?;

    if !content.starts_with("---\n") {
        return Ok(None);
    }

    let rest = &content[4..];
    let Some(end_pos) = rest.find("\n---") else {
        return Ok(None);
    };

    let yaml = &rest[..end_pos];
    let body_start = end_pos + 4; // skip "\n---"
    let body = if body_start < rest.len() {
        // Skip the newline after closing ---
        let skip = if rest.as_bytes().get(body_start) == Some(&b'\n') { 1 } else { 0 };
        rest[body_start + skip..].to_string()
    } else {
        String::new()
    };

    let value: serde_json::Value = serde_yaml::from_str(yaml).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to parse entity frontmatter: {}", e),
        ))
    })?;

    Ok(Some((value, body)))
}

/// Find an entity file by name, case-insensitive.
pub fn find_entity_file(dir: &Path, name: &str) -> Option<PathBuf> {
    let lower = name.to_lowercase();
    fs::read_dir(dir).ok()?.flatten().find_map(|entry| {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) == Some("md") {
            let stem = path.file_stem()?.to_str()?;
            if stem.to_lowercase() == lower {
                return Some(path);
            }
        }
        None
    })
}

/// Create a new People note.
#[allow(clippy::too_many_arguments)]
pub fn create_person_note(
    people_dir: &Path,
    name: &str,
    role: Option<&str>,
    company: Option<&str>,
    aliases: &[&str],
    context: &str,
    meeting_slug: &str,
    date: &str,
    tmp_dir: &Path,
) -> Result<()> {
    let alias_yaml = if aliases.is_empty() {
        "[]".to_string()
    } else {
        format!("[{}]", aliases.iter().map(|a| format!("\"{}\"", a)).collect::<Vec<_>>().join(", "))
    };

    let mut content = format!(
        r#"---
title: "{name}"
date: "{date}"
tags: [people]
aliases: {alias_yaml}
type: person
company: "{company}"
role: "{role}"
last-contact: "{date}"
status: active
related:
  - "[[{meeting_slug}]]"
---

# {name}

## Context
- **Company:** {company}
- **Role:** {role}

## Notes
- From [[{meeting_slug}]]: {context}
"#,
        name = name,
        date = date,
        alias_yaml = alias_yaml,
        company = company.unwrap_or("Unknown"),
        role = role.unwrap_or("Unknown"),
        meeting_slug = meeting_slug,
        context = context,
    );

    // Clean up "Unknown" fields if not provided
    if company.is_none() {
        content = content.replace("company: \"Unknown\"\n", "");
        content = content.replace("- **Company:** Unknown\n", "");
    }
    if role.is_none() {
        content = content.replace("role: \"Unknown\"\n", "");
        content = content.replace("- **Role:** Unknown\n", "");
    }

    let path = people_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing People note with a new meeting reference.
pub fn enrich_person_note(
    path: &Path,
    new_aliases: &[&str],
    context: &str,
    meeting_slug: &str,
    date: &str,
    tmp_dir: &Path,
) -> Result<()> {
    let content = fs::read_to_string(path)?;

    let Some((mut fm, body)) = read_entity_frontmatter(path)?.map(|(fm, body)| (fm, body)) else {
        return Ok(());
    };

    // Update related (no duplicates)
    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        let already_present = related.iter().any(|v| v.as_str() == Some(&meeting_ref));
        if !already_present {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else {
        fm["related"] = serde_json::json!([meeting_ref]);
    }

    // Update last-contact if newer
    if let Some(existing_date) = fm.get("last-contact").and_then(|v| v.as_str()) {
        if date > existing_date {
            fm["last-contact"] = serde_json::Value::String(date.to_string());
        }
    } else {
        fm["last-contact"] = serde_json::Value::String(date.to_string());
    }

    // Merge aliases
    if !new_aliases.is_empty() {
        let existing_aliases: Vec<String> = fm
            .get("aliases")
            .and_then(|v| v.as_array())
            .map(|arr| arr.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default();

        let mut merged = existing_aliases;
        for alias in new_aliases {
            if !merged.iter().any(|a| a.to_lowercase() == alias.to_lowercase()) {
                merged.push(alias.to_string());
            }
        }
        fm["aliases"] = serde_json::json!(merged);
    }

    // Re-serialize frontmatter
    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize entity frontmatter: {}", e),
        ))
    })?;

    // Append to ## Notes section (or create it)
    let notes_bullet = format!("- From [[{}]]: {}", meeting_slug, context);
    let updated_body = if body.contains("## Notes") {
        // Find the Notes section and append
        let notes_pos = body.find("## Notes").unwrap();
        let after_heading = &body[notes_pos + 8..]; // skip "## Notes"
        let next_section = after_heading.find("\n## ");
        let insert_pos = match next_section {
            Some(pos) => notes_pos + 8 + pos,
            None => body.len(),
        };
        let mut new_body = body[..insert_pos].to_string();
        if !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        new_body.push_str(&notes_bullet);
        new_body.push('\n');
        new_body.push_str(&body[insert_pos..]);
        new_body
    } else {
        format!("{}\n## Notes\n{}\n", body.trim_end(), notes_bullet)
    };

    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}

/// Create a new Concept note.
pub fn create_concept_note(
    concepts_dir: &Path,
    name: &str,
    description: &str,
    meeting_slug: &str,
    date: &str,
    tmp_dir: &Path,
) -> Result<()> {
    let content = format!(
        r#"---
title: "{name}"
date: "{date}"
tags: [concept]
type: concept
status: active
related:
  - "[[{meeting_slug}]]"
---

# {name}

## What is it?
{description}

## Sources
- [[{meeting_slug}]] — extracted {date}
"#,
        name = name,
        date = date,
        description = description,
        meeting_slug = meeting_slug,
    );

    let path = concepts_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing Concept note with a new source reference.
pub fn enrich_concept_note(path: &Path, meeting_slug: &str, date: &str, tmp_dir: &Path) -> Result<()> {
    let Some((mut fm, body)) = read_entity_frontmatter(path)? else {
        return Ok(());
    };

    // Update related
    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        if !related.iter().any(|v| v.as_str() == Some(&meeting_ref)) {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else {
        fm["related"] = serde_json::json!([meeting_ref]);
    }

    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize entity frontmatter: {}", e),
        ))
    })?;

    let source_line = format!("- [[{}]] — extracted {}", meeting_slug, date);
    let updated_body = if body.contains("## Sources") {
        let pos = body.find("## Sources").unwrap();
        let after = &body[pos + 10..];
        let next = after.find("\n## ");
        let insert_pos = match next {
            Some(p) => pos + 10 + p,
            None => body.len(),
        };
        let mut new_body = body[..insert_pos].to_string();
        if !new_body.ends_with('\n') {
            new_body.push('\n');
        }
        new_body.push_str(&source_line);
        new_body.push('\n');
        new_body.push_str(&body[insert_pos..]);
        new_body
    } else {
        format!("{}\n## Sources\n{}\n", body.trim_end(), source_line)
    };

    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}

/// Create a new Project note.
pub fn create_project_note(
    projects_dir: &Path,
    name: &str,
    description: &str,
    meeting_slug: &str,
    date: &str,
    tmp_dir: &Path,
) -> Result<()> {
    let content = format!(
        r#"---
title: "{name}"
date: "{date}"
tags: [project]
type: project
status: active
related:
  - "[[{meeting_slug}]]"
---

# {name}

Mentioned in [[{meeting_slug}]]: {description}
"#,
        name = name,
        date = date,
        description = description,
        meeting_slug = meeting_slug,
    );

    let path = projects_dir.join(format!("{}.md", name));
    write_atomic(&path, content.as_bytes(), tmp_dir)
}

/// Enrich an existing Project note with a new mention.
pub fn enrich_project_note(
    path: &Path,
    description: &str,
    meeting_slug: &str,
    tmp_dir: &Path,
) -> Result<()> {
    let Some((mut fm, body)) = read_entity_frontmatter(path)? else {
        return Ok(());
    };

    let meeting_ref = format!("[[{}]]", meeting_slug);
    if let Some(related) = fm.get_mut("related").and_then(|v| v.as_array_mut()) {
        if !related.iter().any(|v| v.as_str() == Some(&meeting_ref)) {
            related.push(serde_json::Value::String(meeting_ref));
        }
    } else {
        fm["related"] = serde_json::json!([meeting_ref]);
    }

    let fm_yaml = serde_yaml::to_string(&fm).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize entity frontmatter: {}", e),
        ))
    })?;

    let mention_line = format!("\nMentioned in [[{}]]: {}", meeting_slug, description);
    let updated_body = format!("{}{}\n", body.trim_end(), mention_line);

    let full = format!("---\n{}---\n{}", fm_yaml, updated_body);
    write_atomic(path, full.as_bytes(), tmp_dir)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib entity_note_tests -- --nocapture`
Expected: All 12 tests PASS

- [ ] **Step 5: Run linter**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 6: Commit**

```bash
git add src/storage.rs
git commit -m "feat: add entity note create/enrich functions for People, Concepts, Projects"
```

---

## Chunk 5: Sync Loop Orchestration + Re-exports + Final Verification

### Task 10: Update re-exports in `lib.rs`

**Files:**
- Modify: `src/lib.rs`

- [ ] **Step 1: Add re-exports**

In `src/lib.rs`, update the summary re-exports. After the existing `pub use` lines, ensure these are exported:

```rust
#[cfg(feature = "summaries")]
pub use summary::{
    build_context_preamble, parse_summary_output, ExtractedEntities, PersonEntity, ConceptEntity,
    ProjectEntity,
};
```

And for storage:

```rust
pub use storage::{
    create_concept_note, create_person_note, create_project_note, enrich_concept_note,
    enrich_person_note, enrich_project_note, find_entity_file, read_entity_frontmatter,
    read_frontmatter, write_atomic, PeopleIndex, Paths,
};
```

And for util:

```rust
pub use util::count_transcript_words;
```

- [ ] **Step 2: Verify compilation**

Run: `cargo check --all-features`
Expected: Compiles

- [ ] **Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "feat: re-export new entity types and functions"
```

### Task 11: Wire everything together in `sync_all()`

**Files:**
- Modify: `src/sync.rs` (restructure the loop)

- [ ] **Step 1: Update `sync_all()` to integrate triage, parsing, and entity reconciliation**

This is the largest change. In `src/sync.rs`, the `sync_all()` function needs these modifications:

1. **Add new imports** at the top of `src/sync.rs`:

```rust
use crate::util::count_transcript_words;
```

2. **After the `summarize_state` setup block (around line 87), add entity directory creation:**

Note: this must come AFTER `summarize_state` is resolved so we can check both the user flag AND API key availability.

```rust
    // Create entity directories only when summarization is enabled AND an API key is available
    #[cfg(feature = "summaries")]
    let entity_dirs_ready = summarize_state.is_some();
    #[cfg(feature = "summaries")]
    if entity_dirs_ready {
        let _ = std::fs::create_dir_all(paths.vault_dir.join("People"));
        let _ = std::fs::create_dir_all(paths.vault_dir.join("Concepts"));
        let _ = std::fs::create_dir_all(paths.vault_dir.join("Projects"));
    }
    #[cfg(not(feature = "summaries"))]
    let entity_dirs_ready = false;
```

3. **After entity directory creation, build PeopleIndex and context preamble:**

```rust
    #[cfg(feature = "summaries")]
    let mut people_index = if entity_dirs_ready {
        crate::storage::PeopleIndex::build(&paths.vault_dir.join("People"))
    } else {
        crate::storage::PeopleIndex::build(&std::path::PathBuf::new()) // empty
    };

    #[cfg(feature = "summaries")]
    let mut context_preamble = if entity_dirs_ready {
        crate::summary::build_context_preamble(&paths.vault_dir)
    } else {
        String::new()
    };
```

4. **Add entity stat counters** alongside existing counters:

```rust
    #[cfg(feature = "summaries")]
    let mut people_count = 0u32;
    #[cfg(feature = "summaries")]
    let mut concepts_count = 0u32;
    #[cfg(feature = "summaries")]
    let mut projects_count = 0u32;
```

5. **Inside the document loop, after fetching transcript and before summarization, add triage:**

```rust
        // Triage: check if transcript has enough content
        let word_count = count_transcript_words(&transcript);
        let status = if word_count < 20 { "stub" } else { "substantive" };
```

6. **Wrap the existing summarization block in the triage check.** Change from always summarizing to only summarizing when `status == "substantive"`:

The summarization section currently runs unconditionally. Wrap it:

```rust
        // AI summarization via Claude API (only for substantive transcripts)
        #[cfg(feature = "summaries")]
        let (summary_text, extracted_entities): (Option<String>, Option<crate::summary::ExtractedEntities>) =
            if status == "substantive" {
                if let Some((ref config, ref key, ref claude_client)) = summarize_state {
                    let input = crate::summary::format_transcript_for_llm(&transcript, &meta);
                    match crate::summary::summarize_transcript(&input, key, config, claude_client, &context_preamble) {
                        Ok(raw_summary) => {
                            summarized += 1;
                            let (clean_md, entities) = crate::summary::parse_summary_output(&raw_summary);
                            (Some(clean_md), entities)
                        }
                        Err(e) => {
                            eprintln!("Warning: Failed to summarize {}: {}", doc_summary.id, e);
                            (None, None)
                        }
                    }
                } else {
                    (None, None)
                }
            } else {
                (None, None)
            };

        #[cfg(not(feature = "summaries"))]
        let (summary_text, extracted_entities): (Option<String>, Option<()>) = (None, None);
```

7. **Build the `related` list from extracted entities:**

```rust
        // Build related list from extracted entities
        let related: Vec<String> = {
            #[cfg(feature = "summaries")]
            {
                match &extracted_entities {
                    Some(entities) => {
                        let mut r = Vec::new();
                        for p in &entities.people {
                            r.push(format!("[[{}]]", p.name));
                        }
                        for c in &entities.concepts {
                            r.push(format!("[[{}]]", c.name));
                        }
                        for p in &entities.projects {
                            r.push(format!("[[{}]]", p.name));
                        }
                        r
                    }
                    None => vec![],
                }
            }
            #[cfg(not(feature = "summaries"))]
            { vec![] }
        };
```

8. **Update the `to_markdown()` call** to pass `related` and `status`:

```rust
        let md = to_markdown(
            &transcript,
            &meta,
            &doc_summary.id,
            notes_md.as_deref(),
            summary_text.as_deref(),
            related,
            Some(status),
        )?;
```

9. **After writing files and setting file times, add entity reconciliation:**

```rust
        // Entity reconciliation: create/update People, Concepts, Projects notes
        #[cfg(feature = "summaries")]
        if let Some(ref entities) = extracted_entities {
            if entity_dirs_ready && !dry_run {
                let slug = crate::util::slugify(meta.title.as_deref().unwrap_or("untitled"));
                let date = meta.created_at.format("%Y-%m-%d").to_string();
                let meeting_slug = format!("{}_{}", date, slug);

                // Extract attendee names for disambiguation
                let attendee_names: Vec<String> = if let Some(ref rich) = meta.attendees {
                    rich.iter().filter_map(|a| a.name.clone()).collect()
                } else {
                    meta.participants.clone()
                };

                // People
                for person in &entities.people {
                    let match_result = people_index.find_match(&person.name, &attendee_names);
                    if let Some((canonical, existing_path)) = match_result {
                        // Enrich existing
                        let alias_refs: Vec<&str> = person.aliases.iter().map(|s| s.as_str()).collect();
                        if let Err(e) = crate::storage::enrich_person_note(
                            &existing_path, &alias_refs, &person.context, &meeting_slug, &date, &paths.tmp_dir,
                        ) {
                            eprintln!("Warning: Failed to enrich People/{}: {}", canonical, e);
                        }
                    } else if person.name.contains(' ') {
                        // Only create for full names
                        let people_dir = paths.vault_dir.join("People");
                        let alias_refs: Vec<&str> = person.aliases.iter().map(|s| s.as_str()).collect();
                        if let Err(e) = crate::storage::create_person_note(
                            &people_dir,
                            &person.name,
                            person.role.as_deref(),
                            person.company.as_deref(),
                            &alias_refs,
                            &person.context,
                            &meeting_slug,
                            &date,
                            &paths.tmp_dir,
                        ) {
                            eprintln!("Warning: Failed to create People/{}: {}", person.name, e);
                        } else {
                            people_index.add_person(&person.name, &people_dir, &alias_refs);
                        }
                    }
                    people_count += 1;
                }

                // Concepts
                let concepts_dir = paths.vault_dir.join("Concepts");
                for concept in &entities.concepts {
                    let existing = crate::storage::find_entity_file(&concepts_dir, &concept.name);
                    if let Some(existing_path) = existing {
                        if let Err(e) = crate::storage::enrich_concept_note(&existing_path, &meeting_slug, &date, &paths.tmp_dir) {
                            eprintln!("Warning: Failed to enrich Concepts/{}: {}", concept.name, e);
                        }
                    } else {
                        if let Err(e) = crate::storage::create_concept_note(
                            &concepts_dir, &concept.name, &concept.description, &meeting_slug, &date, &paths.tmp_dir,
                        ) {
                            eprintln!("Warning: Failed to create Concepts/{}: {}", concept.name, e);
                        } else {
                            // Update in-memory context preamble so subsequent meetings see this concept
                            context_preamble = crate::summary::build_context_preamble(&paths.vault_dir);
                        }
                    }
                    concepts_count += 1;
                }

                // Projects
                let projects_dir = paths.vault_dir.join("Projects");
                for project in &entities.projects {
                    let existing = crate::storage::find_entity_file(&projects_dir, &project.name);
                    if let Some(existing_path) = existing {
                        if let Err(e) = crate::storage::enrich_project_note(&existing_path, &project.description, &meeting_slug, &paths.tmp_dir) {
                            eprintln!("Warning: Failed to enrich Projects/{}: {}", project.name, e);
                        }
                    } else {
                        if let Err(e) = crate::storage::create_project_note(
                            &projects_dir, &project.name, &project.description, &meeting_slug, &date, &paths.tmp_dir,
                        ) {
                            eprintln!("Warning: Failed to create Projects/{}: {}", project.name, e);
                        } else {
                            // Update in-memory context preamble
                            context_preamble = crate::summary::build_context_preamble(&paths.vault_dir);
                        }
                    }
                    projects_count += 1;
                }
            }
        }
```

10. **Update the stats message** to include entity counts:

```rust
    #[cfg(feature = "summaries")]
    let stats_msg = format!(
        "synced {} docs ({} new/updated, {} skipped, {} summarized, {} people, {} concepts, {} projects)",
        docs.len(), synced, skipped, summarized, people_count, concepts_count, projects_count
    );
```

- [ ] **Step 2: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 3: Run linter on both feature sets**

Run: `cargo clippy --all-features -- -D warnings && cargo clippy --no-default-features -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Run formatter**

Run: `cargo fmt`

- [ ] **Step 5: Commit**

```bash
git add src/sync.rs
git commit -m "feat: wire entity extraction and reconciliation into sync loop"
```

### Task 12: Integration tests

**Files:**
- Modify: `tests/workflow_integration.rs`

- [ ] **Step 1: Add entity extraction integration tests**

Append to `tests/workflow_integration.rs`:

```rust
/// Test: parse_summary_output strips entity JSON and returns clean markdown
#[test]
fn test_summary_output_parsing_integration() {
    let raw = r#"## Summary
- Key point

## People
| [[Alice]] | Engineer |

<!-- baez-entities
{"people": [{"name": "Alice Smith", "role": "Engineer", "company": "Acme", "aliases": ["Alice"], "context": "Led discussion"}], "concepts": [{"name": "API Design", "description": "Building APIs first", "existing": false}], "projects": []}
-->"#;

    let (markdown, entities) = baez::summary::parse_summary_output(raw);

    // Markdown is clean
    assert!(markdown.contains("## Summary"));
    assert!(!markdown.contains("baez-entities"));

    // Entities are parsed
    let entities = entities.unwrap();
    assert_eq!(entities.people.len(), 1);
    assert_eq!(entities.people[0].name, "Alice Smith");
    assert_eq!(entities.concepts.len(), 1);
}

/// Test: to_markdown with related and status produces correct frontmatter
#[test]
fn test_to_markdown_with_related_and_status() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc1".into()),
        title: Some("Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec!["Alice".into()],
        duration_seconds: None,
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let related = vec!["[[Alice Smith]]".into(), "[[API Design]]".into()];
    let output = baez::to_markdown(&raw, &meta, "doc1", None, None, related, Some("substantive")).unwrap();

    assert!(output.frontmatter_yaml.contains("related:"));
    assert!(output.frontmatter_yaml.contains("[[Alice Smith]]"));
    assert!(output.frontmatter_yaml.contains("status: substantive"));
}

/// Test: triage classifies short transcripts as stubs
#[test]
fn test_triage_stub_classification() {
    use baez::model::{RawTranscript, TranscriptEntry};

    let stub = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: None,
            start: None,
            end: None,
            text: "hello world".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };
    assert!(baez::count_transcript_words(&stub) < 20);

    let substantive_text = (0..25).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
    let substantive = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: None,
            start: None,
            end: None,
            text: substantive_text,
            source: None,
            id: None,
            is_final: None,
        }],
    };
    assert!(baez::count_transcript_words(&substantive) >= 20);
}

/// Test: entity note creation with tempdir
#[test]
fn test_entity_note_creation_integration() {
    use tempfile::TempDir;
    use std::fs;

    let temp = TempDir::new().unwrap();
    let people_dir = temp.path().join("People");
    let concepts_dir = temp.path().join("Concepts");
    let projects_dir = temp.path().join("Projects");
    let tmp_dir = temp.path().join("tmp");

    fs::create_dir_all(&people_dir).unwrap();
    fs::create_dir_all(&concepts_dir).unwrap();
    fs::create_dir_all(&projects_dir).unwrap();
    fs::create_dir_all(&tmp_dir).unwrap();

    // Create entities
    baez::create_person_note(
        &people_dir, "Alice Smith", Some("Engineer"), Some("Acme"),
        &["Alice"], "Led discussion", "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    baez::create_concept_note(
        &concepts_dir, "API Design", "Building APIs first",
        "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    baez::create_project_note(
        &projects_dir, "Project Atlas", "Migration tool",
        "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    // Verify files exist with correct content
    assert!(people_dir.join("Alice Smith.md").exists());
    assert!(concepts_dir.join("API Design.md").exists());
    assert!(projects_dir.join("Project Atlas.md").exists());

    let people_content = fs::read_to_string(people_dir.join("Alice Smith.md")).unwrap();
    assert!(people_content.contains("[[2025-01-15_standup]]"));
    assert!(people_content.contains("Led discussion"));

    // Enrich the person note
    baez::enrich_person_note(
        &people_dir.join("Alice Smith.md"),
        &["AS"], "Reviewed migration plan", "2025-01-20_planning", "2025-01-20", &tmp_dir,
    ).unwrap();

    let enriched = fs::read_to_string(people_dir.join("Alice Smith.md")).unwrap();
    assert!(enriched.contains("[[2025-01-15_standup]]"));
    assert!(enriched.contains("[[2025-01-20_planning]]"));
    assert!(enriched.contains("Reviewed migration plan"));
    assert!(enriched.contains("last-contact: \"2025-01-20\""));
}
```

- [ ] **Step 2: Run integration tests**

Run: `cargo test --all-features --test workflow_integration -- --nocapture`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add tests/workflow_integration.rs
git commit -m "test: add integration tests for entity extraction and knowledge graph"
```

### Task 13: Final CI verification

- [ ] **Step 1: Run full CI equivalent**

Run: `just ci`
Expected: Format OK, lint OK, all tests pass

- [ ] **Step 2: Verify both feature sets compile**

Run: `cargo check --all-features && cargo check --no-default-features`
Expected: Both compile without errors

- [ ] **Step 3: Run tests without default features**

Run: `cargo test --no-default-features`
Expected: All tests PASS (entity/summary code gated behind `summaries` feature)
