# Baez

Sync [Granola](https://granola.ai) meeting transcripts into an
[Obsidian](https://obsidian.md) vault as enriched, read-only markdown — with
optional AI summaries powered by Claude.

## What it does

- Fetches all your Granola transcripts via the API
- Writes Obsidian-flavored markdown with `[[wiki-links]]`, `#tags`, and Dataview-compatible frontmatter
- Organizes files by date: `Vault/Granola/2025/01/2025-01-15_standup.md`
- Preserves raw JSON API responses for archival
- Incremental sync — only fetches documents that changed since last run
- Generates AI meeting summaries using Claude (Anthropic API), with Obsidian-native formatting

## Install

```sh
cargo install --path .
```

## API Keys

Baez needs two separate API keys: one for Granola (to fetch transcripts) and one
for Anthropic (to generate AI summaries). They are configured independently.

### Granola API Key

The Granola token is used to authenticate with the Granola API and fetch your
meeting transcripts. Baez resolves it in this order:

1. **`--token` flag** (highest priority)
   ```sh
   baez sync --token <granola-session-token>
   ```

2. **`BAEZ_GRANOLA_TOKEN` environment variable**
   ```sh
   export BAEZ_GRANOLA_TOKEN=<granola-session-token>
   baez sync
   ```

3. **`BEARER_TOKEN` environment variable** (deprecated, use `BAEZ_GRANOLA_TOKEN`)

4. **Config file** (`~/.config/baez/config.json` → `granola_token` field)

5. **Granola's local session file** (automatic, no setup needed)
   If you're logged into the Granola desktop app on macOS, baez reads the token
   directly from `~/Library/Application Support/Granola/supabase.json`. This is
   the default — if you use the Granola app, you don't need to configure
   anything.

### Anthropic API Key (for AI Summaries)

The Anthropic API key is used to generate meeting summaries via Claude. This is
optional — sync works without it, you'll just get transcripts without summaries.

1. **`BAEZ_ANTHROPIC_API_KEY` environment variable** (highest priority)
   ```sh
   export BAEZ_ANTHROPIC_API_KEY=sk-ant-...
   baez sync
   ```

2. **`ANTHROPIC_API_KEY` environment variable** (also accepted, standard Anthropic convention)
   ```sh
   export ANTHROPIC_API_KEY=sk-ant-...
   baez sync
   ```

3. **Config file** (`~/.config/baez/config.json` → `anthropic_api_key` field)

4. **macOS Keychain** (persistent, recommended on macOS)
   ```sh
   baez set-api-key sk-ant-...
   ```
   This stores the key in the macOS system keychain under the service `baez`
   with the account name `anthropic_api_key`. The key persists across terminal
   sessions and reboots.

If no Anthropic API key is found, `baez sync` prints a warning and continues
without summarization. You can also explicitly skip summaries with
`--no-summarize`.

## Usage

### Set up your vault

```sh
# Option 1: Pass vault path directly
baez sync --vault /path/to/your/obsidian-vault

# Option 2: Set environment variable
export BAEZ_VAULT=/path/to/your/obsidian-vault
baez sync

# Option 3: Save in config file (supports all settings)
mkdir -p ~/.config/baez
echo '{"vault": "/path/to/your/obsidian-vault"}' > ~/.config/baez/config.json
baez sync
```

### Config file

All settings can be stored in `~/.config/baez/config.json`:

```json
{
  "vault": "/path/to/your/obsidian-vault",
  "granola_token": "eyJ...",
  "anthropic_api_key": "sk-ant-..."
}
```

All fields are optional. CLI flags and environment variables take precedence
over config file values.

**Security note:** If you store API keys in the config file, restrict its
permissions so only your user can read it:

```sh
chmod 600 ~/.config/baez/config.json
```

Baez will print a warning at startup if the config file contains keys and is
readable by other users. On macOS, the keychain (`baez set-api-key`) is the
most secure option for the Anthropic API key.

### Commands

```
baez sync                  Sync all documents (with AI summaries if key is set)
baez sync --force          Force re-sync, ignoring cache
baez sync --no-summarize   Sync without AI summaries
baez list                  List all documents
baez fetch <id>            Fetch a specific document
baez open                  Open vault directory
baez fix-dates             Fix file modification dates
```

### AI Summaries

Summaries are generated during `baez sync` when an Anthropic API key is
available. You can also summarize individual documents:

```sh
baez summarize <doc-id>           Print summary to stdout
baez summarize <doc-id> --save    Update the ## Summary section in the markdown file
```

Configure the model and input limits:

```sh
baez set-config --show                            Show current configuration
baez set-config --model claude-sonnet-4-20250514  Change model
baez set-config --context-window 300000           Set max input size (chars)
baez set-config --prompt-file my-prompt.txt       Use a custom prompt
```

The default model is `claude-opus-4-6` with a 600,000 character input limit
(~150K tokens) and 4,096 max output tokens.

## Vault layout

```
Vault/
└── Granola/
    ├── 2025/
    │   ├── 01/
    │   │   ├── 2025-01-15_standup.md
    │   │   └── 2025-01-16_planning.md
    │   └── 02/
    │       └── ...
    └── .baez/
        ├── raw/                  # Raw JSON API responses
        ├── summaries/            # AI-generated summaries
        ├── tmp/                  # Atomic write temp dir
        ├── summary_config.json   # Summarization settings
        └── .sync_cache.json      # Incremental sync state
```

## Markdown format

Each document gets Dataview-compatible frontmatter:

```yaml
doc_id: abc123
source: granola
date: "2025-01-15"
created: 2025-01-15T10:00:00Z
title: Sprint Planning
attendees: [Alice, Bob]
duration_minutes: 30
tags: [planning]
generator: baez
```

The body includes `[[wiki-links]]` for attendees and speakers, `#granola` and
`#meeting/*` tags, an AI-generated `## Summary` section (with Key Decisions,
Action Items, Discussion Highlights, and Open Questions), user notes, and the
full transcript.

## License

MIT
