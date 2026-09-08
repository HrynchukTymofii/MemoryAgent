use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("storage: {0}")]
    Storage(String),

    #[error("audio: {0}")]
    Audio(String),

    #[error("transcription: {0}")]
    Transcription(String),

    #[error("routing: {0}")]
    Routing(String),

    /// The router understood the command but could not resolve a slot with
    /// enough margin to act. Carries the candidates so the interface can ask a
    /// narrow question rather than restating the whole command.
    #[error("ambiguous: {slot} has {} candidates", candidates.len())]
    Ambiguous {
        slot: String,
        candidates: Vec<(String, f32)>,
    },

    #[error("not found: {0}")]
    NotFound(String),

    #[error("permission denied: {0}")]
    Permission(String),

    #[error("quota exhausted: {used} of {limit} captures used this week")]
    QuotaExhausted { used: u32, limit: u32 },

    #[error(transparent)]
    Other(#[from] anyhow_compat::AnyError),
}

/// Keeps `memos-core` free of an `anyhow` dependency while still letting callers
/// wrap arbitrary errors.
pub mod anyhow_compat {
    pub type AnyError = Box<dyn std::error::Error + Send + Sync + 'static>;
}
