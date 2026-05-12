//! Blocking HTTP client for the Granola public API.
//!
//! Handles throttling, Bearer API-key auth, retries, and fail-fast errors.

use crate::{Error, ListNotesResponse, Note, NoteSummary, Result};
use rand::Rng;
use reqwest::blocking::Client;
use std::time::Duration;

const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: Duration = Duration::from_secs(1);
const DEFAULT_BASE_URL: &str = "https://public-api.granola.ai/v1";
const PAGE_SIZE: u32 = 30;

/// Holds both the raw JSON text and the parsed value from an API response.
/// Raw text is preserved so callers can archive the verbatim response.
#[derive(Debug)]
pub struct ApiResponse<T> {
    pub raw: String,
    pub parsed: T,
}

fn truncate_str(s: &str, max_chars: usize) -> String {
    if s.len() <= max_chars {
        return s.to_string();
    }
    let mut boundary = max_chars;
    while boundary > 0 && !s.is_char_boundary(boundary) {
        boundary -= 1;
    }
    if boundary == 0 {
        return String::new();
    }
    format!("{}...", &s[..boundary])
}

/// Blocking HTTP client for the Granola public API.
pub struct ApiClient {
    client: Client,
    base_url: String,
    api_key: String,
    throttle_min: u64,
    throttle_max: u64,
}

impl ApiClient {
    /// Create a new client. Uses `https://public-api.granola.ai/v1` when
    /// `base_url` is `None`.
    pub fn new(api_key: String, base_url: Option<String>) -> Result<Self> {
        let client = Client::builder().timeout(Duration::from_secs(30)).build()?;

        Ok(ApiClient {
            client,
            base_url: base_url.unwrap_or_else(|| DEFAULT_BASE_URL.into()),
            api_key,
            throttle_min: 100,
            throttle_max: 300,
        })
    }

    pub fn with_throttle(mut self, min_ms: u64, max_ms: u64) -> Self {
        self.throttle_min = min_ms;
        self.throttle_max = max_ms;
        self
    }

    pub fn disable_throttle(mut self) -> Self {
        self.throttle_min = 0;
        self.throttle_max = 0;
        self
    }

    fn throttle(&self) {
        if self.throttle_max > 0 {
            let sleep_ms = rand::thread_rng().gen_range(self.throttle_min..=self.throttle_max);
            std::thread::sleep(Duration::from_millis(sleep_ms));
        }
    }

    /// GET a JSON resource at `path` (relative to base URL). Retries transient
    /// errors via `util::retry_with_backoff`.
    fn get_with_raw<T: serde::de::DeserializeOwned>(&self, path: &str) -> Result<ApiResponse<T>> {
        let url = format!("{}{}", self.base_url, path);

        let raw = crate::util::retry_with_backoff(
            MAX_RETRIES,
            INITIAL_RETRY_DELAY,
            || {
                let response = self
                    .client
                    .get(&url)
                    .header("Authorization", format!("Bearer {}", self.api_key))
                    .header("Accept", "application/json")
                    .header("User-Agent", "baez/0.2 (Rust)")
                    .send()?;

                self.throttle();

                let status = response.status();
                if !status.is_success() {
                    let message = response.text().unwrap_or_default();
                    let preview = truncate_str(&message, 200);
                    return Err(Error::Api {
                        endpoint: path.into(),
                        status: status.as_u16(),
                        message: preview,
                    });
                }

                response.text().map_err(Error::from)
            },
            is_retryable,
        )?;

        let parsed = serde_json::from_str(&raw).map_err(|e| {
            eprintln!("Failed to parse response from {}: {}", path, e);
            eprintln!(
                "Response body (first 500 chars): {}",
                truncate_str(&raw, 500)
            );
            Error::Parse(e)
        })?;

        Ok(ApiResponse { raw, parsed })
    }

    /// Fetch a single page of the notes list. Used internally by `list_notes`.
    fn list_notes_page(
        &self,
        cursor: Option<&str>,
        updated_after: Option<&str>,
    ) -> Result<ListNotesResponse> {
        let mut path = format!("/notes?page_size={}", PAGE_SIZE);
        if let Some(c) = cursor {
            path.push_str("&cursor=");
            path.push_str(&urlencode(c));
        }
        if let Some(u) = updated_after {
            path.push_str("&updated_after=");
            path.push_str(&urlencode(u));
        }

        self.get_with_raw::<ListNotesResponse>(&path)
            .map(|r| r.parsed)
    }

    /// Iterate the notes list, paginating through all results. If
    /// `updated_after` is provided, filters server-side.
    pub fn list_notes(&self, updated_after: Option<&str>) -> Result<Vec<NoteSummary>> {
        let mut all = Vec::new();
        let mut cursor: Option<String> = None;
        loop {
            let page = self.list_notes_page(cursor.as_deref(), updated_after)?;
            all.extend(page.notes);
            if !page.has_more || page.cursor.is_none() {
                break;
            }
            cursor = page.cursor;
        }
        Ok(all)
    }

    /// Fetch a single note with the full transcript included.
    pub fn get_note(&self, note_id: &str) -> Result<Note> {
        self.get_note_with_raw(note_id).map(|r| r.parsed)
    }

    /// Fetch a single note with raw JSON preserved for archival.
    pub fn get_note_with_raw(&self, note_id: &str) -> Result<ApiResponse<Note>> {
        let path = format!("/notes/{}?include=transcript", note_id);
        self.get_with_raw::<Note>(&path)
    }
}

/// Percent-encode a query string value. Sufficient for our use (cursor and ISO
/// timestamps); not a general URL encoder.
fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

/// Determine if an error is worth retrying (network errors, 429, 5xx).
fn is_retryable(err: &Error) -> bool {
    match err {
        Error::Network(_) => true,
        Error::Api { status, .. } => *status == 429 || *status >= 500,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_truncate_str_short() {
        assert_eq!(truncate_str("hello", 100), "hello");
    }

    #[test]
    fn test_truncate_str_long() {
        let result = truncate_str("hello world", 7);
        assert!(result.starts_with("hello"));
        assert!(result.ends_with("..."));
    }

    #[test]
    fn test_truncate_str_utf8() {
        let text = "Hello 世界 World";
        let result = truncate_str(text, 10);
        assert!(!result.is_empty());
    }

    #[test]
    fn test_api_client_new_defaults_base_url() {
        let client = ApiClient::new("test_key".into(), None).unwrap();
        assert_eq!(client.base_url, "https://public-api.granola.ai/v1");
        assert_eq!(client.api_key, "test_key");
    }

    #[test]
    fn test_api_client_custom_base() {
        let client = ApiClient::new("key".into(), Some("https://custom.api".into())).unwrap();
        assert_eq!(client.base_url, "https://custom.api");
    }

    #[test]
    fn test_api_client_throttle_config() {
        let client = ApiClient::new("key".into(), None)
            .unwrap()
            .with_throttle(50, 150);
        assert_eq!(client.throttle_min, 50);
        assert_eq!(client.throttle_max, 150);
    }

    #[test]
    fn test_urlencode_basics() {
        assert_eq!(urlencode("hello"), "hello");
        assert_eq!(urlencode("a=b&c"), "a%3Db%26c");
        assert_eq!(
            urlencode("2026-05-11T00:00:00Z"),
            "2026-05-11T00%3A00%3A00Z"
        );
    }

    #[test]
    fn test_is_retryable() {
        assert!(is_retryable(&Error::Api {
            endpoint: "/notes".into(),
            status: 500,
            message: "".into(),
        }));
        assert!(is_retryable(&Error::Api {
            endpoint: "/notes".into(),
            status: 429,
            message: "".into(),
        }));
        assert!(!is_retryable(&Error::Api {
            endpoint: "/notes".into(),
            status: 401,
            message: "".into(),
        }));
        assert!(!is_retryable(&Error::Api {
            endpoint: "/notes".into(),
            status: 404,
            message: "".into(),
        }));
    }
}
