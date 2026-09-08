//! Domain types shared by every other crate.
//!
//! This crate performs no I/O and depends on no platform. If something here
//! needs a database, a model or a Windows API, it belongs in another crate.

pub mod error;
pub mod ids;
pub mod intent;
pub mod model;

pub use error::{Error, Result};
pub use ids::Id;
pub use intent::{Confidence, Intent, RoutedCommand, Slots, Tier};
pub use model::{Collection, KnowledgeItem, Source, SourceKind, SyncState};

/// Wall-clock timestamps are UTC everywhere internally; the interface layer is
/// the only place that converts to local time.
pub type Timestamp = chrono::DateTime<chrono::Utc>;

pub fn now() -> Timestamp {
    chrono::Utc::now()
}
