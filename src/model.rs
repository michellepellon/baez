//! Serde data models for the Granola public API and Obsidian frontmatter.
//!
//! All API models use `#[serde(default)]` on optional fields for forward compat.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A user reference in the public API. Returned for `owner` and each `attendee`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct User {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
}

/// Calendar event linked to a note. The public API exposes start/end times here
/// (the only source of meeting duration on the public API).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CalendarEvent {
    #[serde(default)]
    pub start: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end: Option<DateTime<Utc>>,
}

/// Folder a note belongs to.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Folder {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub parent_folder_id: Option<String>,
}

/// Speaker info on a transcript entry. `source` is "microphone" or "speaker"
/// indicating which audio channel produced the line; `diarization_label` is the
/// per-speaker label when diarization is enabled.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Speaker {
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub diarization_label: Option<String>,
}

/// A single transcript line from the public API.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct TranscriptEntry {
    #[serde(default)]
    pub speaker: Option<Speaker>,
    pub text: String,
    #[serde(default)]
    pub start_time: Option<DateTime<Utc>>,
    #[serde(default)]
    pub end_time: Option<DateTime<Utc>>,
}

/// Lightweight note summary returned by `GET /v1/notes` (list endpoint).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NoteSummary {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub owner: Option<User>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Full note returned by `GET /v1/notes/{id}` with `?include=transcript`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Note {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub owner: Option<User>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub web_url: Option<String>,
    #[serde(default)]
    pub calendar_event: Option<CalendarEvent>,
    #[serde(default)]
    pub attendees: Vec<User>,
    #[serde(default)]
    pub folder_membership: Vec<Folder>,
    #[serde(default)]
    pub summary_text: Option<String>,
    #[serde(default)]
    pub summary_markdown: Option<String>,
    #[serde(default)]
    pub transcript: Option<Vec<TranscriptEntry>>,
}

impl Note {
    /// Derive meeting duration in seconds from the linked calendar event, if both
    /// start and end are present. Returns `None` otherwise — the public API does
    /// not expose a direct duration field.
    pub fn duration_seconds(&self) -> Option<i64> {
        let cal = self.calendar_event.as_ref()?;
        let start = cal.start?;
        let end = cal.end?;
        Some((end - start).num_seconds().max(0))
    }

    /// Attendee names with email as fallback, suitable for frontmatter and
    /// the metadata line in markdown.
    pub fn attendee_names(&self) -> Vec<String> {
        self.attendees
            .iter()
            .filter_map(|u| u.name.clone().or_else(|| u.email.clone()))
            .collect()
    }
}

/// Response from the list-notes endpoint.
#[derive(Debug, Clone, Deserialize)]
pub struct ListNotesResponse {
    pub notes: Vec<NoteSummary>,
    #[serde(rename = "hasMore")]
    pub has_more: bool,
    #[serde(default)]
    pub cursor: Option<String>,
}

/// Obsidian frontmatter for a meeting note. Dataview-compatible.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Frontmatter {
    pub doc_id: String,
    pub source: String,
    /// YYYY-MM-DD date string for Dataview
    #[serde(default)]
    pub date: Option<String>,
    /// Full ISO timestamp
    #[serde(alias = "created_at")]
    pub created: DateTime<Utc>,
    #[serde(default, alias = "remote_updated_at")]
    pub updated: Option<DateTime<Utc>>,
    #[serde(default)]
    pub title: Option<String>,
    /// Flat list of attendee names (Dataview-queryable)
    #[serde(default, alias = "participants")]
    pub attendees: Vec<String>,
    /// Duration in minutes, derived from calendar_event
    #[serde(default)]
    pub duration_minutes: Option<u64>,
    #[serde(default, alias = "labels")]
    pub tags: Vec<String>,
    /// Wiki-linked entity references for Obsidian graph connectivity
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
    /// Transcript quality: "substantive" or "stub"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    pub generator: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_note_duration_from_calendar_event() {
        let note = Note {
            id: "not_abc".into(),
            title: None,
            owner: None,
            created_at: "2026-05-11T10:00:00Z".parse().unwrap(),
            updated_at: None,
            web_url: None,
            calendar_event: Some(CalendarEvent {
                start: Some("2026-05-11T10:00:00Z".parse().unwrap()),
                end: Some("2026-05-11T10:30:00Z".parse().unwrap()),
            }),
            attendees: vec![],
            folder_membership: vec![],
            summary_text: None,
            summary_markdown: None,
            transcript: None,
        };
        assert_eq!(note.duration_seconds(), Some(1800));
    }

    #[test]
    fn test_note_duration_missing_calendar() {
        let note = Note {
            id: "not_abc".into(),
            title: None,
            owner: None,
            created_at: "2026-05-11T10:00:00Z".parse().unwrap(),
            updated_at: None,
            web_url: None,
            calendar_event: None,
            attendees: vec![],
            folder_membership: vec![],
            summary_text: None,
            summary_markdown: None,
            transcript: None,
        };
        assert_eq!(note.duration_seconds(), None);
    }

    #[test]
    fn test_attendee_names_falls_back_to_email() {
        let note = Note {
            id: "not_abc".into(),
            title: None,
            owner: None,
            created_at: "2026-05-11T10:00:00Z".parse().unwrap(),
            updated_at: None,
            web_url: None,
            calendar_event: None,
            attendees: vec![
                User {
                    name: Some("Alice".into()),
                    email: Some("alice@example.com".into()),
                },
                User {
                    name: None,
                    email: Some("bob@example.com".into()),
                },
                User {
                    name: None,
                    email: None,
                },
            ],
            folder_membership: vec![],
            summary_text: None,
            summary_markdown: None,
            transcript: None,
        };
        assert_eq!(note.attendee_names(), vec!["Alice", "bob@example.com"]);
    }

    #[test]
    fn test_list_notes_response_parses() {
        let json = r#"{
            "notes": [
                {
                    "id": "not_abc",
                    "title": "Test",
                    "created_at": "2026-05-11T10:00:00Z",
                    "updated_at": "2026-05-11T11:00:00Z"
                }
            ],
            "hasMore": false,
            "cursor": null
        }"#;
        let parsed: ListNotesResponse = serde_json::from_str(json).unwrap();
        assert_eq!(parsed.notes.len(), 1);
        assert!(!parsed.has_more);
        assert_eq!(parsed.cursor, None);
    }

    #[test]
    fn test_note_parses_with_transcript() {
        let json = r###"{
            "id": "not_abc",
            "title": "Test Meeting",
            "owner": {"name": "Owner", "email": "owner@example.com"},
            "created_at": "2026-05-11T10:00:00Z",
            "updated_at": "2026-05-11T11:00:00Z",
            "attendees": [{"name": "Alice", "email": "alice@example.com"}],
            "summary_markdown": "## Summary\n- thing",
            "transcript": [
                {"speaker": {"source": "microphone"}, "text": "Hello", "start_time": "2026-05-11T10:00:01Z"}
            ]
        }"###;
        let note: Note = serde_json::from_str(json).unwrap();
        assert_eq!(note.id, "not_abc");
        assert_eq!(note.attendees.len(), 1);
        assert!(note.transcript.is_some());
        assert_eq!(note.transcript.unwrap().len(), 1);
    }
}
