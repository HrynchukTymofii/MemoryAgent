//! Tier 1, in a process of its own.
//!
//! ADR-0008. Reads `protocol::Request` from stdin, one JSON object per line,
//! and writes `protocol::Response` to stdout the same way. The model, the KV
//! cache and the grammar all live here; the app holds nothing but a pipe.
//!
//! Two rules this file exists to keep:
//!
//! 1. **Stdout is the protocol and nothing else.** A stray `println!` would be
//!    parsed as a message and desynchronise the conversation. Every diagnostic
//!    goes to stderr, which the parent forwards into its own log.
//! 2. **Stdin closing means the app is gone.** The read loop ends and the
//!    process exits, so a crashed or killed parent cannot leave a
//!    half-gigabyte model resident. That is the whole orphan-prevention
//!    strategy, and it works because it is the operating system closing the
//!    handle rather than anything this code has to remember to do.
//!
//! Run it by hand to see it work:
//!
//! ```text
//! echo {"op":"collections","collections":["Study"]} | memos-router models/llm/router.gguf
//! ```

use std::io::{BufRead, Write};
use std::sync::mpsc::{channel, Sender};
use std::sync::Arc;

use memos_llm::protocol::{ModelState, Request, Response, PROTOCOL};
use memos_llm::runner::Router;

fn main() {
    // stderr, explicitly. The default writer is stdout, and that is the wire.
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let Some(model) = std::env::args().nth(1).map(std::path::PathBuf::from) else {
        eprintln!("usage: memos-router <model.gguf>");
        std::process::exit(2);
    };
    if !model.exists() {
        eprintln!("no model at {}", model.display());
        std::process::exit(2);
    }

    let out = writer();
    let router = Router::new();
    let mut started = false;

    // Locking stdin for the whole run: nothing else reads it, and the lock is
    // what lets the iterator borrow the buffer instead of allocating per line.
    let stdin = std::io::stdin();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) => l,
            // A read error on a pipe means the far end is gone, which is the
            // same as EOF as far as this process is concerned.
            Err(e) => {
                tracing::debug!(error = %e, "stdin closed");
                break;
            }
        };
        if line.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Request>(&line) {
            // Not fatal. One unreadable line is a bug in the parent worth
            // saying loudly, but killing the router over it would turn a
            // cosmetic problem into a lost tier.
            Err(e) => tracing::error!(error = %e, line = %line, "unreadable request"),

            Ok(Request::Collections { collections }) => {
                if started {
                    router.collections_changed(collections);
                } else {
                    started = true;
                    router.start(model.clone(), collections);
                    watch_loading(&router, out.clone());
                }
            }

            Ok(Request::Route { id, transcript }) => {
                // Serial on purpose: the model is one context on one thread, so
                // concurrency here would only queue somewhere less visible. The
                // app sends one command at a time and gives up after 1.5 s.
                let reply = match router.route(&transcript) {
                    Some((json, decode)) => Response::Routed { id, json, decode },
                    None => Response::Declined {
                        id,
                        error: match router.state() {
                            ModelState::Ready => "no answer within the deadline".into(),
                            other => format!("router is {}", state_name(other)),
                        },
                    },
                };
                let _ = out.send(reply);
            }
        }
    }

    tracing::info!("parent closed the pipe; router exiting");
}

/// One thread owns stdout.
///
/// The loading watcher and the request loop both produce messages, and two
/// threads interleaving writes would split a line down the middle. A channel
/// into a single writer makes that unrepresentable rather than unlikely.
fn writer() -> Sender<Response> {
    let (tx, rx) = channel::<Response>();
    std::thread::Builder::new()
        .name("stdout".into())
        .spawn(move || {
            let stdout = std::io::stdout();
            for msg in rx {
                let mut lock = stdout.lock();
                // Flushed per message. The app is waiting on this line with a
                // deadline, and a buffered reply that arrives after it is
                // indistinguishable from no reply at all.
                if lock.write_all(msg.line().as_bytes()).is_err() || lock.flush().is_err() {
                    break;
                }
            }
        })
        .expect("spawn stdout thread");
    tx
}

/// Tell the app when the model becomes usable, or fails to.
///
/// Polled rather than pushed because `Router` publishes its state for a UI to
/// read, and giving it a callback for one consumer would be a worse shape than
/// checking twenty times a second for the couple of seconds a load takes.
fn watch_loading(router: &Arc<Router>, out: Sender<Response>) {
    let router = router.clone();
    std::thread::Builder::new()
        .name("loading".into())
        .spawn(move || {
            while router.state() == ModelState::Loading {
                std::thread::sleep(std::time::Duration::from_millis(50));
            }
            let _ = out.send(Response::State {
                state: router.state(),
                detail: router.detail(),
                protocol: PROTOCOL,
            });
        })
        .expect("spawn loading thread");
}

fn state_name(s: ModelState) -> &'static str {
    match s {
        ModelState::Loading => "still loading",
        ModelState::Ready => "ready",
        ModelState::Missing => "missing",
        ModelState::Failed => "unavailable",
    }
}
