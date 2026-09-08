//! The embedding worker.
//!
//! Everything here is deliberately off the critical path. A capture commits and
//! is acknowledged with no vector attached (§4, stage 6); this thread notices
//! afterwards and backfills. Two consequences worth stating plainly:
//!
//! 1. **A missing or broken embedding model degrades search, it does not break
//!    capture.** The keyword half of retrieval works on a machine that has never
//!    embedded anything, which is exactly the state of every machine for the
//!    first few seconds after install.
//! 2. **The queue is derived, not authoritative.** It is "items with no vector",
//!    so a job row lost to a crash costs nothing and a half-finished backfill
//!    resumes by itself on the next launch.

use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;

use memos_db::Db;
use parking_lot::RwLock;

#[cfg(feature = "embeddings")]
use memos_embed::{model::Role, Embedder, OnnxEmbedder};

/// How many items are embedded per pass.
///
/// A transformer's throughput comes from batching, but a large batch pads every
/// row out to the longest one in it — so this is sized for the common case of a
/// handful of short captures, not for a maximal backfill.
const BATCH: usize = 16;

/// How long the worker waits before looking again when the queue is empty.
///
/// A capture nudges it awake, so this is only a backstop for a nudge that was
/// dropped (the channel is bounded) or for rows written by something other than
/// the capture path. Long enough to be free, short enough that a stuck backlog
/// clears itself within a minute.
const IDLE_POLL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Loading,
    Ready,
    Missing,
    Failed,
}

#[derive(serde::Serialize)]
pub struct EmbedStatus {
    pub state: ModelState,
    pub detail: String,
    /// Vectors stored. With `pending`, this is the honest answer to "is search
    /// working yet?" — a number the Hub shows rather than a spinner.
    pub embedded: u32,
    pub pending: u32,
    pub model_id: String,
    pub dim: u32,
}

pub struct Embeddings {
    #[cfg(feature = "embeddings")]
    model: RwLock<Option<Arc<OnnxEmbedder>>>,
    state: RwLock<ModelState>,
    detail: RwLock<String>,
    /// Bounded and non-blocking: a nudge is a hint that there is work, and
    /// dropping one when the worker is already behind loses nothing, because
    /// what it will do next is re-read the queue anyway.
    nudge: RwLock<Option<SyncSender<()>>>,
}

impl Embeddings {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            #[cfg(feature = "embeddings")]
            model: RwLock::new(None),
            state: RwLock::new(ModelState::Loading),
            detail: RwLock::new(String::new()),
            nudge: RwLock::new(None),
        })
    }

    pub fn status(&self, db: &Db) -> EmbedStatus {
        EmbedStatus {
            state: *self.state.read(),
            detail: self.detail.read().clone(),
            embedded: db.embedding_count().unwrap_or(0),
            pending: db
                .items_awaiting_embedding(1_000)
                .map(|v| v.len() as u32)
                .unwrap_or(0),
            model_id: self.model_id(),
            dim: self.dim() as u32,
        }
    }

    /// Tell the worker there is something to do.
    pub fn nudge(&self) {
        if let Some(tx) = self.nudge.read().as_ref() {
            let _ = tx.try_send(());
        }
    }

    /// Start the worker: load the model, then drain the queue forever.
    pub fn start(self: &Arc<Self>, db: Arc<Db>) {
        let me = self.clone();
        let (tx, rx) = sync_channel::<()>(1);
        *self.nudge.write() = Some(tx);

        std::thread::Builder::new()
            .name("embed".into())
            .spawn(move || {
                me.load();
                loop {
                    // Drain to empty before sleeping: one nudge may stand for
                    // several captures, and a backlog left behind would only be
                    // noticed by search being quietly wrong.
                    while me.drain_once(&db) > 0 {}
                    let _ = rx.recv_timeout(IDLE_POLL);
                }
            })
            .expect("spawn embedding worker");
    }
}

#[cfg(feature = "embeddings")]
impl Embeddings {
    fn load(&self) {
        let data = crate::data_dir();
        let Some(dir) = memos_embed::find_model_dir(None, &data) else {
            *self.state.write() = ModelState::Missing;
            *self.detail.write() =
                "No embedding model found. Run scripts/fetch-models.ps1 embedding".into();
            tracing::warn!("no embedding model; search falls back to keywords only");
            crate::hotkey::diag("embedding model MISSING (keyword search still works)");
            return;
        };
        match OnnxEmbedder::load(&dir) {
            Ok(e) => {
                let (id, dim) = (e.model_id().to_string(), e.dim());
                *self.model.write() = Some(Arc::new(e));
                *self.state.write() = ModelState::Ready;
                *self.detail.write() = dir.display().to_string();
                crate::hotkey::diag(&format!("embedding model ready: {id} ({dim}d)"));
            }
            Err(e) => {
                *self.state.write() = ModelState::Failed;
                *self.detail.write() = e.to_string();
                tracing::error!(?e, "embedding model failed to load");
                crate::hotkey::diag(&format!("embedding model FAILED: {e}"));
            }
        }
    }

    /// Embed a search query.
    ///
    /// Uses the model's query prefix, which is not decoration: an asymmetric
    /// model like bge scores a question against a passage measurably worse when
    /// both are encoded the same way.
    pub fn embed_query(&self, text: &str) -> Option<Vec<f32>> {
        let model = self.model.read().clone()?;
        match model.embed_as(&[text], Role::Query) {
            Ok(mut v) if !v.is_empty() => Some(v.remove(0)),
            Ok(_) => None,
            Err(e) => {
                // Search still runs on keywords alone, so this is a degraded
                // result rather than a failed one.
                tracing::warn!(?e, "query embedding failed; keyword search only");
                None
            }
        }
    }

    /// One pass over the queue. Returns how many items were embedded.
    fn drain_once(&self, db: &Db) -> usize {
        let Some(model) = self.model.read().clone() else {
            return 0;
        };
        let batch = match db.items_awaiting_embedding(BATCH) {
            Ok(b) if !b.is_empty() => b,
            Ok(_) => return 0,
            Err(e) => {
                tracing::error!(?e, "could not read the embedding queue");
                return 0;
            }
        };

        let started = std::time::Instant::now();
        let texts: Vec<&str> = batch.iter().map(|(_, t)| t.as_str()).collect();
        let vectors = match model.embed(&texts) {
            Ok(v) => v,
            Err(e) => {
                // Charge the attempt to every item in the batch, then stop this
                // pass. Retrying immediately would spin against a broken model.
                tracing::error!(?e, count = batch.len(), "embedding batch failed");
                for (id, _) in &batch {
                    let _ = db.embedding_failed(*id, &e.to_string());
                }
                return 0;
            }
        };

        let mut done = 0;
        for ((id, _), vector) in batch.iter().zip(vectors) {
            match db.put_embedding(*id, model.model_id(), &vector) {
                Ok(()) => {
                    let _ = db.mark_embedded(*id);
                    done += 1;
                }
                Err(e) => {
                    tracing::error!(?e, %id, "could not store an embedding");
                    let _ = db.embedding_failed(*id, &e.to_string());
                }
            }
        }
        tracing::info!(
            count = done,
            took_ms = started.elapsed().as_millis() as u64,
            "embedded"
        );
        done
    }

    pub fn model_id(&self) -> String {
        self.model
            .read()
            .as_ref()
            .map(|m| m.model_id().to_string())
            .unwrap_or_default()
    }

    pub fn dim(&self) -> usize {
        self.model.read().as_ref().map(|m| m.dim()).unwrap_or(0)
    }
}

/// Without the feature there is no model, and search is keyword-only.
///
/// Not an error state: it is the same degradation as a missing model file, and
/// the product is expected to work in it.
#[cfg(not(feature = "embeddings"))]
impl Embeddings {
    fn load(&self) {
        *self.state.write() = ModelState::Missing;
        *self.detail.write() = "Built without the `embeddings` feature.".into();
        tracing::warn!("built without embeddings; search is keyword-only");
    }

    pub fn embed_query(&self, _text: &str) -> Option<Vec<f32>> {
        None
    }

    fn drain_once(&self, _db: &Db) -> usize {
        0
    }

    pub fn model_id(&self) -> String {
        String::new()
    }

    pub fn dim(&self) -> usize {
        0
    }
}
