# Build release binary (default features)
build:
    cargo build --release

# Run tests (default features)
test:
    cargo test --lib

# Run all tests across all feature sets
test-all:
    cargo test --all-features --no-fail-fast

# Check compilation (fast, no codegen)
check:
    cargo check

# Format code
fmt:
    cargo fmt

# Run clippy linter
lint:
    cargo clippy --all-features -- -D warnings
    cargo clippy --no-default-features -- -D warnings

# Full CI check: format, lint, test
ci: fmt lint test-all

# Install to ~/.cargo/bin
install:
    cargo install --path .

# Clean build artifacts
clean:
    cargo clean
