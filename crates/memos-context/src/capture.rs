//! Collecting a context object, under a deadline.

use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::{Context, ContextPermissions};

/// Collection runs on its own thread with this ceiling.
///
/// UI Automation calls cross a process boundary into the target application and
/// can block for seconds if it is busy or hung. Without a deadline a single
/// misbehaving app would stall every capture — so context is gathered on a
/// worker and abandoned if it overruns. A partial context is useful; a stalled
/// capture is not.
pub const DEADLINE: Duration = Duration::from_millis(400);

/// A collection already running in the background.
pub struct Pending {
    rx: mpsc::Receiver<Context>,
    started: Instant,
}

impl Pending {
    /// Take the result, waiting only for whatever remains of the deadline.
    ///
    /// By the time this is called the user has usually been speaking for
    /// seconds, so collection has long since finished and this returns
    /// immediately.
    pub fn finish(self) -> Context {
        let mut ctx = drain_until(&self.rx, self.started);
        ctx.elapsed_ms = self.started.elapsed().as_millis() as u32;
        ctx.captured_at = Some(chrono::Local::now().to_rfc3339());
        ctx
    }
}

/// Begin collecting immediately and return without waiting.
///
/// This is the form the capture path uses: started the moment the shortcut is
/// pressed so it overlaps the user's speech and costs nothing on the timeline.
pub fn start(perms: ContextPermissions) -> Pending {
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("context".into())
        .spawn(move || collect_staged(perms, |c| tx.send(c).is_ok()))
        .ok();
    Pending { rx, started }
}

/// Gather context now, blocking until done or [`DEADLINE`].
pub fn collect(perms: ContextPermissions) -> Context {
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("context".into())
        .spawn(move || collect_staged(perms, |c| tx.send(c).is_ok()))
        .ok();

    let mut ctx = drain_until(&rx, started);
    ctx.elapsed_ms = started.elapsed().as_millis() as u32;
    ctx.captured_at = Some(chrono::Local::now().to_rfc3339());
    ctx
}

/// Keep the most complete context that arrived before the deadline.
///
/// Collection reports in stages — the cheap fields first, the page text after —
/// so a document read that overruns costs only the page, not the URL and title
/// gathered milliseconds earlier. Taking the *last* message rather than the
/// first is what makes the expensive stage strictly additive.
fn drain_until(rx: &mpsc::Receiver<Context>, started: Instant) -> Context {
    let mut best: Option<Context> = None;
    loop {
        let remaining = DEADLINE.saturating_sub(started.elapsed());
        match rx.recv_timeout(remaining) {
            Ok(ctx) => best = Some(ctx),
            // Disconnected means the worker finished: nothing better is coming.
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if best.is_none() {
                    tracing::warn!("context collection exceeded {DEADLINE:?}; continuing without it");
                } else {
                    tracing::debug!("context deadline reached; using what arrived");
                }
                break;
            }
        }
    }
    best.unwrap_or_default()
}

/// Collect, reporting progressively. `emit` returns false once nobody is
/// listening, which is the signal to stop paying for work no one will read.
#[cfg(windows)]
fn collect_staged(perms: ContextPermissions, mut emit: impl FnMut(Context) -> bool) {
    use crate::windows_impl as w;

    let mut ctx = Context::default();
    w::init_com();

    let Some(hwnd) = w::foreground_window() else {
        emit(ctx);
        return;
    };

    if perms.window {
        ctx.active_window_title = w::window_title(hwnd);
        ctx.active_application = w::process_name(hwnd);
    }

    let uia = (perms.selection || perms.url || perms.page)
        // One automation instance for every lookup: creating it costs a COM
        // activation, and doing that more than once per capture is pure waste.
        .then(w::automation)
        .flatten();

    if let Some(uia) = &uia {
        if perms.selection {
            ctx.selected_text = w::selected_text(uia);
        }
        if perms.url {
            ctx.current_url = w::current_url(uia, hwnd);
        }
    }

    if perms.clipboard {
        ctx.clipboard_text = w::clipboard_text();
    }

    // Everything cheap is in hand. Publish it before the one call that can
    // block for hundreds of milliseconds.
    if !emit(ctx.clone()) {
        return;
    }

    // A selection is a more direct answer to "what is this" than the page it
    // sits in, so when there is one, the expensive read is skipped entirely.
    if perms.page && ctx.selected_text.is_none() {
        if let Some(uia) = &uia {
            ctx.page_text = w::document_text(uia, hwnd, crate::readable::MAX_CHARS as i32)
                .as_deref()
                .and_then(crate::readable::extract);
            if ctx.page_text.is_some() {
                emit(ctx);
            }
        }
    }
}

#[cfg(windows)]
#[allow(dead_code)]
fn collect_blocking(perms: ContextPermissions) -> Context {
    use crate::windows_impl as w;

    let mut ctx = Context::default();
    w::init_com();

    let Some(hwnd) = w::foreground_window() else {
        return ctx;
    };

    if perms.window {
        ctx.active_window_title = w::window_title(hwnd);
        ctx.active_application = w::process_name(hwnd);
    }

    if perms.selection || perms.url {
        // One automation instance for both lookups: creating it costs a COM
        // activation, and doing that twice per capture is pure waste.
        if let Some(uia) = w::automation() {
            if perms.selection {
                ctx.selected_text = w::selected_text(&uia);
            }
            if perms.url {
                ctx.current_url = w::current_url(&uia, hwnd);
            }
        }
    }

    if perms.clipboard {
        ctx.clipboard_text = w::clipboard_text();
    }

    ctx
}

#[cfg(not(windows))]
fn collect_staged(_perms: ContextPermissions, mut emit: impl FnMut(Context) -> bool) {
    // macOS lands at M7: the Accessibility API plus an explicit permission
    // prompt. Only this function changes; `Context` is platform-neutral.
    emit(Context::default());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_always_returns_within_the_deadline() {
        // The guarantee that matters: whatever the foreground application is
        // doing, capture is not held up by it.
        let started = Instant::now();
        let ctx = collect(ContextPermissions::default());
        let took = started.elapsed();
        assert!(
            took < DEADLINE + Duration::from_millis(250),
            "collection took {took:?}, past the deadline"
        );
        assert!(ctx.captured_at.is_some());
    }

    #[test]
    fn disabled_permissions_collect_nothing() {
        let none = ContextPermissions {
            window: false,
            url: false,
            selection: false,
            page: false,
            clipboard: false,
        };
        let ctx = collect(none);
        assert!(ctx.active_application.is_none());
        assert!(ctx.active_window_title.is_none());
        assert!(ctx.selected_text.is_none());
        assert!(ctx.current_url.is_none());
        assert!(ctx.page_text.is_none());
        assert!(ctx.clipboard_text.is_none());
    }

    #[test]
    fn the_cheap_fields_survive_a_stage_that_never_finishes() {
        // The regression this guards: adding the page read made every field
        // hostage to it, so one slow document lost the URL and title that had
        // been in hand for 300 ms.
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        std::thread::spawn(move || {
            let _ = tx.send(Context {
                current_url: Some("https://react.dev/learn".into()),
                ..Default::default()
            });
            // The expensive stage, hanging past the deadline.
            std::thread::sleep(DEADLINE * 3);
            let _ = tx.send(Context {
                page_text: Some("too late".into()),
                ..Default::default()
            });
        });

        let ctx = drain_until(&rx, started);
        assert_eq!(ctx.current_url.as_deref(), Some("https://react.dev/learn"));
        assert!(ctx.page_text.is_none(), "the late stage must not be waited for");
        assert!(started.elapsed() < DEADLINE * 2);
    }

    #[test]
    fn a_later_stage_replaces_an_earlier_one() {
        let (tx, rx) = mpsc::channel();
        let started = Instant::now();
        let _ = tx.send(Context {
            current_url: Some("https://react.dev/learn".into()),
            ..Default::default()
        });
        let _ = tx.send(Context {
            current_url: Some("https://react.dev/learn".into()),
            page_text: Some("the article".into()),
            ..Default::default()
        });
        drop(tx);

        let ctx = drain_until(&rx, started);
        assert_eq!(ctx.page_text.as_deref(), Some("the article"));
    }
}
