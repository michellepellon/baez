// ABOUTME: Integration tests for end-to-end workflows
// ABOUTME: Tests vault layout, file paths, and the to_markdown contract end-to-end

use std::fs;
use tempfile::TempDir;

#[test]
fn test_vault_directory_layout() {
    use baez::storage::Paths;

    let temp_dir = TempDir::new().unwrap();
    let paths = Paths::new(Some(temp_dir.path().to_path_buf())).unwrap();
    paths.ensure_dirs().unwrap();

    assert!(paths.granola_dir.exists(), "Granola/ should exist");
    assert!(paths.baez_dir.exists(), ".baez/ should exist");
    assert!(paths.raw_dir.exists(), ".baez/raw/ should exist");
    assert!(
        paths.summaries_dir.exists(),
        ".baez/summaries/ should exist"
    );
    assert!(paths.tmp_dir.exists(), ".baez/tmp/ should exist");

    assert_eq!(paths.granola_dir, temp_dir.path().join("Granola"));
    assert_eq!(
        paths.baez_dir,
        temp_dir.path().join("Granola").join(".baez")
    );
}

#[test]
fn test_doc_path_date_folders() {
    use baez::storage::Paths;
    use chrono::{DateTime, Utc};

    let temp_dir = TempDir::new().unwrap();
    let paths = Paths::new(Some(temp_dir.path().to_path_buf())).unwrap();

    let created_at: DateTime<Utc> = "2025-03-15T10:00:00Z".parse().unwrap();
    let path = paths.doc_path(&created_at, "team-standup");

    assert_eq!(
        path,
        temp_dir
            .path()
            .join("Granola")
            .join("2025")
            .join("03")
            .join("2025-03-15_team-standup.md")
    );
}

#[test]
fn test_raw_file_naming_uses_note_suffix() {
    use baez::storage::Paths;

    let temp_dir = TempDir::new().unwrap();
    let paths = Paths::new(Some(temp_dir.path().to_path_buf())).unwrap();
    paths.ensure_dirs().unwrap();

    let base = "2025-01-15_planning";
    let note_path = paths.raw_dir.join(format!("{}_note.json", base));
    fs::write(&note_path, r#"{"id":"not_abc"}"#).unwrap();

    assert!(note_path.exists());
}

#[test]
fn test_to_markdown_full_shape() {
    use baez::model::{CalendarEvent, Note, Speaker, TranscriptEntry, User};

    let note = Note {
        id: "not_abc123def456".into(),
        title: Some("Sprint Planning".into()),
        owner: Some(User {
            name: Some("Owner".into()),
            email: None,
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
            text: "Let's plan the sprint.".into(),
            start_time: Some("2026-05-11T10:00:05Z".parse().unwrap()),
            end_time: None,
        }]),
    };

    let out = baez::convert::to_markdown(
        &note,
        Some("## Summary\n- Plan sprint"),
        vec!["[[Alice]]".into()],
        Some("substantive"),
    )
    .unwrap();

    // Frontmatter
    assert!(out.frontmatter_yaml.contains("doc_id: not_abc123def456"));
    assert!(out.frontmatter_yaml.contains("title: Sprint Planning"));
    assert!(out.frontmatter_yaml.contains("status: substantive"));
    assert!(out.frontmatter_yaml.contains("duration_minutes: 30"));
    assert!(out.frontmatter_yaml.contains("[[Alice]]"));
    assert!(out.frontmatter_yaml.contains("generator: baez"));

    // Body
    assert!(out.body.contains("# Sprint Planning"));
    assert!(out.body.contains("Date: 2026-05-11"));
    assert!(out.body.contains("Duration: 30m"));
    assert!(out.body.contains("Participants: [[Alice]], [[Bob]]"));
    assert!(out.body.contains("#granola"));
    assert!(out.body.contains("## Summary"));
    assert!(out.body.contains("Plan sprint"));
    assert!(out
        .body
        .contains("**Speaker 1 (00:00:05):** Let's plan the sprint."));
}

#[test]
fn test_to_markdown_stub_no_summary() {
    use baez::model::Note;

    let note = Note {
        id: "not_short".into(),
        title: Some("Empty".into()),
        owner: None,
        created_at: "2026-05-11T10:00:00Z".parse().unwrap(),
        updated_at: None,
        web_url: None,
        calendar_event: None,
        attendees: vec![],
        folder_membership: vec![],
        summary_text: None,
        summary_markdown: None,
        transcript: Some(vec![]),
    };

    let out = baez::convert::to_markdown(&note, None, vec![], Some("stub")).unwrap();
    assert!(out.frontmatter_yaml.contains("status: stub"));
    assert!(!out.body.contains("## Summary"));
    assert!(out.body.contains("_No transcript content available._"));
}

#[cfg(feature = "summaries")]
#[test]
fn test_summary_output_parsing_integration() {
    let raw = r#"## Summary
- Bullet one
- Bullet two

## Action Items
- [ ] [owner:: [[Alice Smith]]] [action:: Deploy by Friday]

<!-- baez-entities
{
  "people": [{"name": "Alice Smith", "role": "engineer", "company": "Acme", "aliases": [], "context": "Lead on deploy"}],
  "concepts": [],
  "projects": []
}
-->"#;

    let (markdown, entities) = baez::parse_summary_output(raw);

    assert!(markdown.contains("## Summary"));
    assert!(markdown.contains("Bullet one"));
    assert!(!markdown.contains("baez-entities"));

    let entities = entities.expect("entity JSON should parse");
    assert_eq!(entities.people.len(), 1);
    assert_eq!(entities.people[0].name, "Alice Smith");
}
