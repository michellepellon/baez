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
- `baez sync --vault /path/to/vault` — Sync transcripts
- `baez sync --force` — Force re-sync all
- `baez sync --dry-run` — Preview sync without writing
- `baez list` — List all documents
- `baez fetch <doc-id>` — Fetch a single document
- `baez summarize <doc-id> --save` — Summarize and save to file

## System (Darwin/macOS)
- `git`, `ls`, `cd`, `grep`, `find` — Standard Unix tools (macOS versions)
- `open <dir>` — Open directory in Finder (used by `baez open`)
