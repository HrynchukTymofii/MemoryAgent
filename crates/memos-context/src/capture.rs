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
    // Everything published so far, keeping the most complete. This is not a
    // wait: by the time a capture ends the user has been speaking for seconds,
    // and the page reader has been publishing improvements throughout.
    let mut best: Option<Context> = None;
    while let Ok(ctx) = rx.try_recv() {
        best = Some(ctx);
    }
    if let Some(ctx) = best {
        return ctx;
    }

    // Nothing at all yet — a very short utterance, or a slow first read. Wait
    // out what remains of the deadline for something rather than nothing, then
    // give up: the capture matters more than the context attached to it.
    let remaining = DEADLINE.saturating_sub(started.elapsed());
    rx.recv_timeout(remaining).unwrap_or_else(|_| {
        tracing::warn!("context collection exceeded {DEADLINE:?}; continuing without it");
        Context::default()
    })
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
            read_page_while_speaking(uia, hwnd, ctx, emit);
        }
    }
}

/// Read the page repeatedly, publishing each improvement.
///
/// **Chromium builds its accessibility tree lazily.** It does no such work until
/// a client asks for it, and the tree is not finished when the first request
/// returns. Measured against a docs page in Edge: the first read returned
/// *nothing*, and the same read seconds later returned 15,637 characters. A
/// single retry a few milliseconds later does not fix it, because the delay is
/// seconds, not milliseconds.
///
/// The symptom is the worst kind. Not an error — a memory that quietly holds
/// one section of an article, or only its headings, and looks entirely fine
/// until a search fails to find it a month later. One real capture stored 687
/// characters of a page that had 15,000.
///
/// Polling is the right shape because the deadline here is not a stopwatch: the
/// user is holding the key and speaking, and every attempt lands in time that
/// was already being spent (§4, stage 3). Each improvement is published as it
/// arrives, so whatever has been read by the time they stop talking is what the
/// capture uses — and `emit` returning false says nobody is listening any more,
/// which ends the loop immediately.
#[cfg(windows)]
fn read_page_while_speaking(
    uia: &windows::Win32::UI::Accessibility::IUIAutomation,
    hwnd: windows::Win32::Foundation::HWND,
    mut ctx: Context,
    mut emit: impl FnMut(Context) -> bool,
) {
    use crate::windows_impl as w;

    /// Pauses between attempts. They lengthen because a tree that was not ready
    /// at 200 ms is waiting on rendering, not on scheduling — and the total,
    /// 2.1 s, is about as long as a person speaks a command for.
    const BACKOFF_MS: &[u64] = &[0, 200, 300, 500, 500, 600];
    /// Enough text that waiting for more is not worth another attempt.
    const SETTLED: usize = 1_200;

    let mut best = 0usize;
    for pause in BACKOFF_MS {
        if *pause > 0 {
            std::thread::sleep(Duration::from_millis(*pause));
        }
        let Some(raw) = w::document_text(uia, hwnd, crate::readable::MAX_CHARS as i32) else {
            continue;
        };
        let len = raw.chars().count();
        if len <= best {
            continue;
        }
        best = len;
        if let Some(text) = crate::readable::extract(&raw) {
            ctx.page_text = Some(text);
            if !emit(ctx.clone()) {
                return;
            }
        }
        if best >= SETTLED {
            return;
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

/// The macOS collection, in one pass.
///
/// No staging, unlike Windows. The staged shape exists to publish the cheap
/// fields before a page read that can block for hundreds of milliseconds while
/// Chromium builds its accessibility tree — and `page_text` is not read here, so
/// there is no expensive second half to hide. Every lookup below is bounded by
/// the messaging timeout the element carries, and the whole set costs a few
/// milliseconds when the application answers and nothing when it does not.
///
/// `page_text` stays `None` on this platform. Reading it means walking the web
/// area's accessibility tree, and doing that *correctly* means solving the lazy
/// tree problem the Windows path documents at length — a partial read is not a
/// smaller version of the feature, it is a memory that silently holds a
/// paragraph of a page and looks fine until a search misses it a month later.
/// A selection or a URL is a real referent, so capture stays useful without it.
#[cfg(target_os = "macos")]
fn collect_staged(perms: ContextPermissions, mut emit: impl FnMut(Context) -> bool) {
    use crate::macos_impl as m;

    let mut ctx = Context::default();

    // Read once and cache: this is a lookup per capture either way, and the
    // answer is what decides whether an empty context means "nothing to see" or
    // "you have not granted the permission yet".
    if !m::is_trusted() {
        tracing::warn!(
            "Accessibility not granted; context capture will be empty. \
             System Settings > Privacy & Security > Accessibility."
        );
        if perms.clipboard {
            ctx.clipboard_text = m::clipboard_text();
        }
        emit(ctx);
        return;
    }

    let Some(app) = m::focused_application() else {
        if perms.clipboard {
            ctx.clipboard_text = m::clipboard_text();
        }
        emit(ctx);
        return;
    };

    if perms.window {
        ctx.active_application = m::application_name(&app);
        ctx.active_window_title = m::window_title(&app);
    }
    if perms.selection {
        ctx.selected_text = m::selected_text(&app);
    }
    if perms.url {
        ctx.current_url = m::current_url(&app);
    }
    if perms.clipboard {
        ctx.clipboard_text = m::clipboard_text();
    }

    emit(ctx);
}

#[cfg(not(any(windows, target_os = "macos")))]
fn collect_staged(_perms: ContextPermissions, mut emit: impl FnMut(Context) -> bool) {
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
