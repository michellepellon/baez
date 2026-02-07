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
pub use storage::{read_frontmatter, write_atomic, Paths};
pub use sync::sync_all;
