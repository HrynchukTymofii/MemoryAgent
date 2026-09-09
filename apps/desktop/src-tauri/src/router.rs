//! Tier 1, as the application sees it.
//!
//! One method — "here is a transcript Tier 0 could not route, can you?" — with
//! a child process, a pipe and the JSON parsing all behind it.
//!
//! Escalation is deliberately narrow. Tier 0 answers the formulaic majority in
//! microseconds and never consults this; a route costs ~600 ms, which is
//! affordable exactly once per command that would otherwise have failed, and
//! not at all on the ones that already worked.
//!
//! ## Why the model is not in this process (ADR-0008)
//!
//! Two reasons, and only the second one is interesting.
//!
//! The linker: whisper.cpp and llama.cpp each vendor their own copy of ggml, so
//! putting both in one executable defines every ggml symbol twice and the build
//! fails outright.
//!
//! The real one: llama.cpp calls `abort()` on a failed assertion, and one was
//! hit during development over a grammar edge case. In this process that takes
//! the tray, the hotkey and the capture loop with it — the entire product, over
//! the tier that exists to make unusual phrasings work. Out here it takes the
//! router, `route` returns `None`, the command comes back "not understood", and
//! `supervise` starts a fresh one. That is the same degradation as having no
//! model at all, which is a state the app already handles everywhere.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::Arc;

use parking_lot::{Mutex, RwLock};

use memos_core::RoutedCommand;
use memos_llm::protocol::{Request, Response, PROTOCOL};
use memos_llm::Decode;

pub use memos_llm::protocol::ModelState;

/// How long the app waits for a reply.
///
/// Longer than the sidecar's own `ROUTE_TIMEOUT` on purpose, by enough to cover
/// two pipe hops. The sidecar's deadline should be the one that fires, because
/// it answers with a reason; ours firing first would turn every slow route into
/// the same silent nothing and hide which it was.
const ROUTE_DEADLINE: std::time::Duration =
    memos_llm::ROUTE_TIMEOUT.saturating_add(std::time::Duration::from_millis(250));

/// How many times a crashing router is given another chance.
///
/// A crash from one bad grammar state is worth restarting for — the next
/// command is probably fine. A model that dies on load will die on load again,
/// and retrying it forever would spend the user's CPU on a fixed conclusion, so
/// after this the tier stays down and says so.
const MAX_RESTARTS: u32 = 3;

/// Overrides where the sidecar is looked for.
pub const SIDECAR_ENV: &str = "MEMOS_ROUTER_BIN";

/// Where the router model lives, if it has been fetched.
pub fn find_model() -> Option<PathBuf> {
    memos_llm::find_model(None, &crate::data_dir())
}

/// Where the sidecar binary lives, if it has been built.
///
/// Anchored on this executable rather than the working directory, because the
/// working directory is not ours to predict — `tauri dev` runs the app from
/// `apps/desktop/src-tauri`, and a shortcut can set it to anything.
///
/// Beside this binary first: that is where the installer puts it, and where
/// `cargo build` puts it, so both layouts work with no build-time switch. Then
/// the sibling profile directory, so a debug app finds a release router — which
/// is the common case in development, since nobody wants to compile llama.cpp
/// twice.
fn find_sidecar() -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        "memos-router.exe"
    } else {
        "memos-router"
    };
    let mut candidates: Vec<PathBuf> = Vec::new();
    // An explicit override, for pointing a packaged app at a router built
    // somewhere else — and the seam the supervision test drives a real,
    // deliberately failing router through.
    if let Ok(explicit) = std::env::var(SIDECAR_ENV) {
        candidates.push(PathBuf::from(explicit));
    }
    if let Ok(here) = std::env::current_exe() {
        if let Some(dir) = here.parent() {
            candidates.push(dir.join(exe));
            if let Some(target) = dir.parent() {
                candidates.push(target.join("release").join(exe));
                candidates.push(target.join("debug").join(exe));
            }
        }
    }
    candidates.into_iter().find(|p| p.exists())
}

/// One reply, as the thread waiting for it sees it: the model's JSON and what
/// the decode measured about it, or why there is neither.
type Answer = Result<(String, Decode), String>;

/// The live end of the pipe.
///
/// `Child` is kept rather than detached so a crashed router can be reaped
/// instead of lingering, and `stdin` is the write half — dropping it closes the
/// pipe, which is how the sidecar learns to exit.
struct Link {
    child: Child,
    stdin: ChildStdin,
    /// Which spawn this is. A reader thread from a previous process must not be
    /// able to declare the current one dead when it finally notices its own EOF.
    generation: u64,
}

pub struct Tier1 {
    link: Mutex<Option<Link>>,
    state: RwLock<ModelState>,
    detail: RwLock<String>,

    /// Replies land on a different thread than the one waiting for them.
    waiting: Mutex<HashMap<u64, SyncSender<Answer>>>,
    next_id: AtomicU64,

    /// Everything needed to bring the router back after a crash. The collection
    /// list is held rather than re-read because it is what the grammar and the
    /// prefilled prompt are built from — a restart that came back with a stale
    /// set would silently make a valid destination unreachable (ADR-0003).
    model: RwLock<Option<PathBuf>>,
    collections: RwLock<Vec<String>>,
    generation: AtomicU64,
    restarts: AtomicU32,
}

impl Tier1 {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            link: Mutex::new(None),
            state: RwLock::new(ModelState::Missing),
            detail: RwLock::new(String::new()),
            waiting: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            model: RwLock::new(None),
            collections: RwLock::new(Vec::new()),
            generation: AtomicU64::new(0),
            restarts: AtomicU32::new(0),
        })
    }

    pub fn state(&self) -> ModelState {
        *self.state.read()
    }

    pub fn detail(&self) -> String {
        self.detail.read().clone()
    }

    /// Start the router process and let it load, off-thread.
    ///
    /// Returns immediately. Loading costs seconds and the first capture must not
    /// be what waits for it — Tier 0 answers most commands without this model,
    /// and the rest degrade to "not understood" until it is ready, which is
    /// what they did before it existed at all.
    pub fn start(self: &Arc<Self>, model: PathBuf, collections: Vec<String>) {
        *self.model.write() = Some(model);
        *self.collections.write() = collections;
        self.spawn();
    }

    /// The collection set changed, so the grammar and the prefilled prompt are
    /// both stale. ADR-0003 singles this out: a stale grammar does not fail, it
    /// silently makes a valid destination unreachable.
    pub fn collections_changed(&self, collections: Vec<String>) {
        *self.collections.write() = collections.clone();
        self.send(Request::Collections { collections });
    }

    /// Route a transcript Tier 0 gave up on.
    ///
    /// `None` covers every way this can not work — no model, no sidecar, still
    /// loading, crashed, too slow, or output the grammar should have prevented.
    /// They are all the same thing to the caller: Tier 1 did not resolve this,
    /// so the command stays unrouted, exactly as it was before this tier
    /// existed.
    pub fn route(&self, transcript: &str, collections: &[String]) -> Option<RoutedCommand> {
        if self.state() != ModelState::Ready {
            return None;
        }
        let started = std::time::Instant::now();

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let (reply, answer) = sync_channel(1);
        // Registered before the request is sent, never after: the reply can
        // arrive on the reader thread before this one is scheduled again, and a
        // slot that is not there yet is a route silently lost to a race.
        self.waiting.lock().insert(id, reply);

        let sent = self.send(Request::Route {
            id,
            transcript: transcript.to_string(),
        });
        let result = if sent {
            answer.recv_timeout(ROUTE_DEADLINE)
        } else {
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected)
        };
        self.waiting.lock().remove(&id);

        let (json, decode) = match result {
            Ok(Ok(answer)) => answer,
            Ok(Err(why)) => {
                tracing::warn!(why, "router declined");
                return None;
            }
            Err(_) => {
                tracing::warn!("router did not answer within {ROUTE_DEADLINE:?}");
                return None;
            }
        };

        let took = started.elapsed().as_millis() as u32;
        match memos_llm::parse(transcript, &json, collections, decode, took) {
            Ok(cmd) => {
                tracing::info!(
                    intent = cmd.intent.as_str(),
                    collection = ?cmd.slots.collection,
                    logprob = cmd.confidence.logprob,
                    margin = cmd.confidence.margin,
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

    /// Launch the sidecar and start reading from it.
    fn spawn(self: &Arc<Self>) {
        let Some(model) = self.model.read().clone() else {
            return self.down(ModelState::Missing, "No router model.".into());
        };
        let Some(exe) = find_sidecar() else {
            return self.down(
                ModelState::Missing,
                format!(
                    "No router binary. Build it with {}",
                    memos_core::scripts::BUILD_ROUTER
                ),
            );
        };

        let mut command = Command::new(&exe);
        command
            .arg(&model)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // No console window. Without this a tray app spawning a child flashes a
        // black rectangle onto the user's screen at login, which is a very
        // visible cost for a process they are not supposed to know exists.
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = match command.spawn() {
            Ok(c) => c,
            Err(e) => {
                return self.down(
                    ModelState::Failed,
                    format!("could not start {}: {e}", exe.display()),
                )
            }
        };
        let (Some(stdin), Some(stdout), Some(stderr)) =
            (child.stdin.take(), child.stdout.take(), child.stderr.take())
        else {
            let _ = child.kill();
            return self.down(ModelState::Failed, "router pipes were not created".into());
        };

        let generation = self.generation.fetch_add(1, Ordering::SeqCst) + 1;
        *self.state.write() = ModelState::Loading;
        *self.detail.write() = String::new();
        *self.link.lock() = Some(Link {
            child,
            stdin,
            generation,
        });

        let me = self.clone();
        std::thread::Builder::new()
            .name("router-reader".into())
            .spawn(move || {
                for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                    me.deliver(&line);
                }
                me.supervise(generation);
            })
            .expect("spawn router reader");

        // The sidecar's own diagnostics, kept at debug: what the app needs to
        // act on arrives as a `State` message instead, and duplicating the
        // router's log at info would double every line of a normal startup.
        std::thread::Builder::new()
            .name("router-stderr".into())
            .spawn(move || {
                for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                    tracing::debug!(target: "router", "{line}");
                }
            })
            .expect("spawn router stderr");

        // Loading begins on the first collection list, so this is also the
        // "please start" message. Sent after the reader exists, so the reply to
        // it cannot be missed.
        let collections = self.collections.read().clone();
        self.send(Request::Collections { collections });
        tracing::info!(exe = %exe.display(), generation, "router process started");
    }

    /// One line from the sidecar.
    fn deliver(&self, line: &str) {
        match serde_json::from_str::<Response>(line) {
            // A router built against a different protocol is worse than no
            // router: it loads, reports itself ready, and then declines every
            // command for a reason nothing on screen explains. Said plainly
            // instead, in the one place the user looks for it.
            Ok(Response::State { protocol, .. }) if protocol != PROTOCOL => self.down(
                ModelState::Failed,
                format!(
                    "The router binary is out of date (protocol {protocol}, expected \
                     {PROTOCOL}). Rebuild it: {}",
                    memos_core::scripts::BUILD_ROUTER
                ),
            ),
            Ok(Response::State { state, detail, .. }) => {
                *self.state.write() = state;
                *self.detail.write() = detail.clone();
                if state == ModelState::Ready {
                    // A router that came up cleanly has earned its restart
                    // budget back. Otherwise three crashes spread over a week
                    // of uptime would retire the tier for good.
                    self.restarts.store(0, Ordering::Relaxed);
                    tracing::info!(detail, "router ready");
                } else {
                    tracing::warn!(detail, "router unavailable");
                }
            }
            Ok(Response::Routed { id, json, decode }) => self.answer(id, Ok((json, decode))),
            Ok(Response::Declined { id, error }) => self.answer(id, Err(error)),
            // The sidecar writes nothing but protocol to stdout, so this means
            // the two ends disagree about the protocol. By far the likeliest
            // cause is a router binary older than the app — they are built by
            // separate commands, so it is possible to update one and not the
            // other. Worth saying loudly; not worth killing the tier over.
            Err(e) => tracing::error!(
                error = %e,
                line,
                rebuild_with = memos_core::scripts::BUILD_ROUTER,
                "unreadable router reply — rebuild the router"
            ),
        }
    }

    fn answer(&self, id: u64, result: Answer) {
        match self.waiting.lock().remove(&id) {
            Some(tx) => {
                let _ = tx.send(result);
            }
            // The caller already gave up and the overlay has moved on. Not an
            // error — just a route that finished after it stopped mattering.
            None => tracing::debug!(id, "reply for a request nobody is waiting on"),
        }
    }

    /// The router process ended. Decide whether it comes back.
    fn supervise(self: &Arc<Self>, generation: u64) {
        // A reader thread for a process that has already been replaced has
        // nothing to say about the current one.
        {
            let mut link = self.link.lock();
            match link.as_ref() {
                Some(l) if l.generation != generation => return,
                None => return,
                _ => {}
            }
            if let Some(mut l) = link.take() {
                // Reaped rather than left behind. The pipe is closed by now, so
                // this is a formality on Windows and not one everywhere else.
                let _ = l.child.kill();
                let _ = l.child.wait();
            }
        }

        // Anything still in flight died with the process. Waking those callers
        // now beats making each of them sit out the full deadline for an answer
        // that is never coming.
        for (_, tx) in self.waiting.lock().drain() {
            let _ = tx.send(Err("the router process exited".into()));
        }

        let n = self.restarts.fetch_add(1, Ordering::SeqCst) + 1;
        if n > MAX_RESTARTS {
            return self.down(
                ModelState::Failed,
                format!("The router stopped {n} times and was not restarted again."),
            );
        }
        tracing::warn!(generation, attempt = n, "router exited; restarting");
        *self.state.write() = ModelState::Loading;
        self.spawn();
    }

    /// Write one request, if there is a process to write it to.
    ///
    /// `false` means there was not. Every caller treats that as "Tier 1 did not
    /// answer", which is a state the whole escalation path already handles.
    fn send(&self, request: Request) -> bool {
        let mut link = self.link.lock();
        let Some(l) = link.as_mut() else {
            return false;
        };
        if let Err(e) = l.stdin.write_all(request.line().as_bytes()) {
            // A broken pipe here is the same crash the reader thread is about
            // to notice; leave the restart to it so it happens exactly once.
            tracing::warn!(error = %e, "could not reach the router");
            return false;
        }
        // Flushed per message: the sidecar is line-oriented and a buffered
        // request is one it never sees.
        l.stdin.flush().is_ok()
    }

    /// There will be no Tier 1 this run, and here is what to do about it.
    ///
    /// Public because the reason can be known before the router is ever
    /// started — a missing model file is found by the app, not by the sidecar —
    /// and a tier that is off for a fixable reason should say which one.
    pub fn unavailable(&self, why: &str) {
        self.down(ModelState::Missing, why.to_string());
    }

    fn down(&self, state: ModelState, why: String) {
        tracing::warn!(why, "Tier 1 unavailable");
        *self.state.write() = state;
        *self.detail.write() = why;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Replies arrive on the reader thread and are matched to waiters by id.
    /// Getting this wrong after a timeout would attribute one command's answer
    /// to the next one — the user speaks about React and the previous route's
    /// destination is what gets used.
    #[test]
    fn a_reply_reaches_the_caller_that_asked_for_it() {
        let t = Tier1::new();
        let (tx_a, a) = sync_channel(1);
        let (tx_b, b) = sync_channel(1);
        t.waiting.lock().insert(1, tx_a);
        t.waiting.lock().insert(2, tx_b);

        t.deliver(
            r#"{"event":"routed","id":2,"json":"{\"intent\":\"save\"}",
                "decode":{"logprob":0.93,"margin":0.61}}"#,
        );
        let (json, decode) = b.try_recv().unwrap().expect("routed");
        assert_eq!(json, r#"{"intent":"save"}"#);
        assert_eq!(decode.margin, 0.61);
        // The other waiter is untouched, and its slot is still registered.
        assert!(a.try_recv().is_err());
        assert!(t.waiting.lock().contains_key(&1));

        t.deliver(r#"{"event":"declined","id":1,"error":"no answer within the deadline"}"#);
        assert_eq!(
            a.try_recv().unwrap(),
            Err("no answer within the deadline".to_string())
        );
        assert!(t.waiting.lock().is_empty());
    }

    /// A reply for a command the overlay has already moved on from, and a line
    /// the two ends disagree about, are both survivable. Neither may panic on
    /// the reader thread, because that thread ending is what the supervisor
    /// reads as "the router died".
    #[test]
    fn a_stray_or_unreadable_reply_is_survivable() {
        let t = Tier1::new();
        t.deliver(r#"{"event":"routed","id":99,"json":"{}","decode":{"logprob":1.0,"margin":1.0}}"#);
        t.deliver("not json at all");
        t.deliver(r#"{"event":"nonsense"}"#);
        assert_eq!(t.state(), ModelState::Missing);
    }

    /// The restart budget is per crash-run, not per lifetime: a router that
    /// comes back cleanly earns it back. Otherwise three unrelated crashes
    /// spread over a week of uptime would retire the tier for good.
    #[test]
    fn coming_up_ready_restores_the_restart_budget() {
        let t = Tier1::new();
        t.restarts.store(MAX_RESTARTS, Ordering::Relaxed);
        t.deliver(
            r#"{"event":"state","state":"ready","detail":"269 tokens prefilled","protocol":1}"#,
        );
        assert_eq!(t.state(), ModelState::Ready);
        assert_eq!(t.detail(), "269 tokens prefilled");
        assert_eq!(t.restarts.load(Ordering::Relaxed), 0);

        // A failure does not, so a model that cannot load still stops.
        t.deliver(
            r#"{"event":"state","state":"failed","detail":"context: out of memory","protocol":1}"#,
        );
        assert_eq!(t.state(), ModelState::Failed);
        assert_eq!(t.restarts.load(Ordering::Relaxed), 0);
    }

    /// The two binaries are built by separate commands, so one can be older
    /// than the other. That has to be a sentence the user can act on, not a
    /// router that says "ready" and then silently answers nothing.
    #[test]
    fn a_router_older_than_the_app_says_so() {
        let t = Tier1::new();
        t.deliver(r#"{"event":"state","state":"ready","detail":"loaded"}"#);
        assert_eq!(t.state(), ModelState::Failed);
        assert!(t.detail().contains("out of date"), "{}", t.detail());
        assert!(t.detail().contains("build-router"), "{}", t.detail());
    }

    /// With no process there is nothing to route to, and every caller has to
    /// see that as "Tier 1 did not answer" rather than waiting out the deadline
    /// for a reply that was never sent.
    #[test]
    fn routing_with_no_router_gives_up_immediately() {
        let t = Tier1::new();
        assert!(!t.send(Request::Collections {
            collections: vec!["Study".into()]
        }));
        assert!(t.route("stick this in with the python stuff", &["Study".into()]).is_none());
        assert!(t.waiting.lock().is_empty());
    }

    /// A dead process's reader thread must not be able to tear down the
    /// replacement that has already taken its place.
    #[test]
    fn a_superseded_reader_does_not_restart_anything() {
        let t = Tier1::new();
        t.supervise(0);
        assert_eq!(t.restarts.load(Ordering::Relaxed), 0);
        assert_eq!(t.state(), ModelState::Missing);
    }

    /// The promise ADR-0008 is built on, against the real binary: a router that
    /// dies is restarted, and one that keeps dying is eventually left alone
    /// rather than respawned forever.
    ///
    /// Driven by giving the real sidecar a model path that does not exist,
    /// which is the shortest route to a process that starts and then exits.
    /// Skipped when the sidecar has not been built — it is an optional
    /// component, and `cargo test` must not require llama.cpp to have compiled.
    #[test]
    fn a_router_that_keeps_dying_is_eventually_left_alone() {
        // Located from the manifest rather than through `find_sidecar`, whose
        // anchor under `cargo test` is the test harness in `target/debug/deps`
        // and not the app.
        let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../..")
            .canonicalize()
            .expect("workspace root");
        let name = if cfg!(windows) {
            "memos-router.exe"
        } else {
            "memos-router"
        };
        let Some(sidecar) = ["release", "debug"]
            .into_iter()
            .map(|p| workspace.join("target").join(p).join(name))
            .find(|p| p.exists())
        else {
            eprintln!("no sidecar built; skipping. {}", memos_core::scripts::BUILD_ROUTER);
            return;
        };
        std::env::set_var(SIDECAR_ENV, &sidecar);

        let t = Tier1::new();
        t.start(PathBuf::from("a-model-that-is-not-there.gguf"), vec!["Study".into()]);

        // Each attempt is a process spawn and an immediate exit; the whole
        // sequence is milliseconds, but it is several thread handoffs, so this
        // waits for the outcome rather than assuming it has already happened.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
        while t.state() != ModelState::Failed && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(20));
        }

        assert_eq!(t.state(), ModelState::Failed, "{}", t.detail());
        assert!(t.detail().contains("not restarted again"), "{}", t.detail());
        assert!(t.link.lock().is_none(), "a failed router left a process behind");
        // Tried, and then stopped trying. Both halves matter: no restarts at
        // all would mean one crash retires the tier, and no ceiling would mean
        // a broken model spawns processes for as long as the app runs.
        assert_eq!(t.restarts.load(Ordering::Relaxed), MAX_RESTARTS + 1);

        // And with the tier down, a command is simply not routed — the same
        // thing that happens on a machine with no model at all.
        assert!(t.route("stick this in with the python stuff", &["Study".into()]).is_none());

        std::env::remove_var(SIDECAR_ENV);
    }
}
