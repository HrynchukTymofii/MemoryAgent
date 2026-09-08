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
        let remaining = DEADLINE.saturating_sub(self.started.elapsed());
        let mut ctx = self.rx.recv_timeout(remaining).unwrap_or_else(|_| {
            tracing::warn!("context collection exceeded {DEADLINE:?}; continuing without it");
            Context::default()
        });
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
        .spawn(move || {
            let _ = tx.send(collect_blocking(perms));
        })
        .ok();
    Pending { rx, started }
}

/// Gather context now, blocking until done or [`DEADLINE`].
pub fn collect(perms: ContextPermissions) -> Context {
    let started = Instant::now();
    let (tx, rx) = mpsc::channel();

    std::thread::Builder::new()
        .name("context".into())
        .spawn(move || {
            let _ = tx.send(collect_blocking(perms));
        })
        .ok();

    let mut ctx = rx.recv_timeout(DEADLINE).unwrap_or_else(|_| {
        tracing::warn!("context collection exceeded {DEADLINE:?}; continuing without it");
        Context::default()
    });
    ctx.elapsed_ms = started.elapsed().as_millis() as u32;
    ctx.captured_at = Some(chrono::Local::now().to_rfc3339());
    ctx
}

#[cfg(windows)]
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
fn collect_blocking(_perms: ContextPermissions) -> Context {
    // macOS lands at M7: the Accessibility API plus an explicit permission
    // prompt. Only this function changes; `Context` is platform-neutral.
    Context::default()
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
            clipboard: false,
        };
        let ctx = collect(none);
        assert!(ctx.active_application.is_none());
        assert!(ctx.active_window_title.is_none());
        assert!(ctx.selected_text.is_none());
        assert!(ctx.current_url.is_none());
        assert!(ctx.clipboard_text.is_none());
    }
}
