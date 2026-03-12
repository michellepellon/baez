# Vault Enhancements Design

Four independent features that deepen baez's Obsidian integration: backlink-aware updates, Dataview inline fields in summaries, daily notes linking, and watch mode.

**Approach:** Integrated into the existing sync pipeline (Approach A). Each feature is a targeted addition to the current flow — no new commands, no plugin system, no post-processing pass.

## Feature 1: Backlink-Aware Updates

### Problem
`sync_all()` overwrites meeting files entirely on re-sync. If a user adds their own notes, links, or annotations below the generated content, re-syncing destroys them.

### Design
Introduce a content marker `<!-- baez-managed-above -->` that separates baez-generated content from user-added content.

**Marker placement:** `to_markdown()` in `convert.rs` appends the marker as the last line of the generated body, after the transcript section.

**Write logic:** New function `write_with_user_content(path, generated_content, tmp_dir)` in `storage.rs`:
- If file does not exist: write `generated_content` (which includes the marker) via `write_atomic()`
- If file exists and contains `<!-- baez-managed-above -->`: split at the first occurrence of the marker, replace everything above (including the marker line) with `generated_content`, preserve everything below
- If file exists but has no marker: overwrite entirely (legacy file; the new content will include the marker for future syncs)

The `generated_content` parameter includes the full frontmatter + body + marker. This means frontmatter is always refreshed on re-sync — manually added frontmatter fields (e.g., `status:: reviewed`) will **not** survive a re-sync. This is a known limitation; user-added metadata should go below the marker.

**Caller change:** `sync_all()` in `sync.rs` and the `Fetch` command in `main.rs` call `write_with_user_content()` instead of `write_atomic()` for markdown files only. Raw JSON files continue to use `write_atomic()`. The function must be `pub` and re-exported in `lib.rs`.

**Concurrency note:** The read-split-write cycle is not atomic (only the final rename is). If a user edits the file in Obsidian at the exact moment baez writes, user edits could be lost. This is accepted as low-risk for a single-user CLI tool — Obsidian itself has the same limitation with its own sync.

### Files Modified
- `src/convert.rs` — append marker to `to_markdown()` body output
- `src/storage.rs` — add `pub fn write_with_user_content()`
- `src/lib.rs` — re-export `write_with_user_content`
- `src/sync.rs` — call `write_with_user_content()` for markdown files
- `src/main.rs` — call `write_with_user_content()` in the `Fetch` command handler

### Edge Cases
- User deletes the marker: next sync overwrites entirely (marker is re-added)
- User moves the marker upward: everything above the marker (including any generated content that is now above the new marker position) is replaced with fresh generated content. Everything below the marker is preserved. User content that was moved above the marker is overwritten — this is the expected consequence of moving the marker.
- Multiple markers: split on the first occurrence only

---

## Feature 2: Dataview Inline Fields in Summaries

### Problem
AI-generated summaries produce human-readable markdown but are not queryable by Obsidian's Dataview plugin. Users cannot run queries like "show me all action items assigned to Alice across all meetings."

### Design
Replace `DEFAULT_SUMMARY_PROMPT` in `summary.rs` with an updated version that uses Dataview inline field syntax.

**Changes from existing prompt:**
- `## Key Decisions`: Numbered list → bulleted list with `[decision:: ...]` inline field
- `## Action Items`: `**[[Owner]]**: Task *(due: ..., priority: ...)*` → `[owner:: [[Owner]]] [action:: Task] *(due: ..., priority: ...)*` with `- [ ]` checkbox preserved
- `## Summary`, `## Discussion Highlights`, `## Open Questions`: unchanged (these are narrative sections, not queryable fields)
- `[[wiki-links]]` for person names: preserved in both `owner::` fields and Discussion Highlights

**Complete updated prompt:**
```
You are an expert meeting summarizer producing Obsidian-optimized markdown.

Given the transcript below, produce a structured summary with these sections:

## Summary
3–7 bullet points capturing the meeting's essence.

## Key Decisions
Bulleted list of decisions made, each wrapped in a Dataview inline field:
- [decision:: Approve Q2 budget for infrastructure migration]
- [decision:: Defer mobile app to Q3]
If no decisions were made, write "None."

## Action Items
Bulleted checklist. Each item uses Dataview inline fields for owner and action, with optional due date and priority:
- [ ] [owner:: [[Alice Smith]]] [action:: Deploy staging environment by Friday] *(due: 2025-03-20, priority: high)*
- [ ] [owner:: [[Bob Chen]]] [action:: Update API documentation] *(priority: medium)*
Owner names must be [[wiki-links]]. Due dates and priorities are optional — only include if mentioned.

## Discussion Highlights
Group by topic using ### subheadings. Use [[wiki-links]] for people's names.

## Open Questions
Bulleted list of unresolved items.

Rules:
- Use [[wiki-links]] for all person names (e.g. [[Alice Smith]]).
- Use `- [ ]` checkboxes for action items.
- Use markdown headers (##, ###) for sections.
- Preserve important names, dates, and numbers accurately.
- Only use information from the transcript; label any inferences as "(inferred)".
- Be explicit when something is unclear, missing, or not specified.
- Ignore small talk; focus on substance.
- Use Dataview inline field syntax [field:: value] exactly as shown in the examples above.
```

**Snapshot test impact:** The insta snapshot in `src/snapshots/` tests `to_markdown()` output, not the summary prompt. The snapshot test does not call Claude and does not include a summary section, so it is unaffected. Any tests that assert on `DEFAULT_SUMMARY_PROMPT` content directly will need updating.

**Backward compatibility:** Existing summaries are unaffected. Users re-summarize with `baez summarize <id> --save` or `baez sync --force` to get the new format.

### Files Modified
- `src/summary.rs` — replace `DEFAULT_SUMMARY_PROMPT` constant

---

## Feature 3: Daily Notes Linking

### Problem
Synced meeting files exist in `Granola/YYYY/MM/` but have no connection to the user's daily notes. The Obsidian graph and daily note workflow don't reflect that meetings happened on a given day.

### Design
After writing a meeting file during sync, append a link to the corresponding daily note at `Daily Notes/YYYY-MM-DD.md`.

**Timezone handling:** Daily notes use the **local date** derived from the meeting's `created_at` UTC timestamp converted to the system's local timezone via `chrono::Local`. A meeting at `2025-01-15T23:30:00Z` links to `2025-01-15.md` in UTC-5 but `2025-01-16.md` in UTC+1. This matches Obsidian's daily note behavior, which is inherently local-time. The meeting markdown file path continues to use UTC (existing behavior, unchanged).

**Daily note meeting entry format:**
```markdown
## Meetings

- [[2025-01-15_standup|Sprint Standup]] (30m, [[Alice]], [[Bob]])
```

Uses Obsidian's `[[filename|display title]]` alias syntax. The link resolves by filename stem — Obsidian uses shortest-path matching, and the `YYYY-MM-DD_slug` prefix makes collisions extremely unlikely. Parenthetical shows duration and wiki-linked attendees.

**Opt-in guard:** `update_daily_note()` only runs if the `{vault_dir}/Daily Notes/` directory already exists. If it does not exist, the function returns silently. This avoids creating an unwanted directory for users who don't use daily notes. No `--no-daily-notes` flag is needed — the directory's existence is the opt-in signal.

**Section boundary algorithm:**
1. Read the daily note content
2. Search for a line matching `## Meetings` (exact heading match)
3. If not found → append `\n\n## Meetings\n\n{entry}` at end of file
4. If found → find the section boundary: scan forward from the line after `## Meetings` until the next line starting with `## ` (any level-2 heading) or end of file
5. Within the section, search for an existing line containing `[[{filename_stem}|` or `[[{filename_stem}]]`
6. If found → replace that entire line with the new entry
7. If not found → insert the new entry on the last non-blank line before the section boundary (before the next `## ` heading or EOF)

**Error handling:** Failures in `update_daily_note()` are non-fatal. If the daily note cannot be written (permissions, I/O error), print a warning to stderr and continue the sync. A daily note failure should never abort the sync of meeting files.

**Implementation:** New function `update_daily_note(vault_dir, created_at, slug, title, duration_minutes, attendees)` in `storage.rs`. Called from `sync_all()` after writing each meeting file. Skipped during `--dry-run`.

### Files Modified
- `src/storage.rs` — add `update_daily_note()` function
- `src/sync.rs` — call `update_daily_note()` after writing each meeting file

### Edge Cases
- Meeting with no title: use `"Untitled Meeting"` as display text
- Meeting with no duration: omit the duration from the parenthetical (e.g., `([[Alice]], [[Bob]])`)
- Meeting with no attendees and no duration: omit the parenthetical entirely
- Daily note does not exist and `Daily Notes/` directory exists: create the daily note file
- `Daily Notes/` directory does not exist: skip silently (opt-in guard)
- Re-sync updating a meeting link: match by filename stem (e.g., `2025-01-15_standup`) to find and replace the existing line

---

## Feature 4: Watch Mode

### Problem
Users must manually run `baez sync` to fetch new meetings. There's no way to keep the vault continuously up to date.

### Design
Add `--watch` and `--interval` flags to the `Sync` command. When `--watch` is set, sync runs in a loop.

**CLI:**
```
baez sync --watch                    # Poll every 5 minutes (default)
baez sync --watch --interval 120     # Poll every 120 seconds
```

**Behavior:**
- First iteration runs a normal sync (respecting `--force`, `--no-summarize`, `--dry-run`)
- After each sync, prints a one-line status with timestamp: `[2025-01-15 10:05:00] Synced 3 new, 47 skipped. Next check in 300s.`
- `--force` applies only to the first iteration: the watch loop calls `sync_all(force=true)` on the first pass, then `sync_all(force=false)` for all subsequent passes
- On sync failure: print the error to stderr and continue to the next iteration (transient failures are expected)
- On Ctrl+C (SIGINT): exit cleanly with code 0
- Subsequent iterations use quiet output — suppress the progress bar and per-document lines, only print the one-line status. First iteration uses normal verbose output.

**Signal handling:** Use `std::sync::atomic::AtomicBool` with the `ctrlc` crate to register a SIGINT handler. Check the flag before each iteration and periodically during sleep (sleep in 1-second increments, checking the flag each time, rather than one long sleep).

**New dependency:** `ctrlc = "3.4"` — added unconditionally (not feature-gated). This avoids unsafe `libc::signal` calls and is the standard Rust approach. Adds ~3 transitive dependencies.

**Interval validation:** Minimum interval is 30 seconds. `--interval` values below 30 are rejected by clap with a validation error message: `"Interval must be at least 30 seconds to avoid API rate limiting"`. Default is 300 seconds (5 minutes).

### Files Modified
- `Cargo.toml` — add `ctrlc = "3.4"` dependency
- `src/cli.rs` — add `--watch` (bool) and `--interval` (u64, default 300, min 30) flags to `Commands::Sync`
- `src/main.rs` — wrap `sync_all()` in a loop when `--watch` is set, with sleep, signal handling, and quiet mode

### Edge Cases
- `--watch` without `--interval`: defaults to 300 seconds (5 minutes)
- `--interval 10`: rejected, below 30s minimum
- `--watch --dry-run`: allowed, useful for monitoring
- Auth token expiry during long watch: sync fails with `Error::Auth`, prints warning, retries next interval (user can update token in another terminal; next iteration picks up the new env var or session file)

---

## Implementation Order

These features are independent and should be built in this order for logical progression:

1. **Backlink-aware updates** — establishes the marker and `write_with_user_content()` foundation, changes how all markdown files are written
2. **Dataview inline fields** — prompt-only change, fully independent, quick to implement
3. **Daily notes linking** — adds new file I/O to the sync loop, builds on the now-stable write path
4. **Watch mode** — wraps the sync loop, adds a dependency, tests all features end-to-end

## Testing Strategy

### Backlink-aware updates
- Unit tests for `write_with_user_content()` covering: new file (no existing), file with marker (user content preserved), legacy file without marker (full overwrite), file with multiple markers (first occurrence used)
- Integration test: write a file, append user content below the marker, re-sync, verify user content is intact

### Dataview inline fields
- Verify `DEFAULT_SUMMARY_PROMPT` contains the Dataview field syntax rules and examples
- Update any tests that assert on prompt content
- Manual verification that Dataview queries (`TABLE decision, action, owner FROM "Granola"`) work in Obsidian

### Daily notes linking
- Unit tests for `update_daily_note()` covering all append rules: no daily note exists, daily note exists without `## Meetings`, section exists and meeting is new, section exists and meeting is already linked (update), section has content after it
- Test the opt-in guard: function returns silently when `Daily Notes/` directory is absent
- Test non-fatal error handling: I/O error does not propagate
- Integration test with `tempfile::TempDir`

### Watch mode
- Test that `--interval 10` is rejected by clap validation
- Test that `--force` is true on first iteration, false on subsequent (extract the loop logic into a testable function that yields the `force` parameter per iteration)
- Test that sync errors do not abort the loop (mock a failing sync, verify the loop continues)
- Manual testing for actual polling and Ctrl+C behavior
