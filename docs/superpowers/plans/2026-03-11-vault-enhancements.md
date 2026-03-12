# Vault Enhancements Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add four Obsidian integration features to baez: backlink-aware updates, Dataview inline fields in summaries, daily notes linking, and watch mode.

**Architecture:** Each feature is a targeted addition to the existing sync pipeline. No new commands or modules — just new functions in `storage.rs`, a prompt update in `summary.rs`, CLI flags in `cli.rs`, and loop logic in `main.rs`.

**Tech Stack:** Rust 1.86, clap 4.5, chrono 0.4, ctrlc 3.4 (new dep). Tests use tempfile, insta, assert_fs.

**Spec:** `docs/superpowers/specs/2026-03-11-vault-enhancements-design.md`

---

## File Structure

| File | Changes |
|------|---------|
| `src/storage.rs` | Add `write_with_user_content()`, `update_daily_note()`, `format_meeting_entry()` |
| `src/convert.rs:181` | Append `<!-- baez-managed-above -->` marker to body in `to_markdown()` |
| `src/lib.rs:27` | Re-export `write_with_user_content` |
| `src/sync.rs` | Add `SyncStats` return type; call `write_with_user_content()` instead of `write_atomic()`; call `update_daily_note()` after each doc |
| `src/main.rs:91` | Call `write_with_user_content()` in Fetch handler; add watch loop in Sync handler |
| `src/summary.rs:11-37` | Replace `DEFAULT_SUMMARY_PROMPT` constant |
| `src/cli.rs:60-70` | Add `--watch`, `--interval` flags to `Commands::Sync` |
| `Cargo.toml:29` | Add `ctrlc = "3.4"` dependency |

---

## Chunk 1: Backlink-Aware Updates

### Task 1: Add `write_with_user_content()` with tests

**Files:**
- Modify: `src/storage.rs` (add function after `write_atomic` at line 228)

- [ ] **Step 1: Write the failing tests**

Add a new test module `user_content_tests` at the end of `src/storage.rs`, before the closing of the file:

```rust
#[cfg(test)]
mod user_content_tests {
    use super::*;
    use tempfile::TempDir;

    fn marker() -> &'static str {
        CONTENT_MARKER
    }

    #[test]
    fn test_new_file_written_directly() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("new.md");
        let content = format!("# Hello\n{}\n", marker());
        write_with_user_content(&target, content.as_bytes(), &paths.tmp_dir).unwrap();

        assert_eq!(fs::read_to_string(&target).unwrap(), content);
    }

    #[test]
    fn test_preserves_user_content_below_marker() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("existing.md");

        // Write initial content with marker + user content
        let initial = format!("# Old Title\n{}\n\n## My Notes\nUser stuff here\n", marker());
        fs::write(&target, &initial).unwrap();

        // Re-sync with new generated content
        let new_generated = format!("# New Title\n{}\n", marker());
        write_with_user_content(&target, new_generated.as_bytes(), &paths.tmp_dir).unwrap();

        let result = fs::read_to_string(&target).unwrap();
        assert!(result.starts_with("# New Title\n"));
        assert!(result.contains(marker()));
        assert!(result.contains("## My Notes\nUser stuff here\n"));
    }

    #[test]
    fn test_legacy_file_without_marker_overwritten() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("legacy.md");
        fs::write(&target, "# Old content without marker\n").unwrap();

        let new_content = format!("# New content\n{}\n", marker());
        write_with_user_content(&target, new_content.as_bytes(), &paths.tmp_dir).unwrap();

        let result = fs::read_to_string(&target).unwrap();
        assert_eq!(result, new_content);
    }

    #[test]
    fn test_multiple_markers_splits_on_first() {
        let temp = TempDir::new().unwrap();
        let paths = Paths::new(Some(temp.path().to_path_buf())).unwrap();
        paths.ensure_dirs().unwrap();

        let target = temp.path().join("multi.md");
        let initial = format!(
            "# Title\n{}\nUser note\n{}\nMore user stuff\n",
            marker(), marker()
        );
        fs::write(&target, &initial).unwrap();

        let new_generated = format!("# Updated\n{}\n", marker());
        write_with_user_content(&target, new_generated.as_bytes(), &paths.tmp_dir).unwrap();

        let result = fs::read_to_string(&target).unwrap();
        assert!(result.starts_with("# Updated\n"));
        assert!(result.contains("User note\n"));
        assert!(result.contains("More user stuff\n"));
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib user_content_tests -- --nocapture`
Expected: FAIL — `write_with_user_content` not found

- [ ] **Step 3: Implement `write_with_user_content()`**

Add this function in `src/storage.rs` after `write_atomic()` (after line 228):

```rust
/// Marker that separates baez-generated content from user-added content.
pub const CONTENT_MARKER: &str = "<!-- baez-managed-above -->";

/// Write generated markdown to `path`, preserving any user content below the marker.
///
/// - New file: writes `generated_content` directly
/// - Existing file with marker: replaces everything above (and including) the marker,
///   preserves everything below
/// - Existing file without marker: overwrites entirely (legacy file)
pub fn write_with_user_content(path: &Path, generated_content: &[u8], tmp_dir: &Path) -> Result<()> {
    if path.exists() {
        let existing = fs::read_to_string(path)?;
        if let Some(marker_pos) = existing.find(CONTENT_MARKER) {
            // Split after the marker line, skipping the trailing newline
            // to prevent blank line accumulation on repeated re-syncs
            let mut after_marker = marker_pos + CONTENT_MARKER.len();
            if existing.as_bytes().get(after_marker) == Some(&b'\n') {
                after_marker += 1;
            }
            let user_content = &existing[after_marker..];

            let mut merged = Vec::new();
            merged.extend_from_slice(generated_content);
            merged.extend_from_slice(user_content.as_bytes());

            return write_atomic(path, &merged, tmp_dir);
        }
    }
    // New file or legacy file without marker
    write_atomic(path, generated_content, tmp_dir)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib user_content_tests -- --nocapture`
Expected: All 4 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/storage.rs
git commit -m "feat: add write_with_user_content() for preserving user content below marker"
```

### Task 2: Append marker in `to_markdown()` and update snapshot

**Files:**
- Modify: `src/convert.rs:179-184` (append marker before returning)
- Modify: `src/snapshots/baez__convert__snapshot_tests__markdown_output_snapshot.snap` (updated snapshot)

- [ ] **Step 1: Modify `to_markdown()` to append the marker**

In `src/convert.rs`, change the end of `to_markdown()` (around line 179) from:

```rust
    Ok(MarkdownOutput {
        frontmatter_yaml,
        body,
    })
```

to:

```rust
    // Append content marker for backlink-aware updates
    body.push_str("\n<!-- baez-managed-above -->\n");

    Ok(MarkdownOutput {
        frontmatter_yaml,
        body,
    })
```

- [ ] **Step 2: Run tests to see what breaks**

Run: `cargo test --lib`
Expected: Snapshot test fails (output now includes the marker). Other convert tests may need assertion updates if they check exact body content.

- [ ] **Step 3: Update snapshots and fix any broken assertions**

Run: `cargo insta review` to accept the updated snapshot.

No other tests in `convert.rs` perform exact-match assertions on the full body content — they use substring assertions (`contains`, `starts_with`) which are unaffected by the appended marker. Only the insta snapshot test needs updating.

- [ ] **Step 4: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/convert.rs src/snapshots/
git commit -m "feat: append content marker to markdown output for user content preservation"
```

### Task 3: Wire up callers and re-export

**Files:**
- Modify: `src/lib.rs:27` (add re-export)
- Modify: `src/sync.rs:239` (change write call)
- Modify: `src/main.rs:91` (change write call)

- [ ] **Step 1: Add re-export in `lib.rs`**

In `src/lib.rs` line 27, change:

```rust
pub use storage::{read_frontmatter, write_atomic, Paths};
```

to:

```rust
pub use storage::{read_frontmatter, write_atomic, write_with_user_content, Paths};
```

- [ ] **Step 2: Update `sync.rs` to use `write_with_user_content` for markdown**

In `src/sync.rs` line 239, change:

```rust
        write_atomic(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;
```

to:

```rust
        write_with_user_content(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;
```

Also update the import in `src/sync.rs` line 8. Change:

```rust
    storage::{read_frontmatter, set_file_time, write_atomic, Paths},
```

to:

```rust
    storage::{read_frontmatter, set_file_time, write_atomic, write_with_user_content, Paths},
```

- [ ] **Step 3: Update `main.rs` Fetch handler**

In `src/main.rs` line 91, change:

```rust
            baez::storage::write_atomic(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;
```

to:

```rust
            baez::storage::write_with_user_content(&doc_path, full_md.as_bytes(), &paths.tmp_dir)?;
```

- [ ] **Step 4: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 5: Run linter**

Run: `cargo clippy --all-features -- -D warnings && cargo clippy --no-default-features -- -D warnings`
Expected: No warnings

- [ ] **Step 6: Commit**

```bash
git add src/lib.rs src/sync.rs src/main.rs
git commit -m "feat: wire up write_with_user_content in sync and fetch paths"
```

---

## Chunk 2: Dataview Inline Fields in Summaries

### Task 4: Update the summary prompt

**Files:**
- Modify: `src/summary.rs:11-37` (replace `DEFAULT_SUMMARY_PROMPT`)

- [ ] **Step 1: Replace `DEFAULT_SUMMARY_PROMPT`**

In `src/summary.rs`, replace lines 11-37 (the entire `DEFAULT_SUMMARY_PROMPT` constant) with:

```rust
const DEFAULT_SUMMARY_PROMPT: &str = r#"You are an expert meeting summarizer producing Obsidian-optimized markdown.

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
- Use Dataview inline field syntax [field:: value] exactly as shown in the examples above."#;
```

- [ ] **Step 2: Run tests**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS. The existing `test_summary_prompt_format` (line 451) asserts on "Summary", "Action Items", "Key Decisions", "Open Questions", "[[wiki-links]]" — all of which are present in the new prompt.

- [ ] **Step 3: Run linter**

Run: `cargo clippy --all-features -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Commit**

```bash
git add src/summary.rs
git commit -m "feat: add Dataview inline fields to summary prompt (decision, action, owner)"
```

---

## Chunk 3: Daily Notes Linking

### Task 5: Add `update_daily_note()` with tests

**Files:**
- Modify: `src/storage.rs` (add function and test module)

- [ ] **Step 1: Write the failing tests**

Add a new test module `daily_note_tests` at the end of `src/storage.rs`:

```rust
#[cfg(test)]
mod daily_note_tests {
    use super::*;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn make_vault(temp: &TempDir) -> PathBuf {
        let vault = temp.path().to_path_buf();
        let daily_dir = vault.join("Daily Notes");
        fs::create_dir_all(&daily_dir).unwrap();
        vault
    }

    #[test]
    fn test_creates_daily_note_when_none_exists() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let created = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();

        update_daily_note(&vault, &created, "standup", Some("Sprint Standup"), Some(30), &["Alice".into(), "Bob".into()]).unwrap();

        let daily_path = vault.join("Daily Notes").join("2025-01-15.md");
        assert!(daily_path.exists());
        let content = fs::read_to_string(&daily_path).unwrap();
        assert!(content.contains("## Meetings"));
        assert!(content.contains("[[2025-01-15_standup|Sprint Standup]]"));
        assert!(content.contains("30m"));
        assert!(content.contains("[[Alice]]"));
        assert!(content.contains("[[Bob]]"));
    }

    #[test]
    fn test_appends_section_to_existing_note_without_meetings() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let daily_path = vault.join("Daily Notes").join("2025-01-15.md");
        fs::write(&daily_path, "# Daily Note\n\nSome existing content.\n").unwrap();

        let created = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        update_daily_note(&vault, &created, "standup", Some("Standup"), Some(15), &[]).unwrap();

        let content = fs::read_to_string(&daily_path).unwrap();
        assert!(content.starts_with("# Daily Note\n\nSome existing content.\n"));
        assert!(content.contains("## Meetings"));
        assert!(content.contains("[[2025-01-15_standup|Standup]]"));
    }

    #[test]
    fn test_appends_new_meeting_to_existing_section() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let daily_path = vault.join("Daily Notes").join("2025-01-15.md");
        fs::write(&daily_path, "# Daily\n\n## Meetings\n\n- [[2025-01-15_standup|Standup]] (15m)\n").unwrap();

        let created = Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap();
        update_daily_note(&vault, &created, "planning", Some("Planning"), Some(60), &["Carol".into()]).unwrap();

        let content = fs::read_to_string(&daily_path).unwrap();
        assert!(content.contains("[[2025-01-15_standup|Standup]]"));
        assert!(content.contains("[[2025-01-15_planning|Planning]]"));
    }

    #[test]
    fn test_updates_existing_meeting_link() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let daily_path = vault.join("Daily Notes").join("2025-01-15.md");
        fs::write(&daily_path, "## Meetings\n\n- [[2025-01-15_standup|Old Title]] (15m)\n").unwrap();

        let created = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        update_daily_note(&vault, &created, "standup", Some("New Title"), Some(30), &["Alice".into()]).unwrap();

        let content = fs::read_to_string(&daily_path).unwrap();
        assert!(!content.contains("Old Title"));
        assert!(content.contains("[[2025-01-15_standup|New Title]]"));
        assert!(content.contains("30m"));
    }

    #[test]
    fn test_skips_when_daily_notes_dir_missing() {
        let temp = TempDir::new().unwrap();
        let vault = temp.path().to_path_buf();
        // Do NOT create Daily Notes/ directory

        let created = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();
        // Should return Ok(()) silently
        update_daily_note(&vault, &created, "standup", Some("Standup"), Some(30), &[]).unwrap();

        assert!(!vault.join("Daily Notes").exists());
    }

    #[test]
    fn test_meeting_with_no_title_no_duration_no_attendees() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let created = Utc.with_ymd_and_hms(2025, 1, 15, 10, 0, 0).unwrap();

        update_daily_note(&vault, &created, "untitled", None, None, &[]).unwrap();

        let content = fs::read_to_string(vault.join("Daily Notes").join("2025-01-15.md")).unwrap();
        assert!(content.contains("[[2025-01-15_untitled|Untitled Meeting]]"));
        // No parenthetical when no duration and no attendees
        assert!(!content.contains("("));
    }

    #[test]
    fn test_section_boundary_with_content_after_meetings() {
        let temp = TempDir::new().unwrap();
        let vault = make_vault(&temp);
        let daily_path = vault.join("Daily Notes").join("2025-01-15.md");
        fs::write(&daily_path, "## Meetings\n\n- [[2025-01-15_standup|Standup]] (15m)\n\n## Tasks\n\n- Do something\n").unwrap();

        let created = Utc.with_ymd_and_hms(2025, 1, 15, 14, 0, 0).unwrap();
        update_daily_note(&vault, &created, "planning", Some("Planning"), Some(60), &[]).unwrap();

        let content = fs::read_to_string(&daily_path).unwrap();
        // New meeting should be in the Meetings section, not after Tasks
        let meetings_pos = content.find("## Meetings").unwrap();
        let tasks_pos = content.find("## Tasks").unwrap();
        let planning_pos = content.find("[[2025-01-15_planning|Planning]]").unwrap();
        assert!(planning_pos > meetings_pos);
        assert!(planning_pos < tasks_pos);
    }
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib daily_note_tests -- --nocapture`
Expected: FAIL — `update_daily_note` not found

- [ ] **Step 3: Implement `format_meeting_entry()` and `update_daily_note()`**

Add these functions in `src/storage.rs` after `write_with_user_content()`:

```rust
/// Format a single meeting entry line for a daily note.
///
/// Format: `- [[YYYY-MM-DD_slug|Title]] (Xm, [[Alice]], [[Bob]])`
fn format_meeting_entry(
    slug: &str,
    title: Option<&str>,
    duration_minutes: Option<u64>,
    attendees: &[String],
    local_date: &str,
) -> String {
    let display_title = title.unwrap_or("Untitled Meeting");
    let filename_stem = format!("{}_{}", local_date, slug);
    let link = format!("[[{}|{}]]", filename_stem, display_title);

    let mut parts = Vec::new();
    if let Some(mins) = duration_minutes {
        parts.push(format!("{}m", mins));
    }
    for name in attendees {
        parts.push(format!("[[{}]]", name));
    }

    if parts.is_empty() {
        format!("- {}", link)
    } else {
        format!("- {} ({})", link, parts.join(", "))
    }
}

/// Update the daily note for the given meeting date.
///
/// Appends or updates a meeting link in the `## Meetings` section of
/// `{vault_dir}/Daily Notes/YYYY-MM-DD.md`. Only runs if the
/// `Daily Notes/` directory already exists (opt-in guard).
///
/// Errors are non-fatal: returns `Ok(())` on I/O failure after printing a warning.
pub fn update_daily_note(
    vault_dir: &Path,
    created_at: &DateTime<Utc>,
    slug: &str,
    title: Option<&str>,
    duration_minutes: Option<u64>,
    attendees: &[String],
) -> Result<()> {
    let daily_dir = vault_dir.join("Daily Notes");
    if !daily_dir.is_dir() {
        return Ok(());
    }

    // Convert UTC to local date for daily note filename
    let local_dt: chrono::DateTime<chrono::Local> = created_at.with_timezone(&chrono::Local);
    let local_date = local_dt.format("%Y-%m-%d").to_string();
    let daily_path = daily_dir.join(format!("{}.md", local_date));

    let filename_stem = format!("{}_{}", local_date, slug);
    let entry = format_meeting_entry(slug, title, duration_minutes, attendees, &local_date);

    let result = (|| -> std::result::Result<(), std::io::Error> {
        if !daily_path.exists() {
            // Create new daily note with just the Meetings section
            let content = format!("## Meetings\n\n{}\n", entry);
            fs::write(&daily_path, content)?;
            return Ok(());
        }

        let content = fs::read_to_string(&daily_path)?;
        let lines: Vec<&str> = content.lines().collect();

        // Find ## Meetings heading
        let meetings_idx = lines.iter().position(|l| l.trim() == "## Meetings");

        if let Some(heading_idx) = meetings_idx {
            // Find section boundary (next ## heading or end)
            let section_end = lines[heading_idx + 1..]
                .iter()
                .position(|l| l.starts_with("## "))
                .map(|p| heading_idx + 1 + p)
                .unwrap_or(lines.len());

            // Check if this meeting is already linked
            let existing_line = lines[heading_idx + 1..section_end]
                .iter()
                .position(|l| l.contains(&format!("[[{}|", filename_stem)) || l.contains(&format!("[[{}]]", filename_stem)))
                .map(|p| heading_idx + 1 + p);

            let mut new_lines: Vec<String> = lines.iter().map(|l| l.to_string()).collect();

            if let Some(line_idx) = existing_line {
                // Replace existing line
                new_lines[line_idx] = entry;
            } else {
                // Insert before section boundary — find last non-blank line in section
                let insert_at = (heading_idx + 1..section_end)
                    .rev()
                    .find(|&i| !lines[i].trim().is_empty())
                    .map(|i| i + 1)
                    .unwrap_or(heading_idx + 1);
                // If the section is empty (just heading), add a blank line first
                if insert_at == heading_idx + 1 {
                    new_lines.insert(insert_at, String::new());
                    new_lines.insert(insert_at + 1, entry);
                } else {
                    new_lines.insert(insert_at, entry);
                }
            }

            let mut result = new_lines.join("\n");
            if content.ends_with('\n') && !result.ends_with('\n') {
                result.push('\n');
            }
            fs::write(&daily_path, result)?;
        } else {
            // No ## Meetings section — append one at the end
            let mut new_content = content;
            if !new_content.ends_with('\n') {
                new_content.push('\n');
            }
            new_content.push_str(&format!("\n## Meetings\n\n{}\n", entry));
            fs::write(&daily_path, new_content)?;
        }

        Ok(())
    })();

    if let Err(e) = result {
        eprintln!(
            "Warning: Failed to update daily note {}: {}",
            daily_path.display(),
            e
        );
    }

    Ok(())
}
```

The implementation uses fully-qualified `chrono::Local` and `chrono::DateTime<chrono::Local>`, so no new imports are needed in `storage.rs` (the existing `use chrono::{DateTime, Datelike, Utc};` suffices).

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib daily_note_tests -- --nocapture`
Expected: All 7 tests PASS

- [ ] **Step 5: Commit**

```bash
git add src/storage.rs
git commit -m "feat: add update_daily_note() for Obsidian daily notes linking"
```

### Task 6: Wire up daily notes in sync loop

**Files:**
- Modify: `src/sync.rs` (call `update_daily_note` after writing each doc)

- [ ] **Step 1: Add `update_daily_note` call in `sync_all()`**

In `src/sync.rs`, after the `set_file_time` calls and before the cache update (around line 250, after `set_file_time(&doc_path, &meta.created_at)?;`), add:

```rust
        // Update daily note with meeting link (non-fatal, skips if Daily Notes/ missing)
        if !dry_run {
            let attendee_names: Vec<String> = if let Some(ref rich) = meta.attendees {
                rich.iter().filter_map(|a| a.name.clone()).collect()
            } else {
                meta.participants.clone()
            };
            crate::storage::update_daily_note(
                &paths.vault_dir,
                &meta.created_at,
                &slug,
                meta.title.as_deref(),
                meta.duration_seconds.map(|s| s / 60),
                &attendee_names,
            )?;
        }
```

Note: The `dry_run` guard at line 140 already uses `continue` to skip the rest of the loop body, so this code is unreachable during dry-run. The explicit `!dry_run` check is a safety net for clarity.

- [ ] **Step 2: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 3: Commit**

```bash
git add src/sync.rs
git commit -m "feat: wire up daily notes linking in sync loop"
```

---

## Chunk 4: Watch Mode

### Task 7: Add `ctrlc` dependency

**Files:**
- Modify: `Cargo.toml`

- [ ] **Step 1: Add ctrlc to dependencies**

In `Cargo.toml`, add after `filetime = "0.2.26"` (line 29):

```toml
ctrlc = "3.4"
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check`
Expected: Compilation succeeds

- [ ] **Step 3: Commit**

```bash
git add Cargo.toml Cargo.lock
git commit -m "deps: add ctrlc crate for watch mode signal handling"
```

### Task 8: Add `--watch` and `--interval` CLI flags with tests

**Files:**
- Modify: `src/cli.rs:60-70` (add flags to `Commands::Sync`)

- [ ] **Step 1: Add a test for interval validation**

In `src/cli.rs`, find the existing `#[cfg(test)] mod tests` block and add:

```rust
    #[test]
    fn test_interval_must_be_at_least_30() {
        use clap::Parser;
        let result = Cli::try_parse_from(["baez", "sync", "--watch", "--interval", "10"]);
        assert!(result.is_err());
    }

    #[test]
    fn test_watch_defaults() {
        use clap::Parser;
        let cli = Cli::try_parse_from(["baez", "sync", "--watch"]).unwrap();
        if let Some(Commands::Sync { watch, interval, .. }) = cli.command {
            assert!(watch);
            assert_eq!(interval, 300);
        } else {
            panic!("Expected Sync command");
        }
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --lib cli::tests -- --nocapture`
Expected: FAIL — no field `watch` on `Sync`

- [ ] **Step 3: Add flags to `Commands::Sync`**

In `src/cli.rs`, modify the `Sync` variant (lines 60-70) to add the new fields:

```rust
    /// Sync all documents
    Sync {
        /// Force re-sync of all documents, ignoring cache timestamps
        #[arg(long)]
        force: bool,
        /// Skip AI summarization even if API key is configured
        #[arg(long)]
        no_summarize: bool,
        /// Preview what would be synced without writing any files
        #[arg(long)]
        dry_run: bool,
        /// Keep running and re-sync at regular intervals
        #[arg(long)]
        watch: bool,
        /// Polling interval in seconds for watch mode (minimum 30)
        #[arg(long, default_value = "300", value_parser = parse_interval)]
        interval: u64,
    },
```

Also add the interval validator function (near `parse_throttle_range`):

```rust
fn parse_interval(s: &str) -> std::result::Result<u64, String> {
    let val: u64 = s.parse().map_err(|_| format!("'{}' is not a valid number", s))?;
    if val < 30 {
        return Err("Interval must be at least 30 seconds to avoid API rate limiting".into());
    }
    Ok(val)
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --lib cli::tests -- --nocapture`
Expected: All tests PASS (including the new ones)

- [ ] **Step 5: Commit**

```bash
git add src/cli.rs
git commit -m "feat: add --watch and --interval flags to sync command"
```

### Task 9: Add `SyncStats` return type to `sync_all()`

**Files:**
- Modify: `src/sync.rs` (change return type, add struct)

The spec requires watch mode to print sync counts (`Synced 3 new, 47 skipped`). Currently `sync_all()` returns `Result<()>` and prints stats internally. We need it to return counts so the watch loop can format the status line.

- [ ] **Step 1: Add `SyncStats` struct and change return type**

In `src/sync.rs`, add the struct before `sync_all()` (after the `CacheEntry` struct):

```rust
/// Statistics returned by `sync_all()` for reporting.
pub struct SyncStats {
    pub total: usize,
    pub synced: usize,
    pub skipped: usize,
}
```

Then change the `sync_all()` signature from `-> Result<()>` to `-> Result<SyncStats>`.

At the end of `sync_all()`, change the final block from:

```rust
    pb.finish_with_message(stats_msg);

    Ok(())
```

to:

```rust
    pb.finish_with_message(stats_msg);

    Ok(SyncStats {
        total: docs.len(),
        synced,
        skipped,
    })
```

- [ ] **Step 2: Update callers in `main.rs`**

In `src/main.rs`, all calls to `sync_all()` currently use `?` and discard the result. No changes needed — `let _ = sync_all(...)?;` or just `sync_all(...)?;` both work since the `Result<SyncStats>` is consumed by `?` for the error case and the `SyncStats` is simply dropped. The watch loop in the next step will use the return value.

- [ ] **Step 3: Run tests**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 4: Commit**

```bash
git add src/sync.rs
git commit -m "refactor: return SyncStats from sync_all() for watch mode status reporting"
```

### Task 10: Implement watch loop in `main.rs`

**Files:**
- Modify: `src/main.rs` (wrap sync in loop when `--watch`)

- [ ] **Step 1: Update the Sync match arm with watch loop**

In `src/main.rs`, change the `Sync` match arm (around line 33) from:

```rust
        Some(baez::cli::Commands::Sync {
            force,
            no_summarize,
            dry_run,
        }) => {
            let client = create_client(&cli)?;
            let paths = Paths::new(cli.vault)?;
            sync_all(&client, &paths, force, !no_summarize, cli.verbose, dry_run)?;
        }
```

to:

```rust
        Some(baez::cli::Commands::Sync {
            force,
            no_summarize,
            dry_run,
            watch,
            interval,
        }) => {
            let client = create_client(&cli)?;
            let paths = Paths::new(cli.vault)?;

            if watch {
                use std::sync::atomic::{AtomicBool, Ordering};
                use std::sync::Arc;

                let running = Arc::new(AtomicBool::new(true));
                let r = running.clone();
                ctrlc::set_handler(move || {
                    r.store(false, Ordering::SeqCst);
                })
                .expect("Failed to set Ctrl+C handler");

                let mut first = true;
                while running.load(Ordering::SeqCst) {
                    let use_force = force && first;
                    // Subsequent iterations use quiet output (suppress progress bar)
                    let verbose = if first { cli.verbose } else { false };
                    match sync_all(&client, &paths, use_force, !no_summarize, verbose, dry_run) {
                        Ok(stats) => {
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                            println!(
                                "[{}] Synced {} new, {} skipped. Next check in {}s.",
                                now, stats.synced, stats.skipped, interval
                            );
                        }
                        Err(e) => {
                            let now = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
                            eprintln!("[{}] Sync error: [E{}] {}", now, e.exit_code(), e);
                        }
                    }
                    first = false;

                    // Sleep in 1-second increments to check for Ctrl+C
                    for _ in 0..interval {
                        if !running.load(Ordering::SeqCst) {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_secs(1));
                    }
                }
                println!("\nWatch mode stopped.");
            } else {
                sync_all(&client, &paths, force, !no_summarize, cli.verbose, dry_run)?;
            }
        }
```

Note: The `verbose` variable controls whether `sync_all` shows the progress bar and per-document lines. On subsequent iterations `verbose` is `false`, suppressing the progress bar. The one-line status with sync counts is printed by the watch loop itself using the returned `SyncStats`.

- [ ] **Step 2: Run full test suite**

Run: `cargo test --all-features --no-fail-fast`
Expected: All tests PASS

- [ ] **Step 3: Run linter on both feature sets**

Run: `cargo clippy --all-features -- -D warnings && cargo clippy --no-default-features -- -D warnings`
Expected: No warnings

- [ ] **Step 4: Run formatter**

Run: `cargo fmt`

- [ ] **Step 5: Commit**

```bash
git add src/main.rs
git commit -m "feat: add watch mode with configurable polling interval and graceful shutdown"
```

---

## Final Verification

### Task 11: Full CI check

- [ ] **Step 1: Run full CI equivalent**

Run: `just ci`
Expected: Format OK, lint OK, all tests pass

- [ ] **Step 2: Verify both feature sets compile**

Run: `cargo check --all-features && cargo check --no-default-features`
Expected: Both compile without errors
