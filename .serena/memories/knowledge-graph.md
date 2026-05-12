# Feature: Knowledge Graph Extraction

## What It Is
When summarization runs, baez extracts entities (people, concepts, projects) from each meeting and weaves them into a navigable graph: `People/`, `Concepts/`, `Projects/` directories at the **vault root** (not inside `Granola/`), interconnected with meeting notes via `related:` frontmatter and `[[wiki-links]]`.

## End-to-End Flow
1. **Triage:** `count_transcript_words(&transcript)` < 20 → mark as `status: "stub"`, skip summary entirely.
2. **Context preamble:** Before each Claude call, `build_context_preamble(vault_dir)` scans `Concepts/` and `Projects/` and produces a preamble listing existing names. Claude is instructed to reference existing entities by exact name rather than duplicate.
3. **Summarization:** Claude returns markdown summary (10 sections) followed by `<!-- baez-entities { ... } -->` HTML comment containing JSON entities.
4. **Parse:** `parse_summary_output(raw)` splits markdown from entity JSON. Parse failure is non-fatal — markdown is preserved, entities are `None`.
5. **Reconciliation** (`summarize_and_reconcile` in `sync.rs`):
   - **People:** `PeopleIndex::find_match(name, attendees)` runs 4-step lookup:
     1. Exact match on lowercased canonical name
     2. Alias match (from frontmatter `aliases:` array)
     3. First-name-only disambiguation against attendee list (single match)
     4. Levenshtein ≤ 2, skipped for names ≤ 5 chars, ambiguous → None
     - Match → `enrich_person_note` (adds meeting ref to `related`, updates `last-contact`, merges aliases case-insensitively, appends to `## Notes`)
     - No match + name contains space → `create_person_note` + `PeopleIndex::add_person`
     - Skip if no match and name is single-token (avoid orphan first-name notes)
   - **Concepts & Projects:** `find_entity_file(dir, name)` case-insensitive lookup → `enrich_*` or `create_*`
   - After creating a concept/project, `*ctx.context_preamble = build_context_preamble(vault_dir)` rebuilds the preamble so subsequent docs in the same run see the new entity.
6. **Meeting backlinks:** `to_markdown(..., related, status)` includes a `related: ["[[Person A]]", "[[Concept X]]"]` array in the meeting note's frontmatter. In the summarize-only and summarize-all paths, `merge_frontmatter_related(md_path, new_links, tmp_dir)` does a set-union merge on the existing file's frontmatter (hand-rolled string parser preserves field order).

## Entity Note Shapes
- **People** (`Vault/People/Name.md`): frontmatter `type: person, role, company, aliases, last-contact, status, related[]`; body has `## Context` and `## Notes` sections.
- **Concepts** (`Vault/Concepts/Name.md`): frontmatter `type: concept, status: active, related[]`; body has `## What is it?` and `## Sources`.
- **Projects** (`Vault/Projects/Name.md`): frontmatter `type: project, status: active, related[]`; body opens with `Mentioned in [[meeting]]: description`, more mentions appended.

## Implementation Files
- `src/summary.rs` — prompt design, `ExtractedEntities` types, `parse_summary_output`, `build_context_preamble`
- `src/storage.rs` — `PeopleIndex`, `create_*_note`, `enrich_*_note`, `find_entity_file`, `read_entity_frontmatter`, `merge_frontmatter_related`
- `src/sync.rs` — `SummarizationContext`, `summarize_and_reconcile` (the orchestration point)
- `src/util.rs` — `levenshtein_distance` (strsim wrapper)
- `src/model.rs` — `Frontmatter.related: Vec<String>` and `Frontmatter.status: Option<String>`
- `src/convert.rs` — `to_markdown(... , related, status)` signature

## Design Decisions Worth Remembering
- **Entity JSON in HTML comment** keeps the data inline and structured but invisible in rendered Obsidian — no sidecar file needed.
- **Single-token names are not auto-created** as People notes; only multi-word names. Prevents orphaning first names that should resolve to existing full-name notes.
- **Levenshtein threshold = 2, length floor = 6** chosen to balance typo tolerance against false positives on short names (e.g. "Ben" vs "Ken").
- **`existing: bool` field** in the entity JSON is informational — baez treats every entity by lookup, so this is mostly for Claude to self-check against the preamble.
- **Reconciliation is feature-gated.** No knowledge graph without the `summaries` feature.

## Specs
- `docs/superpowers/specs/2026-03-14-knowledge-graph-extraction-design.md`
- `docs/superpowers/plans/2026-03-14-knowledge-graph-extraction.md`
