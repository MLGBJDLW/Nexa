/// Data model structs for the ask-core crate.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Represents a content source (directory or file root).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Source {
    pub id: String,
    pub kind: String,
    pub root_path: String,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub watch_enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Represents a single document (file) ingested from a source.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Document {
    pub id: Uuid,
    pub source_id: Uuid,
    pub path: String,
    pub title: String,
    pub content_hash: String,
    pub file_type: FileType,
    pub size_bytes: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub indexed_at: Option<DateTime<Utc>>,
}

/// Supported file types for ingestion.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum FileType {
    Markdown,
    PlainText,
    Log,
}

/// A chunk is a segment of a document used for indexing and retrieval.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Chunk {
    pub id: Uuid,
    pub document_id: Uuid,
    pub content: String,
    pub chunk_index: u32,
    pub start_byte: u64,
    pub end_byte: u64,
    pub heading_path: Vec<String>,
    pub created_at: DateTime<Utc>,
}

/// An evidence card surfaced by a search query.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EvidenceCard {
    pub chunk_id: Uuid,
    pub document_id: Uuid,
    pub source_name: String,
    pub document_path: String,
    pub document_title: String,
    pub content: String,
    pub heading_path: Vec<String>,
    pub score: f64,
    pub highlights: Vec<Highlight>,
}

/// A highlighted span within content.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Highlight {
    pub start: usize,
    pub end: usize,
    pub term: String,
}

/// A search query with optional filters.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchQuery {
    pub text: String,
    pub filters: SearchFilters,
    pub limit: u32,
    pub offset: u32,
}

/// Filters applied to narrow search results.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    pub source_ids: Vec<Uuid>,
    pub file_types: Vec<FileType>,
    pub date_from: Option<DateTime<Utc>>,
    pub date_to: Option<DateTime<Utc>>,
}

/// A playbook is a saved, composable set of evidence cards.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Playbook {
    pub id: Uuid,
    pub title: String,
    pub description: String,
    pub citations: Vec<PlaybookCitation>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A citation within a playbook, referencing a specific chunk.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookCitation {
    pub id: Uuid,
    pub playbook_id: Uuid,
    pub chunk_id: Uuid,
    pub annotation: String,
    pub order: u32,
}
