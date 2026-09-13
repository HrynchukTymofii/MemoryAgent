//! Show what context we can read from a window.
//!
//!   cargo run -p memos-context --example context_check            # the focused window
//!   cargo run -p memos-context --example context_check -- react   # a window by title
//!
//! With no argument, focus a browser (with something selected) before the
//! countdown ends. With one, it reads the named window and reports the page
//! text before and after trimming.
use memos_context::{capture, ContextPermissions};

fn main() {
    // With an argument, read a named window rather than the focused one. Two
    // reasons that matters: focusing a window to inspect it changes what is on
    // screen, and a diagnostic that reads "whatever you happen to have open" is
    // one you cannot run in front of anybody.
    if let Some(needle) = std::env::args().nth(1) {
        page_report(&needle);
        return;
    }

    println!("Focus another window now - reading in 4 seconds...\n");
    std::thread::sleep(std::time::Duration::from_secs(4));

    let perms = ContextPermissions {
        clipboard: true, // opt in explicitly for the diagnostic
        ..Default::default()
    };
    let ctx = capture::collect(perms);

    let show = |k: &str, v: &Option<String>| {
        let val = v.as_deref().unwrap_or("(none)");
        let val = if val.chars().count() > 90 {
            format!("{}...", val.chars().take(90).collect::<String>())
        } else {
            val.to_string()
        };
        println!("{k:<20} {val}");
    };

    println!("--- context ---");
    show("application", &ctx.active_application);
    show("window title", &ctx.active_window_title);
    show("url", &ctx.current_url);
    show("selected text", &ctx.selected_text);
    show("clipboard", &ctx.clipboard_text);
    show("clipboard image", &memos_context::clipboard_image_text());
    // Length matters more than the text: it is the number that says whether
    // this page will be findable by what it says or only by its title.
    match &ctx.page_text {
        Some(t) => println!(
            "{:<20} {} chars, {} lines\n{:<20} {}",
            "page text",
            t.chars().count(),
            t.lines().count(),
            "  first line",
            t.lines().next().unwrap_or("").chars().take(80).collect::<String>()
        ),
        None => println!("{:<20} (none - no prose found on this window)", "page text"),
    }
    println!("{:<20} {}", "suggested title", ctx.suggested_title().unwrap_or("(none)".into()));
    println!("{:<20} {}", "has referent", ctx.has_referent());
    println!("{:<20} {} ms", "collected in", ctx.elapsed_ms);
}

/// What one window exposes, before and after trimming.
///
/// The two numbers answer the question the stored text cannot: a short capture
/// is either a page that exposed only part of itself to the accessibility tree,
/// or a trimmer that threw the rest away. Those need opposite fixes, and from
/// the saved item alone they look identical.
#[cfg(windows)]
fn page_report(needle: &str) {
    use memos_context::{readable, windows_impl as w};

    w::init_com();
    let Some(hwnd) = w::find_window(needle) else {
        println!("No window whose title contains {needle:?}");
        return;
    };
    println!("window   {}", w::window_title(hwnd).unwrap_or_default());

    let Some(uia) = w::automation() else {
        println!("UI Automation unavailable");
        return;
    };

    // The same backoff the capture path uses, printed attempt by attempt —
    // this is where Chromium's lazily-built accessibility tree becomes visible
    // instead of merely suspected.
    let started = std::time::Instant::now();
    let mut raw = None;
    for pause in [0u64, 200, 300, 500, 500, 600] {
        if pause > 0 {
            std::thread::sleep(std::time::Duration::from_millis(pause));
        }
        let attempt = w::document_text(&uia, hwnd, readable::MAX_CHARS as i32);
        println!(
            "  +{:>4} ms  {}",
            started.elapsed().as_millis(),
            match &attempt {
                Some(t) => format!("{} chars", t.chars().count()),
                None => "nothing".to_string(),
            }
        );
        let better = attempt
            .as_deref()
            .map(|t| t.chars().count())
            .unwrap_or(0)
            > raw.as_deref().map(|t: &str| t.chars().count()).unwrap_or(0);
        if better {
            raw = attempt;
        }
        if raw.as_deref().is_some_and(|t| t.chars().count() >= 1_200) {
            break;
        }
    }

    let Some(raw) = raw else {
        println!("raw      (no document element, or it is empty)");
        return;
    };
    println!(
        "raw      {} chars, {} lines, settled in {} ms",
        raw.chars().count(),
        raw.lines().count(),
        started.elapsed().as_millis()
    );

    match readable::extract(&raw) {
        None => println!("trimmed  (nothing in it read as prose)"),
        Some(kept) => {
            println!(
                "trimmed  {} chars, {} lines ({}% kept)",
                kept.chars().count(),
                kept.lines().count(),
                kept.chars().count() * 100 / raw.chars().count().max(1)
            );
            println!("\n--- kept, first 5 lines ---");
            for line in kept.lines().take(5) {
                println!("  {}", line.chars().take(100).collect::<String>());
            }
            // Word counts, because the word count per line *is* the rule: eight
            // or more is prose, and three short lines in a row are a menu.
            println!("\n--- raw, first 15 lines ---");
            for line in raw.lines().take(15) {
                let words = line.split_whitespace().count();
                println!("  [{words:>3}w] {}", line.chars().take(88).collect::<String>());
            }
        }
    }
}

#[cfg(not(windows))]
fn page_report(_needle: &str) {
    println!("Windows only until M7.");
}
