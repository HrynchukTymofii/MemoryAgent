//! Show what context we can read from the current foreground window.
//!
//!   cargo run -p memos-context --example context_check
//!
//! Focus a browser (with something selected) before the countdown ends.
use memos_context::{capture, ContextPermissions};

fn main() {
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
    println!("{:<20} {}", "suggested title", ctx.suggested_title().unwrap_or("(none)".into()));
    println!("{:<20} {}", "has referent", ctx.has_referent());
    println!("{:<20} {} ms", "collected in", ctx.elapsed_ms);
}
