//! Tier 1, as the application sees it.
//!
//! One method — "here is a transcript Tier 0 could not route, can you?" — with
//! the model, the grammar and the JSON parsing all behind it. The feature gate
//! lives here rather than at every call site, so the escalation path reads the
//! same whether the router was compiled in or not.
//!
//! Escalation is deliberately narrow. Tier 0 answers the formulaic majority in
//! microseconds and never consults this; a route costs ~600 ms, which is
//! affordable exactly once per command that would otherwise have failed, and
//! not at all on the ones that already worked.

use std::path::PathBuf;
use std::sync::Arc;

use memos_core::RoutedCommand;

#[cfg(feature = "router")]
pub use memos_llm::runner::ModelState;

/// Where the router model lives, if it has been fetched.
#[cfg(feature = "router")]
pub fn find_model() -> Option<PathBuf> {
    memos_llm::runner::find_model(None, &crate::data_dir())
}

#[cfg(not(feature = "router"))]
pub fn find_model() -> Option<PathBuf> {
    None
}

#[cfg(feature = "router")]
pub struct Tier1(Arc<memos_llm::runner::Router>);

#[cfg(feature = "router")]
impl Tier1 {
    pub fn new() -> Arc<Self> {
        Arc::new(Self(memos_llm::runner::Router::new()))
    }

    /// Load the model and prefill the prompt, off-thread.
    pub fn start(&self, model: PathBuf, collections: Vec<String>) {
        self.0.start(model, collections);
    }

    pub fn state(&self) -> ModelState {
        self.0.state()
    }

    pub fn detail(&self) -> String {
        self.0.detail()
    }

    /// The collection set changed, so the grammar and the prefilled prompt are
    /// both stale. ADR-0003 singles this out: a stale grammar does not fail, it
    /// silently makes a valid destination unreachable.
    pub fn collections_changed(&self, collections: Vec<String>) {
        self.0.collections_changed(collections);
    }

    /// Route a transcript Tier 0 gave up on.
    ///
    /// `None` covers every way this can not work — no model, still loading, too
    /// slow, or output the grammar should have prevented. They are all the same
    /// thing to the caller: Tier 1 did not resolve this, so the command stays
    /// unrouted, exactly as it was before this tier existed.
    pub fn route(&self, transcript: &str, collections: &[String]) -> Option<RoutedCommand> {
        let started = std::time::Instant::now();
        let json = self.0.route(transcript)?;
        let took = started.elapsed().as_millis() as u32;

        match memos_llm::parse(transcript, &json, collections, took) {
            Ok(cmd) => {
                tracing::info!(
                    intent = cmd.intent.as_str(),
                    collection = ?cmd.slots.collection,
                    routing_ms = took,
                    "escalated to Tier 1"
                );
                Some(cmd)
            }
            // The grammar is supposed to make this unreachable, so it is worth
            // saying loudly rather than swallowing: it means the structural
            // guarantee this tier rests on did not hold.
            Err(e) => {
                tracing::warn!(error = %e, json = %json, "Tier 1 output rejected");
                None
            }
        }
    }
}

/// Without the feature there is no router, and Tier 0 is the whole system.
#[cfg(not(feature = "router"))]
pub struct Tier1;

#[cfg(not(feature = "router"))]
#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    Missing,
}

#[cfg(not(feature = "router"))]
impl Tier1 {
    pub fn new() -> Arc<Self> {
        Arc::new(Self)
    }

    pub fn start(&self, _model: PathBuf, _collections: Vec<String>) {}

    pub fn state(&self) -> ModelState {
        ModelState::Missing
    }

    pub fn detail(&self) -> String {
        "Built without the `router` feature.".into()
    }

    pub fn collections_changed(&self, _collections: Vec<String>) {}

    pub fn route(&self, _transcript: &str, _collections: &[String]) -> Option<RoutedCommand> {
        None
    }
}
