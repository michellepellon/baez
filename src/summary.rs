//! AI summarization using the Claude API (Anthropic Messages API).
//!
//! Chunks long transcripts and generates structured meeting summaries.

use crate::model::{DocumentMetadata, RawTranscript};
use crate::util::normalize_timestamp;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

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

## People
Table of all people mentioned with their role/context in this meeting:
| Person | Role/Context |
|--------|-------------|
| [[Alice Smith]] | Engineering Manager, led API discussion |

## Project Ideas
- [[Project Name]] — what was discussed, potential next steps

## Blog Ideas
- **Idea title** — why it's worth writing about, angle to take

## Concepts
- [[Concept Name]] — brief description and why it matters

## Ideas
- Idea description — enough context to act on later

Rules:
- Use [[wiki-links]] for all person names (e.g. [[Alice Smith]]).
- Use `- [ ]` checkboxes for action items.
- Use markdown headers (##, ###) for sections.
- Preserve important names, dates, and numbers accurately.
- Only use information from the transcript; label any inferences as "(inferred)".
- Be explicit when something is unclear, missing, or not specified.
- Ignore small talk; focus on substance.
- Use Dataview inline field syntax [field:: value] exactly as shown in the examples above.
- If a section has no items, write "None." under the heading — do not omit the section.

After the markdown summary, output a machine-readable JSON entity block for automated processing.
Wrap it in an HTML comment so it does not render in Obsidian:

<!-- baez-entities
{
  "people": [
    {"name": "Full Name", "role": "their role if known", "company": "their company if known", "aliases": ["nickname", "abbreviation"], "context": "one-line context from this meeting"}
  ],
  "concepts": [
    {"name": "Concept Name", "description": "one-line description", "existing": true}
  ],
  "projects": [
    {"name": "Project Name", "description": "one-line description", "existing": false}
  ]
}
-->

Rules for the entity block:
- Include ALL people mentioned by name (full names only — skip first-name-only references unless you can infer the full name from context).
- For concepts: set "existing" to true if the concept appears in the provided existing concepts list, false otherwise.
- For projects: set "existing" to true if the project appears in the provided existing projects list, false otherwise.
- Only include genuinely reusable concepts, not trivial topics.
- The JSON must be valid. Use null for missing optional fields (role, company)."#;

/// Simplified prompt for intermediate chunk summaries (no entity extraction).
const CHUNK_SUMMARY_PROMPT: &str = r#"You are an expert meeting summarizer. Summarize the following transcript chunk into a concise narrative. Focus on key points, decisions, and action items. Use [[wiki-links]] for person names. This is a partial transcript — do not state conclusions about the overall meeting."#;

const CLAUDE_API_URL: &str = "https://api.anthropic.com/v1/messages";
const ANTHROPIC_VERSION: &str = "2023-06-01";

/// Configuration for Claude API summarization.
#[derive(Serialize, Deserialize, Clone)]
pub struct SummaryConfig {
    pub model: String,
    pub max_input_chars: usize,
    pub max_tokens: usize,
    pub custom_prompt: Option<String>,
    #[serde(default)]
    pub temperature: Option<f32>,
}

impl Default for SummaryConfig {
    fn default() -> Self {
        Self {
            model: "claude-opus-4-6".to_string(),
            max_input_chars: 600_000, // ~150K tokens
            max_tokens: 8192,
            custom_prompt: None,
            temperature: None,
        }
    }
}

impl SummaryConfig {
    pub fn load(config_path: &Path) -> Result<Self> {
        if !config_path.exists() {
            return Ok(Self::default());
        }

        let content = std::fs::read_to_string(config_path)?;
        serde_json::from_str(&content).map_err(|e| {
            Error::Filesystem(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Failed to parse summary config: {}", e),
            ))
        })
    }

    pub fn save(&self, config_path: &Path, tmp_dir: &Path) -> Result<()> {
        let json = serde_json::to_string_pretty(self)?;
        crate::storage::write_atomic(config_path, json.as_bytes(), tmp_dir)
    }

    pub fn prompt(&self) -> &str {
        self.custom_prompt
            .as_deref()
            .unwrap_or(DEFAULT_SUMMARY_PROMPT)
    }
}

/// Entities extracted from a Claude summary for vault knowledge graph updates.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractedEntities {
    #[serde(default)]
    pub people: Vec<PersonEntity>,
    #[serde(default)]
    pub concepts: Vec<ConceptEntity>,
    #[serde(default)]
    pub projects: Vec<ProjectEntity>,
}

/// A person mentioned in a meeting transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonEntity {
    pub name: String,
    #[serde(default)]
    pub role: Option<String>,
    #[serde(default)]
    pub company: Option<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: String,
}

/// A reusable concept or insight extracted from a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConceptEntity {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub existing: bool,
}

/// A project mentioned in a meeting.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectEntity {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub existing: bool,
}

const ENTITY_MARKER_START: &str = "<!-- baez-entities";
const ENTITY_MARKER_END: &str = "-->";

/// Separate markdown summary from the JSON entity block.
///
/// Returns (clean_markdown, optional_entities). Parsing failures are non-fatal:
/// if the JSON block is missing or malformed, returns the full text as markdown
/// with `None` for entities.
pub fn parse_summary_output(raw: &str) -> (String, Option<ExtractedEntities>) {
    let Some(marker_start) = raw.find(ENTITY_MARKER_START) else {
        return (raw.to_string(), None);
    };

    let markdown = raw[..marker_start].trim_end().to_string();

    let json_start = marker_start + ENTITY_MARKER_START.len();
    let Some(marker_end) = raw[json_start..].find(ENTITY_MARKER_END) else {
        eprintln!("Warning: Found baez-entities marker but no closing -->");
        return (raw.to_string(), None);
    };

    let json_str = raw[json_start..json_start + marker_end].trim();

    match serde_json::from_str::<ExtractedEntities>(json_str) {
        Ok(entities) => (markdown, Some(entities)),
        Err(e) => {
            eprintln!("Warning: Failed to parse baez-entities JSON: {}", e);
            (markdown, None)
        }
    }
}

/// Format a raw transcript into plain text suitable for LLM input.
/// Includes a metadata header (title, date, duration, participants) followed by
/// `Speaker (HH:MM:SS): text` lines.
pub fn format_transcript_for_llm(raw: &RawTranscript, meta: &DocumentMetadata) -> String {
    let mut out = String::new();

    // Metadata header
    if let Some(ref title) = meta.title {
        out.push_str(&format!("Title: {}\n", title));
    }
    out.push_str(&format!(
        "Date: {}\n",
        meta.created_at.format("%Y-%m-%d %H:%M UTC")
    ));
    if let Some(secs) = meta.duration_seconds {
        let mins = secs / 60;
        out.push_str(&format!("Duration: {} minutes\n", mins));
    }
    if !meta.participants.is_empty() {
        out.push_str(&format!("Participants: {}\n", meta.participants.join(", ")));
    }
    out.push_str("\n---\n\n");

    // Transcript entries
    for entry in &raw.entries {
        let speaker = entry.speaker.as_deref().unwrap_or("Speaker");
        let timestamp = entry
            .start
            .as_deref()
            .and_then(normalize_timestamp)
            .map(|ts| format!(" ({})", ts))
            .unwrap_or_default();
        out.push_str(&format!("{}{}: {}\n", speaker, timestamp, entry.text));
    }

    out
}

/// Build a reusable HTTP client for Claude API calls.
pub fn build_claude_client() -> Result<reqwest::blocking::Client> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(120))
        .build()
        .map_err(|e| Error::Summarization(format!("Failed to build HTTP client: {}", e)))
}

/// Summarize transcript text using the Claude Messages API (blocking).
pub fn summarize_transcript(
    transcript_text: &str,
    api_key: &str,
    config: &SummaryConfig,
    client: &reqwest::blocking::Client,
    context_preamble: &str,
) -> Result<String> {
    let chunks = chunk_transcript(transcript_text, config.max_input_chars);

    if chunks.len() > 1 {
        // Multi-chunk: use simplified prompt for intermediate summaries
        let chunk_config = SummaryConfig {
            custom_prompt: Some(CHUNK_SUMMARY_PROMPT.to_string()),
            ..config.clone()
        };
        let mut chunk_summaries = Vec::new();
        for (i, chunk) in chunks.iter().enumerate() {
            println!("Summarizing chunk {}/{}...", i + 1, chunks.len());
            let summary = call_claude_api(client, chunk, api_key, &chunk_config)?;
            chunk_summaries.push(summary);
        }
        // Final pass: combine chunk summaries with full prompt + context preamble
        let combined = format!(
            "{}\n\nCombined meeting summary from {} chunks:\n\n{}",
            context_preamble,
            chunks.len(),
            chunk_summaries.join("\n\n---\n\n")
        );
        call_claude_api(client, &combined, api_key, config)
    } else {
        // Single chunk: full prompt with context preamble
        let input = if context_preamble.is_empty() {
            chunks[0].clone()
        } else {
            format!("{}\n\n{}", context_preamble, &chunks[0])
        };
        call_claude_api(client, &input, api_key, config)
    }
}

const MAX_RETRIES: u32 = 2;
const INITIAL_RETRY_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// POST to Claude Messages API and extract the text response.
fn call_claude_api(
    client: &reqwest::blocking::Client,
    text: &str,
    api_key: &str,
    config: &SummaryConfig,
) -> Result<String> {
    let full_prompt = format!(
        "{}\n\nTranscript:\n<<<TRANSCRIPT_START>>>\n{}\n<<<TRANSCRIPT_END>>>",
        config.prompt(),
        text
    );

    let message = serde_json::json!({
        "role": "user",
        "content": full_prompt
    });

    let mut body = serde_json::json!({
        "model": config.model,
        "max_tokens": config.max_tokens,
        "messages": [message]
    });

    if let Some(temp) = config.temperature {
        body["temperature"] = serde_json::json!(temp);
    }

    let response_text = crate::util::retry_with_backoff(
        MAX_RETRIES,
        INITIAL_RETRY_DELAY,
        || {
            let response = client
                .post(CLAUDE_API_URL)
                .header("x-api-key", api_key)
                .header("anthropic-version", ANTHROPIC_VERSION)
                .header("content-type", "application/json")
                .json(&body)
                .send()
                .map_err(|e| Error::Summarization(format!("Claude API request failed: {}", e)))?;

            let status = response.status();
            let text = response.text().map_err(|e| {
                Error::Summarization(format!("Failed to read Claude API response: {}", e))
            })?;

            if !status.is_success() {
                return Err(Error::Summarization(format!(
                    "Claude API error ({}): {}",
                    status, text
                )));
            }

            Ok(text)
        },
        |err| {
            // Retry on network errors and overloaded/rate-limit responses
            match err {
                Error::Summarization(msg) => {
                    msg.contains("request failed")
                        || msg.contains("429")
                        || msg.contains("529")
                        || msg.contains("500")
                        || msg.contains("502")
                        || msg.contains("503")
                }
                _ => false,
            }
        },
    )?;

    let response_json: serde_json::Value = serde_json::from_str(&response_text)
        .map_err(|e| Error::Summarization(format!("Failed to parse Claude API response: {}", e)))?;

    // Extract text from content array
    response_json["content"]
        .as_array()
        .and_then(|arr| arr.first())
        .and_then(|block| block["text"].as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| Error::Summarization("No text content in Claude API response".into()))
}

/// Replace or insert a `## Summary` section in existing markdown content.
/// If a `## Summary` section exists, replaces its content up to the next `## ` heading.
/// Otherwise, inserts before `## Notes` or before `---` separator.
pub fn update_summary_in_markdown(content: &str, new_summary: &str) -> String {
    let summary_section = format!("## Summary\n\n{}", new_summary);

    // Find existing ## Summary section
    if let Some(start) = content.find("\n## Summary\n") {
        let section_start = start + 1; // skip the leading newline
                                       // Find the end of the summary section (next ## heading or --- separator)
        let rest = &content[section_start..];
        let section_end = rest[12..] // skip "## Summary\n\n" prefix to search for next section
            .find("\n## ")
            .map(|pos| section_start + 12 + pos + 1)
            .or_else(|| {
                rest[12..]
                    .find("\n---")
                    .map(|pos| section_start + 12 + pos + 1)
            })
            .unwrap_or(content.len());

        let mut result = String::new();
        result.push_str(&content[..section_start]);
        result.push_str(&summary_section);
        result.push_str("\n\n");
        result.push_str(&content[section_end..]);
        return result;
    }

    // No existing summary section — insert before ## Notes or ---
    if let Some(pos) = content.find("\n## Notes\n") {
        let insert_at = pos + 1;
        let mut result = String::new();
        result.push_str(&content[..insert_at]);
        result.push_str(&summary_section);
        result.push_str("\n\n");
        result.push_str(&content[insert_at..]);
        return result;
    }

    if let Some(pos) = content.find("\n---\n") {
        let insert_at = pos + 1;
        let mut result = String::new();
        result.push_str(&content[..insert_at]);
        result.push_str(&summary_section);
        result.push_str("\n\n");
        result.push_str(&content[insert_at..]);
        return result;
    }

    // Fallback: append at the end
    format!("{}\n\n{}\n", content, summary_section)
}

/// Unified API key lookup with precedence:
/// 1. `BAEZ_ANTHROPIC_API_KEY` env var (tool-specific)
/// 2. `ANTHROPIC_API_KEY` env var (cross-tool standard)
/// 3. Config file `~/.config/baez/config.json` field `anthropic_api_key`
/// 4. macOS keychain
pub fn get_api_key() -> Option<String> {
    get_api_key_verbose(false)
}

/// Like [`get_api_key`], but prints which source was used when `verbose` is true.
pub fn get_api_key_verbose(verbose: bool) -> Option<String> {
    if let Ok(key) = std::env::var("BAEZ_ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            if verbose {
                eprintln!("[verbose] Anthropic API key: BAEZ_ANTHROPIC_API_KEY env var");
            }
            return Some(key);
        }
    }
    if let Ok(key) = std::env::var("ANTHROPIC_API_KEY") {
        if !key.is_empty() {
            if verbose {
                eprintln!("[verbose] Anthropic API key: ANTHROPIC_API_KEY env var");
            }
            return Some(key);
        }
    }
    if let Some(key) = get_api_key_from_config() {
        if verbose {
            eprintln!("[verbose] Anthropic API key: config file (anthropic_api_key)");
        }
        return Some(key);
    }
    if let Ok(key) = get_api_key_from_keychain() {
        if verbose {
            eprintln!("[verbose] Anthropic API key: macOS keychain");
        }
        return Some(key);
    }
    None
}

fn get_api_key_from_config() -> Option<String> {
    crate::storage::read_config_field("anthropic_api_key")
        .ok()
        .flatten()
        .filter(|k| !k.is_empty())
}

/// Read the Anthropic API key from the macOS system keychain.
pub fn get_api_key_from_keychain() -> Result<String> {
    #[cfg(target_os = "macos")]
    {
        use keyring::Entry;

        let entry = Entry::new("baez", "anthropic_api_key")
            .map_err(|e| Error::Auth(format!("Failed to access keychain: {}", e)))?;

        entry.get_password().map_err(|e| {
            Error::Auth(format!(
                "Anthropic API key not found in keychain. Set it with: baez set-api-key <key>. Error: {}",
                e
            ))
        })
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::Auth(
            "Keychain access only supported on macOS. Set BAEZ_ANTHROPIC_API_KEY or ANTHROPIC_API_KEY environment variable."
                .into(),
        ))
    }
}

/// Store the Anthropic API key in the macOS system keychain.
pub fn set_api_key_in_keychain(_api_key: &str) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        use keyring::Entry;

        let entry = Entry::new("baez", "anthropic_api_key")
            .map_err(|e| Error::Auth(format!("Failed to access keychain: {}", e)))?;

        entry
            .set_password(_api_key)
            .map_err(|e| Error::Auth(format!("Failed to store API key in keychain: {}", e)))?;

        println!("Anthropic API key stored in keychain");
        Ok(())
    }

    #[cfg(not(target_os = "macos"))]
    {
        Err(Error::Auth(
            "Keychain access only supported on macOS. Set BAEZ_ANTHROPIC_API_KEY or ANTHROPIC_API_KEY environment variable."
                .into(),
        ))
    }
}

fn chunk_transcript(text: &str, max_chars: usize) -> Vec<String> {
    if text.len() <= max_chars {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut current_chunk = String::new();

    for line in text.lines() {
        if current_chunk.len() + line.len() + 1 > max_chars && !current_chunk.is_empty() {
            chunks.push(current_chunk.clone());
            current_chunk.clear();
        }
        current_chunk.push_str(line);
        current_chunk.push('\n');
    }

    if !current_chunk.is_empty() {
        chunks.push(current_chunk);
    }

    chunks
}

/// Scan Concepts/ and Projects/ directories for existing entity names.
/// Returns a prompt preamble string listing them for Claude to reference.
pub fn build_context_preamble(vault_dir: &std::path::Path) -> String {
    let mut sections = Vec::new();

    if let Some(names) = scan_entity_dir(&vault_dir.join("Concepts")) {
        if !names.is_empty() {
            let mut section = String::from(
                "Existing concepts in the vault (reference by exact name if relevant, only propose new ones if genuinely distinct):\n"
            );
            for name in &names {
                section.push_str(&format!("- {}\n", name));
            }
            sections.push(section);
        }
    }

    if let Some(names) = scan_entity_dir(&vault_dir.join("Projects")) {
        if !names.is_empty() {
            let mut section = String::from(
                "Existing projects in the vault (reference by exact name if relevant):\n",
            );
            for name in &names {
                section.push_str(&format!("- {}\n", name));
            }
            sections.push(section);
        }
    }

    sections.join("\n")
}

/// Scan a directory for .md files and return their filename stems.
fn scan_entity_dir(dir: &std::path::Path) -> Option<Vec<String>> {
    if !dir.is_dir() {
        return None;
    }

    let mut names: Vec<String> = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let path = e.path();
            if path.extension().and_then(|s| s.to_str()) == Some("md") {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .map(|s| s.to_string())
            } else {
                None
            }
        })
        .collect();

    names.sort();
    Some(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    #[test]
    fn test_chunk_transcript_short() {
        let text = "Short transcript";
        let chunks = chunk_transcript(text, 1000);
        assert_eq!(chunks.len(), 1);
        assert!(chunks[0].contains("Short transcript"));
    }

    #[test]
    fn test_chunk_transcript_long() {
        let text = "Line 1\n".repeat(200); // 1400 chars
        let chunks = chunk_transcript(&text, 500);
        assert!(chunks.len() > 1);
        for chunk in &chunks {
            assert!(chunk.len() <= 500 || chunk.lines().count() == 1);
        }
    }

    #[test]
    fn test_summary_prompt_format() {
        assert!(DEFAULT_SUMMARY_PROMPT.contains("Summary"));
        assert!(DEFAULT_SUMMARY_PROMPT.contains("Action Items"));
        assert!(DEFAULT_SUMMARY_PROMPT.contains("Key Decisions"));
        assert!(DEFAULT_SUMMARY_PROMPT.contains("Open Questions"));
        assert!(DEFAULT_SUMMARY_PROMPT.contains("[[wiki-links]]"));
    }

    #[test]
    fn test_summary_config_defaults() {
        let config = SummaryConfig::default();
        assert_eq!(config.model, "claude-opus-4-6");
        assert_eq!(config.max_input_chars, 600_000);
        assert_eq!(config.max_tokens, 8192);
        assert!(config.custom_prompt.is_none());
        assert!(config.temperature.is_none());
    }

    #[test]
    fn test_format_transcript_for_llm() {
        let raw = RawTranscript {
            entries: vec![
                TranscriptEntry {
                    document_id: Some("doc123".into()),
                    speaker: Some("Alice".into()),
                    start: Some("2025-10-01T21:35:12.500Z".into()),
                    end: None,
                    text: "Hello everyone".into(),
                    source: None,
                    id: None,
                    is_final: None,
                },
                TranscriptEntry {
                    document_id: Some("doc123".into()),
                    speaker: Some("Bob".into()),
                    start: Some("2025-10-01T21:35:20.000Z".into()),
                    end: None,
                    text: "Hi there".into(),
                    source: None,
                    id: None,
                    is_final: None,
                },
            ],
        };

        let meta = DocumentMetadata {
            id: Some("doc123".into()),
            title: Some("Test Meeting".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec!["Alice".into(), "Bob".into()],
            duration_seconds: Some(3600),
            labels: vec![],
            creator: None,
            attendees: None,
        };

        let output = format_transcript_for_llm(&raw, &meta);
        assert!(output.contains("Title: Test Meeting"));
        assert!(output.contains("Duration: 60 minutes"));
        assert!(output.contains("Participants: Alice, Bob"));
        assert!(output.contains("Alice (21:35:12): Hello everyone"));
        assert!(output.contains("Bob (21:35:20): Hi there"));
    }

    #[test]
    fn test_format_transcript_for_llm_minimal() {
        let raw = RawTranscript {
            entries: vec![TranscriptEntry {
                document_id: None,
                speaker: None,
                start: None,
                end: None,
                text: "Just text".into(),
                source: None,
                id: None,
                is_final: None,
            }],
        };

        let meta = DocumentMetadata {
            id: None,
            title: None,
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec![],
            duration_seconds: None,
            labels: vec![],
            creator: None,
            attendees: None,
        };

        let output = format_transcript_for_llm(&raw, &meta);
        assert!(output.contains("Speaker: Just text"));
        assert!(!output.contains("Title:"));
        assert!(!output.contains("Duration:"));
    }

    #[test]
    fn test_update_summary_in_markdown_replace_existing() {
        let content = "# Meeting\n\n## Summary\n\nOld summary text.\n\n## Notes\n\nSome notes.\n";
        let result = update_summary_in_markdown(content, "New summary text.");
        assert!(result.contains("## Summary\n\nNew summary text."));
        assert!(result.contains("## Notes\n\nSome notes."));
        assert!(!result.contains("Old summary text."));
    }

    #[test]
    fn test_update_summary_in_markdown_insert_before_notes() {
        let content = "# Meeting\n\n## Notes\n\nSome notes.\n";
        let result = update_summary_in_markdown(content, "New summary.");
        assert!(result.contains("## Summary\n\nNew summary."));
        assert!(result.contains("## Notes\n\nSome notes."));
        let summary_pos = result.find("## Summary").unwrap();
        let notes_pos = result.find("## Notes").unwrap();
        assert!(summary_pos < notes_pos);
    }

    #[test]
    fn test_update_summary_in_markdown_insert_before_separator() {
        let content = "# Meeting\n\n---\n\nTranscript here.\n";
        let result = update_summary_in_markdown(content, "New summary.");
        assert!(result.contains("## Summary\n\nNew summary."));
        let summary_pos = result.find("## Summary").unwrap();
        let separator_pos = result.find("---").unwrap();
        assert!(summary_pos < separator_pos);
    }

    #[test]
    fn test_update_summary_in_markdown_append_fallback() {
        let content = "# Meeting\n\nSome content.";
        let result = update_summary_in_markdown(content, "New summary.");
        assert!(result.contains("## Summary\n\nNew summary."));
    }

    #[test]
    fn test_get_api_key_from_env() {
        // Test that env var is checked (can't easily test keychain in unit tests)
        std::env::remove_var("BAEZ_ANTHROPIC_API_KEY");
        std::env::remove_var("ANTHROPIC_API_KEY");
        // With no env var and no keychain, get_api_key returns None (on non-macOS)
        // On macOS it may try keychain — just ensure it doesn't panic
        let _ = get_api_key();
    }
}

#[cfg(test)]
mod parse_tests {
    use super::*;

    #[test]
    fn test_parse_valid_markdown_and_json() {
        let input = r#"## Summary
- Point one
- Point two

## People
| [[Alice Smith]] | Engineer |

<!-- baez-entities
{
  "people": [{"name": "Alice Smith", "role": "Engineer", "company": "Acme", "aliases": ["Alice"], "context": "Led discussion"}],
  "concepts": [{"name": "API Design", "description": "Building APIs first", "existing": false}],
  "projects": []
}
-->"#;

        let (markdown, entities) = parse_summary_output(input);

        assert!(markdown.contains("## Summary"));
        assert!(markdown.contains("## People"));
        assert!(!markdown.contains("baez-entities"));
        assert!(!markdown.contains("-->"));

        let entities = entities.unwrap();
        assert_eq!(entities.people.len(), 1);
        assert_eq!(entities.people[0].name, "Alice Smith");
        assert_eq!(entities.people[0].role.as_deref(), Some("Engineer"));
        assert_eq!(entities.people[0].aliases, vec!["Alice"]);
        assert_eq!(entities.concepts.len(), 1);
        assert_eq!(entities.concepts[0].name, "API Design");
        assert!(!entities.concepts[0].existing);
        assert!(entities.projects.is_empty());
    }

    #[test]
    fn test_parse_no_json_block() {
        let input = "## Summary\n- Just markdown\n\n## Notes\nSome notes.";
        let (markdown, entities) = parse_summary_output(input);
        assert_eq!(markdown, input);
        assert!(entities.is_none());
    }

    #[test]
    fn test_parse_malformed_json() {
        let input = "## Summary\n\n<!-- baez-entities\n{invalid json\n-->";
        let (markdown, entities) = parse_summary_output(input);
        assert!(markdown.contains("## Summary"));
        assert!(entities.is_none());
    }

    #[test]
    fn test_parse_empty_entities() {
        let input = r#"## Summary

<!-- baez-entities
{"people": [], "concepts": [], "projects": []}
-->"#;
        let (markdown, entities) = parse_summary_output(input);
        assert!(markdown.contains("## Summary"));
        let entities = entities.unwrap();
        assert!(entities.people.is_empty());
        assert!(entities.concepts.is_empty());
        assert!(entities.projects.is_empty());
    }

    #[test]
    fn test_parse_strips_trailing_whitespace() {
        let input = "## Summary\n- Point\n\n<!-- baez-entities\n{\"people\": [], \"concepts\": [], \"projects\": []}\n-->\n";
        let (markdown, _) = parse_summary_output(input);
        assert!(!markdown.contains("baez-entities"));
    }
}

#[cfg(test)]
mod context_tests {
    use super::*;

    #[test]
    fn test_build_context_preamble_empty() {
        let temp = tempfile::TempDir::new().unwrap();
        let result = build_context_preamble(temp.path());
        assert!(result.is_empty());
    }

    #[test]
    fn test_build_context_preamble_with_concepts_and_projects() {
        let temp = tempfile::TempDir::new().unwrap();
        let concepts_dir = temp.path().join("Concepts");
        let projects_dir = temp.path().join("Projects");
        std::fs::create_dir_all(&concepts_dir).unwrap();
        std::fs::create_dir_all(&projects_dir).unwrap();

        std::fs::write(concepts_dir.join("API Design.md"), "# API Design").unwrap();
        std::fs::write(concepts_dir.join("Conway's Law.md"), "# Conway's Law").unwrap();
        std::fs::write(projects_dir.join("Project Atlas.md"), "# Project Atlas").unwrap();

        let result = build_context_preamble(temp.path());
        assert!(result.contains("Existing concepts"));
        assert!(result.contains("- API Design"));
        assert!(result.contains("- Conway's Law"));
        assert!(result.contains("Existing projects"));
        assert!(result.contains("- Project Atlas"));
    }

    #[test]
    fn test_build_context_preamble_ignores_non_md_files() {
        let temp = tempfile::TempDir::new().unwrap();
        let concepts_dir = temp.path().join("Concepts");
        std::fs::create_dir_all(&concepts_dir).unwrap();

        std::fs::write(concepts_dir.join("Real Concept.md"), "").unwrap();
        std::fs::write(concepts_dir.join(".DS_Store"), "").unwrap();
        std::fs::write(concepts_dir.join("notes.txt"), "").unwrap();

        let result = build_context_preamble(temp.path());
        assert!(result.contains("- Real Concept"));
        assert!(!result.contains(".DS_Store"));
        assert!(!result.contains("notes"));
    }
}
