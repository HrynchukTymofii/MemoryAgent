//! Routing, as a tool-calling agent.
//!
//! One sentence goes out with the action space attached; what comes back is a
//! sequence of tool calls, executed here against the local store and fed back
//! until the model says it is done. This replaces the grammar-first policy of
//! ADR-0003 as the primary path — see ADR-0011 for why.
//!
//! Three things this file is careful about, all of them consequences of running
//! on the capture path rather than in a chat window:
//!
//! 1. **What leaves the machine.** The transcript, the window title, the URL
//!    and a short excerpt of the selection. Not the page, not the clipboard,
//!    and not the stored memories — a router needs to know what the user is
//!    pointing at, not to read it.
//! 2. **The turn is bounded.** A capped number of round trips and a request
//!    timeout, because the user is standing in front of a pill waiting for it
//!    to say something.
//! 3. **Failure is a value, not a panic.** Every way this can fail — no key,
//!    no network, a refusal, a malformed reply — comes back as an error the
//!    caller can fall back from, because there is still a grammar downstairs.

pub mod tools;

use std::time::{Duration, Instant};

use memos_context::Context;
use serde_json::{json, Value};

pub use tools::Step;

/// The model. Opus 5, because the whole point of this tier is that it is the
/// one that can actually read an awkward sentence and work out the two actions
/// hiding in it.
pub const MODEL: &str = "claude-opus-5";

/// How long one request may take.
///
/// Generous by the standards of the latency budget and tight by the standards
/// of a reasoning model: past this the command has failed as a voice command
/// whatever the model was going to say.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

/// How many times the model may call tools before we stop feeding it results.
///
/// Four covers everything the action space can express — the longest real plan
/// is make a collection, save into it, and say so. A model looping on a failing
/// tool is the case this actually guards.
const MAX_TURNS: u32 = 4;

/// Longest excerpt of the user's selection sent with the command.
const MAX_EXCERPT: usize = 400;

/// Cap on one response, thinking included. Not a cost control — a stop so a
/// runaway turn ends in an error rather than in a two-minute wait.
const MAX_TOKENS: u32 = 8192;

#[derive(Debug, thiserror::Error)]
pub enum CloudError {
    #[error("no API key")]
    NoKey,
    #[error("the network: {0}")]
    Network(String),
    #[error("the API refused: {0}")]
    Status(String),
    #[error("declined: {0}")]
    Refused(String),
    #[error("unreadable reply: {0}")]
    Malformed(String),
    #[error("still working after {0} turns")]
    Unfinished(u32),
}

/// What one command turned into.
#[derive(Debug, Clone)]
pub struct Plan {
    /// Every step that was executed, in order.
    pub steps: Vec<Step>,
    /// The model's closing sentence, if it wrote one. The overlay prefers the
    /// outcome of the last step — this is what a command that did nothing
    /// executable (a question, a misfire) has to show instead.
    pub say: Option<String>,
    pub took_ms: u32,
}

/// The agent. Cheap to construct, safe to share, holds no conversation state:
/// each command is its own turn and nothing carries over between them.
pub struct Cloud {
    key: String,
    model: String,
    http: reqwest::blocking::Client,
}

impl Cloud {
    /// `None` when there is no key, which is a state the app is expected to be
    /// in — a fresh install has not been given one, and the grammar still runs.
    pub fn new(key: Option<String>) -> Option<Self> {
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty())?;
        let http = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .ok()?;
        Some(Self { key, model: MODEL.to_string(), http })
    }

    /// Run one command to completion.
    ///
    /// `act` executes a step and returns the one-line outcome the model is told
    /// about. It is a callback rather than a trait so this crate never learns
    /// what a database is: it knows the shape of an action and nothing about
    /// performing one.
    pub fn run(
        &self,
        transcript: &str,
        ctx: &Context,
        collections: &[String],
        mut act: impl FnMut(&Step) -> String,
    ) -> Result<Plan, CloudError> {
        let started = Instant::now();
        let mut messages = vec![json!({"role": "user", "content": command(transcript, ctx)})];
        let mut steps = Vec::new();

        for turn in 0..MAX_TURNS {
            let reply = self.post(&messages, collections)?;

            let stop = reply.get("stop_reason").and_then(Value::as_str).unwrap_or("");
            if stop == "refusal" {
                let why = reply
                    .get("stop_details")
                    .and_then(|d| d.get("explanation"))
                    .and_then(Value::as_str)
                    .unwrap_or("no reason given");
                return Err(CloudError::Refused(why.to_string()));
            }

            let content = reply
                .get("content")
                .and_then(Value::as_array)
                .ok_or_else(|| CloudError::Malformed("no content".into()))?
                .clone();

            let calls: Vec<&Value> = content
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("tool_use"))
                .collect();

            if calls.is_empty() {
                return Ok(Plan {
                    steps,
                    say: say(&content),
                    took_ms: started.elapsed().as_millis() as u32,
                });
            }

            // Every result goes back in one user message. Splitting them across
            // several is how a harness quietly teaches the model to stop asking
            // for more than one thing at a time.
            let mut results = Vec::with_capacity(calls.len());
            for call in calls {
                let id = call.get("id").and_then(Value::as_str).unwrap_or_default();
                let name = call.get("name").and_then(Value::as_str).unwrap_or_default();
                let input = call.get("input").cloned().unwrap_or_else(|| json!({}));

                match tools::step(id, name, &input) {
                    Ok(step) => {
                        tracing::info!(intent = step.intent.as_str(), turn, "cloud step");
                        let outcome = act(&step);
                        results.push(json!({
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": outcome,
                        }));
                        steps.push(step);
                    }
                    // Reported as a failed result rather than dropped, so the
                    // model can correct itself instead of waiting for an answer
                    // that is never coming.
                    Err(why) => results.push(json!({
                        "type": "tool_result",
                        "tool_use_id": id,
                        "content": why,
                        "is_error": true,
                    })),
                }
            }

            messages.push(json!({"role": "assistant", "content": content}));
            messages.push(json!({"role": "user", "content": results}));
        }

        Err(CloudError::Unfinished(MAX_TURNS))
    }

    fn post(&self, messages: &[Value], collections: &[String]) -> Result<Value, CloudError> {
        let body = json!({
            "model": self.model,
            "max_tokens": MAX_TOKENS,
            // Adaptive rather than off: with thinking disabled this model will
            // occasionally write a tool call into its visible text, where it
            // reads like an answer and never runs. Low effort is the latency
            // control instead, and this is a routing decision, not a hard one.
            "thinking": {"type": "adaptive"},
            "output_config": {"effort": "low"},
            // A policy decline on a routing turn should degrade to a different
            // model rather than to "not understood", so the fallback is opted
            // into here rather than left to the caller.
            "fallbacks": "default",
            "system": system(collections),
            "tools": tools::schemas(),
            "messages": messages,
        });

        let response = self
            .http
            .post("https://api.anthropic.com/v1/messages")
            .header("content-type", "application/json")
            .header("x-api-key", &self.key)
            .header("anthropic-version", "2023-06-01")
            .header("anthropic-beta", "server-side-fallback-2026-07-01")
            .json(&body)
            .send()
            .map_err(|e| CloudError::Network(e.to_string()))?;

        let status = response.status();
        let text = response
            .text()
            .map_err(|e| CloudError::Network(e.to_string()))?;
        if !status.is_success() {
            // The body carries the real reason — an invalid key, a rate limit,
            // a malformed tool schema — and a bare status code would send
            // whoever reads the log looking in the wrong place.
            return Err(CloudError::Status(format!(
                "{status}: {}",
                text.chars().take(300).collect::<String>()
            )));
        }
        serde_json::from_str(&text).map_err(|e| CloudError::Malformed(e.to_string()))
    }
}

/// The stable half of the prompt: who the model is and where things can go.
///
/// Cached, and ordered so the cacheable part is genuinely stable — the
/// instructions never change and the collection list changes when the user
/// makes a collection, which is rare enough that a prefix hit is the normal
/// case. Nothing per-command appears here; that is what the user message is for.
fn system(collections: &[String]) -> Value {
    let mut text = String::from(
        "You route voice commands for a personal memory app. The user spoke one \
         sentence while looking at something on their screen, and it was \
         transcribed imperfectly — words are dropped, especially the verb. Work \
         out what they wanted and call the tools that do it.\n\n\
         Rules:\n\
         - \"this\", \"that\", \"it\" mean what they are looking at. Save it.\n\
         - A sentence can be more than one action. \"Put this under a new \
         collection for X\" is create_collection then save.\n\
         - Only pass a collection path that appears in the list below. If the \
         user named somewhere else, make it first.\n\
         - Match the tree's shape: a subject usually belongs under an existing \
         top-level collection rather than beside it.\n\
         - Prefer acting on a plausible reading over asking. The user can undo, \
         and a command that does nothing is worse than one filed a level off.\n\
         - If nothing in the sentence is actionable, call no tools and reply \
         with one short line saying so.\n\
         - Never explain yourself at length. Anything you say is read aloud off \
         a small pill, so one line at most.\n\n\
         Collections:\n",
    );
    if collections.is_empty() {
        text.push_str("(none yet — create_collection before filing anything)\n");
    }
    for path in collections {
        text.push_str("  ");
        text.push_str(path);
        text.push('\n');
    }

    json!([{"type": "text", "text": text, "cache_control": {"type": "ephemeral"}}])
}

/// The volatile half: this command, and what the user was pointing at.
///
/// Deliberately thin. The window title and the URL are what a destination is
/// chosen from; the excerpt is there so "this" has a subject when the title is
/// useless. The page text is not sent — it is the largest thing on the capture
/// path and the least necessary for deciding where something goes.
fn command(transcript: &str, ctx: &Context) -> String {
    let mut out = format!("Command: {transcript}\n\nOn screen:\n");
    let mut line = |label: &str, value: Option<&str>| {
        if let Some(v) = value.map(str::trim).filter(|v| !v.is_empty()) {
            out.push_str(&format!("  {label}: {v}\n"));
        }
    };
    line("app", ctx.active_application.as_deref());
    line("window", ctx.active_window_title.as_deref());
    line("url", ctx.current_url.as_deref());

    match ctx.selected_text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(selection) => {
            let excerpt: String = selection.chars().take(MAX_EXCERPT).collect();
            out.push_str(&format!("  selected: {excerpt}\n"));
        }
        None if ctx.page_text.as_deref().is_some_and(|p| !p.trim().is_empty()) => {
            out.push_str("  (a readable page, not shown)\n");
        }
        None => out.push_str("  (nothing selected)\n"),
    }
    out
}

/// The model's closing line, if it wrote one.
fn say(content: &[Value]) -> Option<String> {
    let text: String = content
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join(" ");
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(200).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_key_is_not_an_agent() {
        assert!(Cloud::new(None).is_none());
        assert!(Cloud::new(Some("   ".into())).is_none());
        assert!(Cloud::new(Some("sk-ant-test".into())).is_some());
    }

    /// The prompt is the cached prefix, so anything per-command that leaked
    /// into it would invalidate the cache on every single capture.
    #[test]
    fn the_system_prompt_holds_nothing_per_command() {
        let a = system(&["Study".to_string(), "Life".to_string()]);
        let b = system(&["Study".to_string(), "Life".to_string()]);
        assert_eq!(a, b);
        assert_eq!(a[0]["cache_control"]["type"], "ephemeral");
        assert!(a[0]["text"].as_str().unwrap().contains("  Study\n"));
    }

    /// An empty library is a real state — a fresh install — and the prompt has
    /// to tell the model what to do about it rather than listing nothing.
    #[test]
    fn an_empty_library_says_so() {
        let s = system(&[]);
        assert!(s[0]["text"].as_str().unwrap().contains("create_collection before filing"));
    }

    /// What leaves the machine is a product decision, so it is a test.
    #[test]
    fn the_page_and_the_clipboard_stay_here() {
        let ctx = Context {
            active_application: Some("chrome.exe".into()),
            active_window_title: Some("Unprompted — speaking practice".into()),
            current_url: Some("https://unprompted.cool".into()),
            page_text: Some("a whole article about speaking in public".into()),
            clipboard_text: Some("a password, probably".into()),
            ..Default::default()
        };
        let sent = command("save this to public speaking", &ctx);
        assert!(sent.contains("unprompted.cool"));
        assert!(sent.contains("(a readable page, not shown)"));
        assert!(!sent.contains("a whole article"), "{sent}");
        assert!(!sent.contains("password"), "{sent}");
    }

    #[test]
    fn a_long_selection_is_cut_to_the_excerpt() {
        let ctx = Context {
            selected_text: Some("word ".repeat(500)),
            ..Default::default()
        };
        let sent = command("save this", &ctx);
        assert!(sent.len() < MAX_EXCERPT + 200, "sent {} chars", sent.len());
    }
}
