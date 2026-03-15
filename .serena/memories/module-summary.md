# Module: summary.rs — AI Summarization (Feature-Gated)

## Purpose
Claude API integration for generating structured meeting summaries. Gated behind `summaries` feature flag.

## Key Symbols
- **`SummaryConfig`** (struct): `{ model, max_input_chars, max_tokens, custom_prompt, temperature }`
  - Default: model `claude-opus-4-6`, 600K chars, 4096 tokens
  - `load(path)` / `save(path, tmp_dir)` — JSON config persistence
  - `prompt()` → custom or default prompt
- **`DEFAULT_SUMMARY_PROMPT`** (const): Structured prompt requesting Summary, Key Decisions, Action Items, Discussion Highlights, Open Questions. Uses `[[wiki-links]]` and `- [ ]` checkboxes.
- **`format_transcript_for_llm(raw, meta)`** → `String`: Formats metadata header + `Speaker (HH:MM:SS): text` lines
- **`summarize_transcript(text, api_key, config, client)`** → `Result<String>`: Chunks long transcripts, calls Claude, combines chunk summaries
- **`build_claude_client()`** → `Result<reqwest::blocking::Client>`: 120s timeout
- **`call_claude_api()`** (private): POST to Messages API with retry
- **`update_summary_in_markdown(content, new_summary)`** → `String`: Replace/insert `## Summary` section
- **`get_api_key()` / `get_api_key_verbose(verbose)`**: Precedence: `BAEZ_ANTHROPIC_API_KEY` → `ANTHROPIC_API_KEY` → config file → macOS keychain
- **`get_api_key_from_keychain()` / `set_api_key_in_keychain()`**: macOS-only via `keyring` crate

## Claude API Details
- URL: `https://api.anthropic.com/v1/messages`
- Version: `2023-06-01`
- Headers: `x-api-key`, `anthropic-version`, `content-type`
- Chunking: splits transcript by lines when > `max_input_chars`, summarizes chunks then combines
- Retry: 2 retries, 2s initial delay, retries on network/429/5xx
