//! Utility functions for slugging, timestamps, and retries.
//!
//! Provides consistent filename generation, time formatting, and retry logic.

use crate::model::TimestampValue;
use chrono::{DateTime, Utc};
use std::thread;
use std::time::Duration;

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

/// Normalize a [`TimestampValue`] (seconds or string) to `HH:MM:SS` format.
pub fn normalize_timestamp_legacy(ts: &TimestampValue) -> Option<String> {
    match ts {
        TimestampValue::Seconds(secs) => {
            let total_secs = *secs as u64;
            let hours = total_secs / 3600;
            let minutes = (total_secs % 3600) / 60;
            let seconds = total_secs % 60;
            Some(format!("{:02}:{:02}:{:02}", hours, minutes, seconds))
        }
        TimestampValue::String(s) => normalize_timestamp(s),
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
    use crate::model::TimestampValue;

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

    #[test]
    fn test_normalize_timestamp_legacy_seconds() {
        let ts = TimestampValue::Seconds(3665.5);
        assert_eq!(normalize_timestamp_legacy(&ts), Some("01:01:05".into()));
    }

    #[test]
    fn test_normalize_timestamp_legacy_string() {
        let ts = TimestampValue::String("00:12:34.567".into());
        assert_eq!(normalize_timestamp_legacy(&ts), Some("00:12:34".into()));
    }
}
