# Summarization Cache

## Problem

The sync cache (`.baez/.sync_cache.json`) tracks which documents have been synced from the Granola API, but does not track whether each document has been summarized. When a sync is interrupted mid-summarization or summarization is enabled after initial sync, there is no way to resume — `baez sync` skips all cached docs, and `--force` re-syncs everything from the API unnecessarily.

Of 145 synced documents, only 59 have been summarized. The remaining 86 cannot be reached without re-syncing.

## Solution

Add a separate summarization cache and a standalone `baez summarize-all` command. The summarization cache tracks which documents have been summarized and with which model. The standalone command processes unsummarized docs from local raw JSON files without hitting the Granola API.

All new code is gated behind `#[cfg(feature = "summaries")]`, consistent with existing summarization code throughout the codebase.

## Summary Cache

**File:** `.baez/.summary_cache.json`

```rust
#[cfg(feature = "summaries")]
struct SummaryCacheEntry {
    summarized_at: DateTime<Utc>,
    model: String,              // model used, e.g. "claude-sonnet-4-20250514"
}
// Stored as HashMap<doc_id, SummaryCacheEntry>
```

The `filename` needed to locate raw JSON files on disk comes from the **sync cache** entry, not the summary cache. This avoids duplicating that data.

Saved incrementally after each successful summarization, same atomic-write pattern as the sync cache.

## Sync Loop Changes

The sync loop in `sync_all()` gains a second cache check. When the sync cache says a doc is unchanged:

1. Check the summary cache — if the doc is already summarized, skip entirely (current behavior).
2. If NOT summarized and summarization is enabled, enter a "summarize-only" path:
   - Look up the `filename` from the **sync cache** entry
   - Load transcript from `.baez/raw/{filename}_transcript.json`
   - Load metadata from `.baez/raw/{filename}_metadata.json`
   - If raw files are missing/unreadable, log a warning and skip (don't abort the batch)
   - Summarize via Claude API
   - Run entity reconciliation (People/Concepts/Projects)
   - Update the existing markdown file using `summary::update_summary_in_markdown()`
   - Update frontmatter `related` field (merge new wiki-links, preserving existing ones)
   - Update summary cache

When a doc IS being synced (new or updated):
- Sync as today (fetch from API, write files, update sync cache)
- If summarization enabled and transcript is substantive, summarize and update summary cache

The "summarize-only" path never hits the Granola API — it reads exclusively from local files.

A new `summarize_only` counter is added to the sync stats output alongside the existing `summarized` counter.

## Standalone `baez summarize-all` Command

```
baez summarize-all [--force] [--dry-run]
```

Uses `summarize-all` to avoid ambiguity with the existing `baez summarize <doc_id>` single-doc command. Clap's derive API cannot cleanly disambiguate optional positional args from flags.

1. Load both sync cache and summary cache.
2. Iterate sync cache entries (inventory of all known docs, using sync cache `filename` to locate raw files).
3. For each entry not in summary cache (or `--force`):
   - Load raw transcript + metadata JSON from disk
   - If raw files are missing/unreadable, log a warning and skip
   - Skip stubs (< 20 words, same threshold as sync)
   - Summarize via Claude API
   - Update existing markdown file using `summary::update_summary_in_markdown()`
   - Update frontmatter `related` field (merge, not replace)
   - Run entity reconciliation
   - Update summary cache incrementally
4. Progress bar + stats: `summarized N docs (X skipped, Y new, Z stubs)`

No Granola API calls. Uses sync cache as the doc inventory and raw JSON files as input.

## Markdown Re-rendering

Use the existing `summary::update_summary_in_markdown()` function (summary.rs:396-445) for injecting/replacing the `## Summary` section. It already handles:
- Replacing an existing `## Summary` section
- Inserting before `## Notes` if no summary exists
- Inserting before `---` separator as fallback
- Appending at end as final fallback

**Additional work needed:** updating the frontmatter `related` field with entity wiki-links. This requires:
- Parse the existing YAML frontmatter from the file
- Merge new `related` entries as a union (preserving manually-added links, deduplicating)
- Re-serialize and replace the frontmatter block
- Write atomically

This frontmatter update is a new helper in `storage.rs` (e.g. `merge_frontmatter_related()`), separate from the summary injection.

New docs synced for the first time still generate full markdown from scratch as today.

## Shared Summarization Logic

Both the sync loop and the standalone command perform the same work. Extract into a shared function to avoid duplication. Group parameters into a context struct to keep the signature clean:

```rust
#[cfg(feature = "summaries")]
struct SummarizationContext<'a> {
    config: &'a SummaryConfig,
    api_key: &'a str,
    client: &'a reqwest::blocking::Client,
    context_preamble: &'a mut String,  // mutable: rebuilt after new entity creation
    paths: &'a Paths,
    people_index: &'a mut PeopleIndex,
    summary_cache: &'a mut HashMap<String, SummaryCacheEntry>,
    summary_cache_path: &'a Path,
    dry_run: bool,
}

#[cfg(feature = "summaries")]
fn summarize_and_reconcile(
    ctx: &mut SummarizationContext,
    doc_id: &str,
    transcript: &Transcript,
    meta: &Metadata,
    slug: &str,
) -> Result<(Option<String>, Option<ExtractedEntities>)>
```

The `context_preamble` is `&mut String` so it can be rebuilt after entity creation, matching the current sync loop behavior (sync.rs:413, 441).

This function handles: Claude API call, summary parsing, entity extraction, entity note reconciliation, and summary cache update.

## Files to Modify

- **`src/sync.rs`** — Add `SummaryCacheEntry`, `load_summary_cache`, `save_summary_cache`, `SummarizationContext`, `summarize_and_reconcile` (all `pub(crate)`, gated behind `#[cfg(feature = "summaries")]`). Modify sync loop to check summary cache for skipped docs. Add `summarize_only` counter to stats.
- **`src/cli.rs`** — Add `SummarizeAll` command variant with `--force` and `--dry-run` flags, gated behind `#[cfg(feature = "summaries")]`.
- **`src/main.rs`** — Wire up the new `SummarizeAll` command.
- **`src/storage.rs`** — Add `merge_frontmatter_related()` helper for updating the `related` field in existing markdown files.

## Edge Cases

- **Missing raw files:** Warn and skip, don't abort the batch. Matches existing error-handling pattern for entity reconciliation.
- **Model changes:** The `model` field in `SummaryCacheEntry` records which model was used. `--force` re-summarizes everything. A `--model` filter flag can be added later if needed.
- **Concurrent execution:** Not supported. The atomic write pattern prevents corruption, but two runs would duplicate work. A lock file can be added later if this becomes a real problem.
