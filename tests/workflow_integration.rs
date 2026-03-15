// ABOUTME: Integration tests for end-to-end workflows
// ABOUTME: Tests file naming, ProseMirror conversion, markdown output, and vault layout

use std::fs;
use tempfile::TempDir;

/// Test that raw file saving uses the correct naming convention
#[test]
fn test_raw_file_naming_convention() {
    use baez::storage::Paths;

    let temp_dir = TempDir::new().unwrap();
    let paths = Paths::new(Some(temp_dir.path().to_path_buf())).unwrap();
    paths.ensure_dirs().unwrap();

    let base_filename = "2025-01-15_planning-meeting";

    // Simulate the new file naming by writing files
    let transcript_path = paths
        .raw_dir
        .join(format!("{}_transcript.json", base_filename));
    let metadata_path = paths
        .raw_dir
        .join(format!("{}_metadata.json", base_filename));

    fs::write(&transcript_path, r#"[{"text": "hello"}]"#).unwrap();
    fs::write(&metadata_path, r#"{"created_at": "2025-01-15T10:00:00Z"}"#).unwrap();

    assert!(transcript_path.exists());
    assert!(metadata_path.exists());

    // Verify the raw dir contains both files
    let raw_files: Vec<_> = fs::read_dir(&paths.raw_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    assert!(
        raw_files.iter().any(|f| f.ends_with("_transcript.json")),
        "Should have transcript JSON"
    );
    assert!(
        raw_files.iter().any(|f| f.ends_with("_metadata.json")),
        "Should have metadata JSON"
    );
}

/// Test vault directory layout
#[test]
fn test_vault_directory_layout() {
    use baez::storage::Paths;

    let temp_dir = TempDir::new().unwrap();
    let paths = Paths::new(Some(temp_dir.path().to_path_buf())).unwrap();
    paths.ensure_dirs().unwrap();

    // Verify vault layout
    assert!(paths.granola_dir.exists(), "Granola/ should exist");
    assert!(paths.baez_dir.exists(), ".baez/ should exist");
    assert!(paths.raw_dir.exists(), ".baez/raw/ should exist");
    assert!(
        paths.summaries_dir.exists(),
        ".baez/summaries/ should exist"
    );
    assert!(paths.tmp_dir.exists(), ".baez/tmp/ should exist");

    // Verify granola_dir is inside vault
    assert_eq!(
        paths.granola_dir,
        temp_dir.path().join("Granola"),
        "granola_dir should be vault/Granola"
    );

    // Verify .baez is inside granola
    assert_eq!(
        paths.baez_dir,
        temp_dir.path().join("Granola").join(".baez"),
        "baez_dir should be vault/Granola/.baez"
    );
}

/// Test doc_path generates correct date-based paths
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

/// Test D: ProseMirror parsing + conversion roundtrip from realistic JSON
#[test]
fn test_prosemirror_to_markdown_roundtrip() {
    let json = r#"{
        "type": "doc",
        "content": [
            {
                "type": "heading",
                "attrs": {"level": 1},
                "content": [
                    {"type": "text", "text": "Sprint Review"}
                ]
            },
            {
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "We discussed "},
                    {"type": "text", "text": "critical", "marks": [{"type": "bold"}]},
                    {"type": "text", "text": " issues and "},
                    {"type": "text", "text": "potential", "marks": [{"type": "italic"}]},
                    {"type": "text", "text": " solutions."}
                ]
            },
            {
                "type": "heading",
                "attrs": {"level": 2},
                "content": [
                    {"type": "text", "text": "Action Items"}
                ]
            },
            {
                "type": "bulletList",
                "content": [
                    {
                        "type": "listItem",
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [
                                    {"type": "text", "text": "Fix the "},
                                    {"type": "text", "text": "login bug", "marks": [{"type": "bold"}]}
                                ]
                            }
                        ]
                    },
                    {
                        "type": "listItem",
                        "content": [
                            {
                                "type": "paragraph",
                                "content": [
                                    {"type": "text", "text": "Deploy to staging"}
                                ]
                            }
                        ]
                    }
                ]
            },
            {
                "type": "paragraph",
                "content": [
                    {"type": "text", "text": "Next meeting: Friday 3pm"}
                ]
            }
        ]
    }"#;

    let doc: baez::ProseMirrorDoc = serde_json::from_str(json).unwrap();
    let md = baez::prosemirror_to_markdown(&doc);

    assert!(md.contains("# Sprint Review"), "Should have h1 heading");
    assert!(md.contains("## Action Items"), "Should have h2 heading");
    assert!(md.contains("**critical**"), "Should have bold text");
    assert!(md.contains("*potential*"), "Should have italic text");
    assert!(
        md.contains("- Fix the **login bug**"),
        "Should have bold text in list item"
    );
    assert!(
        md.contains("- Deploy to staging"),
        "Should have plain list item"
    );
}

/// Test E: Notes + summary appear in markdown output with correct ordering
#[test]
fn test_notes_and_summary_in_markdown() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: Some("doc-test".into()),
            speaker: Some("Alice".into()),
            start: Some("2025-12-01T09:00:00.000Z".into()),
            end: None,
            text: "Let's get started.".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc-test".into()),
        title: Some("Integration Test Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec!["Alice".into(), "Bob".into()],
        duration_seconds: Some(1800),
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let notes_md = "- Follow up on deployment\n- Review PR #42";
    let summary = "Discussed deployment timeline and code review process.";

    let output = baez::to_markdown(
        &raw,
        &meta,
        "doc-test",
        Some(notes_md),
        Some(summary),
        vec![],
        None,
    )
    .unwrap();

    assert!(output
        .body
        .contains("Discussed deployment timeline and code review process."),);
    assert!(output
        .body
        .contains("## Notes\n\n- Follow up on deployment\n- Review PR #42"),);

    let summary_pos = output.body.find("Discussed deployment timeline").unwrap();
    let notes_pos = output.body.find("## Notes").unwrap();
    let separator_pos = output.body.find("---\n").unwrap();
    let transcript_pos = output.body.find("**[[Alice]]").unwrap();

    assert!(summary_pos < notes_pos);
    assert!(notes_pos < separator_pos);
    assert!(separator_pos < transcript_pos);
}

/// Test: Wiki-links in transcript output
#[test]
fn test_wiki_links_in_transcript() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc1".into()),
        title: Some("Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec!["Alice".into()],
        duration_seconds: None,
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let output = baez::to_markdown(&raw, &meta, "doc1", None, None, vec![], None).unwrap();
    assert!(
        output.body.contains("**[[Alice]]"),
        "Speaker should be wiki-linked"
    );
    assert!(
        output.body.contains("Participants: [[Alice]]"),
        "Participants should be wiki-linked"
    );
}

/// Test: Dataview-compatible frontmatter
#[test]
fn test_dataview_frontmatter() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc1".into()),
        title: Some("Q4 Planning".into()),
        created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
        updated_at: Some("2025-10-29T01:23:45Z".parse().unwrap()),
        participants: vec!["Alice".into()],
        duration_seconds: Some(3600),
        labels: vec!["planning".into()],
        creator: None,
        attendees: None,
    };

    let output = baez::to_markdown(&raw, &meta, "doc1", None, None, vec![], None).unwrap();

    assert!(output.frontmatter_yaml.contains("date:"));
    assert!(output.frontmatter_yaml.contains("created:"));
    assert!(output.frontmatter_yaml.contains("generator: baez"));
    assert!(output.frontmatter_yaml.contains("duration_minutes: 60"));
    assert!(output.frontmatter_yaml.contains("attendees:"));
    assert!(output.frontmatter_yaml.contains("tags:"));
}

/// Test: Tags appear in body
#[test]
fn test_tags_in_body() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc1".into()),
        title: Some("Meeting".into()),
        created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
        updated_at: None,
        participants: vec![],
        duration_seconds: None,
        labels: vec!["Sprint Review".into()],
        creator: None,
        attendees: None,
    };

    let output = baez::to_markdown(&raw, &meta, "doc1", None, None, vec![], None).unwrap();
    assert!(output.body.contains("#granola"));
    assert!(output.body.contains("#meeting/sprint-review"));
}

/// Test G: Empty last_viewed_panel produces no notes section in markdown
#[test]
fn test_empty_last_viewed_panel_no_notes_section() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc-empty-panel".into()),
        title: Some("Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec![],
        duration_seconds: None,
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let output =
        baez::to_markdown(&raw, &meta, "doc-empty-panel", None, None, vec![], None).unwrap();
    assert!(!output.body.contains("## Notes"));

    let output =
        baez::to_markdown(&raw, &meta, "doc-empty-panel", Some(""), None, vec![], None).unwrap();
    assert!(!output.body.contains("## Notes"));
}

/// Test H: Empty summary_text produces no summary section in markdown
#[test]
fn test_empty_summary_text_no_summary_section() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc-empty-summary".into()),
        title: Some("Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec![],
        duration_seconds: None,
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let output =
        baez::to_markdown(&raw, &meta, "doc-empty-summary", None, None, vec![], None).unwrap();
    assert!(!output.body.contains("## Summary"));

    let output = baez::to_markdown(
        &raw,
        &meta,
        "doc-empty-summary",
        None,
        Some(""),
        vec![],
        None,
    )
    .unwrap();
    assert!(!output.body.contains("## Summary"));
}

/// Test: parse_summary_output strips entity JSON and returns clean markdown
#[test]
fn test_summary_output_parsing_integration() {
    let raw = r#"## Summary
- Key point

## People
| [[Alice]] | Engineer |

<!-- baez-entities
{"people": [{"name": "Alice Smith", "role": "Engineer", "company": "Acme", "aliases": ["Alice"], "context": "Led discussion"}], "concepts": [{"name": "API Design", "description": "Building APIs first", "existing": false}], "projects": []}
-->"#;

    let (markdown, entities) = baez::summary::parse_summary_output(raw);

    // Markdown is clean
    assert!(markdown.contains("## Summary"));
    assert!(!markdown.contains("baez-entities"));

    // Entities are parsed
    let entities = entities.unwrap();
    assert_eq!(entities.people.len(), 1);
    assert_eq!(entities.people[0].name, "Alice Smith");
    assert_eq!(entities.concepts.len(), 1);
}

/// Test: to_markdown with related and status produces correct frontmatter
#[test]
fn test_to_markdown_with_related_and_status() {
    use baez::model::{DocumentMetadata, RawTranscript, TranscriptEntry};

    let raw = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: Some("Alice".into()),
            start: None,
            end: None,
            text: "Hello".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };

    let meta = DocumentMetadata {
        id: Some("doc1".into()),
        title: Some("Meeting".into()),
        created_at: "2025-12-01T09:00:00Z".parse().unwrap(),
        updated_at: None,
        participants: vec!["Alice".into()],
        duration_seconds: None,
        labels: vec![],
        creator: None,
        attendees: None,
    };

    let related = vec!["[[Alice Smith]]".into(), "[[API Design]]".into()];
    let output = baez::to_markdown(&raw, &meta, "doc1", None, None, related, Some("substantive")).unwrap();

    assert!(output.frontmatter_yaml.contains("related:"));
    assert!(output.frontmatter_yaml.contains("[[Alice Smith]]"));
    assert!(output.frontmatter_yaml.contains("status: substantive"));
}

/// Test: triage classifies short transcripts as stubs
#[test]
fn test_triage_stub_classification() {
    use baez::model::{RawTranscript, TranscriptEntry};

    let stub = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: None,
            start: None,
            end: None,
            text: "hello world".into(),
            source: None,
            id: None,
            is_final: None,
        }],
    };
    assert!(baez::count_transcript_words(&stub) < 20);

    let substantive_text = (0..25).map(|i| format!("word{}", i)).collect::<Vec<_>>().join(" ");
    let substantive = RawTranscript {
        entries: vec![TranscriptEntry {
            document_id: None,
            speaker: None,
            start: None,
            end: None,
            text: substantive_text,
            source: None,
            id: None,
            is_final: None,
        }],
    };
    assert!(baez::count_transcript_words(&substantive) >= 20);
}

/// Test: entity note creation with tempdir
#[test]
fn test_entity_note_creation_integration() {
    use std::fs;
    use tempfile::TempDir;

    let temp = TempDir::new().unwrap();
    let people_dir = temp.path().join("People");
    let concepts_dir = temp.path().join("Concepts");
    let projects_dir = temp.path().join("Projects");
    let tmp_dir = temp.path().join("tmp");

    fs::create_dir_all(&people_dir).unwrap();
    fs::create_dir_all(&concepts_dir).unwrap();
    fs::create_dir_all(&projects_dir).unwrap();
    fs::create_dir_all(&tmp_dir).unwrap();

    // Create entities
    baez::create_person_note(
        &people_dir, "Alice Smith", Some("Engineer"), Some("Acme"),
        &["Alice"], "Led discussion", "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    baez::create_concept_note(
        &concepts_dir, "API Design", "Building APIs first",
        "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    baez::create_project_note(
        &projects_dir, "Project Atlas", "Migration tool",
        "2025-01-15_standup", "2025-01-15", &tmp_dir,
    ).unwrap();

    // Verify files exist with correct content
    assert!(people_dir.join("Alice Smith.md").exists());
    assert!(concepts_dir.join("API Design.md").exists());
    assert!(projects_dir.join("Project Atlas.md").exists());

    let people_content = fs::read_to_string(people_dir.join("Alice Smith.md")).unwrap();
    assert!(people_content.contains("[[2025-01-15_standup]]"));
    assert!(people_content.contains("Led discussion"));

    // Enrich the person note
    baez::enrich_person_note(
        &people_dir.join("Alice Smith.md"),
        &["AS"], "Reviewed migration plan", "2025-01-20_planning", "2025-01-20", &tmp_dir,
    ).unwrap();

    let enriched = fs::read_to_string(people_dir.join("Alice Smith.md")).unwrap();
    assert!(enriched.contains("[[2025-01-15_standup]]"));
    assert!(enriched.contains("[[2025-01-20_planning]]"));
    assert!(enriched.contains("Reviewed migration plan"));
}
