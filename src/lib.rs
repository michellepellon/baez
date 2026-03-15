//! Baez: sync Granola meeting transcripts into an Obsidian vault.
//!
//! Re-exports core modules for external use.

pub mod api;
pub mod auth;
pub mod cli;
pub mod convert;
pub mod error;
pub mod model;
pub mod storage;
pub mod sync;
pub mod util;

#[cfg(feature = "summaries")]
pub mod summary;

pub use api::{ApiClient, ApiResponse};
pub use auth::resolve_token;
pub use convert::{prosemirror_to_markdown, to_markdown, MarkdownOutput};
pub use error::{Error, Result};
pub use model::{
    Attendee, CompanyInfo, DocumentMetadata, DocumentSummary, Employment, Frontmatter, LinkedIn,
    PersonDetails, PersonInfo, PersonName, ProseMirrorDoc, ProseMirrorMark, ProseMirrorNode,
    PublicNote, RawTranscript,
};
pub use storage::{
    create_concept_note, create_person_note, create_project_note, enrich_concept_note,
    enrich_person_note, enrich_project_note, find_entity_file, read_entity_frontmatter,
    read_frontmatter, write_atomic, PeopleIndex, Paths,
};
pub use sync::sync_all;
pub use util::count_transcript_words;

#[cfg(feature = "summaries")]
pub use summary::{
    build_context_preamble, parse_summary_output, ExtractedEntities, PersonEntity, ConceptEntity,
    ProjectEntity,
};
