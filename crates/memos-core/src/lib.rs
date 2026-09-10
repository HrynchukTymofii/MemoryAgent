//! Domain types shared by every other crate.
//!
//! This crate performs no I/O and depends on no platform. If something here
//! needs a database, a model or a Windows API, it belongs in another crate.

pub mod achieve;
pub mod error;
pub mod ids;
pub mod intent;
pub mod model;

pub use achieve::{Earned, Kind, Recap, Streak, Totals};
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

/// The helper scripts, named for the platform the message is being read on.
///
/// These names appear in errors the user is expected to act on — "no model,
/// run this" — so telling a Mac user to run a PowerShell script is not a
/// cosmetic wrong answer. It is an instruction that cannot be followed.
///
/// A `cfg` on a string rather than a lookup: there is exactly one right answer
/// per build, and it is known when the binary is compiled.
pub mod scripts {
    #[cfg(windows)]
    pub const FETCH_MODELS: &str = "scripts/fetch-models.ps1";
    #[cfg(not(windows))]
    pub const FETCH_MODELS: &str = "scripts/fetch-models.sh";

    #[cfg(windows)]
    pub const BUILD_ROUTER: &str = "scripts/build-router.ps1";
    #[cfg(not(windows))]
    pub const BUILD_ROUTER: &str = "scripts/build-router.sh";
}
