//! Speech to text.
//!
//! Stages 3 and 4 of the latency budget. The `Transcriber` trait is the seam
//! that keeps §4's dual-path arrangement possible: local whisper drives the
//! overlay and the router synchronously, while a cloud model may later
//! re-transcribe the same audio and upgrade the stored text. Both are the same
//! interface; neither knows about the other.

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum SttError {
    #[error("model not found at {0}")]
    ModelMissing(PathBuf),
    #[error("model failed to load: {0}")]
    Load(String),
    #[error("transcription failed: {0}")]
    Run(String),
    #[error("whisper support was not compiled in (build with --features whisper)")]
    Unavailable,
}

#[derive(Debug, Clone, Default)]
pub struct Transcript {
    pub text: String,
    /// Time spent inside the model, for the §4 budget.
    pub inference_ms: u32,
    /// Seconds of audio processed. `audio_secs / (inference_ms/1000)` is the
    /// realtime factor, which is what actually predicts whether a machine can
    /// keep up.
    pub audio_secs: f32,
}

impl Transcript {
    pub fn realtime_factor(&self) -> f32 {
        if self.inference_ms == 0 {
            return 0.0;
        }
        self.audio_secs / (self.inference_ms as f32 / 1000.0)
    }
}

/// Extra context that measurably improves accuracy on the words that matter.
#[derive(Debug, Clone, Default)]
pub struct Hints {
    /// Collection names, tags and recently seen entities.
    ///
    /// Fed to whisper as an initial prompt, which biases decoding toward these
    /// tokens. Proper nouns are simultaneously what the intent router depends on
    /// most and what a generic model gets wrong most: this is the difference
    /// between "put this in Flari" and "put this in flurry". It is also a
    /// capability cloud APIs largely do not expose.
    pub vocabulary: Vec<String>,
}

impl Hints {
    /// Whisper's prompt is capped (224 tokens), and an over-long prompt crowds
    /// out the audio it is meant to help. Keep it to the most useful terms.
    pub fn to_prompt(&self) -> Option<String> {
        if self.vocabulary.is_empty() {
            return None;
        }
        let mut out = String::from("Vocabulary: ");
        for (i, w) in self.vocabulary.iter().enumerate() {
            if out.len() + w.len() + 2 > 400 {
                break;
            }
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(w);
        }
        out.push('.');
        Some(out)
    }
}

pub trait Transcriber: Send + Sync {
    /// Transcribe 16 kHz mono samples.
    fn transcribe(&self, samples: &[f32], hints: &Hints) -> Result<Transcript, SttError>;
}

/// Default model location, relative to the repository root during development.
pub fn default_model_path() -> PathBuf {
    PathBuf::from("models/stt/ggml-base.en.bin")
}

/// Search the likely locations for a model.
///
/// Checked in order rather than assuming one: during development the model sits
/// in the repo, and once installed it lives beside the user's data. Returning
/// the first that exists keeps both working without a build-time switch.
pub fn find_model(explicit: Option<&Path>, data_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = explicit {
        candidates.push(p.to_path_buf());
    }
    candidates.push(data_dir.join("models/ggml-base.en.bin"));
    candidates.push(data_dir.join("models/ggml-tiny.en.bin"));
    for name in ["ggml-base.en.bin", "ggml-tiny.en.bin", "ggml-small.en.bin"] {
        candidates.push(PathBuf::from("models/stt").join(name));
        candidates.push(PathBuf::from("../../models/stt").join(name));
        candidates.push(PathBuf::from("../../../models/stt").join(name));
    }
    candidates.into_iter().find(|p| p.exists())
}

#[cfg(feature = "whisper")]
mod whisper_impl {
    use super::*;
    use std::time::Instant;
    use whisper_rs::{FullParams, SamplingStrategy, WhisperContext, WhisperContextParameters};

    pub struct WhisperTranscriber {
        ctx: WhisperContext,
        threads: i32,
    }

    impl WhisperTranscriber {
        /// Load a model. Expensive (hundreds of milliseconds), so it happens
        /// once at startup and the context is kept resident — loading per
        /// command would put the whole cost on the critical path.
        pub fn load(model: &Path) -> Result<Self, SttError> {
            if !model.exists() {
                return Err(SttError::ModelMissing(model.to_path_buf()));
            }
            // Route whisper.cpp's and ggml's own C-level logging into `tracing`.
            // Without this they write a full decoder trace straight to stdout —
            // dozens of lines per capture, drowning our structured logs and
            // making the terminal useless during real use. The `set_print_*`
            // params do not cover it: that output comes from the library's
            // logging callback, not from the inference parameters.
            static HOOKS: std::sync::Once = std::sync::Once::new();
            HOOKS.call_once(whisper_rs::install_logging_hooks);

            let started = Instant::now();
            let ctx = WhisperContext::new_with_params(model, WhisperContextParameters::default())
                .map_err(|e| SttError::Load(e.to_string()))?;

            // Leave a core free. Saturating every thread makes the whole desktop
            // stutter during a capture, which reads as the app being slow even
            // when transcription itself is fast.
            let threads = (std::thread::available_parallelism()
                .map(|n| n.get())
                .unwrap_or(4)
                .saturating_sub(1))
            .clamp(1, 8) as i32;

            tracing::info!(
                model = %model.display(),
                load_ms = started.elapsed().as_millis() as u64,
                threads,
                "whisper model loaded"
            );
            Ok(Self { ctx, threads })
        }
    }

    impl Transcriber for WhisperTranscriber {
        fn transcribe(&self, samples: &[f32], hints: &Hints) -> Result<Transcript, SttError> {
            let audio_secs = samples.len() as f32 / crate::ring::SAMPLE_RATE as f32;
            // Whisper pads to 30 s internally; anything under ~1 s is mostly
            // padding and produces confident nonsense.
            if audio_secs < 0.3 {
                return Ok(Transcript {
                    text: String::new(),
                    inference_ms: 0,
                    audio_secs,
                });
            }

            let mut state = self
                .ctx
                .create_state()
                .map_err(|e| SttError::Run(e.to_string()))?;

            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_n_threads(self.threads);
            params.set_translate(false);
            params.set_language(Some("en"));
            // Quiet: whisper.cpp prints progress to stdout otherwise, which
            // corrupts our structured logs.
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            // Short commands have no useful history to condition on, and
            // carrying it over is the main cause of whisper's repetition loops.
            params.set_no_context(true);
            params.set_suppress_blank(true);

            // Shrink the encoder's context window to fit the audio.
            //
            // This is the single biggest win available here. whisper pads every
            // request to 30 s internally and the encoder runs over the whole
            // padded window, so a 3-second command costs almost exactly what a
            // 30-second one does — the cost is fixed, not per-second, which is
            // why short commands showed a far worse realtime factor than long
            // ones. `audio_ctx` caps how many mel frames the encoder sees, at
            // 50 per second of audio.
            //
            // The floor matters: below roughly 512 the encoder loses too much
            // receptive field and accuracy degrades noticeably, so a very short
            // clip is not shrunk any further. The margin above the true length
            // covers whisper's own padding and trailing silence.
            const FRAMES_PER_SEC: f32 = 50.0;
            const MIN_CTX: i32 = 512;
            const MAX_CTX: i32 = 1500;
            let needed = (audio_secs * FRAMES_PER_SEC).ceil() as i32 + 128;
            params.set_audio_ctx(needed.clamp(MIN_CTX, MAX_CTX));

            let prompt = hints.to_prompt();
            if let Some(p) = prompt.as_deref() {
                params.set_initial_prompt(p);
            }

            let started = Instant::now();
            state
                .full(params, samples)
                .map_err(|e| SttError::Run(e.to_string()))?;
            let inference_ms = started.elapsed().as_millis() as u32;

            let mut text = String::new();
            let mut segments = 0u32;
            let mut speechless = 0u32;
            for seg in state.as_iter() {
                segments += 1;
                // whisper reports how confident it is that a segment is *not*
                // speech. Using it beats pattern-matching the output: the model
                // knows silence better than a list of the annotations it happens
                // to emit, and a false capture would otherwise be saved as a
                // memory titled "[BLANK_AUDIO]".
                if seg.no_speech_probability() > 0.6 {
                    speechless += 1;
                    continue;
                }
                if let Ok(s) = seg.to_str_lossy() {
                    text.push_str(&s);
                }
            }

            if segments > 0 && speechless == segments {
                tracing::debug!("all {segments} segments scored as non-speech; treating as silence");
            }

            Ok(Transcript {
                text: clean(&text),
                inference_ms,
                audio_secs,
            })
        }
    }

    /// Strip whisper's artefacts.
    ///
    /// On silence or noise the model reliably emits bracketed annotations such
    /// as `[BLANK_AUDIO]` or `(wind blowing)`. Passing those to the intent
    /// router would have it confidently save a memory titled "[BLANK_AUDIO]".
    fn clean(raw: &str) -> String {
        let mut out = String::with_capacity(raw.len());
        let mut depth = 0i32;
        for c in raw.chars() {
            match c {
                '[' | '(' => depth += 1,
                ']' | ')' => depth = (depth - 1).max(0),
                _ if depth == 0 => out.push(c),
                _ => {}
            }
        }
        out.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn strips_bracketed_annotations() {
            assert_eq!(clean("[BLANK_AUDIO]"), "");
            assert_eq!(clean(" (wind)  save this to react "), "save this to react");
            assert_eq!(clean("save this"), "save this");
        }
    }
}

#[cfg(feature = "whisper")]
pub use whisper_impl::WhisperTranscriber;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_vocabulary_yields_no_prompt() {
        assert!(Hints::default().to_prompt().is_none());
    }

    #[test]
    fn prompt_is_bounded() {
        // An over-long prompt crowds out the audio it exists to help.
        let hints = Hints {
            vocabulary: (0..500).map(|i| format!("Collection{i}")).collect(),
        };
        let p = hints.to_prompt().unwrap();
        assert!(p.len() < 420, "prompt must stay bounded, got {}", p.len());
        assert!(p.starts_with("Vocabulary: "));
    }

    #[test]
    fn realtime_factor() {
        let t = Transcript {
            text: String::new(),
            inference_ms: 500,
            audio_secs: 5.0,
        };
        assert_eq!(t.realtime_factor(), 10.0);
    }
}
