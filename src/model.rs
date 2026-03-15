//! Serde data models for Granola API responses.
//!
//! Tolerant parsing with optional fields and flexible timestamps.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Lightweight document listing from the `/v2/get-documents` endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentSummary {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    /// User's manual notes — ProseMirror doc stored as raw Value to tolerate
    /// malformed structures (e.g. content as map instead of array)
    #[serde(default)]
    pub notes: Option<serde_json::Value>,
    /// AI-generated summary panel — a wrapper object whose `content` field
    /// holds the ProseMirror doc
    #[serde(default)]
    pub last_viewed_panel: Option<serde_json::Value>,
}

impl DocumentSummary {
    /// Extract user notes from the `notes` field.
    /// Falls back to `last_viewed_panel.content` when `notes` is present but
    /// malformed (e.g. ProseMirror `content` is a map instead of an array).
    pub fn user_notes(&self) -> Option<ProseMirrorDoc> {
        self.notes
            .as_ref()
            .and_then(|v| serde_json::from_value::<ProseMirrorDoc>(v.clone()).ok())
            .or_else(|| {
                self.last_viewed_panel
                    .as_ref()
                    .and_then(|v| v.get("content"))
                    .and_then(|v| serde_json::from_value(v.clone()).ok())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_document_summary_deserialize_minimal() {
        let json = r#"{"id": "doc123", "created_at": "2025-10-28T15:04:05Z"}"#;
        let doc: DocumentSummary = serde_json::from_str(json).unwrap();
        assert_eq!(doc.id, "doc123");
        assert!(doc.title.is_none());
        assert!(doc.updated_at.is_none());
    }

    #[test]
    fn test_document_summary_deserialize_full() {
        let json = r#"{
            "id": "doc123",
            "title": "Planning Meeting",
            "created_at": "2025-10-28T15:04:05Z",
            "updated_at": "2025-10-29T01:23:45Z",
            "extra_field": "ignored"
        }"#;
        let doc: DocumentSummary = serde_json::from_str(json).unwrap();
        assert_eq!(doc.id, "doc123");
        assert_eq!(doc.title.as_deref(), Some("Planning Meeting"));
        assert!(doc.updated_at.is_some());
    }
}

/// Rich attendee information from Granola API
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attendee {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<PersonDetails>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonDetails {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub person: Option<PersonInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub company: Option<CompanyInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<PersonName>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub employment: Option<Employment>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub linkedin: Option<LinkedIn>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PersonName {
    #[serde(default, rename = "fullName", skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Employment {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedIn {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub handle: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanyInfo {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

/// Full document metadata from the `/v1/get-document-metadata` endpoint.
///
/// All fields are optional/defaulted because the Granola API sometimes returns
/// minimal responses (e.g. only `creator` + `attendees`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentMetadata {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default = "Utc::now")]
    pub created_at: DateTime<Utc>,
    #[serde(default)]
    pub updated_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub participants: Vec<String>,
    #[serde(default)]
    pub duration_seconds: Option<u64>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub creator: Option<Attendee>,
    #[serde(default)]
    pub attendees: Option<Vec<Attendee>>,
}

#[cfg(test)]
mod metadata_tests {
    use super::*;

    #[test]
    fn test_document_metadata_deserialize() {
        let json = r#"{
            "id": "doc123",
            "title": "Q4 Planning",
            "created_at": "2025-10-28T15:04:05Z",
            "updated_at": "2025-10-29T01:23:45Z",
            "participants": ["Alice", "Bob"],
            "duration_seconds": 3600,
            "labels": ["Planning", "Q4"]
        }"#;
        let meta: DocumentMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.participants.len(), 2);
        assert_eq!(meta.duration_seconds, Some(3600));
        assert_eq!(meta.labels.len(), 2);
        assert!(meta.creator.is_none());
        assert!(meta.attendees.is_none());
    }

    #[test]
    fn test_document_metadata_with_rich_attendees() {
        let json = r#"{
            "id": "doc123",
            "title": "Q4 Planning",
            "created_at": "2025-10-28T15:04:05Z",
            "participants": ["Alice Smith", "Bob Jones"],
            "creator": {
                "name": "Alice Smith",
                "email": "alice@acme.com",
                "details": {
                    "person": {
                        "name": { "fullName": "Alice Smith" },
                        "employment": { "title": "Engineering Manager" },
                        "linkedin": { "handle": "alicesmith" }
                    },
                    "company": { "name": "Acme Corp" }
                }
            },
            "attendees": [
                {
                    "name": "Alice Smith",
                    "email": "alice@acme.com",
                    "details": {
                        "person": {
                            "name": { "fullName": "Alice Smith" },
                            "employment": { "title": "Engineering Manager" },
                            "linkedin": { "handle": "alicesmith" }
                        },
                        "company": { "name": "Acme Corp" }
                    }
                },
                {
                    "name": "Bob Jones",
                    "email": "bob@acme.com"
                }
            ]
        }"#;
        let meta: DocumentMetadata = serde_json::from_str(json).unwrap();

        // Creator
        let creator = meta.creator.as_ref().unwrap();
        assert_eq!(creator.name.as_deref(), Some("Alice Smith"));
        assert_eq!(creator.email.as_deref(), Some("alice@acme.com"));
        let details = creator.details.as_ref().unwrap();
        let person = details.person.as_ref().unwrap();
        assert_eq!(
            person.name.as_ref().unwrap().full_name.as_deref(),
            Some("Alice Smith")
        );
        assert_eq!(
            person.employment.as_ref().unwrap().title.as_deref(),
            Some("Engineering Manager")
        );
        assert_eq!(
            person.linkedin.as_ref().unwrap().handle.as_deref(),
            Some("alicesmith")
        );
        let company = details.company.as_ref().unwrap();
        assert_eq!(company.name.as_deref(), Some("Acme Corp"));

        // Attendees
        let attendees = meta.attendees.as_ref().unwrap();
        assert_eq!(attendees.len(), 2);
        assert_eq!(attendees[1].name.as_deref(), Some("Bob Jones"));
        assert_eq!(attendees[1].email.as_deref(), Some("bob@acme.com"));
        assert!(attendees[1].details.is_none());
    }

    #[test]
    fn test_attendee_minimal() {
        let json = r#"{"name": "Alice"}"#;
        let attendee: Attendee = serde_json::from_str(json).unwrap();
        assert_eq!(attendee.name.as_deref(), Some("Alice"));
        assert!(attendee.email.is_none());
        assert!(attendee.details.is_none());
    }

    #[test]
    fn test_attendee_unknown_fields_tolerated() {
        let json = r#"{
            "name": "Alice",
            "email": "alice@example.com",
            "some_future_field": "should not break",
            "details": {
                "person": {
                    "name": { "fullName": "Alice" },
                    "some_other_field": 42
                }
            }
        }"#;
        let attendee: Attendee = serde_json::from_str(json).unwrap();
        assert_eq!(attendee.name.as_deref(), Some("Alice"));
    }

    #[test]
    fn test_metadata_missing_created_at() {
        // Some API responses omit created_at entirely (only creator + attendees).
        let json = r#"{
            "creator": {"name": "Alice", "email": "alice@example.com"},
            "attendees": [{"name": "Bob"}]
        }"#;
        let meta: DocumentMetadata = serde_json::from_str(json).unwrap();
        assert!(meta.id.is_none());
        assert!(meta.title.is_none());
        // created_at defaults to roughly "now"
        let age = Utc::now() - meta.created_at;
        assert!(age.num_seconds() < 5);
        assert_eq!(meta.attendees.unwrap().len(), 1);
    }

    #[test]
    fn test_metadata_with_unknown_api_fields() {
        let json = r#"{
            "id": "doc123",
            "created_at": "2025-10-28T15:04:05Z",
            "participants": [],
            "some_brand_new_field": {"nested": true},
            "another_future_field": [1, 2, 3]
        }"#;
        let meta: DocumentMetadata = serde_json::from_str(json).unwrap();
        assert_eq!(meta.id.as_deref(), Some("doc123"));
    }
}

/// Response from the official Granola public API GET /v1/notes/{id}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicNote {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub summary_text: Option<String>,
}

/// ProseMirror document root node
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProseMirrorDoc {
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub content: Option<Vec<ProseMirrorNode>>,
}

/// ProseMirror content node (paragraph, heading, text, list, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProseMirrorNode {
    #[serde(rename = "type")]
    pub node_type: String,
    #[serde(default)]
    pub content: Option<Vec<ProseMirrorNode>>,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub attrs: Option<serde_json::Value>,
    #[serde(default)]
    pub marks: Option<Vec<ProseMirrorMark>>,
}

/// ProseMirror inline mark (bold, italic, etc.)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProseMirrorMark {
    #[serde(rename = "type")]
    pub mark_type: String,
}

/// Raw transcript from the `/v1/get-document-transcript` endpoint (array of entries).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawTranscript {
    pub entries: Vec<TranscriptEntry>,
}

/// A single utterance in a transcript.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranscriptEntry {
    #[serde(default)]
    pub document_id: Option<String>,
    #[serde(rename = "start_timestamp", default)]
    pub start: Option<String>,
    #[serde(rename = "end_timestamp", default)]
    pub end: Option<String>,
    pub text: String,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub is_final: Option<bool>,
    #[serde(default)]
    pub speaker: Option<String>,
}

/// Legacy transcript segment type (kept for backward compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub start: Option<TimestampValue>,
    #[serde(default)]
    pub end: Option<TimestampValue>,
    pub text: String,
}

/// Legacy monologue type (kept for backward compatibility).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Monologue {
    #[serde(default)]
    pub speaker: Option<String>,
    #[serde(default)]
    pub start: Option<TimestampValue>,
    pub blocks: Vec<Block>,
}

/// A text block within a monologue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Block {
    pub text: String,
}

/// A timestamp that can be either seconds (f64) or a string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TimestampValue {
    Seconds(f64),
    String(String),
}

#[cfg(test)]
mod transcript_tests {
    use super::*;

    #[test]
    fn test_raw_transcript_deserialize() {
        let json = r#"[
            {
                "document_id": "doc123",
                "speaker": "Alice",
                "start_timestamp": "2025-10-01T21:35:12.500Z",
                "end_timestamp": "2025-10-01T21:35:18.000Z",
                "text": "Hello",
                "source": "microphone",
                "id": "entry1",
                "is_final": true
            }
        ]"#;
        let transcript: RawTranscript = serde_json::from_str(json).unwrap();
        assert_eq!(transcript.entries.len(), 1);
        assert_eq!(transcript.entries[0].text, "Hello");
        assert_eq!(transcript.entries[0].speaker.as_deref(), Some("Alice"));
    }

    #[test]
    fn test_raw_transcript_minimal() {
        let json = r#"[
            {"text": "Just text"}
        ]"#;
        let transcript: RawTranscript = serde_json::from_str(json).unwrap();
        assert_eq!(transcript.entries.len(), 1);
        assert_eq!(transcript.entries[0].text, "Just text");
        assert!(transcript.entries[0].speaker.is_none());
    }
}

/// Frontmatter for Obsidian-flavored markdown files.
/// Designed for Dataview compatibility.
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
    /// Duration in minutes
    #[serde(default, alias = "duration_seconds")]
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

// Keep created_at as alias target for backward compat reads
impl Frontmatter {
    /// Backward-compat accessor: return the `created` field as `created_at`
    pub fn created_at(&self) -> DateTime<Utc> {
        self.created
    }
}

#[cfg(test)]
mod frontmatter_tests {
    use super::*;

    #[test]
    fn test_frontmatter_roundtrip() {
        let fm = Frontmatter {
            doc_id: "doc123".into(),
            source: "granola".into(),
            date: Some("2025-10-28".into()),
            created: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated: Some("2025-10-29T01:23:45Z".parse().unwrap()),
            title: Some("Test Meeting".into()),
            attendees: vec!["Alice".into(), "Bob".into()],
            duration_minutes: Some(60),
            tags: vec!["planning".into()],
            related: vec![],
            status: None,
            generator: "baez".into(),
        };

        let yaml = serde_yaml::to_string(&fm).unwrap();
        let parsed: Frontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.doc_id, "doc123");
        assert_eq!(parsed.attendees.len(), 2);
        assert_eq!(parsed.duration_minutes, Some(60));
        assert_eq!(parsed.tags, vec!["planning"]);
    }

    #[test]
    fn test_frontmatter_backward_compat_old_format() {
        // Old YAML with created_at/participants/labels/duration_seconds should parse
        let yaml = r#"
doc_id: doc123
source: granola
created_at: 2025-10-28T15:04:05Z
title: Old Meeting
participants: [Alice]
duration_seconds: 3600
labels: [Planning]
generator: muesli 1.0
"#;
        let parsed: Frontmatter = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.doc_id, "doc123");
        assert_eq!(parsed.created.to_string(), "2025-10-28 15:04:05 UTC");
        assert_eq!(parsed.attendees, vec!["Alice"]);
        assert_eq!(parsed.duration_minutes, Some(3600)); // Note: old files store seconds here
        assert_eq!(parsed.tags, vec!["Planning"]);
    }

    #[test]
    fn test_frontmatter_with_related_and_status() {
        let fm = Frontmatter {
            doc_id: "doc123".into(),
            source: "granola".into(),
            date: Some("2025-10-28".into()),
            created: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated: None,
            title: Some("Test".into()),
            attendees: vec![],
            duration_minutes: None,
            tags: vec![],
            generator: "baez".into(),
            related: vec!["[[Alice Smith]]".into(), "[[API Design]]".into()],
            status: Some("substantive".into()),
        };

        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(yaml.contains("related:"));
        assert!(yaml.contains("[[Alice Smith]]"));
        assert!(yaml.contains("status: substantive"));

        let parsed: Frontmatter = serde_yaml::from_str(&yaml).unwrap();
        assert_eq!(parsed.related.len(), 2);
        assert_eq!(parsed.status.as_deref(), Some("substantive"));
    }

    #[test]
    fn test_frontmatter_empty_related_not_serialized() {
        let fm = Frontmatter {
            doc_id: "doc123".into(),
            source: "granola".into(),
            date: None,
            created: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated: None,
            title: None,
            attendees: vec![],
            duration_minutes: None,
            tags: vec![],
            generator: "baez".into(),
            related: vec![],
            status: None,
        };

        let yaml = serde_yaml::to_string(&fm).unwrap();
        assert!(!yaml.contains("related:"));
        assert!(!yaml.contains("status:"));
    }

    #[test]
    fn test_frontmatter_backward_compat_no_related_status() {
        let yaml = r#"
doc_id: doc123
source: granola
created: 2025-10-28T15:04:05Z
generator: baez
"#;
        let parsed: Frontmatter = serde_yaml::from_str(yaml).unwrap();
        assert!(parsed.related.is_empty());
        assert!(parsed.status.is_none());
    }

    #[test]
    fn test_frontmatter_new_format() {
        let yaml = r#"
doc_id: doc123
source: granola
date: "2025-10-28"
created: 2025-10-28T15:04:05Z
updated: 2025-10-29T01:23:45Z
title: New Meeting
attendees: [Alice, Bob]
duration_minutes: 60
tags: [planning]
generator: baez
"#;
        let parsed: Frontmatter = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(parsed.doc_id, "doc123");
        assert_eq!(parsed.date.as_deref(), Some("2025-10-28"));
        assert_eq!(parsed.attendees, vec!["Alice", "Bob"]);
        assert_eq!(parsed.duration_minutes, Some(60));
        assert_eq!(parsed.tags, vec!["planning"]);
        assert_eq!(parsed.generator, "baez");
    }
}

#[cfg(test)]
mod public_note_tests {
    use super::*;

    #[test]
    fn test_public_note_deserialize_minimal() {
        let json = r#"{"id": "note-abc-123"}"#;
        let note: PublicNote = serde_json::from_str(json).unwrap();
        assert_eq!(note.id, "note-abc-123");
        assert!(note.title.is_none());
        assert!(note.summary_text.is_none());
    }

    #[test]
    fn test_public_note_deserialize_full() {
        let json = r#"{
            "id": "note-abc-123",
            "title": "Sprint Planning",
            "summary_text": "We discussed Q1 priorities and assigned tasks."
        }"#;
        let note: PublicNote = serde_json::from_str(json).unwrap();
        assert_eq!(note.id, "note-abc-123");
        assert_eq!(note.title.as_deref(), Some("Sprint Planning"));
        assert_eq!(
            note.summary_text.as_deref(),
            Some("We discussed Q1 priorities and assigned tasks.")
        );
    }
}

#[cfg(test)]
mod prosemirror_tests {
    use super::*;

    #[test]
    fn test_prosemirror_paragraph() {
        let json = r#"{
            "type": "doc",
            "content": [
                {
                    "type": "paragraph",
                    "content": [
                        {"type": "text", "text": "Hello world"}
                    ]
                }
            ]
        }"#;
        let doc: ProseMirrorDoc = serde_json::from_str(json).unwrap();
        assert_eq!(doc.node_type, "doc");
        let content = doc.content.as_ref().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].node_type, "paragraph");
        let para_content = content[0].content.as_ref().unwrap();
        assert_eq!(para_content[0].text.as_deref(), Some("Hello world"));
    }

    #[test]
    fn test_prosemirror_empty_doc() {
        let json = r#"{"type": "doc"}"#;
        let doc: ProseMirrorDoc = serde_json::from_str(json).unwrap();
        assert_eq!(doc.node_type, "doc");
        assert!(doc.content.is_none());
    }

    #[test]
    fn test_document_summary_with_notes_and_lvp() {
        let json = r#"{
            "id": "doc123",
            "created_at": "2025-10-28T15:04:05Z",
            "notes": {
                "type": "doc",
                "content": [{"type": "paragraph", "content": [{"type": "text", "text": "User notes"}]}]
            },
            "last_viewed_panel": {
                "title": "Summary",
                "content": {
                    "type": "doc",
                    "content": [{"type": "heading", "attrs": {"level": 3}, "content": [{"type": "text", "text": "Key Points"}]}]
                }
            }
        }"#;
        let doc: DocumentSummary = serde_json::from_str(json).unwrap();

        let user = doc.user_notes().unwrap();
        assert_eq!(user.node_type, "doc");
    }

    #[test]
    fn test_document_summary_without_any_notes() {
        let json = r#"{"id": "doc123", "created_at": "2025-10-28T15:04:05Z"}"#;
        let doc: DocumentSummary = serde_json::from_str(json).unwrap();
        assert!(doc.user_notes().is_none());
    }
}
