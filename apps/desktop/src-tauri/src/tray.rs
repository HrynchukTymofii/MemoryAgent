//! System tray. The application's real home — the Hub is a window you visit.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Hub", true, None::<&str>)?;
    let latency = MenuItem::with_id(app, "latency", "Latency report", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &latency, &sep, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Personal Memory OS — hold Ctrl+Win to capture")
        .menu(&menu)
        // Left-click opens the Hub; without this the menu shows on both
        // buttons, which feels wrong on Windows.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_hub(app),
            "latency" => {
                if let Some(state) = app.try_state::<crate::AppState>() {
                    let r = state.latency.report();
                    tracing::info!(
                        "latency over {} captures: p50 {:.1} ms, p95 {:.1} ms, worst {:.1} ms — {}",
                        r.count,
                        r.p50_ms,
                        r.p95_ms,
                        r.worst_ms,
                        if r.within_budget { "within 50 ms budget" } else { "OVER BUDGET" }
                    );
                }
                show_hub(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_hub(tray.app_handle());
            }
        })
        .build(app)?;

    Ok(())
}

fn show_hub<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
