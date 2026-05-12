# Suggested Commands

## Development
- `cargo build --release` or `just build` — Build release binary
- `cargo check` or `just check` — Fast compilation check (no codegen)
- `cargo fmt` or `just fmt` — Format code
- `cargo clippy --all-features -- -D warnings` — Lint with all features
- `cargo clippy --no-default-features -- -D warnings` — Lint without optional features

## Testing
- `cargo test --lib` or `just test` — Run unit tests (default features)
- `cargo test --all-features --no-fail-fast` or `just test-all` — Run all tests across all feature sets
- `cargo test --lib <module_name>` — Run tests for a specific module (e.g., `cargo test --lib convert`)
- `cargo insta review` — Review and accept/reject snapshot test changes

## Full CI
- `just ci` — Format + lint + test-all (matches GitHub Actions)

## Install & Run
- `cargo install --path .` or `just install` — Install to ~/.cargo/bin
- `baez sync --vault /path/to/vault` — Sync transcripts (also runs summarization + entity extraction by default)
- `baez sync --force` — Force re-sync all (ignores both caches)
- `baez sync --no-summarize` — Sync without AI summaries or entity extraction
- `baez sync --dry-run` — Preview sync without writing
- `baez list` — List all documents
- `baez fetch <doc-id>` — Fetch a single document
- `baez summarize <doc-id> --save` — Summarize a single doc and save to file
- `baez summarize-all` — Batch-summarize all synced docs that lack summaries (reads local raw JSON, never hits Granola)
- `baez summarize-all --force` — Re-summarize everything (use after changing model)
- `baez summarize-all --dry-run` — Preview what would be summarized
- `baez set-api-key <key>` — Store Anthropic key in macOS keychain
- `baez set-config --show` — Show summarization config
- `baez set-config --model claude-sonnet-4-20250514` — Change model
- `baez fix-dates` — Repair file mtimes from frontmatter `created` field

## System (Darwin/macOS)
- `git`, `ls`, `cd`, `grep`, `find` — Standard Unix tools (macOS versions)
- `open <dir>` — Open directory in Finder (used by `baez open`)
