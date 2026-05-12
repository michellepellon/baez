# Granola Token Auto-Refresh

## Problem

Baez authenticates to the Granola API with an access token read from the desktop app's session file at `~/Library/Application Support/Granola/supabase.json`. These tokens are short-lived (~6 hours) WorkOS User Management JWTs. When the desktop app isn't running, no one refreshes them, and `baez sync` fails with a confusing 401:

```
baez: [E4] API error 401 on /v2/get-documents: {"message":"Unauthorized"}
```

The user has to open the Granola desktop app, let it refresh the token, then re-run baez. This makes baez unsuitable as a headless cron job and creates friction for everyday use.

## Solution

When baez sources its token from the Granola session file, attempt a refresh using the `refresh_token` already present in the file. The refresh hits WorkOS directly (the same endpoint Granola's app uses) and updates `supabase.json` in place so the desktop app stays in sync. No separate baez-managed cache.

Refresh is **reactive only** — triggered when a Granola request returns 401. There is no proactive expiry check: the 401-retry path handles all expired-token cases in one extra round-trip, which is cheaper than the code and tests needed to predict expiry.

Refresh is automatic and unconditional for the supabase.json source. Tokens supplied via CLI flag, environment variable, or config file are not refreshed (they have no refresh_token).

## Pinned Constants

```rust
const GRANOLA_AUTH_DOMAIN: &str = "https://auth.granola.ai";
const GRANOLA_CLIENT_ID: &str = "client_01JZJ0XBDAT8PHJWQY09Y0VD61";
```

Both are non-secret WorkOS tenant identifiers extracted from the JWT we observed in supabase.json. The auth domain is pinned (rather than read from the JWT `iss` field) for SSRF safety: a tampered supabase.json cannot redirect baez's refresh_token POST to an attacker-controlled host. The client_id is similarly pinned. Rotation of either would require a baez code update — Granola desktop would force a re-auth at the same time, giving the user an obvious signal.

JWT decoding is deliberately avoided. We read `obtained_at` and `expires_in` from supabase.json's `workos_tokens` JSON when needed, which means no `base64` dependency and no JWT parsing surface.

## Credentials Type

Lives in `src/auth.rs`. Replaces the `String` return type of `resolve_token()`.

```rust
pub enum Credentials {
    Static(String),
    Refreshable(RefCell<RefreshableState>),
}

pub struct RefreshableState {
    pub access_token: String,
    pub refresh_token: String,
    pub supabase_path: PathBuf,
    pub http_client: reqwest::blocking::Client,
}

impl Credentials {
    pub fn access_token(&self) -> String;
    pub fn refresh_with_cas(&self) -> Result<bool>;  // false for Static
}
```

`access_token()` returns the current token without checking expiry or making any network calls. Refresh happens only via `refresh_with_cas()`, which is invoked from `ApiClient` on 401 responses.

Interior mutability via `RefCell` is appropriate here: baez is single-threaded and blocking. The `RefCell::borrow_mut` panic-on-conflict serves as a tripwire if concurrency is ever introduced accidentally.

## resolve_credentials

Replaces `resolve_token` with the same chain order:

1. CLI `--token` flag → `Credentials::Static`
2. `BAEZ_GRANOLA_TOKEN` env var → `Credentials::Static`
3. `BEARER_TOKEN` env var (deprecated) → `Credentials::Static`
4. Config file `granola_token` → `Credentials::Static`
5. supabase.json (parse `workos_tokens`) → `Credentials::Refreshable`

The supabase.json branch parses the nested stringified-JSON `workos_tokens` field and constructs a `RefreshableState` with the current `access_token`, `refresh_token`, and the file's path (for write-back).

If parsing supabase.json fails, the chain returns an `Error::Auth` with the underlying cause — same as today.

## CAS Refresh

`refresh_with_cas` implements compare-and-swap against supabase.json, then refreshes if no other process beat us to it.

```
1. For Static credentials → return Ok(false). No-op.
2. For Refreshable:
   a. Re-read supabase.json's workos_tokens.refresh_token.
   b. If it differs from our in-memory refresh_token, another process
      (or Granola desktop) refreshed. Adopt the new tokens from the file,
      update in-memory state, return Ok(true). No HTTP call.
   c. Otherwise, POST to {GRANOLA_AUTH_DOMAIN}/user_management/authenticate:
        { "grant_type": "refresh_token",
          "client_id": GRANOLA_CLIENT_ID,
          "refresh_token": current_refresh_token }
      Retries via util::retry_with_backoff with a refresh-specific
      is_retryable_refresh() predicate defined in auth.rs (network errors,
      5xx are retryable; 4xx are terminal). Distinct from api.rs's
      is_retryable() because the retryability rules for the WorkOS endpoint
      differ — e.g. 422 here is a hard rejection of the refresh_token, not
      a transient issue worth retrying.
   d. Parse response. If response omits refresh_token, reuse the current one.
   e. Write supabase.json back, preserving session_id, user_info, and any
      unknown fields inside workos_tokens. Use storage::write_atomic with
      the file's existing mode.
   f. If the supabase.json write fails, emit a warning and continue —
      the next baez run will retry the write. In-memory state still updates.
   g. Update in-memory state, return Ok(true).
```

Any unrecoverable refresh failure (WorkOS 400/401, malformed response, network failure after retries) returns `Error::Auth(format!("refresh failed: {reason}"))`. The user is expected to open the Granola desktop app to re-authenticate.

## ApiClient Integration

`ApiClient` swaps its `token: String` field for `credentials: Credentials`. Each request method gains a 401-retry wrapper:

```rust
let response = send_with(self.credentials.access_token())?;
if response.status() == 401 {
    if self.credentials.refresh_with_cas()? {
        return send_with(self.credentials.access_token());
    }
}
Ok(response)
```

Single retry attempt — a second 401 propagates as `Error::Api` and exits with code 4. Existing `util::retry_with_backoff` for network-level retries is unchanged and orthogonal to the refresh path.

For `Credentials::Static`, `refresh_with_cas` returns `Ok(false)`, the retry doesn't fire, and behavior matches the current implementation exactly.

## supabase.json Write-Back

The file structure must be preserved:

```json
{
  "workos_tokens": "<stringified JSON>",
  "session_id": "...",
  "user_info": "<stringified JSON>"
}
```

The `workos_tokens` payload contains `access_token`, `refresh_token`, `expires_in`, `token_type`, `obtained_at`, `session_id`, `external_id`, `sign_in_method`. On refresh we update only `access_token`, `refresh_token`, `expires_in`, and `obtained_at` — other fields are preserved as-is. Top-level `session_id` and `user_info` are untouched.

Implementation: parse the whole file as `serde_json::Value`, mutate the specific paths via JSON pointer-style access, re-serialize, write atomically.

The existing `storage::write_atomic` writes with a hardcoded `0o600` mode, which is wrong for supabase.json — Granola may rely on its original mode (typically `0o644`). Add a sibling helper `storage::write_atomic_preserve_mode(path, content, tmp_dir)` that stats the existing file, captures its mode, and re-applies it after the rename. The new baez code uses this for supabase.json; the existing `write_atomic` continues to be used elsewhere unchanged.

## Error Handling

| Failure | Behavior |
|---|---|
| supabase.json missing on cold start | `Error::Auth("No Granola token found...")` — unchanged from today |
| supabase.json malformed | `Error::Auth("Failed to parse Granola session file: <reason>. Try re-logging in via the Granola desktop app.")` |
| WorkOS refresh: 400/401 (refresh_token rejected) | `Error::Auth("refresh failed: <status> <reason>. Open the Granola desktop app to re-authenticate.")` Exit 2. |
| WorkOS refresh: network/5xx | `util::retry_with_backoff` (2 retries, 2s+4s). Final failure → `Error::Auth("refresh failed: <reason>")`. |
| supabase.json write fails | eprintln warning, refresh otherwise succeeds, in-memory state updated. Next run retries. |
| Granola API 401 after one refresh+retry | Propagate as `Error::Api` (the original 401) |
| Granola API 401 with `Credentials::Static` | Propagate as `Error::Api` — no refresh possible |

The `is_retryable()` predicate for refresh requests treats network errors and 5xx as retryable; 400/401/422 as terminal.

## Verbose Telemetry

`--verbose` prints to stderr when refresh activity happens:

```
[verbose] Granola access token rejected (401), attempting refresh...
[verbose] Adopted refreshed tokens from supabase.json (another process refreshed)
[verbose] Refresh succeeded, retrying request
[verbose] Refresh failed: <reason>
```

No token values are ever logged.

## Files to Modify

- **`src/auth.rs`** — Add `Credentials` enum, `RefreshableState` struct, `refresh_with_cas()`, supabase.json write-back helper, `is_retryable_refresh()` predicate for WorkOS, pinned constants. Replace `resolve_token` with `resolve_credentials`.
- **`src/storage.rs`** — Add `write_atomic_preserve_mode()` helper that stats the existing file before write to preserve its mode through the temp+rename. Used only for supabase.json.
- **`src/api.rs`** — Change `ApiClient.token: String` to `ApiClient.credentials: Credentials`. Wrap `post_with_raw` and `get` with the 401-retry pattern.
- **`src/main.rs`** — Call `resolve_credentials` instead of `resolve_token`. One-line change.
- **`src/lib.rs`** — Update re-exports: `pub use auth::{resolve_credentials, Credentials, RefreshableState};`
- **`tests/api_integration.rs`** — Migrate `ApiClient::new("test-token".into(), ...)` call sites to `ApiClient::new(Credentials::Static("test-token".into()), ...)`. Mechanical.

No new dependencies. No new feature flag. No changes to `Cargo.toml`.

## Testing

### Unit tests (inline in `src/auth.rs`)

- `resolve_credentials` returns `Static` from each of: CLI flag, BAEZ_GRANOLA_TOKEN, config file
- `resolve_credentials` returns `Refreshable` populated from supabase.json
- `resolve_credentials` returns `Error::Auth` when no source
- `refresh_with_cas` for `Static` returns `Ok(false)` without I/O
- `refresh_with_cas` CAS-hit: file `refresh_token` differs from in-memory → adopt without HTTP
- `refresh_with_cas` CAS-miss path: WorkOS call → state update → file write
- `refresh_with_cas` WorkOS response missing `refresh_token` → current refresh_token reused
- `refresh_with_cas` supabase.json write failure → warning emitted, in-memory still updated, returns `Ok(true)`
- `refresh_with_cas` WorkOS 400/401 → `Error::Auth` with actionable message
- `refresh_with_cas` WorkOS 500 → retry → eventual `Error::Auth`
- supabase.json write-back preserves `session_id`, `user_info`, and unknown fields inside `workos_tokens`
- supabase.json write-back preserves existing file mode

### Unit tests (inline in `src/api.rs`)

- `post_with_raw` 401 with `Refreshable` triggers refresh and retries once
- `post_with_raw` 401 with `Static` does not retry
- Non-401 errors do not invoke refresh
- Second 401 after refresh propagates as `Error::Api` (no infinite loop)

### Integration tests (new `tests/auth_refresh_integration.rs`)

Uses the existing wiremock + `spawn_blocking` pattern from `tests/api_integration.rs`.

- End-to-end happy path: Granola returns 401, WorkOS returns valid refresh response, retry succeeds, supabase.json updated
- CAS adoption: pre-populate supabase.json with newer `refresh_token` between requests, verify no WorkOS call is made
- Refresh rejected: WorkOS returns 400 → `Error::Auth` with re-login message
- WorkOS unreachable: 500 three times → retries → final `Error::Auth`

### Coverage tests deliberately skipped

- Multi-process true concurrency. CAS handles the deterministic interleavings we can simulate. A genuine simultaneous-refresh race is rare for a personal CLI; the worst case is one wasted refresh that the second process recovers from via its own CAS check.
- Macos keychain interaction. Not relevant here.
- JWT decoding. We don't decode JWTs.

## Edge Cases

- **First refresh after `BEARER_TOKEN` deprecation.** Unaffected. `BEARER_TOKEN` resolves to `Static` and never refreshes, same as today.
- **Vault is on a different machine than supabase.json.** Supabase path is hardcoded to `~/Library/Application Support/Granola/supabase.json`. If the user's setup deviates (network home directory, sandbox), the cold-start parse will fail and we surface the existing auth chain error.
- **Granola rotates `client_id` or auth domain.** Refresh will start failing with 400/404. User sees `Error::Auth("refresh failed: ...")`. Fix is a one-line update to the pinned constant and a re-release.
- **WorkOS introduces a breaking change to the refresh response shape.** Response parsing fails, surfaces as `Error::Auth("refresh failed: response parse: <reason>")`. Diagnosable with `--verbose`.
- **User deletes supabase.json mid-sync.** The next CAS read fails; surfaces as `Error::Auth`. Sync aborts with an actionable error.

## Out of Scope

- Triggering refresh proactively before requests. Not worth the code; reactive handles every case for ~200ms extra latency.
- Refresh for non-session-file token sources. Those tokens have no refresh_token; the user supplied them deliberately.
- Storing refresh_token in macOS keychain. The hybrid write-back to supabase.json (plaintext) means moving one copy to keychain doesn't raise the bar.
- A new `baez auth refresh` or `baez auth status` command. Not necessary; refresh is transparent.
- An `--no-refresh` flag. YAGNI; can be added if a real need emerges.
