//! Utility functions for slugging, timestamps, and retries.
//!
//! Provides consistent filename generation, time formatting, and retry logic.

use chrono::{DateTime, Utc};
use std::error::Error as _;
use std::thread;
use std::time::Duration;

/// Format a `reqwest::Error` with its full source chain and a category tag.
///
/// reqwest's `Display` only renders the top-level message (e.g. "error sending
/// request for url (...)") and hides the underlying cause. This walks
/// `source()` to expose the real failure (timeout, connection reset, body
/// error) and prefixes a category derived from reqwest's classifier methods.
pub fn format_reqwest_error(err: &reqwest::Error) -> String {
    let category = if err.is_timeout() {
        "timeout"
    } else if err.is_connect() {
        "connect"
    } else if err.is_body() {
        "body"
    } else if err.is_decode() {
        "decode"
    } else if err.is_request() {
        "request"
    } else if err.is_status() {
        "status"
    } else {
        "transport"
    };

    let mut msg = format!("[{}] {}", category, err);
    let mut source: Option<&dyn std::error::Error> = err.source();
    while let Some(cause) = source {
        msg.push_str(&format!(": {}", cause));
        source = cause.source();
    }
    msg
}

/// Convert text to a URL-safe slug for filenames. Returns `"untitled"` for empty input.
pub fn slugify(text: &str) -> String {
    let slug = slug::slugify(text);
    // Handle empty slugs (happens when title is only special chars)
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}

/// Build a unique slug for a document, appending a short ID hash for untitled meetings.
pub fn doc_slug(title: Option<&str>, doc_id: &str) -> String {
    let base = slugify(title.unwrap_or("untitled"));
    if base == "untitled" {
        // Append first 8 chars of doc_id to disambiguate
        let short_id: String = doc_id
            .chars()
            .filter(|c| c.is_alphanumeric())
            .take(8)
            .collect();
        format!("untitled-{}", short_id)
    } else {
        base
    }
}

/// Count the total number of words across all transcript entries.
/// Used for triage: transcripts with < 20 words are classified as stubs.
pub fn count_transcript_words(entries: &[crate::model::TranscriptEntry]) -> usize {
    entries
        .iter()
        .map(|e| e.text.split_whitespace().count())
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("Q4 Planning!!!"), "q4-planning");
        assert_eq!(slugify(""), "untitled");
    }

    #[test]
    fn test_slugify_special_chars() {
        assert_eq!(slugify("Föö Bär"), "foo-bar");
        assert_eq!(slugify("Test@#$%123"), "test-123");
        assert_eq!(slugify("!!!@@@###"), "untitled"); // Only special chars
    }
}

/// Normalize a timestamp string to `HH:MM:SS` format.
///
/// Accepts ISO 8601 datetimes and `HH:MM:SS.sss` fractional formats.
pub fn normalize_timestamp(ts: &str) -> Option<String> {
    // Try to parse as ISO 8601 datetime
    if let Ok(dt) = ts.parse::<DateTime<Utc>>() {
        // Extract time portion and format as HH:MM:SS
        return Some(dt.format("%H:%M:%S").to_string());
    }

    // Fallback: try to parse as HH:MM:SS or HH:MM:SS.sss
    if let Some(pos) = ts.find('.') {
        Some(ts[..pos].to_string())
    } else if ts.contains(':') {
        Some(ts.to_string())
    } else {
        None
    }
}

/// Retry a fallible operation with exponential backoff.
///
/// Retries up to `max_retries` times when `should_retry` returns true for the error.
/// Backoff starts at `initial_delay` and doubles each attempt.
pub fn retry_with_backoff<T, E>(
    max_retries: u32,
    initial_delay: Duration,
    mut operation: impl FnMut() -> std::result::Result<T, E>,
    should_retry: impl Fn(&E) -> bool,
) -> std::result::Result<T, E> {
    let mut delay = initial_delay;
    let mut attempts = 0;

    loop {
        match operation() {
            Ok(val) => return Ok(val),
            Err(err) => {
                attempts += 1;
                if attempts > max_retries || !should_retry(&err) {
                    return Err(err);
                }
                eprintln!(
                    "Request failed, retrying in {}ms (attempt {}/{})...",
                    delay.as_millis(),
                    attempts,
                    max_retries
                );
                thread::sleep(delay);
                delay *= 2;
            }
        }
    }
}

#[cfg(test)]
mod retry_tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn test_retry_succeeds_first_try() {
        let result = retry_with_backoff(
            3,
            Duration::from_millis(1),
            || Ok::<_, &str>("success"),
            |_| true,
        );
        assert_eq!(result.unwrap(), "success");
    }

    #[test]
    fn test_retry_succeeds_after_failures() {
        let attempts = Cell::new(0);
        let result = retry_with_backoff(
            3,
            Duration::from_millis(1),
            || {
                let n = attempts.get() + 1;
                attempts.set(n);
                if n < 3 {
                    Err("transient")
                } else {
                    Ok("recovered")
                }
            },
            |_| true,
        );
        assert_eq!(result.unwrap(), "recovered");
        assert_eq!(attempts.get(), 3);
    }

    #[test]
    fn test_retry_exhausted() {
        let attempts = Cell::new(0);
        let result = retry_with_backoff(
            2,
            Duration::from_millis(1),
            || {
                attempts.set(attempts.get() + 1);
                Err::<(), _>("permanent")
            },
            |_| true,
        );
        assert_eq!(result.unwrap_err(), "permanent");
        assert_eq!(attempts.get(), 3); // initial + 2 retries
    }

    #[test]
    fn test_retry_skips_non_retryable() {
        let attempts = Cell::new(0);
        let result = retry_with_backoff(
            3,
            Duration::from_millis(1),
            || {
                attempts.set(attempts.get() + 1);
                Err::<(), _>("not retryable")
            },
            |_| false,
        );
        assert_eq!(result.unwrap_err(), "not retryable");
        assert_eq!(attempts.get(), 1); // no retries
    }
}

#[cfg(test)]
mod timestamp_tests {
    use super::*;

    #[test]
    fn test_normalize_timestamp_iso8601() {
        assert_eq!(
            normalize_timestamp("2025-10-01T21:35:24.568Z"),
            Some("21:35:24".into())
        );
        assert_eq!(
            normalize_timestamp("2025-10-01T09:05:10.000Z"),
            Some("09:05:10".into())
        );
    }

    #[test]
    fn test_normalize_timestamp_hms() {
        assert_eq!(normalize_timestamp("00:12:34.567"), Some("00:12:34".into()));
        assert_eq!(normalize_timestamp("00:05:10"), Some("00:05:10".into()));
    }
}

/// Compute Levenshtein edit distance between two strings.
pub fn levenshtein_distance(a: &str, b: &str) -> usize {
    strsim::levenshtein(a, b)
}

#[cfg(test)]
mod triage_tests {
    use super::*;
    use crate::model::TranscriptEntry;

    fn make_entry(text: &str) -> TranscriptEntry {
        TranscriptEntry {
            speaker: None,
            text: text.into(),
            start_time: None,
            end_time: None,
        }
    }

    #[test]
    fn test_empty_transcript() {
        let entries: Vec<TranscriptEntry> = vec![];
        assert_eq!(count_transcript_words(&entries), 0);
    }

    #[test]
    fn test_stub_transcript() {
        let entries = vec![make_entry("hello world")];
        assert_eq!(count_transcript_words(&entries), 2);
    }

    #[test]
    fn test_substantive_transcript() {
        let words: String = (0..25)
            .map(|i| format!("word{}", i))
            .collect::<Vec<_>>()
            .join(" ");
        let entries = vec![make_entry(&words)];
        assert_eq!(count_transcript_words(&entries), 25);
    }

    #[test]
    fn test_whitespace_only_entries() {
        let entries = vec![make_entry("   "), make_entry(""), make_entry("\n\t")];
        assert_eq!(count_transcript_words(&entries), 0);
    }

    #[test]
    fn test_multiple_entries_summed() {
        let entries = vec![make_entry("one two three"), make_entry("four five")];
        assert_eq!(count_transcript_words(&entries), 5);
    }
}

#[cfg(test)]
mod levenshtein_tests {
    use super::*;

    #[test]
    fn test_identical_strings() {
        assert_eq!(levenshtein_distance("alice", "alice"), 0);
    }

    #[test]
    fn test_one_edit() {
        assert_eq!(levenshtein_distance("alice", "alce"), 1);
    }

    #[test]
    fn test_two_edits() {
        assert_eq!(levenshtein_distance("smith", "smyth"), 1);
    }

    #[test]
    fn test_completely_different() {
        assert!(levenshtein_distance("alice", "bob") > 2);
    }
}
