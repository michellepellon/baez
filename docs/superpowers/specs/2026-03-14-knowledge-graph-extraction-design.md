# Knowledge Graph Extraction Design

Enhance baez's sync pipeline to produce structured meeting summaries with 7 extraction categories, create/update People/Concepts/Projects entity notes in the vault, and add `related`/`status` frontmatter fields — turning synced transcripts into an Obsidian knowledge graph.

**Approach:** Single Claude API call per meeting (Approach A). One enriched prompt produces both the markdown summary and a machine-readable JSON entity block. Baez parses the JSON to create/update vault entity notes during `sync_all()`. No new commands, no multi-agent dispatch, no second LLM pass.

**Relationship to vault enhancements spec:** This spec **supersedes Feature 2** (Dataview Inline Fields) of `2026-03-11-vault-enhancements-design.md` — the enhanced prompt here includes the Dataview inline field syntax and extends it with 5 additional extraction categories plus the JSON entity block. Features 1 (backlink-aware updates), 3 (daily notes linking), and 4 (watch mode) from the vault enhancements spec remain independent and compatible. When both specs are implemented, markdown files should be written via `write_with_user_content()` (from vault enhancements Feature 1) rather than `write_atomic()`, and the `<!-- baez-managed-above -->` marker should appear after all generated content including the summary sections.

---

## Feature 1: Enhanced Summary Prompt

### Problem
The current `DEFAULT_SUMMARY_PROMPT` produces 5 narrative sections (Summary, Key Decisions, Action Items, Discussion Highlights, Open Questions). These are human-readable but not machine-queryable. The output doesn't identify people, concepts, or projects in a structured way that baez can act on.

### Design

Replace `DEFAULT_SUMMARY_PROMPT` in `summary.rs` with an enriched prompt that produces:

**Markdown sections (written to vault file):**
1. `## Summary` — 3-7 bullet points
2. `## Key Decisions` — Dataview inline fields: `- [decision:: ...]`
3. `## Action Items` — `- [ ] [owner:: [[Name]]] [action:: Task] *(due: ..., priority: ...)*`
4. `## Discussion Highlights` — grouped by topic with `###` subheadings, `[[wiki-links]]`
5. `## Open Questions` — bulleted list
6. `## People` — table: `| [[Person]] | Role/Context |`
7. `## Project Ideas` — `[[Project Name]]` with context
8. `## Blog Ideas` — angle and why it's worth writing
9. `## Concepts` — `[[Concept Name]]` with brief description
10. `## Ideas` — general thoughts worth preserving

**JSON entity block (stripped before writing, consumed by baez):**
```
<!-- baez-entities
{
  "people": [
    {"name": "Alice Smith", "role": "Engineering Manager", "company": "Acme Corp", "aliases": ["Alice", "AS"], "context": "Led discussion on API migration timeline"}
  ],
  "concepts": [
    {"name": "API-First Design", "description": "Building APIs before UIs to enable parallel development", "existing": true}
  ],
  "projects": [
    {"name": "Project Atlas", "description": "Internal tool migration initiative", "existing": false}
  ]
}
-->
```

**Prompt context preamble:** Before the transcript, the prompt includes lists of existing Concepts and Projects from the vault so Claude can set `existing: true` and reuse exact names:
```
Existing concepts in the vault (reference by exact name if relevant,
only propose new ones if genuinely distinct):
- API-First Design
- Conway's Law

Existing projects in the vault (reference by exact name if relevant):
- Project Atlas
- Homelab Dashboard
```

People/ names are NOT included in the prompt — name matching is handled post-hoc via fuzzy matching.

**Context scanning:** `build_context_preamble(vault_dir)` scans `Vault/Concepts/` and `Vault/Projects/` once per `sync_all()` call. It reads **filename stems only** (strip `.md` extension) — no frontmatter parsing. This is fast and avoids I/O on potentially hundreds of files. Lists are held in memory and updated as new entities are created during the sync. If either directory doesn't exist or is empty, that section is omitted.

**`max_tokens` default increase:** `SummaryConfig::default()` changes `max_tokens` from `4096` to `8192`. The expanded prompt (10 markdown sections + JSON entity block) routinely produces 3-5K tokens for substantive meetings. The old 4096 limit would truncate output. Users can still override via `baez set-config`.

**Chunked transcript handling:** `summarize_transcript()` currently chunks long transcripts, summarizes each chunk, then re-summarizes the combined chunk summaries. With the new prompt, the **intermediate chunk summaries** use a simplified prompt that produces only narrative markdown (no JSON entity block, no structured sections). Only the **final recombination pass** uses the full enhanced prompt with entity extraction. The context preamble (existing Concepts/Projects) is included only in the final pass. This ensures entity extraction sees the full picture rather than partial per-chunk entities.

### Files Modified
- `src/summary.rs` — replace `DEFAULT_SUMMARY_PROMPT`, add `build_context_preamble()`, add `CHUNK_SUMMARY_PROMPT` constant, increase default `max_tokens`, modify `summarize_transcript()` to use different prompts for chunks vs. final pass

---

## Feature 2: Summary Parsing

### Problem
Claude's output contains both the markdown summary (for the vault file) and the JSON entity block (for baez to act on). These must be cleanly separated.

### Design

New function `parse_summary_output(raw: &str) -> (String, Option<ExtractedEntities>)` in `summary.rs`:

1. Search for `<!-- baez-entities` marker
2. If found: everything before is markdown, content between `<!-- baez-entities` and `-->` is JSON
3. Parse JSON into `ExtractedEntities`
4. Return clean markdown (marker stripped) and parsed entities
5. If marker not found: return full text as markdown, `None` for entities
6. If JSON is malformed: log warning, return full text as markdown, `None` for entities

**Data structures (in `summary.rs`):**

```rust
pub struct ExtractedEntities {
    pub people: Vec<PersonEntity>,
    pub concepts: Vec<ConceptEntity>,
    pub projects: Vec<ProjectEntity>,
}

pub struct PersonEntity {
    pub name: String,
    pub role: Option<String>,
    pub company: Option<String>,
    pub aliases: Vec<String>,
    pub context: String,
}

pub struct ConceptEntity {
    pub name: String,
    pub description: String,
    pub existing: bool,
}

pub struct ProjectEntity {
    pub name: String,
    pub description: String,
    pub existing: bool,
}
```

All structs derive `Debug, Clone, Serialize, Deserialize`.

**Robustness:** Parsing failures are always non-fatal. The sync continues with the markdown summary intact; entity notes are simply not created for that document.

### Files Modified
- `src/summary.rs` — add structs and `parse_summary_output()`

---

## Feature 3: Frontmatter Enhancements

### Problem
Meeting files are isolated documents with no outward links to vault entities. No way to filter substantive meetings from empty stubs.

### Design

Add two fields to `Frontmatter` in `model.rs`:

```rust
/// Wiki-linked entity references for Obsidian graph connectivity
#[serde(default, skip_serializing_if = "Vec::is_empty")]
pub related: Vec<String>,

/// Transcript quality: "substantive" or "stub"
#[serde(default, skip_serializing_if = "Option::is_none")]
pub status: Option<String>,
```

**`related`** — populated from the parsed JSON entity block. Contains wiki-links to all People, Concepts, and Projects referenced:
```yaml
related:
  - "[[Alice Smith]]"
  - "[[API-First Design]]"
  - "[[Project Atlas]]"
```

**`status`** — set during triage:
- `"substantive"` — full processing (summarization + entity extraction)
- `"stub"` — transcript had < 20 real words, skipped summarization

Both fields use `skip_serializing_if` so they don't appear when empty/None (backward compat).

**`to_markdown()` signature change** in `convert.rs`:
```rust
pub fn to_markdown(
    raw: &RawTranscript,
    meta: &DocumentMetadata,
    doc_id: &str,
    notes: Option<&str>,
    summary_text: Option<&str>,
    related: Vec<String>,        // NEW
    status: Option<&str>,        // NEW
) -> Result<MarkdownOutput>
```

**Summary insertion change:** Currently `to_markdown()` wraps `summary_text` in a `## Summary` heading (lines 142-148 of `convert.rs`). With the enhanced prompt, Claude's output already contains `## Summary`, `## Key Decisions`, `## People`, etc. as top-level headings. The `summary_text` parameter is now inserted **as-is** into the body (no wrapping `## Summary\n\n` added by baez). The calling code is responsible for passing clean markdown from `parse_summary_output()` which already has the section headings.

**`related` field format:** Meeting frontmatter `related` contains wiki-links to entities (`"[[Alice Smith]]"`, `"[[API-First Design]]"`). Entity note frontmatter `related` contains wiki-links back to meetings (`"[[YYYY-MM-DD_meeting-slug]]"`). This bidirectional linking is intentional — meetings link outward to entities, entities link back to their source meetings.

### Files Modified
- `src/model.rs` — add fields to `Frontmatter`
- `src/convert.rs` — add parameters to `to_markdown()`, populate frontmatter fields, change summary insertion to raw (no heading wrapper)
- `src/lib.rs` — no new re-exports needed (Frontmatter already re-exported)

---

## Feature 4: Triage Logic

### Problem
Sending empty stubs and garbled recordings through Claude summarization wastes API calls and produces useless entity notes.

### Design

New function `count_transcript_words(transcript: &RawTranscript) -> usize` in `util.rs`:
- Iterates `transcript.entries`
- Splits each `entry.text` by whitespace, counts words
- Returns total

**Threshold:** 20 words. Below → `status: "stub"`, skip summarization and entity extraction. Above → `status: "substantive"`, full processing.

**No content-based filtering.** Baez does not skip meetings by topic (medical, personal, etc.). Only quantity-based triage.

**Re-sync upgrade:** If a previously-stubbed meeting gains a real transcript (Granola backfill), `--force` re-sync detects the higher word count and upgrades to `substantive`.

**Integration in `sync_all()`:**
```
word_count = count_transcript_words(&transcript)
if word_count < 20:
    status = "stub"
    summary_text = None
    entities = None
else:
    status = "substantive"
    // proceed with summarization + entity extraction
```

### Files Modified
- `src/util.rs` — add `count_transcript_words()`
- `src/sync.rs` — call before summarization

---

## Feature 5: Entity Note Creation and Enrichment

### Problem
Meeting transcripts mention people, concepts, and projects but don't create corresponding vault notes. Knowledge doesn't compound across meetings.

### Design

Three entity types, all created/updated during `sync_all()` after parsing the JSON entity block.

### People Notes

**Path:** `{vault_dir}/People/Firstname Lastname.md` (Title Case filename)

**New person — create stub:**
```yaml
---
title: "Firstname Lastname"
date: "YYYY-MM-DD"
tags: [people]
aliases: ["Alice", "AS"]
type: person
company: "Acme Corp"
role: "Engineering Manager"
last-contact: "YYYY-MM-DD"
status: active
related: ["[[YYYY-MM-DD_meeting-slug]]"]
---

# Firstname Lastname

## Context
- **Company:** Acme Corp
- **Role:** Engineering Manager

## Notes
- From [[YYYY-MM-DD_meeting-slug]]: Led discussion on API migration timeline
```

**Existing person — enrich:**
1. Read file, parse frontmatter (YAML between `---` delimiters)
2. Add meeting to `related` array if not already present
3. Update `last-contact` if this meeting is more recent
4. Merge new aliases into `aliases` array (deduplicated)
5. Append bullet to `## Notes` section: `- From [[meeting]]: context`

### Concepts Notes

**Path:** `{vault_dir}/Concepts/Concept Name.md`

**New concept (`existing: false`):**
```yaml
---
title: "Concept Name"
date: "YYYY-MM-DD"
tags: [concept]
type: concept
status: active
related: ["[[YYYY-MM-DD_meeting-slug]]"]
---

# Concept Name

## What is it?
Description from JSON block.

## Sources
- [[YYYY-MM-DD_meeting-slug]] — extracted YYYY-MM-DD
```

**Existing concept (`existing: true`):** Add meeting to `related`, append to `## Sources`.

### Projects Notes

**Path:** `{vault_dir}/Projects/Project Name.md`

**New project (`existing: false`):**
```yaml
---
title: "Project Name"
date: "YYYY-MM-DD"
tags: [project]
type: project
status: active
related: ["[[YYYY-MM-DD_meeting-slug]]"]
---

# Project Name

Mentioned in [[YYYY-MM-DD_meeting-slug]]: description from JSON block.
```

**Existing project (`existing: true`):** Add meeting to `related`, append mention line.

### Entity Note Frontmatter Parsing

Entity notes use a different frontmatter schema than meeting files (e.g., `type: person`, `company`, `role`, `last-contact`). The existing `read_frontmatter()` in `storage.rs` parses into the typed `Frontmatter` struct and **cannot be reused** for entity notes. Instead, a new function `read_entity_frontmatter(path) -> Option<(serde_json::Value, String)>` reads the YAML between `---` delimiters and deserializes into `serde_json::Value` for flexible field access. The second return value is the body content after the closing `---`. Modifications update the `Value` in-place, re-serialize to YAML, and concatenate with the (possibly modified) body.

### Enrichment Robustness

When enriching existing entity notes:
- **Missing `## Notes` section** (People): append `\n\n## Notes\n` at the end of the file, then add the bullet.
- **Missing `## Sources` section** (Concepts): append `\n\n## Sources\n` at the end of the file, then add the source line.
- **`existing: true` but file deleted:** If Claude marks a concept/project as `existing: true` but the file no longer exists on disk (deleted between preamble scan and write), fall through to creation. The `existing` flag is a hint, not a guarantee.
- **Duplicate `related` entries:** Before adding a meeting reference to `related`, check if it's already present. Skip if duplicate.

### Filename Normalization

Entity note filenames must be consistent to avoid duplicates on case-insensitive filesystems (macOS default):
- **People:** Title Case derived from the name (`Alice Smith` → `People/Alice Smith.md`). Use the `name` field from the JSON entity block.
- **Concepts and Projects:** Use the exact name from the JSON entity block for new entities. For existing entities, Claude is given the exact filename stem in the preamble and instructed to reuse it — so the name matches. When checking existence, use **case-insensitive filename comparison** to catch capitalization mismatches (e.g., if Claude outputs "API-first Design" but the file is "API-First Design.md").

### Error Handling

All entity note operations are **non-fatal**. If creating/updating a note fails (permissions, I/O, parse error), print a warning to stderr and continue. A failed entity note never aborts the sync.

### Files Modified
- `src/storage.rs` — add `create_person_note()`, `enrich_person_note()`, `create_concept_note()`, `enrich_concept_note()`, `create_project_note()`, `enrich_project_note()`
- `src/sync.rs` — call entity functions after parsing summary output

---

## Feature 6: Fuzzy Name Matching

### Problem
Transcript speaker names are unreliable — misspelled, nicknames, first-name-only. Exact string matching misses obvious connections.

### Design

**New dependency:** `strsim = "0.11"` in `Cargo.toml` for Levenshtein distance.

**`PeopleIndex` struct in `storage.rs`:**
```rust
pub struct PeopleIndex {
    /// Maps lowercase canonical name → file path
    entries: HashMap<String, PathBuf>,
    /// Maps lowercase alias → canonical name
    aliases: HashMap<String, String>,
}
```

**Building the index:**
- Scan `{vault_dir}/People/` directory
- For each `.md` file: extract filename stem as canonical name, read frontmatter for `aliases`
- Build both maps
- Done once per `sync_all()` call, updated in-memory as new People notes are created

**Matching algorithm — `PeopleIndex::find_match(name, attendees) -> Option<(String, PathBuf)>`:**

The `attendees` parameter is a `&[String]` of flat name strings. The caller extracts these from `DocumentMetadata`: if `meta.attendees` (rich `Option<Vec<Attendee>>`) is `Some`, map to `a.name.clone().unwrap_or_default()`; otherwise fall back to `meta.participants` (`Vec<String>`).

Steps:
1. **Exact match:** lowercase `name` against `entries` keys → match
2. **Alias match:** lowercase `name` against `aliases` keys → match
3. **Attendee disambiguation:** if `name` is a single word (first-name-only), check if any name in `attendees` starts with `name` (case-insensitive). If exactly one does, use that full name and re-try steps 1-2.
4. **Fuzzy match:** Levenshtein distance against all `entries` keys and `aliases` keys. Threshold: ≤ 2 edits for names > 5 chars, exact match required for names ≤ 5 chars (short names have too many false positives).
5. **Ambiguity:** if multiple fuzzy matches at the same distance, return `None`.

**First-name-only resolution:** If after all steps the name is still first-name-only with exactly one fuzzy match → link to that person. Zero or multiple matches → leave as plain text in the summary, don't create a People note.

**Responsibility split:**
- `util.rs` — `levenshtein_distance(a, b) -> usize`: thin wrapper around `strsim::levenshtein`. Pure function, no filesystem access.
- `storage.rs` — `PeopleIndex` struct with `build()` and `find_match()` methods. Uses `levenshtein_distance()` internally. Owns the index data and filesystem scanning logic.

### Files Modified
- `Cargo.toml` — add `strsim = "0.11"`
- `src/util.rs` — add `levenshtein_distance()` wrapper
- `src/storage.rs` — add `PeopleIndex` struct with `build()` and `find_match()` methods

---

## Feature 7: Sync Loop Orchestration

### Problem
`sync_all()` needs to coordinate triage, summarization, entity parsing, fuzzy matching, and entity note creation in the right order.

### Design

**Updated `sync_all()` flow:**

```
1. paths.ensure_dirs()
2. Ensure People/, Concepts/, Projects/ directories exist
3. Build PeopleIndex from People/ (once)
4. Build context preamble from Concepts/ + Projects/ (once)
5. Set up summarization state (config, API key, client)
6. Fetch document list
7. Load sync cache

For each document:
    a. Check cache, skip if unchanged (unless --force)
    b. Fetch metadata + transcript
    c. Triage: count_transcript_words()
       - < 20 words → status="stub", skip to (g)
    d. Build prompt: context preamble + transcript
    e. Call Claude API → raw summary output
    f. parse_summary_output() → clean markdown + entities
    g. Build related list from entities (or empty)
    h. to_markdown(..., related, status)
    i. Write markdown (via write_with_user_content if vault enhancements Feature 1 is implemented, otherwise write_atomic) + raw JSON files
    j. If entities present:
       - For each person: PeopleIndex.find_match() → create or enrich
       - For each concept: create or enrich
       - For each project: create or enrich
       - Update in-memory PeopleIndex and context lists
    k. Update sync cache

8. Print stats (synced, skipped, summarized, entities created/updated)
```

**Directory creation:** Step 2 creates `People/`, `Concepts/`, `Projects/` directories at the vault root only when summarization is enabled AND an API key is available. If the user runs `--no-summarize`, these directories are not created.

**The `--dry-run` flag:** Skips all writes including entity notes. Prints what would be synced but does not create People/Concepts/Projects notes.

**Stats enhancement:** The progress/stats output gains entity counts:
```
synced 15 docs (12 new/updated, 3 skipped, 10 summarized, 23 people, 8 concepts, 3 projects)
```

### Files Modified
- `src/sync.rs` — restructure `sync_all()` loop

---

## Implementation Order

These features should be built in this order:

1. **Frontmatter enhancements** (Feature 3) — add `related`/`status` fields, update `to_markdown()` signature. Foundation for everything else.
2. **Triage logic** (Feature 4) — `count_transcript_words()`. Small, independent.
3. **Summary parsing** (Feature 2) — `ExtractedEntities` structs, `parse_summary_output()`. Can be tested with canned input.
4. **Enhanced summary prompt** (Feature 1) — replace `DEFAULT_SUMMARY_PROMPT`, add `build_context_preamble()`. Depends on entity structs existing.
5. **Fuzzy name matching** (Feature 6) — `PeopleIndex`, `strsim` dependency. Independent of prompt changes.
6. **Entity note creation** (Feature 5) — create/enrich functions in `storage.rs`. Depends on all above.
7. **Sync loop orchestration** (Feature 7) — wire everything together in `sync_all()`. Depends on all above.

---

## Testing Strategy

### Triage (util.rs)
- `count_transcript_words()`: empty → 0, stub → below 20, normal → above 20
- Entries with only whitespace or empty strings don't inflate count

### Summary parsing (summary.rs)
- Valid markdown + JSON → clean split
- No JSON block → full markdown + None
- Malformed JSON → warning + None
- Empty entity arrays → valid empty struct
- `build_context_preamble()`: missing dirs → empty, populated dirs → formatted list
- Chunked transcripts: intermediate chunks produce narrative only, final pass includes JSON entity block
- `CHUNK_SUMMARY_PROMPT` does not contain `baez-entities` marker

### Frontmatter (model.rs, convert.rs)
- Roundtrip with `related` and `status`
- Backward compat: old frontmatter without new fields deserializes with defaults
- Empty `related` not serialized (skip_serializing_if)
- Insta snapshot updated

### Fuzzy matching (storage.rs, util.rs)
- `levenshtein_distance()`: basic distance calculations, known pairs
- `PeopleIndex::build()`: scans directory, extracts names and aliases
- `PeopleIndex::find_match()`: exact, alias, fuzzy within threshold, fuzzy beyond threshold, short name (≤ 5 chars) exact only, ambiguous multiple matches → None
- Attendee disambiguation: first-name "Alice" + attendees ["Alice Smith", "Bob"] → resolves to "Alice Smith"
- Attendee disambiguation: first-name "Alice" + attendees ["Alice Smith", "Alice Jones"] → ambiguous, returns None
- Index update after creating new person reflects in subsequent lookups

### Entity notes (storage.rs)
- `read_entity_frontmatter()`: parses YAML + body split, missing frontmatter → None, malformed YAML → None
- Create: correct path, correct frontmatter, Title Case filename for People
- Enrich: adds `related`, updates `last-contact`, appends to Notes section, deduplicates aliases
- Enrich: doesn't duplicate existing `related` entries
- Enrich with missing `## Notes` section: section created then bullet appended
- Enrich with `existing: true` but file deleted: falls through to creation
- Case-insensitive filename matching for Concepts/Projects
- I/O error → warning, no panic

### Frontmatter (model.rs, convert.rs)
- Summary text inserted as-is (no `## Summary` wrapper added by `to_markdown()`)
- Existing tests updated: callers of `to_markdown()` pass new `related` and `status` params

### Integration (tests/workflow_integration.rs)
- Full flow: transcript → triage → summary → entities → People + Concepts + Projects created
- Stub flow: sparse transcript → status: stub, no entities
- Re-sync: enriches existing People note, updates `last-contact`
- Entity directories created only when summarization is enabled

### Not tested with live API
- Claude API calls mocked via canned summary output
- Prompt format verified by asserting key strings exist in `DEFAULT_SUMMARY_PROMPT`
