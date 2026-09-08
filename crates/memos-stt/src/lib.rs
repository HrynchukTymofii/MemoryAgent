//! Audio capture and speech-to-text.
//!
//! Stage 2-4 of the latency budget. The defining decision lives in `capture`:
//! the microphone stream is opened at application start and never closed, so
//! the hotkey costs nothing — it records a cursor into a buffer that is already
//! full of audio, rather than opening a device (100-300 ms) and clipping the
//! first word.
//!
//! whisper.cpp sits behind the optional `whisper` feature: it needs cmake and
//! libclang to build, and a broken native toolchain must never stop the
//! application compiling.

pub mod capture;
pub mod ring;
pub mod transcribe;
pub mod vad;

pub use capture::{AudioCapture, AudioError};
pub use ring::{Cursor, RingBuffer, RETAIN_SECS, SAMPLE_RATE};
pub use transcribe::{find_model, Hints, SttError, Transcriber, Transcript};
pub use vad::{Speech, Vad};

#[cfg(feature = "whisper")]
pub use transcribe::WhisperTranscriber;
