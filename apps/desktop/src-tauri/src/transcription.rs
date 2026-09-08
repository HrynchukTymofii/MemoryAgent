//! The transcription worker.
//!
//! Sits between the hotkey and the model. Two rules shape it:
//!
//! 1. **The model loads once, in the background, at startup.** Loading costs
//!    ~130 ms; paying that per capture would put it straight on the critical
//!    path. Loading it off-thread means the window appears immediately and the
//!    model is ready long before the first shortcut.
//! 2. **Transcription never runs on the hotkey thread.** That thread must stay
//!    free to notice the *next* chord; blocking it for a few hundred
//!    milliseconds would drop a rapid second capture entirely.

use std::path::PathBuf;
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use memos_agent::Tier0;
use memos_context::Context;
use memos_db::Db;
use memos_stt::{Hints, Transcriber, Transcript, Vad};
use parking_lot::RwLock;

/// A finished capture, ready for the interface and (from the next milestone)
/// the intent router.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureResult {
    pub text: String,
    /// What the user was looking at. Collected during the speech, so it costs
    /// nothing on the timeline.
    pub context: Context,
    pub audio_secs: f32,
    pub inference_ms: u32,
    /// Key release to transcript. This is the number §4 budgets at 300 ms for
    /// the tail, and the one that decides whether the product feels instant.
    pub total_ms: u32,
    /// The VAD heard nothing. Distinguished from a failure because it is not an
    /// error — it is an empty capture, and it must not be saved or metered.
    pub empty: bool,
    /// What the command did, once routed and executed. `None` when the
    /// transcript was empty or nothing was recognised.
    pub outcome: Option<memos_agent::Outcome>,
    /// Candidate destinations when a slot was too ambiguous to act on.
    pub ask: Option<Ambiguity>,
}

impl CaptureResult {
    /// A receipt for a command that finished later than its capture did.
    ///
    /// An answered question produces the same shape as a command that never
    /// needed asking — once it is done, how it got there is not the user's
    /// problem. The timings are zero because they were measured on the capture
    /// this answers, and reporting them twice would double-count.
    pub fn receipt(transcript: &str, outcome: memos_agent::Outcome) -> Self {
        Self {
            text: transcript.to_string(),
            context: Context::default(),
            audio_secs: 0.0,
            inference_ms: 0,
            total_ms: 0,
            empty: false,
            outcome: Some(outcome),
            ask: None,
        }
    }
}

/// A narrow question for the user: one slot, a few candidates.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Ambiguity {
    pub slot: String,
    pub options: Vec<memos_agent::Candidate>,
}

pub enum Job {
    Transcribe {
        audio: Vec<f32>,
        hints: Hints,
        context: Context,
        /// When the user released the key, for the end-to-end measurement.
        released: std::time::Instant,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Loading,
    Ready,
    Missing,
    Failed,
}

pub struct Stt {
    db: RwLock<Option<Arc<Db>>>,
    /// Optional throughout. Search degrades to keywords without it rather than
    /// failing, which is the state of the app for the first second after launch
    /// and permanently on a machine with no model.
    embeddings: RwLock<Option<Arc<crate::Embeddings>>>,
    /// Where an unanswered destination question waits for a click.
    questions: RwLock<Option<Arc<crate::question::Pending>>>,
    model: RwLock<Option<Arc<dyn Transcriber>>>,
    state: RwLock<ModelState>,
    detail: RwLock<String>,
    tx: RwLock<Option<Sender<Job>>>,
}

impl Stt {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            db: RwLock::new(None),
            embeddings: RwLock::new(None),
            questions: RwLock::new(None),
            model: RwLock::new(None),
            state: RwLock::new(ModelState::Loading),
            detail: RwLock::new(String::new()),
            tx: RwLock::new(None),
        })
    }

    pub fn state(&self) -> ModelState {
        *self.state.read()
    }

    pub fn detail(&self) -> String {
        self.detail.read().clone()
    }

    /// Load the model off the main thread and start the worker.
    ///
    /// `on_result` runs on the worker thread once a capture is transcribed.
    pub fn attach_db(&self, db: Arc<Db>) {
        *self.db.write() = Some(db);
    }

    pub fn attach_embeddings(&self, e: Arc<crate::Embeddings>) {
        *self.embeddings.write() = Some(e);
    }

    pub fn attach_questions(&self, q: Arc<crate::question::Pending>) {
        *self.questions.write() = Some(q);
    }

    pub fn start(
        self: &Arc<Self>,
        model_path: Option<PathBuf>,
        on_result: impl Fn(CaptureResult) + Send + 'static,
    ) {
        let me = self.clone();
        let (tx, rx) = channel::<Job>();
        *self.tx.write() = Some(tx);

        std::thread::Builder::new()
            .name("transcribe".into())
            .spawn(move || {
                me.load(model_path);

                for job in rx {
                    let Job::Transcribe {
                        audio,
                        hints,
                        context,
                        released,
                    } = job;

                    // Cheap gate before an expensive model run: if the VAD heard
                    // no speech, whisper would confidently hallucinate something
                    // from silence, and that phantom would be saved as a memory
                    // and counted against the weekly quota.
                    let mut vad = Vad::default();
                    vad.push(&audio);
                    let audio_secs = audio.len() as f32 / memos_stt::SAMPLE_RATE as f32;

                    if !vad.heard_speech() {
                        on_result(CaptureResult {
                            text: String::new(),
                            context,
                            audio_secs,
                            inference_ms: 0,
                            total_ms: released.elapsed().as_millis() as u32,
                            empty: true,
                            outcome: None,
                            ask: None,
                        });
                        continue;
                    }

                    let model = me.model.read().clone();
                    let Some(model) = model else {
                        on_result(CaptureResult {
                            text: String::new(),
                            context,
                            audio_secs,
                            inference_ms: 0,
                            total_ms: released.elapsed().as_millis() as u32,
                            empty: true,
                            outcome: None,
                            ask: None,
                        });
                        continue;
                    };

                    match model.transcribe(&audio, &hints) {
                        Ok(Transcript {
                            text,
                            inference_ms,
                            audio_secs,
                        }) => {
                            let empty = text.trim().is_empty();

                            // Route and execute on this thread, before the
                            // result is reported: the receipt must state what
                            // actually happened, not what is about to.
                            let (outcome, ask) = if empty {
                                (None, None)
                            } else {
                                me.route_and_execute(&text, &context)
                            };

                            let total_ms = released.elapsed().as_millis() as u32;
                            tracing::info!(
                                transcript = %text,
                                inference_ms,
                                total_ms,
                                rtf = format!("{:.1}x", audio_secs / (inference_ms.max(1) as f32 / 1000.0)),
                                "transcribed"
                            );
                            on_result(CaptureResult {
                                text,
                                context,
                                audio_secs,
                                inference_ms,
                                total_ms,
                                empty,
                                outcome,
                                ask,
                            });
                        }
                        Err(e) => {
                            tracing::error!(?e, "transcription failed");
                            on_result(CaptureResult {
                                text: String::new(),
                                context,
                                audio_secs,
                                inference_ms: 0,
                                total_ms: released.elapsed().as_millis() as u32,
                                empty: true,
                                outcome: None,
                                ask: None,
                            });
                        }
                    }
                }
            })
            .expect("spawn transcription worker");
    }

    /// Route a transcript through Tier 0 and execute it.
    ///
    /// Tier 1 and Tier 2 escalation land here later; today an unrecognised
    /// shape is reported as such rather than guessed at, which is the same
    /// contract the model tiers will inherit.
    fn route_and_execute(
        &self,
        text: &str,
        context: &Context,
    ) -> (Option<memos_agent::Outcome>, Option<Ambiguity>) {
        let Some(db) = self.db.read().clone() else {
            return (None, None);
        };
        let paths = db.collection_paths().unwrap_or_default();

        match memos_agent::parse(text, &paths) {
            Tier0::Routed(cmd) => {
                tracing::info!(
                    intent = cmd.intent.as_str(),
                    collection = ?cmd.slots.collection,
                    routing_ms = cmd.routing_ms,
                    tier = ?cmd.tier,
                    "routed"
                );
                // Embed the query only for the intents that retrieve. A SAVE
                // pays nothing for a model it does not use, which is what keeps
                // the capture path at the latency the budget assumes.
                let vector = match cmd.intent {
                    memos_core::Intent::Search
                    | memos_core::Intent::Show
                    | memos_core::Intent::Open => self
                        .embeddings
                        .read()
                        .clone()
                        .and_then(|e| e.embed_query(cmd.slots.query.as_deref().unwrap_or(text))),
                    _ => None,
                };

                match memos_agent::execute_with(&db, &cmd, context, vector.as_deref()) {
                    Ok(out) => {
                        // Only a real capture counts against the weekly meter.
                        // Metering a search, or a command that failed, would be
                        // charging the user for nothing.
                        if matches!(out.kind, "save" | "note") {
                            let _ = db.record_capture(&crate::week_start());
                            // A new item means work for the embedding worker.
                            // Nudging beats polling: the backfill starts within
                            // milliseconds of the commit instead of up to 30 s
                            // later, so search is current almost immediately.
                            if let Some(e) = self.embeddings.read().as_ref() {
                                e.nudge();
                            }
                        }
                        if let Some(target) = &out.open {
                            open_target(target);
                        }
                        tracing::info!(summary = %out.summary, took_ms = out.took_ms, "executed");
                        (Some(out), None)
                    }
                    Err(e) => {
                        tracing::error!(?e, "execution failed");
                        (None, None)
                    }
                }
            }
            Tier0::Ambiguous {
                intent,
                slot,
                resolution,
                transcript,
            } => {
                tracing::info!(slot, margin = resolution.margin, "ambiguous; asking");
                // Hold the decision that was already made, minus the one slot
                // nobody could settle. Re-routing the transcript when the answer
                // arrives would run the grammar again and could land somewhere
                // else entirely.
                if let Some(q) = self.questions.read().as_ref() {
                    q.ask(
                        intent,
                        transcript,
                        memos_core::Slots::default(),
                        context.clone(),
                        resolution.candidates.clone(),
                    );
                }
                (
                    None,
                    Some(Ambiguity {
                        slot: slot.to_string(),
                        options: resolution.candidates,
                    }),
                )
            }
            Tier0::Unrecognised => {
                // Tier 1 takes this case once the local router lands. Until
                // then the transcript is still shown, so nothing is lost.
                tracing::debug!(text, "no Tier 0 shape matched");
                (None, None)
            }
        }
    }

    /// Queue a capture. Returns immediately.
    pub fn submit(
        &self,
        audio: Vec<f32>,
        hints: Hints,
        context: Context,
        released: std::time::Instant,
    ) -> bool {
        match self.tx.read().as_ref() {
            Some(tx) => tx
                .send(Job::Transcribe {
                    audio,
                    hints,
                    context,
                    released,
                })
                .is_ok(),
            None => false,
        }
    }

    #[cfg(feature = "whisper")]
    fn load(&self, explicit: Option<PathBuf>) {
        let data = crate::data_dir();
        let Some(path) = memos_stt::find_model(explicit.as_deref(), &data) else {
            *self.state.write() = ModelState::Missing;
            *self.detail.write() =
                "No speech model found. Run scripts/fetch-models.ps1 base.en".into();
            tracing::warn!("no whisper model found; transcription disabled");
            crate::hotkey::diag("whisper model MISSING");
            return;
        };
        match memos_stt::WhisperTranscriber::load(&path) {
            Ok(t) => {
                *self.model.write() = Some(Arc::new(t));
                *self.state.write() = ModelState::Ready;
                *self.detail.write() = path.display().to_string();
                crate::hotkey::diag(&format!("whisper model ready: {}", path.display()));
            }
            Err(e) => {
                *self.state.write() = ModelState::Failed;
                *self.detail.write() = e.to_string();
                tracing::error!(?e, "whisper model failed to load");
                crate::hotkey::diag(&format!("whisper model FAILED: {e}"));
            }
        }
    }

    #[cfg(not(feature = "whisper"))]
    fn load(&self, _explicit: Option<PathBuf>) {
        *self.state.write() = ModelState::Missing;
        *self.detail.write() = "Built without the `whisper` feature.".into();
        tracing::warn!("built without whisper support; transcription disabled");
    }
}

/// Act on what OPEN chose.
///
/// Failure is logged rather than raised: the receipt has already told the user
/// which memory was found, and that is still true even if the browser refused
/// to launch.
fn open_target(target: &memos_agent::OpenTarget) {
    let Some(url) = target.url.clone().or_else(|| target.file_path.clone()) else {
        return;
    };
    if let Err(e) = crate::open_externally(&url) {
        tracing::warn!(%url, error = %e, "could not open the source");
    } else {
        tracing::info!(%url, title = %target.title, "opened");
    }
}
