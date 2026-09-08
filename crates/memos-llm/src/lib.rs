//! Tier 1: the local router.
//!
//! ADR-0003. Tier 0's grammar absorbs the formulaic majority in microseconds
//! and refuses everything else. This is what "everything else" escalates to: a
//! sub-billion-parameter model, running locally, whose output is constrained by
//! a generated GBNF grammar so that it *structurally cannot* produce invalid
//! JSON, an unknown intent, or a collection the user does not have.
//!
//! The model is the least trustworthy component in the system and it is treated
//! that way. It chooses among alternatives we generated; it never invents one.
//!
//! This module is the model-independent half — grammar, prompt, and parsing —
//! and it is fully testable with no model present, which is the half that
//! decides whether the other half is safe.

pub mod grammar;
pub mod protocol;

#[cfg(feature = "local")]
pub mod runner;

pub use protocol::ModelState;

use std::path::{Path, PathBuf};
use std::time::Duration;

use memos_core::{Confidence, Intent, RoutedCommand, Slots, Tier};
use serde::Deserialize;

/// How long a caller will wait for a route before giving up on it.
///
/// The command is already spoken and the user is watching an overlay. Past this
/// point the honest thing is to say nothing was understood, rather than to keep
/// them waiting for a better answer.
///
/// Both ends of the pipe read this constant: the sidecar stops decoding here,
/// and the app waits slightly longer (`router::ROUTE_DEADLINE`) so the sidecar
/// gets to say *why* it gave up instead of just going quiet.
pub const ROUTE_TIMEOUT: Duration = Duration::from_millis(1_500);

/// Locate the router model.
///
/// Same order as the other two: the installed layout first, then the repository
/// one, so development and production need no build-time switch. Lives here
/// rather than beside the runner because the app is what has to find the file —
/// it passes the path to the sidecar, which never searches for anything.
pub fn find_model(explicit: Option<&Path>, data_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = explicit {
        candidates.push(p.to_path_buf());
    }
    candidates.push(data_dir.join("models/llm/router.gguf"));
    for prefix in ["models/llm", "../../models/llm", "../../../models/llm"] {
        candidates.push(PathBuf::from(prefix).join("router.gguf"));
    }
    candidates.into_iter().find(|p| p.exists())
}

#[derive(Debug, thiserror::Error)]
pub enum LlmError {
    #[error("model not found under {0}")]
    ModelMissing(std::path::PathBuf),
    #[error("failed to load the router model: {0}")]
    Load(String),
    #[error("routing failed: {0}")]
    Run(String),
    #[error("the model returned something the grammar should have prevented: {0}")]
    Malformed(String),
    #[error("built without the `local` feature")]
    Unavailable,
}

/// What the model is asked to produce.
///
/// Deliberately flat and tiny. Every field it could emit is a field the grammar
/// permits, so this struct and `grammar::build` have to agree — the test at the
/// bottom of this file is what keeps them in step.
#[derive(Debug, Deserialize, PartialEq)]
pub struct RouterOutput {
    pub intent: String,
    #[serde(default)]
    pub collection: Option<String>,
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
}

/// The instruction the model runs under.
///
/// Short on purpose. The grammar is what makes the output valid, so the prompt
/// only has to make it *correct* — and every token here is prefilled into the
/// KV cache once at startup rather than per command (ADR-0003), so its cost is
/// paid once but its length still bounds how much of the cache is reusable.
pub const SYSTEM_PROMPT: &str = "\
You turn a spoken command into one JSON object. The user is talking to a memory \
assistant while looking at something on screen. \"this\" means what they are \
looking at. Choose the single intent that matches what they asked for, and fill \
only the field that intent needs. Answer with the JSON object and nothing else.";

/// Build the user-turn text for one transcript.
pub fn user_turn(transcript: &str) -> String {
    format!("Command: {transcript}")
}

/// Everything the model should know before it sees a command.
///
/// **The collection list belongs here, not only in the grammar.** That
/// distinction cost a rewrite: the grammar constrains what the model may
/// *emit*, but a model choosing a destination has to know the options exist
/// while it is deciding — it meets the constraint one token at a time, long
/// after the decision is made. Left out, the first run of the router scored 2
/// of 9 on ordinary phrasings and put "file this under job applications" into
/// `Study`.
///
/// It is free per command because it is prefilled once into the KV cache and
/// reused (ADR-0003). That is the entire reason the cache exists — ~800 tokens
/// of prefix that would otherwise be re-read on every single utterance.
pub fn prefix(collections: &[String]) -> String {
    let mut out = String::from(SYSTEM_PROMPT);

    if !collections.is_empty() {
        out.push_str("\n\nThe user's collections, and the only destinations that exist:\n");
        for path in collections {
            out.push_str("- ");
            out.push_str(path);
            out.push('\n');
        }
        out.push_str(
            "\nChoose the most specific one that fits. If none of them clearly fits, \
             leave the collection out rather than picking a vague one.",
        );
    }

    // Worked examples, not rules. A 0.6B model generalises from a handful of
    // these far better than from any amount of instruction — and they are the
    // cheapest tokens in the system, paid for once at startup.
    out.push_str(
        "\n\nExamples:\n\
         \"save this\" -> {\"intent\":\"save\"}\n\
         \"put this with the react stuff\" -> {\"intent\":\"save\",\"collection\":\"<the react collection>\"}\n\
         \"what did I read about hooks\" -> {\"intent\":\"search\",\"query\":\"hooks\"}\n\
         \"open that pgbouncer page\" -> {\"intent\":\"open\",\"query\":\"pgbouncer\"}\n\
         \"pull up that typescript thing\" -> {\"intent\":\"search\",\"query\":\"typescript\"}\n\
         \"note that the bins go out tuesday\" -> {\"intent\":\"note\",\"title\":\"the bins go out tuesday\"}\n\
         A command about putting something somewhere is a save, not a search.",
    );
    out
}

/// Turn the model's JSON into a command.
///
/// `collections` is passed so a destination can be verified rather than
/// trusted. The grammar should already make an unknown one impossible — this
/// checks anyway, because "should" is doing a lot of work in that sentence and
/// the cost of being wrong is a memory filed somewhere that does not exist.
pub fn parse(
    transcript: &str,
    json: &str,
    collections: &[String],
    routing_ms: u32,
) -> Result<RoutedCommand, LlmError> {
    let out: RouterOutput =
        serde_json::from_str(json.trim()).map_err(|e| LlmError::Malformed(e.to_string()))?;

    let intent = Intent::from_name(&out.intent)
        .ok_or_else(|| LlmError::Malformed(format!("unknown intent {:?}", out.intent)))?;
    if !grammar::ROUTABLE.contains(&intent) {
        return Err(LlmError::Malformed(format!(
            "{} is not a Tier 1 intent",
            intent.as_str()
        )));
    }

    let mut slots = Slots::default();
    if let Some(path) = out.collection {
        if !collections.contains(&path) {
            return Err(LlmError::Malformed(format!("no collection {path:?}")));
        }
        slots.collection = Some(path);
    }
    slots.query = out.query.filter(|q| !q.trim().is_empty());
    slots.title = out.title.filter(|t| !t.trim().is_empty());
    slots.tags = out.tag.into_iter().filter(|t| !t.trim().is_empty()).collect();

    // Every intent Tier 1 can emit needs something to act on, except a save,
    // which is complete on its own. A model that filled nothing has not
    // understood the command, and executing that is worse than escalating.
    let empty = slots.collection.is_none()
        && slots.query.is_none()
        && slots.title.is_none()
        && slots.tags.is_empty();
    if empty && intent != Intent::Save {
        return Err(LlmError::Malformed(format!(
            "{} with no slots filled",
            intent.as_str()
        )));
    }

    Ok(RoutedCommand {
        transcript: transcript.to_string(),
        intent,
        slots,
        // Not CERTAIN. Tier 0 matching is a proof; this is a small model's
        // opinion, and section 8 requires confidence to be computed from
        // measurable signals rather than asserted. The decode logprobs fill
        // this in once the local runner lands; until then it is honestly
        // middling rather than dishonestly high.
        confidence: Confidence {
            logprob: 0.7,
            margin: 0.7,
            prior: 1.0,
        },
        tier: Tier::LocalRouter,
        routing_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        ["Study", "Study/Programming/React", "Life/House"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn a_save_with_a_destination_routes() {
        let cmd = parse(
            "put this with my react notes",
            r#"{"intent":"save","collection":"Study/Programming/React"}"#,
            &paths(),
            120,
        )
        .unwrap();
        assert_eq!(cmd.intent, Intent::Save);
        assert_eq!(cmd.slots.collection.as_deref(), Some("Study/Programming/React"));
        assert_eq!(cmd.tier, Tier::LocalRouter);
    }

    #[test]
    fn tier_one_never_reports_certainty() {
        // Tier 0 matching is a proof; this is a small model's opinion, and the
        // disambiguation gate downstream depends on the difference.
        let cmd = parse("save this", r#"{"intent":"save"}"#, &paths(), 90).unwrap();
        assert!(cmd.confidence.score() < Confidence::CERTAIN.score());
    }

    #[test]
    fn a_collection_the_user_does_not_have_is_refused() {
        // The grammar should make this unreachable. If it ever is reached, the
        // failure must be loud rather than a memory filed into a folder that
        // does not exist.
        let err = parse(
            "save this to work stuff",
            r#"{"intent":"save","collection":"Work Stuff"}"#,
            &paths(),
            100,
        );
        assert!(matches!(err, Err(LlmError::Malformed(_))));
    }

    #[test]
    fn an_intent_outside_tier_one_is_refused() {
        // RESEARCH belongs to Tier 2. A router that can route to reasoning can
        // route away from the fast path for anything it finds hard.
        let err = parse("explain this", r#"{"intent":"research"}"#, &paths(), 100);
        assert!(matches!(err, Err(LlmError::Malformed(_))));
    }

    #[test]
    fn malformed_json_is_an_error_not_a_guess() {
        assert!(parse("x", "not json", &paths(), 1).is_err());
        assert!(parse("x", r#"{"intent":}"#, &paths(), 1).is_err());
    }

    #[test]
    fn a_retrieval_with_nothing_to_retrieve_is_refused() {
        let err = parse("find", r#"{"intent":"search"}"#, &paths(), 100);
        assert!(matches!(err, Err(LlmError::Malformed(_))));
    }

    #[test]
    fn an_empty_slot_is_the_same_as_no_slot() {
        // A model that emits "" has filled nothing, and the grammar permits it:
        // the text rule allows zero characters.
        let err = parse("find", r#"{"intent":"search","query":"  "}"#, &paths(), 100);
        assert!(matches!(err, Err(LlmError::Malformed(_))));
    }

    #[test]
    fn a_bare_save_is_complete_on_its_own() {
        // "save this" with nowhere named is a real command, unlike a search
        // with nothing to search for.
        let cmd = parse("save this", r#"{"intent":"save"}"#, &paths(), 80).unwrap();
        assert!(cmd.slots.collection.is_none());
    }

    #[test]
    fn the_grammar_and_the_parser_agree_on_every_field() {
        // These two are one contract in two files. A field the grammar can
        // produce but the parser drops is a slot that silently disappears.
        let g = grammar::build(&paths());
        for field in ["collection", "query", "title", "tag"] {
            assert!(g.contains(field), "grammar cannot emit {field:?}");
        }
        let all = r#"{"intent":"save","collection":"Study"}"#;
        assert!(parse("x", all, &paths(), 1).is_ok());
    }
}
