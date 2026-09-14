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
use memos_db::{count_words, ActivityKind, Db, Notification};
use memos_stt::{Hints, Transcriber, Transcript, Vad};
use parking_lot::RwLock;

use crate::hotkey::Mode;

/// A finished capture, ready for the interface and (from the next milestone)
/// the intent router.
#[derive(Debug, Clone, serde::Serialize)]
pub struct CaptureResult {
    /// Which shortcut produced this. The interface shows a dictation result
    /// differently — there is no command, no receipt and nothing saved, so a
    /// routing verdict would be a lie about what just happened.
    pub mode: Mode,
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
    /// Milestones this command crossed, if it crossed any.
    ///
    /// Carried on the result rather than emitted from the worker because the
    /// worker has no window handle: it can only hand things back through here.
    /// Almost always empty, which is the point — a milestone that arrived with
    /// every capture would not be one.
    pub earned: Vec<Notification>,
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
            // Only a capture ever asks a question, so only a capture is ever
            // answered later.
            mode: Mode::Capture,
            text: transcript.to_string(),
            context: Context::default(),
            audio_secs: 0.0,
            inference_ms: 0,
            total_ms: 0,
            empty: false,
            outcome: Some(outcome),
            ask: None,
            // The answering path records its own use and emits what it earned
            // directly; it has the app handle that the worker lacks.
            earned: Vec::new(),
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
        /// Which chord was held. Decided at the keyboard, not here: by the time
        /// the audio arrives there is nothing left to tell the two apart.
        mode: Mode,
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
    /// Our own API, which is where the dictation formatter runs. Absent when
    /// this build has no `MEMOS_API_URL`, in which case dictation types exactly
    /// what whisper heard.
    api: RwLock<Option<Arc<memos_auth::Auth>>>,
    /// The router. Absent when no API key is configured, and unreachable when
    /// the machine is offline — in both cases the grammar is the system, which
    /// is the whole reason the grammar is still here (ADR-0011).
    cloud: RwLock<Option<Arc<memos_cloud::Cloud>>>,
    /// Why the last command was not routed by the model, if it was not.
    ///
    /// Recorded from the attempt rather than from configuration, because those
    /// are different questions and only this one is worth showing: a key that
    /// is present and rejected, or a machine that is offline, both look fine
    /// from the settings file.
    cloud_error: RwLock<Option<String>>,
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
            api: RwLock::new(None),
            cloud: RwLock::new(None),
            cloud_error: RwLock::new(None),
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

    /// The loaded model, for a caller that transcribes on its own thread.
    pub fn model(&self) -> Option<Arc<dyn Transcriber>> {
        self.model.read().clone()
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

    pub fn attach_cloud(&self, c: Arc<memos_cloud::Cloud>) {
        *self.cloud.write() = Some(c);
    }

    pub fn attach_api(&self, a: Arc<memos_auth::Auth>) {
        *self.api.write() = Some(a);
    }

    /// What the Hub shows about the router.
    ///
    /// `Missing` is not a fault: no key means the grammar is the system, which
    /// is a supported way to run this app. `Failed` means a command was put to
    /// the model within living memory and did not come back.
    pub fn cloud_health(&self) -> (ModelState, String) {
        if self.cloud.read().is_none() {
            return (
                ModelState::Missing,
                "No API key — familiar phrasings still route.".into(),
            );
        }
        match self.cloud_error.read().clone() {
            Some(why) => (ModelState::Failed, why),
            None => (ModelState::Ready, memos_cloud::MODEL.into()),
        }
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
                        mode,
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
                            mode,
                            text: String::new(),
                            context,
                            audio_secs,
                            inference_ms: 0,
                            total_ms: released.elapsed().as_millis() as u32,
                            empty: true,
                            outcome: None,
                            ask: None,
                            earned: Vec::new(),
                        });
                        continue;
                    }

                    let model = me.model.read().clone();
                    let Some(model) = model else {
                        on_result(CaptureResult {
                            mode,
                            text: String::new(),
                            context,
                            audio_secs,
                            inference_ms: 0,
                            total_ms: released.elapsed().as_millis() as u32,
                            empty: true,
                            outcome: None,
                            ask: None,
                            earned: Vec::new(),
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

                            // Whisper hands back one flat run of words. A list
                            // the user clearly spoke as a list — "first ...
                            // second of all ... and finally" — arrives as a
                            // single comma-spattered sentence, and typing that
                            // into their document is not what they dictated.
                            //
                            // Shaped by our own service rather than here: the
                            // model that does it cannot be linked into this
                            // process (llama.cpp and whisper.cpp each vendor
                            // their own ggml), and a service is a sidecar that
                            // happens to have a URL. It is on the critical
                            // path — the words cannot be typed until they come
                            // back — and every way it can fail ends with the raw
                            // transcript, which is the feature working slightly
                            // worse rather than not working.
                            let text = if mode == Mode::Dictate && !empty {
                                me.shaped(&text)
                            } else {
                                text
                            };

                            // Route and execute on this thread, before the
                            // result is reported: the receipt must state what
                            // actually happened, not what is about to.
                            //
                            // Dictation skips all of it. There is no command to
                            // route, no memory to write and no use to record —
                            // the words are going into somebody else's document,
                            // and this app has no business keeping a copy. The
                            // shaping pass above is a different thing entirely:
                            // it formats the text and returns it, and executes
                            // nothing.
                            let (outcome, ask, earned) = if empty || mode == Mode::Dictate {
                                (None, None, Vec::new())
                            } else {
                                me.handle(&text, &context, audio_secs)
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
                                mode,
                                text,
                                context,
                                audio_secs,
                                inference_ms,
                                total_ms,
                                empty,
                                outcome,
                                ask,
                                earned,
                            });
                        }
                        Err(e) => {
                            tracing::error!(?e, "transcription failed");
                            on_result(CaptureResult {
                                mode,
                                text: String::new(),
                                context,
                                audio_secs,
                                inference_ms: 0,
                                total_ms: released.elapsed().as_millis() as u32,
                                empty: true,
                                outcome: None,
                                ask: None,
                                earned: Vec::new(),
                            });
                        }
                    }
                }
            })
            .expect("spawn transcription worker");
    }

    /// Format a dictated transcript, or hand back exactly what was heard.
    ///
    /// Never an error and never empty: the caller is about to type this into
    /// somebody's document, and there is always a correct thing to type — the
    /// words whisper heard. No key, no network, a refusal, a slow turn and a
    /// malformed reply all come out the same way here, which is why this
    /// returns a `String` rather than a `Result` nobody could act on.
    fn shaped(&self, text: &str) -> String {
        let Some(api) = self.api.read().clone() else {
            return text.to_string();
        };
        match api.shape(text) {
            Ok(shaped) if !shaped.text.trim().is_empty() => {
                tracing::info!(took_ms = shaped.took_ms, "dictation shaped");
                shaped.text
            }
            // A service that answered with nothing has not formatted anything,
            // and the empty string is the one reply that would type over the
            // user's words with silence.
            Ok(_) => text.to_string(),
            Err(e) => {
                tracing::warn!(error = %e, "dictation not shaped; typing the raw transcript");
                crate::hotkey::diag(&format!("dictation not shaped: {e}"));
                text.to_string()
            }
        }
    }

    /// Route a transcript, execute it, and count that it happened.
    ///
    /// The counting is here rather than inside `route_and_execute` because it
    /// must happen whatever routing decided — a search is use of the app and
    /// belongs in the streak, and so does a command that ended in a question.
    /// The only transcript that is not counted is one that was never said.
    fn handle(
        &self,
        text: &str,
        context: &Context,
        audio_secs: f32,
    ) -> (Option<memos_agent::Outcome>, Option<Ambiguity>, Vec<Notification>) {
        let (outcome, ask) = self.route_and_execute(text, context);
        let earned = self.record_use(text, audio_secs, outcome.as_ref());
        (outcome, ask, earned)
    }

    /// Add this command to the day's tally, and award anything it just crossed.
    ///
    /// Never allowed to fail the command it describes, for the same reason the
    /// correction log is not: a user whose disk is full should lose the badge,
    /// not the memory.
    fn record_use(
        &self,
        text: &str,
        audio_secs: f32,
        outcome: Option<&memos_agent::Outcome>,
    ) -> Vec<Notification> {
        let Some(db) = self.db.read().clone() else {
            return Vec::new();
        };
        // Only a command that saved something counts as a capture, matching
        // what the weekly meter counts. Everything else still counts as words
        // spoken and as a day the app was used.
        let kind = match outcome {
            Some(o) if matches!(o.kind, "save" | "note") => ActivityKind::Capture,
            _ => ActivityKind::Other,
        };
        let speech_ms = (audio_secs * 1000.0).round().max(0.0) as u32;
        if let Err(e) = db.record_activity(count_words(text), speech_ms, kind) {
            tracing::warn!(?e, "could not record the activity");
            return Vec::new();
        }
        match db.evaluate() {
            Ok(earned) => {
                for e in &earned {
                    tracing::info!(code = ?e.code, title = %e.title, "earned");
                }
                earned
            }
            Err(e) => {
                tracing::warn!(?e, "could not evaluate achievements");
                Vec::new()
            }
        }
    }

    /// Route a transcript and execute it.
    ///
    /// Tier 0 first, always: it is deterministic, costs microseconds, and
    /// handles the formulaic majority. Tier 1 is consulted only when Tier 0
    /// refuses or cannot settle a destination — the two cases that, until now,
    /// ended in "not sure what to do with that" or a question the user had to
    /// answer by hand.
    fn route_and_execute(
        &self,
        text: &str,
        context: &Context,
    ) -> (Option<memos_agent::Outcome>, Option<Ambiguity>) {
        let Some(db) = self.db.read().clone() else {
            return (None, None);
        };
        let paths = db.collection_paths().unwrap_or_default();

        // The cloud agent first, and it is the system: it reads the whole
        // sentence, it can act more than once, and it is the only thing here
        // that can create the destination a command asks to file into.
        if let Some(cloud) = self.cloud.read().clone() {
            match self.run_cloud(&db, &cloud, text, context, &paths) {
                Ok(result) => {
                    *self.cloud_error.write() = None;
                    return result;
                }
                // Not fatal, and not silent. No key, no network, a refusal or a
                // turn that ran long all land here, and the grammar gets the
                // command instead — which is exactly the product without this
                // tier rather than a failure of it.
                Err(e) => {
                    tracing::warn!(error = %e, "the cloud router did not answer; falling back");
                    *self.cloud_error.write() = Some(e.to_string());
                }
            }
        }

        match memos_agent::parse(text, &paths) {
            Tier0::Routed(cmd) => {
                tracing::info!(
                    intent = cmd.intent.as_str(),
                    collection = ?cmd.slots.collection,
                    routing_ms = cmd.routing_ms,
                    tier = ?cmd.tier,
                    "routed"
                );
                let cmd = with_history(&db, cmd);
                log_command(&db, &cmd, context);
                self.execute(&db, &cmd, context, text)
            }
            Tier0::Ambiguous {
                intent,
                slot,
                resolution,
                transcript,
            } => {
                tracing::info!(slot, margin = resolution.margin, "ambiguous; asking");
                // Logged before it is asked, because what is being recorded is
                // the *prediction* — the intent the grammar was sure about and
                // the destination it was not. The user's answer becomes a
                // verdict on this row rather than a second command (ADR-0006).
                let prediction = memos_core::RoutedCommand {
                    id: memos_core::Id::new(),
                    transcript: transcript.clone(),
                    intent,
                    slots: memos_core::Slots::default(),
                    confidence: memos_core::Confidence {
                        logprob: 1.0,
                        margin: resolution.margin,
                        prior: 1.0,
                    },
                    tier: memos_core::Tier::Grammar,
                    routing_ms: 0,
                };
                let command_id = log_command(&db, &with_history(&db, prediction), context);

                // Hold the decision that was already made, minus the one slot
                // nobody could settle. Re-routing the transcript when the answer
                // arrives would run the grammar again and could land somewhere
                // else entirely.
                if let (Some(q), Some(command_id)) = (self.questions.read().as_ref(), command_id) {
                    q.ask(
                        command_id,
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
                // The transcript is still shown, so the user learns they were
                // heard correctly and the phrasing was the problem — which is
                // all a grammar ever had on offer.
                //
                // And it is logged, which matters more than it looks: a row
                // nobody could route is the to-do list, and the only place the
                // system records that it heard something it does not yet handle
                // (ADR-0006).
                tracing::debug!(text, "no shape matched and the cloud tier was not there");
                log_command(
                    &db,
                    &memos_core::RoutedCommand {
                        id: memos_core::Id::new(),
                        transcript: text.to_string(),
                        intent: memos_core::Intent::Unknown,
                        slots: memos_core::Slots::default(),
                        confidence: memos_core::Confidence {
                            logprob: 0.0,
                            margin: 0.0,
                            prior: 0.0,
                        },
                        tier: memos_core::Tier::Grammar,
                        routing_ms: 0,
                    },
                    context,
                );
                (None, None)
            }
        }
    }

    /// Run one command as a plan, and report the last thing it did.
    ///
    /// Every step is logged and executed exactly as a routed command, because
    /// it is one: the tier changes, the action space does not. That keeps undo,
    /// the correction log and the weekly meter working across a plan the same
    /// way they worked across a single grammar match.
    ///
    /// What goes back to the model is the receipt the user would have seen. It
    /// is the honest answer to "did that work", and it carries what a second
    /// step needs — "No collection called X" is how the model learns to make it
    /// before filing into it.
    fn run_cloud(
        &self,
        db: &Db,
        cloud: &memos_cloud::Cloud,
        text: &str,
        context: &Context,
        paths: &[String],
    ) -> Result<(Option<memos_agent::Outcome>, Option<Ambiguity>), memos_cloud::CloudError> {
        let mut last: Option<memos_agent::Outcome> = None;

        let plan = cloud.run(text, context, paths, |step| {
            let cmd = with_history(
                db,
                memos_core::RoutedCommand {
                    id: memos_core::Id::new(),
                    transcript: text.to_string(),
                    intent: step.intent,
                    slots: step.slots.clone(),
                    // A tool call under a strict schema guarantees the *shape*
                    // of the arguments and says nothing about whether the
                    // destination was the right one, so this is not CERTAIN.
                    // ADR-0005: confidence is computed here, never self-reported
                    // by a model.
                    confidence: memos_core::Confidence {
                        logprob: 0.9,
                        margin: 0.9,
                        prior: 1.0,
                    },
                    tier: memos_core::Tier::Cloud,
                    routing_ms: 0,
                },
            );
            log_command(db, &cmd, context);
            let (outcome, _) = self.execute(db, &cmd, context, text);
            let receipt = outcome
                .as_ref()
                .map(|o| o.summary.clone())
                .unwrap_or_else(|| "that did not work".to_string());
            if let Some(o) = outcome {
                last = Some(o);
            }
            receipt
        })?;

        tracing::info!(steps = plan.steps.len(), took_ms = plan.took_ms, "cloud plan");

        // A plan that executed nothing still has something to show: the model's
        // own line, which is what a question or an unactionable sentence
        // produces. Without this the overlay would go blank on exactly the
        // commands the old tiers failed silently.
        if last.is_none() {
            if let Some(say) = plan.say {
                last = Some(memos_agent::Outcome {
                    kind: "said",
                    summary: say,
                    provenance: None,
                    item_id: None,
                    results: Vec::new(),
                    open: None,
                    took_ms: plan.took_ms,
                });
            }
        }
        Ok((last, None))
    }

    /// Execute a routed command, whichever tier produced it.
    ///
    /// Shared on purpose: a command routed by the grammar and the same command
    /// routed by the model must do exactly the same thing, or "it worked
    /// yesterday" becomes a question about which tier happened to catch it.
    fn execute(
        &self,
        db: &Db,
        cmd: &memos_core::RoutedCommand,
        context: &Context,
        text: &str,
    ) -> (Option<memos_agent::Outcome>, Option<Ambiguity>) {
        // Embed the query only for the intents that retrieve. A SAVE pays
        // nothing for a model it does not use, which is what keeps the capture
        // path at the latency the budget assumes.
        let vector = match cmd.intent {
            memos_core::Intent::Search | memos_core::Intent::Show | memos_core::Intent::Open => {
                self.embeddings
                    .read()
                    .clone()
                    .and_then(|e| e.embed_query(cmd.slots.query.as_deref().unwrap_or(text)))
            }
            _ => None,
        };

        // "Save this screenshot" means the image on the clipboard, and naming it
        // is the only thing that lets it be read. Done here, after routing,
        // because OCR is too slow for the context deadline.
        let with_image;
        let context = if cmd.intent == memos_core::Intent::Save && names_image(cmd) {
            with_image = Context {
                image_text: memos_context::clipboard_image_text(),
                ..context.clone()
            };
            &with_image
        } else {
            context
        };

        match memos_agent::execute_with(db, cmd, context, vector.as_deref()) {
            Ok(out) => {
                // Only a real capture counts against the weekly meter. Metering
                // a search, or a command that failed, would be charging the
                // user for nothing.
                if matches!(out.kind, "save" | "note") {
                    let _ = db.record_capture(&crate::week_start());
                    // A new item means work for the embedding worker. Nudging
                    // beats polling: the backfill starts within milliseconds of
                    // the commit instead of up to 30 s later, so search is
                    // current almost immediately.
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

    /// Queue a capture. Returns immediately.
    pub fn submit(
        &self,
        audio: Vec<f32>,
        hints: Hints,
        context: Context,
        mode: Mode,
        released: std::time::Instant,
    ) -> bool {
        match self.tx.read().as_ref() {
            Some(tx) => tx
                .send(Job::Transcribe {
                    audio,
                    hints,
                    context,
                    mode,
                    released,
                })
                .is_ok(),
            None => false,
        }
    }

    #[cfg(feature = "whisper")]
    fn load(&self, explicit: Option<PathBuf>) {
        let data = crate::data_dir();
        let bundle = crate::bundle_dir();
        let Some(path) = memos_stt::find_model(explicit.as_deref(), &data, bundle.as_deref())
        else {
            *self.state.write() = ModelState::Missing;
            *self.detail.write() =
                format!(
                    "No speech model found. Run {} base.en",
                    memos_core::scripts::FETCH_MODELS
                );
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

/// Whether a save names an image in its own words, not in where it is going.
///
/// "Save this to images" is a destination, and reading an old screenshot off
/// the clipboard because a collection happens to be called that would save
/// something the user never meant.
fn names_image(cmd: &memos_core::RoutedCommand) -> bool {
    let destination = cmd.slots.collection.as_deref().unwrap_or("").to_lowercase();
    let destination: Vec<&str> = destination
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    let spoken: Vec<String> = cmd
        .transcript
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty() && !destination.contains(&w.to_lowercase().as_str()))
        .map(str::to_string)
        .collect();
    memos_context::mentions_image(&spoken.join(" "))
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

/// Fill in the one confidence field no router can measure for itself.
///
/// ADR-0005 defines confidence over three signals: the decode's own token
/// probabilities, the margin between candidate resolutions, and **the
/// historical acceptance rate for this intent, for this user**. The first two
/// are produced where the routing happens. The third only exists here, because
/// it is a question about the past rather than about this command.
///
/// A rate is used only once it is one — a single answered question is not
/// evidence, and a threshold that moves on one click would make the system feel
/// arbitrary in exactly the week the user is deciding whether to trust it.
/// Until then the default stands and behaviour is unchanged, which is what
/// ADR-0005 means by conservative cold-start defaults.
fn with_history(db: &Db, mut cmd: memos_core::RoutedCommand) -> memos_core::RoutedCommand {
    if let Ok(Some(rate)) = db.accept_rate(cmd.intent, MIN_CORRECTIONS_FOR_A_RATE) {
        cmd.confidence.prior = rate;
    }
    cmd
}

/// How many verdicts an intent needs before its acceptance rate is believed.
///
/// Five, which is small enough to become real within a week of ordinary use and
/// large enough that no single click moves it far. ADR-0006 puts the comparable
/// figure for kNN few-shot at around ten; a single ratio needs less than a
/// nearest-neighbour search does.
const MIN_CORRECTIONS_FOR_A_RATE: u32 = 5;

/// Write one routing decision to the correction log.
///
/// Never allowed to fail the command it describes. The log is the most valuable
/// table in the system (ADR-0006) and it is still only a record: a user whose
/// disk is full should lose the note about the save, not the save.
fn log_command(
    db: &Db,
    cmd: &memos_core::RoutedCommand,
    context: &Context,
) -> Option<memos_core::Id> {
    match db.log_command(cmd, &context.digest(), cmd.routing_ms) {
        Ok(id) => Some(id),
        Err(e) => {
            tracing::warn!(?e, "could not log the command");
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn save(transcript: &str, collection: Option<&str>) -> memos_core::RoutedCommand {
        memos_core::RoutedCommand {
            id: memos_core::Id::new(),
            transcript: transcript.into(),
            intent: memos_core::Intent::Save,
            slots: memos_core::Slots {
                collection: collection.map(str::to_string),
                ..Default::default()
            },
            confidence: memos_core::Confidence::CERTAIN,
            tier: memos_core::Tier::Grammar,
            routing_ms: 0,
        }
    }

    #[test]
    fn a_save_that_names_an_image_reads_one() {
        assert!(names_image(&save("save this screenshot to react", Some("Study/React"))));
        assert!(!names_image(&save("save this to react", Some("Study/React"))));
    }

    #[test]
    fn a_collection_called_images_is_not_an_image() {
        assert!(!names_image(&save("save this to images", Some("Media/Images"))));
        assert!(names_image(&save("save this image to images", Some("Media/Images"))));
    }
}
