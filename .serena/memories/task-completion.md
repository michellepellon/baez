# What To Do When a Task Is Completed

After making code changes, run these commands to verify correctness:

1. **Format:** `cargo fmt`
2. **Lint (both feature sets):**
   ```
   cargo clippy --all-features -- -D warnings
   cargo clippy --no-default-features -- -D warnings
   ```
3. **Test (all features):** `cargo test --all-features --no-fail-fast`
4. **Or use the all-in-one:** `just ci`

If snapshot tests fail after intentional changes, review with: `cargo insta review`

## Important Notes
- Always test both `--all-features` and `--no-default-features` because `summary.rs` and related CLI commands are feature-gated
- The CI matrix runs on both ubuntu-latest and macos-latest
- Integration tests in `tests/` use async (tokio + wiremock) with `spawn_blocking` bridges
