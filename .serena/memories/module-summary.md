# Module: summary.rs — AI Summarization + Entity Extraction (Feature-Gated)

## Purpose
Claude API integration for generating structured meeting summaries AND extracting entities (people, concepts, projects) for the knowledge graph. Gated behind `summaries` feature flag.

## Key Constants
- **`DEFAULT_SUMMARY_PROMPT`** — Structured 10-section prompt: Summary, Key Decisions, Action Items, Discussion Highlights, Open Questions, People (table), Project Ideas, Blog Ideas, Concepts, Ideas. Plus an entity JSON block instruction wrapped in `<!-- baez-entities ... -->`. Uses Dataview inline fields (`[decision:: ...]`, `[owner:: [[Name]]]`).
- **`CHUNK_SUMMARY_PROMPT`** — Simplified narrative prompt for intermediate chunk summaries (no entity extraction, no overall conclusions)
- **`CLAUDE_API_URL`** = `https://api.anthropic.com/v1/messages`
- **`ANTHROPIC_VERSION`** = `2023-06-01`
- **`ENTITY_MARKER_START`** = `<!-- baez-entities` / **`ENTITY_MARKER_END`** = `-->`
- **`MAX_RETRIES`** = 2, **`INITIAL_RETRY_DELAY`** = 2s

## Key Structs
- **`SummaryConfig`**: `{ model, max_input_chars, max_tokens, custom_prompt, temperature }`
  - Default: model `claude-opus-4-6`, 600K chars, **8192 tokens**, no temperature
  - `load(path)` / `save(path, tmp_dir)` — JSON config persistence
  - `prompt()` → custom or default prompt
- **`ExtractedEntities`**: `{ people: Vec<PersonEntity>, concepts: Vec<ConceptEntity>, projects: Vec<ProjectEntity> }` — all `#[serde(default)]`
- **`PersonEntity`**: `{ name, role?, company?, aliases: Vec<String>, context: String }`
- **`ConceptEntity`**: `{ name, description, existing: bool }`
- **`ProjectEntity`**: `{ name, description, existing: bool }`

## Key Functions
- **`parse_summary_output(raw)`** → `(String, Option<ExtractedEntities>)`
  - Splits on `<!-- baez-entities ... -->`, returns markdown + parsed JSON
  - Non-fatal parsing: missing/malformed JSON returns full text + `None`
- **`format_transcript_for_llm(raw, meta)`** → `String`: Formats metadata header + `Speaker (HH:MM:SS): text` lines
- **`summarize_transcript(text, api_key, config, client, context_preamble)`** → `Result<String>`:
  - Chunks long transcripts via `chunk_transcript()`
  - Single chunk → prepends `context_preamble` (list of existing concepts/projects), single Claude call
  - Multi-chunk → each chunk uses `CHUNK_SUMMARY_PROMPT`, then a final combine pass uses full `DEFAULT_SUMMARY_PROMPT` + preamble
- **`build_claude_client()`** → `Result<reqwest::blocking::Client>`: 120s timeout
- **`call_claude_api()`** (private): POST to Messages API with retry, structured error parsing
- **`chunk_transcript(text, max_chars)`** — line-aware chunking
- **`update_summary_in_markdown(content, new_summary)`** → `String`: Replace existing `## Summary` section, or insert before `## Notes`/`---`/EOF
- **`build_context_preamble(vault_dir)`** → `String`: Scans `Concepts/` and `Projects/` dirs, returns a preamble listing existing names so Claude can reference rather than duplicate. Rebuilt after each new concept/project creation.
- **`scan_entity_dir(dir)`** (private) — collects `*.md` stems
- **`get_api_key()` / `get_api_key_verbose(verbose)`**: Precedence: `BAEZ_ANTHROPIC_API_KEY` → `ANTHROPIC_API_KEY` → config file → macOS keychain
- **`get_api_key_from_config()` / `get_api_key_from_keychain()` / `set_api_key_in_keychain()`**

## Claude API Details
- URL: `https://api.anthropic.com/v1/messages`
- Version: `2023-06-01`
- Headers: `x-api-key`, `anthropic-version`, `content-type`
- Chunking: splits transcript by lines when > `max_input_chars`, summarizes chunks then combines
- Retry: 2 retries, 2s initial delay, retries on network/429/5xx

## Test Modules
- `tests`, `parse_tests`, `context_tests`
