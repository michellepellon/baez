//! Converts raw transcript data to Obsidian-flavored Markdown.
//!
//! Supports `[[wiki-links]]`, `#tags`, Dataview frontmatter, and ProseMirror conversion.

use crate::model::{Attendee, ProseMirrorDoc, ProseMirrorNode};
use crate::util::normalize_timestamp;
use crate::{DocumentMetadata, Frontmatter, RawTranscript, Result};

/// Formats a single attendee line for the Participants section using [[wiki-links]].
fn format_attendee_line(attendee: &Attendee) -> Option<String> {
    let name = attendee.name.as_deref()?;
    let mut parts = vec![format!("[[{}]]", name)];

    if let Some(ref details) = attendee.details {
        if let Some(ref person) = details.person {
            if let Some(ref emp) = person.employment {
                if let Some(ref title) = emp.title {
                    parts.push(title.clone());
                }
            }
        }
        if let Some(ref company) = details.company {
            if let Some(ref company_name) = company.name {
                parts.push(company_name.clone());
            }
        }
    }

    Some(parts.join(", "))
}

/// Format labels as Obsidian tags: lowercase, spaces to hyphens, prefixed with #meeting/.
/// Always includes #granola.
fn format_tags(labels: &[String]) -> Vec<String> {
    let mut tags = vec!["#granola".to_string()];
    for label in labels {
        let tag = label.to_lowercase().replace(' ', "-");
        tags.push(format!("#meeting/{}", tag));
    }
    tags
}

/// The result of converting a transcript to markdown: separate frontmatter and body.
pub struct MarkdownOutput {
    pub frontmatter_yaml: String,
    pub body: String,
}

/// Convert a transcript and metadata into Obsidian-flavored markdown with YAML frontmatter.
pub fn to_markdown(
    raw: &RawTranscript,
    meta: &DocumentMetadata,
    doc_id: &str,
    notes: Option<&str>,
    summary_text: Option<&str>,
) -> Result<MarkdownOutput> {
    // Flatten attendee names for frontmatter
    let attendee_names: Vec<String> = if let Some(ref rich_attendees) = meta.attendees {
        rich_attendees
            .iter()
            .filter_map(|a| a.name.clone())
            .collect()
    } else {
        meta.participants.clone()
    };

    // Convert duration_seconds to duration_minutes
    let duration_minutes = meta.duration_seconds.map(|s| s / 60);

    // Map labels to tags
    let tags: Vec<String> = meta
        .labels
        .iter()
        .map(|l| l.to_lowercase().replace(' ', "-"))
        .collect();

    // Compute date string
    let date_str = meta.created_at.format("%Y-%m-%d").to_string();

    // Build frontmatter
    let frontmatter = Frontmatter {
        doc_id: doc_id.to_string(),
        source: "granola".into(),
        date: Some(date_str),
        created: meta.created_at,
        updated: meta.updated_at,
        title: meta.title.clone(),
        attendees: attendee_names.clone(),
        duration_minutes,
        tags,
        related: vec![],
        status: None,
        generator: "baez".into(),
    };

    let frontmatter_yaml = serde_yaml::to_string(&frontmatter).map_err(|e| {
        crate::Error::Filesystem(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("Failed to serialize frontmatter: {}", e),
        ))
    })?;

    // Build body
    let title = meta.title.as_deref().unwrap_or("Untitled Meeting");
    let mut body = format!("# {}\n\n", title);

    // Metadata line with wiki-links for participants
    let date = meta.created_at.format("%Y-%m-%d");
    let mut meta_parts = vec![format!("Date: {}", date)];

    if let Some(duration) = meta.duration_seconds {
        let minutes = duration / 60;
        meta_parts.push(format!("Duration: {}m", minutes));
    }

    if !attendee_names.is_empty() {
        let wiki_names: Vec<String> = attendee_names
            .iter()
            .map(|n| format!("[[{}]]", n))
            .collect();
        meta_parts.push(format!("Participants: {}", wiki_names.join(", ")));
    }

    body.push_str(&format!("_{}_\n\n", meta_parts.join(" | ")));

    // Obsidian tags line
    let tag_line = format_tags(&meta.labels);
    body.push_str(&tag_line.join(" "));
    body.push_str("\n\n");

    // Rich participants section when attendee data is available
    if let Some(ref attendees) = meta.attendees {
        let rich_lines: Vec<String> = attendees.iter().filter_map(format_attendee_line).collect();
        if !rich_lines.is_empty() {
            body.push_str("## Participants\n\n");
            for line in &rich_lines {
                body.push_str(&format!("- {}\n", line));
            }
            body.push('\n');
        }
    }

    // AI-generated summary section
    if let Some(summary) = summary_text {
        if !summary.is_empty() {
            body.push_str("## Summary\n\n");
            body.push_str(summary);
            body.push_str("\n\n");
        }
    }

    // User ProseMirror notes section (already converted to markdown)
    if let Some(notes_md) = notes {
        if !notes_md.is_empty() {
            body.push_str("## Notes\n\n");
            body.push_str(notes_md);
            body.push_str("\n\n");
        }
    }

    // Separator before transcript
    body.push_str("---\n\n");

    // Transcript content with wiki-linked speaker names
    if raw.entries.is_empty() {
        body.push_str("_No transcript content available._\n");
    } else {
        for entry in &raw.entries {
            let speaker = entry.speaker.as_deref().unwrap_or("Speaker");
            let timestamp = entry
                .start
                .as_deref()
                .and_then(normalize_timestamp)
                .map(|ts| format!(" ({})", ts))
                .unwrap_or_default();
            body.push_str(&format!(
                "**[[{}]]{}:** {}\n",
                speaker, timestamp, entry.text
            ));
        }
    }

    Ok(MarkdownOutput {
        frontmatter_yaml,
        body,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::TranscriptEntry;

    #[test]
    fn test_to_markdown_entries() {
        let raw = RawTranscript {
            entries: vec![
                TranscriptEntry {
                    document_id: Some("doc123".into()),
                    speaker: Some("Alice".into()),
                    start: Some("2025-10-01T21:35:12.500Z".into()),
                    end: Some("2025-10-01T21:35:18.000Z".into()),
                    text: "Hello everyone".into(),
                    source: Some("microphone".into()),
                    id: Some("entry1".into()),
                    is_final: Some(true),
                },
                TranscriptEntry {
                    document_id: Some("doc123".into()),
                    speaker: Some("Bob".into()),
                    start: Some("2025-10-01T21:35:20.000Z".into()),
                    end: Some("2025-10-01T21:35:22.000Z".into()),
                    text: "Hi there".into(),
                    source: Some("microphone".into()),
                    id: Some("entry2".into()),
                    is_final: Some(true),
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

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();

        assert!(output.body.contains("# Test Meeting"));
        assert!(output.body.contains("**[[Alice]]"));
        assert!(output.body.contains("Hello everyone"));
        assert!(output.body.contains("**[[Bob]]"));
        assert!(output.body.contains("Hi there"));
        assert!(output.body.contains("Duration: 60m"));
        assert!(output.body.contains("#granola"));
        assert!(output.frontmatter_yaml.contains("doc123"));
        assert!(output.frontmatter_yaml.contains("generator: baez"));
    }

    #[test]
    fn test_wiki_links_in_participants() {
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
            id: Some("doc123".into()),
            title: Some("Meeting".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec!["Alice".into(), "Bob".into()],
            duration_seconds: None,
            labels: vec![],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();
        assert!(output.body.contains("Participants: [[Alice]], [[Bob]]"));
    }

    #[test]
    fn test_tags_from_labels() {
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
            id: Some("doc123".into()),
            title: Some("Meeting".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec![],
            duration_seconds: None,
            labels: vec!["Planning".into(), "Sprint Review".into()],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();
        assert!(output.body.contains("#granola"));
        assert!(output.body.contains("#meeting/planning"));
        assert!(output.body.contains("#meeting/sprint-review"));

        // Frontmatter tags should be lowercase
        assert!(output.frontmatter_yaml.contains("planning"));
        assert!(output.frontmatter_yaml.contains("sprint-review"));
    }

    #[test]
    fn test_dataview_frontmatter_format() {
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
            id: Some("doc123".into()),
            title: Some("Meeting".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: Some("2025-10-29T01:23:45Z".parse().unwrap()),
            participants: vec!["Alice".into()],
            duration_seconds: Some(3600),
            labels: vec!["planning".into()],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();

        // New frontmatter fields
        assert!(output.frontmatter_yaml.contains("date:"));
        assert!(output.frontmatter_yaml.contains("2025-10-28"));
        assert!(output.frontmatter_yaml.contains("created:"));
        assert!(output.frontmatter_yaml.contains("generator: baez"));
        assert!(output.frontmatter_yaml.contains("duration_minutes: 60"));
        assert!(output.frontmatter_yaml.contains("attendees:"));
    }

    #[test]
    fn test_to_markdown_with_rich_attendees_wiki_links() {
        use crate::model::{CompanyInfo, Employment, PersonDetails, PersonInfo, PersonName};

        let raw = RawTranscript {
            entries: vec![TranscriptEntry {
                document_id: Some("doc123".into()),
                speaker: Some("Alice".into()),
                start: Some("2025-10-01T21:35:12.500Z".into()),
                end: None,
                text: "Hello".into(),
                source: None,
                id: None,
                is_final: None,
            }],
        };

        let meta = DocumentMetadata {
            id: Some("doc123".into()),
            title: Some("Team Standup".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec!["Alice Smith".into(), "Bob Jones".into()],
            duration_seconds: Some(900),
            labels: vec![],
            creator: None,
            attendees: Some(vec![
                crate::model::Attendee {
                    name: Some("Alice Smith".into()),
                    email: Some("alice@acme.com".into()),
                    details: Some(PersonDetails {
                        person: Some(PersonInfo {
                            name: Some(PersonName {
                                full_name: Some("Alice Smith".into()),
                            }),
                            employment: Some(Employment {
                                title: Some("Engineering Manager".into()),
                            }),
                            linkedin: None,
                        }),
                        company: Some(CompanyInfo {
                            name: Some("Acme Corp".into()),
                        }),
                    }),
                },
                crate::model::Attendee {
                    name: Some("Bob Jones".into()),
                    email: Some("bob@acme.com".into()),
                    details: None,
                },
            ]),
        };

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();

        assert!(output.body.contains("## Participants"));
        assert!(output
            .body
            .contains("[[Alice Smith]], Engineering Manager, Acme Corp"));
        assert!(output.body.contains("[[Bob Jones]]"));
        // No email in body for wiki-link format
        assert!(!output.body.contains("alice@acme.com"));
    }

    #[test]
    fn test_to_markdown_empty_transcript() {
        let raw = RawTranscript { entries: vec![] };

        let meta = DocumentMetadata {
            id: Some("doc123".into()),
            title: None,
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec![],
            duration_seconds: None,
            labels: vec![],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(&raw, &meta, "doc123", None, None).unwrap();

        assert!(output.body.contains("# Untitled Meeting"));
        assert!(output.body.contains("_No transcript content available._"));
    }

    #[test]
    fn test_to_markdown_with_summary_and_notes() {
        let raw = RawTranscript {
            entries: vec![TranscriptEntry {
                document_id: Some("doc123".into()),
                speaker: Some("Alice".into()),
                start: Some("2025-10-01T21:35:12.500Z".into()),
                end: None,
                text: "Hello".into(),
                source: None,
                id: None,
                is_final: None,
            }],
        };

        let meta = DocumentMetadata {
            id: Some("doc123".into()),
            title: Some("Meeting".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: None,
            participants: vec!["Alice".into()],
            duration_seconds: None,
            labels: vec![],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(
            &raw,
            &meta,
            "doc123",
            Some("- Action item 1\n- Action item 2"),
            Some("We discussed project priorities."),
        )
        .unwrap();

        assert!(output
            .body
            .contains("## Summary\n\nWe discussed project priorities."));
        assert!(output
            .body
            .contains("## Notes\n\n- Action item 1\n- Action item 2"));
        assert!(output.body.contains("---\n"));
        let summary_pos = output.body.find("## Summary").unwrap();
        let notes_pos = output.body.find("## Notes").unwrap();
        let separator_pos = output.body.find("---\n").unwrap();
        assert!(summary_pos < notes_pos);
        assert!(notes_pos < separator_pos);
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::*;
    use crate::model::TranscriptEntry;

    #[test]
    fn test_markdown_output_snapshot() {
        let raw = RawTranscript {
            entries: vec![
                TranscriptEntry {
                    document_id: Some("doc456".into()),
                    speaker: Some("Alice".into()),
                    start: Some("2025-10-28T15:05:10.000Z".into()),
                    end: Some("2025-10-28T15:05:15.000Z".into()),
                    text: "First thought.".into(),
                    source: Some("microphone".into()),
                    id: Some("entry1".into()),
                    is_final: Some(true),
                },
                TranscriptEntry {
                    document_id: Some("doc456".into()),
                    speaker: Some("Alice".into()),
                    start: Some("2025-10-28T15:05:16.000Z".into()),
                    end: Some("2025-10-28T15:05:20.000Z".into()),
                    text: "Second thought.".into(),
                    source: Some("microphone".into()),
                    id: Some("entry2".into()),
                    is_final: Some(true),
                },
            ],
        };

        let meta = DocumentMetadata {
            id: Some("doc456".into()),
            title: Some("Planning Session".into()),
            created_at: "2025-10-28T15:04:05Z".parse().unwrap(),
            updated_at: Some("2025-10-29T01:23:45Z".parse().unwrap()),
            participants: vec!["Alice".into(), "Bob".into()],
            duration_seconds: Some(3170),
            labels: vec!["Planning".into()],
            creator: None,
            attendees: None,
        };

        let output = to_markdown(&raw, &meta, "doc456", None, None).unwrap();
        let full = format!("---\n{}---\n\n{}", output.frontmatter_yaml, output.body);

        insta::assert_snapshot!(full);
    }
}

/// Converts a ProseMirror document to Markdown text.
pub fn prosemirror_to_markdown(doc: &ProseMirrorDoc) -> String {
    let mut output = String::new();
    if let Some(ref content) = doc.content {
        for node in content {
            convert_node(node, &mut output);
        }
    }
    output.trim_end().to_string()
}

fn convert_node(node: &ProseMirrorNode, output: &mut String) {
    match node.node_type.as_str() {
        "heading" => {
            let level = node
                .attrs
                .as_ref()
                .and_then(|a| a.get("level"))
                .and_then(|l| l.as_u64())
                .unwrap_or(1) as usize;
            let prefix = "#".repeat(level);
            output.push_str(&prefix);
            output.push(' ');
            if let Some(ref content) = node.content {
                for child in content {
                    render_inline(child, output);
                }
            }
            output.push_str("\n\n");
        }
        "paragraph" => {
            if let Some(ref content) = node.content {
                for child in content {
                    render_inline(child, output);
                }
            }
            output.push_str("\n\n");
        }
        "bulletList" => {
            if let Some(ref content) = node.content {
                for child in content {
                    convert_node(child, output);
                }
            }
        }
        "listItem" => {
            output.push_str("- ");
            if let Some(ref content) = node.content {
                for (i, child) in content.iter().enumerate() {
                    if child.node_type == "paragraph" {
                        if let Some(ref para_content) = child.content {
                            for inline_child in para_content {
                                render_inline(inline_child, output);
                            }
                        }
                        if i < content.len() - 1 {
                            output.push('\n');
                        }
                    } else {
                        convert_node(child, output);
                    }
                }
            }
            output.push('\n');
        }
        "text" => {
            render_inline(node, output);
        }
        _ => {}
    }
}

fn render_inline(node: &ProseMirrorNode, output: &mut String) {
    if node.node_type == "text" {
        let text = node.text.as_deref().unwrap_or("");
        if let Some(ref marks) = node.marks {
            let has_bold = marks.iter().any(|m| m.mark_type == "bold");
            let has_italic = marks.iter().any(|m| m.mark_type == "italic");
            if has_bold && has_italic {
                output.push_str("***");
                output.push_str(text);
                output.push_str("***");
            } else if has_bold {
                output.push_str("**");
                output.push_str(text);
                output.push_str("**");
            } else if has_italic {
                output.push('*');
                output.push_str(text);
                output.push('*');
            } else {
                output.push_str(text);
            }
        } else {
            output.push_str(text);
        }
    }
}

#[cfg(test)]
mod prosemirror_convert_tests {
    use super::*;
    use crate::model::{ProseMirrorDoc, ProseMirrorMark, ProseMirrorNode};

    #[test]
    fn test_paragraph_to_text() {
        let doc = ProseMirrorDoc {
            node_type: "doc".into(),
            content: Some(vec![ProseMirrorNode {
                node_type: "paragraph".into(),
                content: Some(vec![ProseMirrorNode {
                    node_type: "text".into(),
                    content: None,
                    text: Some("Hello world".into()),
                    attrs: None,
                    marks: None,
                }]),
                text: None,
                attrs: None,
                marks: None,
            }]),
        };
        let md = prosemirror_to_markdown(&doc);
        assert_eq!(md, "Hello world");
    }

    #[test]
    fn test_bold_text() {
        let doc = ProseMirrorDoc {
            node_type: "doc".into(),
            content: Some(vec![ProseMirrorNode {
                node_type: "paragraph".into(),
                content: Some(vec![ProseMirrorNode {
                    node_type: "text".into(),
                    content: None,
                    text: Some("important".into()),
                    attrs: None,
                    marks: Some(vec![ProseMirrorMark {
                        mark_type: "bold".into(),
                    }]),
                }]),
                text: None,
                attrs: None,
                marks: None,
            }]),
        };
        let md = prosemirror_to_markdown(&doc);
        assert_eq!(md, "**important**");
    }

    #[test]
    fn test_empty_doc() {
        let doc = ProseMirrorDoc {
            node_type: "doc".into(),
            content: None,
        };
        let md = prosemirror_to_markdown(&doc);
        assert_eq!(md, "");
    }
}
