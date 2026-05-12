//! Converts public-API note data into Obsidian-flavored Markdown.
//!
//! Produces a markdown file with Dataview frontmatter, `[[wiki-links]]` for
//! attendees, the AI summary, and the transcript.

use crate::{Error, Frontmatter, Note, Result};
use chrono::{DateTime, Utc};

/// Output produced by `to_markdown`. Caller is responsible for joining
/// `frontmatter_yaml` and `body` with the `---` delimiters.
pub struct MarkdownOutput {
    pub frontmatter_yaml: String,
    pub body: String,
}

/// Convert a `Note` into Obsidian-flavored markdown with YAML frontmatter.
///
/// - `summary_text`: baez's own Claude-generated summary (Granola's `summary_markdown`
///   is not used — we generate our own with the knowledge-graph entity block).
/// - `related`: wiki-link strings to insert into frontmatter (entity backlinks).
/// - `status`: `"substantive"` or `"stub"` based on transcript word count.
pub fn to_markdown(
    note: &Note,
    summary_text: Option<&str>,
    related: Vec<String>,
    status: Option<&str>,
) -> Result<MarkdownOutput> {
    let attendee_names = note.attendee_names();
    let duration_minutes = note.duration_seconds().map(|s| (s / 60).max(0) as u64);
    let date_str = note.created_at.format("%Y-%m-%d").to_string();

    let frontmatter = Frontmatter {
        doc_id: note.id.clone(),
        source: "granola".into(),
        date: Some(date_str),
        created: note.created_at,
        updated: note.updated_at,
        title: note.title.clone(),
        attendees: attendee_names.clone(),
        duration_minutes,
        tags: vec![],
        related,
        status: status.map(|s| s.to_string()),
        generator: "baez".into(),
    };

    let frontmatter_yaml = serde_yaml::to_string(&frontmatter).map_err(|e| {
        Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize frontmatter: {}", e),
        ))
    })?;

    let title = note.title.as_deref().unwrap_or("Untitled Meeting");
    let mut body = format!("# {}\n\n", title);

    let date = note.created_at.format("%Y-%m-%d");
    let mut meta_parts = vec![format!("Date: {}", date)];
    if let Some(secs) = note.duration_seconds() {
        meta_parts.push(format!("Duration: {}m", secs / 60));
    }
    if !attendee_names.is_empty() {
        let wiki_names: Vec<String> = attendee_names
            .iter()
            .map(|n| format!("[[{}]]", n))
            .collect();
        meta_parts.push(format!("Participants: {}", wiki_names.join(", ")));
    }
    body.push_str(&format!("_{}_\n\n", meta_parts.join(" | ")));

    // Baseline #granola tag. Per-meeting labels are not exposed on the public API.
    body.push_str("#granola\n\n");

    if let Some(summary) = summary_text {
        if !summary.is_empty() {
            body.push_str(summary);
            if !summary.ends_with('\n') {
                body.push('\n');
            }
            body.push('\n');
        }
    }

    body.push_str("---\n\n");

    match &note.transcript {
        Some(entries) if !entries.is_empty() => {
            for entry in entries {
                let speaker_label = entry
                    .speaker
                    .as_ref()
                    .and_then(|s| s.diarization_label.clone())
                    .unwrap_or_else(|| "Speaker".to_string());
                let timestamp = entry
                    .start_time
                    .map(|t| format_offset(t, note.created_at))
                    .map(|ts| format!(" ({})", ts))
                    .unwrap_or_default();
                body.push_str(&format!(
                    "**{}{}:** {}\n",
                    speaker_label, timestamp, entry.text
                ));
            }
        }
        _ => {
            body.push_str("_No transcript content available._\n");
        }
    }

    Ok(MarkdownOutput {
        frontmatter_yaml,
        body,
    })
}

/// Format a timestamp as `HH:MM:SS` offset from the meeting's start.
fn format_offset(timestamp: DateTime<Utc>, meeting_start: DateTime<Utc>) -> String {
    let offset_secs = (timestamp - meeting_start).num_seconds().max(0);
    let h = offset_secs / 3600;
    let m = (offset_secs % 3600) / 60;
    let s = offset_secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{CalendarEvent, Speaker, TranscriptEntry, User};

    fn sample_note() -> Note {
        Note {
            id: "not_abc123".into(),
            title: Some("Sprint Planning".into()),
            owner: Some(User {
                name: Some("Owner".into()),
                email: Some("owner@example.com".into()),
            }),
            created_at: "2026-05-11T10:00:00Z".parse().unwrap(),
            updated_at: Some("2026-05-11T11:00:00Z".parse().unwrap()),
            web_url: None,
            calendar_event: Some(CalendarEvent {
                start: Some("2026-05-11T10:00:00Z".parse().unwrap()),
                end: Some("2026-05-11T10:30:00Z".parse().unwrap()),
            }),
            attendees: vec![
                User {
                    name: Some("Alice".into()),
                    email: None,
                },
                User {
                    name: Some("Bob".into()),
                    email: None,
                },
            ],
            folder_membership: vec![],
            summary_text: None,
            summary_markdown: None,
            transcript: Some(vec![TranscriptEntry {
                speaker: Some(Speaker {
                    source: Some("microphone".into()),
                    diarization_label: Some("Speaker 1".into()),
                }),
                text: "Hello, team.".into(),
                start_time: Some("2026-05-11T10:00:05Z".parse().unwrap()),
                end_time: None,
            }]),
        }
    }

    #[test]
    fn test_to_markdown_basic_shape() {
        let note = sample_note();
        let out = to_markdown(&note, None, vec![], Some("substantive")).unwrap();
        assert!(out.frontmatter_yaml.contains("doc_id: not_abc123"));
        assert!(out.frontmatter_yaml.contains("duration_minutes: 30"));
        assert!(out.frontmatter_yaml.contains("status: substantive"));
        assert!(out.body.contains("# Sprint Planning"));
        assert!(out.body.contains("[[Alice]]"));
        assert!(out.body.contains("[[Bob]]"));
        assert!(out.body.contains("#granola"));
        assert!(out.body.contains("Duration: 30m"));
    }

    #[test]
    fn test_to_markdown_transcript_offset() {
        let note = sample_note();
        let out = to_markdown(&note, None, vec![], None).unwrap();
        // The transcript entry starts 5 seconds after the meeting.
        assert!(out.body.contains("(00:00:05)"));
        assert!(out.body.contains("Speaker 1"));
        assert!(out.body.contains("Hello, team."));
    }

    #[test]
    fn test_to_markdown_empty_transcript() {
        let mut note = sample_note();
        note.transcript = Some(vec![]);
        let out = to_markdown(&note, None, vec![], None).unwrap();
        assert!(out.body.contains("_No transcript content available._"));
    }

    #[test]
    fn test_to_markdown_with_summary_and_related() {
        let note = sample_note();
        let out = to_markdown(
            &note,
            Some("## Summary\n- Did stuff"),
            vec!["[[Alice]]".into(), "[[Project X]]".into()],
            Some("substantive"),
        )
        .unwrap();
        assert!(out.body.contains("## Summary"));
        assert!(out.frontmatter_yaml.contains("related"));
        assert!(out.frontmatter_yaml.contains("[[Alice]]"));
    }

    #[test]
    fn test_to_markdown_missing_calendar_no_duration() {
        let mut note = sample_note();
        note.calendar_event = None;
        let out = to_markdown(&note, None, vec![], None).unwrap();
        assert!(!out.body.contains("Duration:"));
        // Frontmatter should not have duration_minutes set
        assert!(!out.frontmatter_yaml.contains("duration_minutes: 30"));
    }
}
