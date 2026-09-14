//! System tray. The application's real home — the Hub is a window you visit.

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime,
};

pub fn install<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Hub", true, None::<&str>)?;
    let meeting = MenuItem::with_id(app, "meeting", START_MEETING, true, None::<&str>)?;
    let latency = MenuItem::with_id(app, "latency", "Latency report", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &meeting, &latency, &sep, &quit])?;

    TrayIconBuilder::with_id("main-tray")
        .icon(app.default_window_icon().unwrap().clone())
        .tooltip("Personal Memory OS — hold Ctrl+Win to capture")
        .menu(&menu)
        // Left-click opens the Hub; without this the menu shows on both
        // buttons, which feels wrong on Windows.
        .show_menu_on_left_click(false)
        .on_menu_event(move |app, event| match event.id.as_ref() {
            "open" => show_hub(app),
            "meeting" => {
                let recording = toggle_meeting(app);
                let _ = meeting.set_text(if recording { STOP_MEETING } else { START_MEETING });
            }
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

const START_MEETING: &str = "Start meeting notes";
const STOP_MEETING: &str = "Stop meeting notes";

/// Start or stop the meeting recorder, and say whether it is now recording.
fn toggle_meeting<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(state) = app.try_state::<crate::AppState>() else {
        return false;
    };
    if state.meeting.is_recording() {
        state.meeting.stop();
        return false;
    }
    let mic = state.audio.lock().as_ref().map(|a| a.ring());
    let dir = app
        .path()
        .document_dir()
        .unwrap_or_else(|_| crate::data_dir())
        .join("Meetings");
    match state.meeting.start(state.stt.clone(), mic, &dir) {
        Ok(path) => {
            crate::hotkey::diag(&format!("meeting notes: {}", path.display()));
            true
        }
        Err(e) => {
            tracing::error!(error = %e, "meeting notes did not start");
            crate::hotkey::diag(&format!("meeting notes did not start: {e}"));
            false
        }
    }
}

fn show_hub<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}
