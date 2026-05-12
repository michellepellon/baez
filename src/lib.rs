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
pub use auth::{resolve_api_key, set_api_key_in_keychain};
pub use convert::{to_markdown, MarkdownOutput};
pub use error::{Error, Result};
pub use model::{
    CalendarEvent, Folder, Frontmatter, ListNotesResponse, Note, NoteSummary, Speaker,
    TranscriptEntry, User,
};
pub use storage::{
    create_concept_note, create_person_note, create_project_note, enrich_concept_note,
    enrich_person_note, enrich_project_note, find_entity_file, read_entity_frontmatter,
    read_frontmatter, write_atomic, Paths, PeopleIndex,
};
#[cfg(feature = "summaries")]
pub use sync::summarize_all_docs;
pub use sync::sync_all;
pub use util::{count_transcript_words, doc_slug};

#[cfg(feature = "summaries")]
pub use summary::{
    build_context_preamble, parse_summary_output, ConceptEntity, ExtractedEntities, PersonEntity,
    ProjectEntity,
};
