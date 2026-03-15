# Module: auth.rs — Granola Token Resolution

## Purpose
Multi-level token resolution chain for Granola API authentication.

## Key Symbols
- **`resolve_token(cli_token, verbose)`** → `Result<String>`
  - Precedence: CLI `--token` → `BAEZ_GRANOLA_TOKEN` → `BEARER_TOKEN` (deprecated) → config file → Granola session file
  - Verbose mode prints which source was used
- **`token_from_config()`** (private): reads `granola_token` from `~/.config/baez/config.json`
- **`try_session_file()`** (private): reads `~/Library/Application Support/Granola/supabase.json`
- **`parse_session_file(path)`** (private): parses `workos_tokens` (stringified JSON) → `access_token`

## Granola Session File Format
```json
{
  "workos_tokens": "{\"access_token\": \"the-token\"}"
}
```
Note: `workos_tokens` is a JSON string that needs double-parsing.
