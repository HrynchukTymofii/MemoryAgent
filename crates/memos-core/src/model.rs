use serde::{Deserialize, Serialize};

use crate::{Id, Timestamp};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    Webpage,
    Highlight,
    VoiceNote,
    Pdf,
    Image,
    File,
    ManualNote,
    Conversation,
}

impl SourceKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            SourceKind::Webpage => "webpage",
            SourceKind::Highlight => "highlight",
            SourceKind::VoiceNote => "voice_note",
            SourceKind::Pdf => "pdf",
            SourceKind::Image => "image",
            SourceKind::File => "file",
            SourceKind::ManualNote => "manual_note",
            SourceKind::Conversation => "conversation",
        }
    }
}

/// Where a local row stands relative to the cloud. Capture writes `Pending` and
/// returns immediately; the sync worker moves it on later (ADR-0001, §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SyncState {
    /// Written locally, not yet pushed. The normal state right after a capture.
    Pending,
    Synced,
    /// Diverged from the server copy; resolved field-level, history retained.
    Conflicted,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Collection {
    pub id: Id,
    pub parent_id: Option<Id>,
    pub name: String,
    /// Materialised path, e.g. `Study/Programming/React`.
    ///
    /// Denormalised on purpose: the router grammar needs the full list of valid
    /// destinations on every command, and walking a recursive CTE for that on
    /// the capture path would be wasteful.
    pub path: String,
    pub created_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Source {
    pub id: Id,
    pub kind: SourceKind,
    pub url: Option<String>,
    pub domain: Option<String>,
    pub file_path: Option<String>,
    pub title: Option<String>,
    pub retrieved_at: Timestamp,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnowledgeItem {
    pub id: Id,
    pub title: String,
    pub content: String,
    pub summary: Option<String>,
    pub collection_id: Option<Id>,
    pub source_id: Option<Id>,
    pub captured_at: Timestamp,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    pub last_accessed_at: Option<Timestamp>,
    /// Reopened items are items that matter — a ranking signal (§7).
    pub access_count: u32,
    pub sync_state: SyncState,
}

impl KnowledgeItem {
    /// A newly captured item. Note there is no embedding: a save is durable and
    /// acknowledged the moment the transaction commits, and the vector is
    /// backfilled by a background worker (§4, stage 7).
    pub fn capture(title: impl Into<String>, content: impl Into<String>) -> Self {
        let now = crate::now();
        Self {
            id: Id::new(),
            title: title.into(),
            content: content.into(),
            summary: None,
            collection_id: None,
            source_id: None,
            captured_at: now,
            created_at: now,
            updated_at: now,
            last_accessed_at: None,
            access_count: 0,
            sync_state: SyncState::Pending,
        }
    }
}
