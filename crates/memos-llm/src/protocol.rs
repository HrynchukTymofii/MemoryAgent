//! The line protocol between the app and the router process.
//!
//! ADR-0008. Tier 1 runs in its own process, so the call that used to be a
//! channel send is now a line of JSON on a pipe. The types live here, in the
//! half of this crate that compiles without llama.cpp, so both ends share one
//! definition and neither can drift from the other.
//!
//! ## Why lines and not a real RPC
//!
//! One request in flight at a time, a handful of bytes each way, and a peer
//! that is a child process rather than a network service. Framing that costs
//! microseconds against a route that costs ~600 ms is not worth a dependency,
//! and newline-delimited JSON stays readable when something goes wrong: the
//! whole conversation can be replayed by hand into the binary's stdin.
//!
//! Every message carries an `id` back so a reply cannot be attributed to the
//! request after the one it belongs to — which is what would happen after a
//! timeout, and the failure would look like the router answering a different
//! command than the one you spoke.

use serde::{Deserialize, Serialize};

/// Where the router model is in its life.
///
/// Lives here rather than beside the runner because the app needs to render it
/// (`router_status`) without linking llama.cpp, and the sidecar needs to send
/// it. Both sides therefore agree on the wire spelling by construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelState {
    /// Loading the weights and prefilling the prompt. Seconds, once, at start.
    Loading,
    /// Answering.
    Ready,
    /// No model file, or no sidecar binary to run it. Tier 0 is the system.
    Missing,
    /// It loaded and then something went wrong, or it kept crashing.
    Failed,
}

/// App to router.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// The user's collections. Sent once at startup to begin loading, and again
    /// whenever the set changes — ADR-0003 singles this out, because a stale
    /// grammar does not fail, it silently makes a valid destination
    /// unreachable.
    Collections { collections: Vec<String> },
    /// Route one transcript Tier 0 could not.
    Route { id: u64, transcript: String },
}

/// Router to app.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Response {
    /// Unsolicited: the model finished loading, or failed to.
    State { state: ModelState, detail: String },
    /// The model's JSON for request `id`. Still unvalidated — `parse` decides
    /// whether it means anything, on the app's side, where the collection list
    /// it has to be checked against actually lives.
    Routed { id: u64, json: String },
    /// Request `id` produced no answer, and why.
    Declined { id: u64, error: String },
}

impl Request {
    /// One line, newline included, ready to write to a pipe.
    ///
    /// Serialisation cannot fail for these types, so a caller is not made to
    /// handle an error that has no reachable branch.
    pub fn line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Request is always serialisable");
        s.push('\n');
        s
    }
}

impl Response {
    pub fn line(&self) -> String {
        let mut s = serde_json::to_string(self).expect("Response is always serialisable");
        s.push('\n');
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two ends are compiled into different binaries and are only ever
    /// updated together by hand. This is the test that notices when they are
    /// not.
    #[test]
    fn every_message_survives_the_pipe() {
        let messages: Vec<Request> = vec![
            Request::Collections {
                collections: vec!["Study".into(), "Study/Programming/React".into()],
            },
            Request::Route {
                id: 7,
                transcript: "stick this in with the python stuff".into(),
            },
        ];
        for m in messages {
            let line = m.line();
            assert!(line.ends_with('\n'));
            assert_eq!(serde_json::from_str::<Request>(line.trim()).unwrap(), m);
        }

        let replies = vec![
            Response::State {
                state: ModelState::Ready,
                detail: "812 tokens prefilled".into(),
            },
            Response::Routed {
                id: 7,
                json: r#"{"intent":"save"}"#.into(),
            },
            Response::Declined {
                id: 7,
                error: "timed out".into(),
            },
        ];
        for r in replies {
            let line = r.line();
            assert_eq!(serde_json::from_str::<Response>(line.trim()).unwrap(), r);
        }
    }

    /// A transcript is user speech and a collection can be any name they typed.
    /// Newlines in either would end the message early and desynchronise the
    /// pipe for every command after it, so this is load-bearing rather than
    /// theoretical.
    #[test]
    fn a_newline_in_the_payload_cannot_split_the_message() {
        let m = Request::Route {
            id: 1,
            transcript: "save this\n{\"op\":\"route\",\"id\":2}".into(),
        };
        let line = m.line();
        assert_eq!(line.matches('\n').count(), 1);
        assert_eq!(serde_json::from_str::<Request>(line.trim()).unwrap(), m);
    }

    /// The frontend renders these four strings (`api.ts`). Renaming a variant
    /// in Rust would otherwise change the wire format silently.
    #[test]
    fn the_state_names_are_the_ones_the_ui_knows() {
        for (state, name) in [
            (ModelState::Loading, "loading"),
            (ModelState::Ready, "ready"),
            (ModelState::Missing, "missing"),
            (ModelState::Failed, "failed"),
        ] {
            assert_eq!(serde_json::to_string(&state).unwrap(), format!("\"{name}\""));
        }
    }
}
