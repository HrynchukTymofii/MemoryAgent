// Release builds must not open a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod embedding;
mod hotkey;
mod question;
mod router;
mod latency;
mod transcription;
mod tray;

use std::sync::Arc;

use embedding::Embeddings;
use hotkey::ChordState;
use latency::{LatencyReport, LatencyTracker};
use memos_db::Db;
use memos_context::ContextPermissions;
use memos_stt::{AudioCapture, Hints};
use transcription::{ModelState, Stt};
use tauri::{Emitter, Manager, PhysicalPosition, Runtime};

/// Shared application state.
pub struct AppState {
    pub db: Arc<Db>,
    /// Held for the process lifetime. Dropping it stops the microphone, so this
    /// is ownership with a purpose, not a stashed handle.
    pub audio: parking_lot::Mutex<Option<AudioCapture>>,
    pub latency: Arc<LatencyTracker>,
    pub stt: Arc<Stt>,
    pub embeddings: Arc<Embeddings>,
    /// The one destination question waiting on an answer, if any.
    pub questions: Arc<question::Pending>,
    pub router: Arc<router::Tier1>,
    /// Sign-in. Present whether or not a provider is configured; an
    /// unconfigured one simply answers "not signed in" to everything, which is
    /// the same answer a signed-out user gets and needs no special casing.
    ///
    pub auth: Arc<memos_auth::Auth>,
    pub config: parking_lot::Mutex<config::Config>,
    /// Read by the dispatch thread on every engagement, so a changed debounce
    /// takes effect without a restart like the binding does.
    pub hold_ms: Arc<std::sync::atomic::AtomicU64>,
}

/// Reported by the overlay from its first animation frame after being shown.
/// Closes the stage-1 measurement.
#[tauri::command]
fn overlay_painted(state: tauri::State<'_, AppState>) -> Option<f64> {
    let ms = state.latency.complete();
    if let Some(ms) = ms {
        let over = if ms > 50.0 { "  ** OVER BUDGET **" } else { "" };
        tracing::info!("hotkey -> overlay painted: {ms:.1} ms{over}");
        // Also append to a file. Stdout is block-buffered once redirected, so
        // samples are lost if the process is killed rather than exiting
        // cleanly — which is exactly how it dies during a measurement run.
        append_line(
            data_dir().join("latency.log"),
            &format!("{:.2} ms{}", ms, over),
        );
    }
    ms
}

fn append_line(path: std::path::PathBuf, line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "{line}");
    }
}

#[tauri::command]
fn latency_report(state: tauri::State<'_, AppState>) -> LatencyReport {
    state.latency.report()
}

/// Live keyboard-hook state, so a dead shortcut can be diagnosed from the Hub
/// instead of guessed at. `events` rising as you type proves the hook receives
/// input; `held_mods` shows exactly which modifier bits a key sets.
#[tauri::command]
fn hook_stats() -> hotkey::HookStats {
    hotkey::stats()
}

#[derive(serde::Serialize)]
struct Settings {
    hotkey: String,
    hold_threshold_ms: u64,
    debug_keys: bool,
    active_chord: String,
    idle_pill: bool,
}

/// One place that reads the config into the shape the Hub expects, so a field
/// added here cannot be forgotten by one of the three commands that return it.
fn settings_of(state: &tauri::State<'_, AppState>) -> Settings {
    let c = state.config.lock().clone();
    Settings {
        hotkey: c.hotkey,
        hold_threshold_ms: c.hold_threshold_ms,
        debug_keys: c.debug_keys,
        active_chord: hotkey::active_chord_label(),
        idle_pill: c.idle_pill,
    }
}

#[tauri::command]
fn get_settings(state: tauri::State<'_, AppState>) -> Settings {
    settings_of(&state)
}

/// Apply a new binding immediately and persist it.
///
/// Validated before anything is written or swapped: a rejected spec must leave
/// the working shortcut untouched. Saving a binding that cannot be parsed would
/// silently disable capture on the next launch, which is the failure this whole
/// screen exists to prevent.
#[tauri::command]
fn set_hotkey(
    state: tauri::State<'_, AppState>,
    spec: String,
    hold_threshold_ms: u64,
) -> Result<Settings, String> {
    let chord = config::Chord::parse(&spec)?;

    let mut cfg = state.config.lock();
    cfg.hotkey = spec;
    cfg.hold_threshold_ms = hold_threshold_ms.clamp(0, 2000);
    cfg.save()?;

    hotkey::set_chord(chord);
    state
        .hold_ms
        .store(cfg.hold_threshold_ms, std::sync::atomic::Ordering::SeqCst);
    hotkey::diag(&format!(
        "shortcut changed to '{}' (hold {} ms)",
        cfg.hotkey, cfg.hold_threshold_ms
    ));

    Ok(Settings {
        hotkey: cfg.hotkey.clone(),
        hold_threshold_ms: cfg.hold_threshold_ms,
        debug_keys: cfg.debug_keys,
        active_chord: hotkey::active_chord_label(),
        idle_pill: cfg.idle_pill,
    })
}

#[derive(serde::Serialize)]
struct MicStatus {
    available: bool,
    device: String,
    input_rate: u32,
    channels: u16,
    /// Live signal level, 0.0-1.0ish. Non-zero proves the stream is genuinely
    /// running rather than merely opened.
    level: f32,
    buffered_secs: f32,
}

#[tauri::command]
fn mic_status(state: tauri::State<'_, AppState>) -> MicStatus {
    match &*state.audio.lock() {
        Some(a) => MicStatus {
            available: true,
            device: a.device_name().to_string(),
            input_rate: a.input_rate(),
            channels: a.input_channels(),
            level: a.level(),
            buffered_secs: {
                let r = a.ring();
                r.oldest().secs_since(r.cursor())
            },
        },
        None => MicStatus {
            available: false,
            device: "none".into(),
            input_rate: 0,
            channels: 0,
            level: 0.0,
            buffered_secs: 0.0,
        },
    }
}

#[derive(serde::Serialize)]
struct SttStatus {
    state: ModelState,
    detail: String,
}

#[tauri::command]
fn stt_status(state: tauri::State<'_, AppState>) -> SttStatus {
    SttStatus {
        state: state.stt.state(),
        detail: state.stt.detail(),
    }
}

/// Live microphone level, polled by the overlay to drive the waveform.
///
/// Polled rather than pushed: at 20 Hz an event stream would be 20 IPC messages
/// a second for a purely decorative signal, and the overlay only needs it while
/// it is visible.
#[tauri::command]
fn capture_level(state: tauri::State<'_, AppState>) -> f32 {
    state.audio.lock().as_ref().map(|a| a.level()).unwrap_or(0.0)
}

#[tauri::command]
fn embed_status(state: tauri::State<'_, AppState>) -> embedding::EmbedStatus {
    state.embeddings.status(&state.db)
}

#[derive(serde::Serialize)]
struct RouterStatus {
    state: router::ModelState,
    detail: String,
}

/// Whether commands the grammar does not recognise get a second chance.
///
/// Worth showing, because the difference is invisible until you phrase
/// something unusually: with the router, an unfamiliar phrasing is understood;
/// without it, the same words come back as "not sure what to do with that".
#[tauri::command]
fn router_status(state: tauri::State<'_, AppState>) -> RouterStatus {
    RouterStatus {
        state: state.router.state(),
        detail: state.router.detail(),
    }
}

/// Resize the overlay to the height the page just measured for itself.
///
/// The page is the only thing that knows how tall its content actually is —
/// row heights fall out of the font Windows resolved and the display scaling,
/// neither of which is knowable from here. Estimating it in Rust clipped the
/// question: content is anchored to the bottom of the window, so a window
/// shorter than its content overflows off the *top* and leaves only the last
/// row visible.
///
/// Clamped at both ends anyway. This number arrives from a webview, and a
/// webview mid-layout can report anything at all.
#[tauri::command]
fn size_overlay(app: tauri::AppHandle, height: u32, width: Option<u32>, css: f64, dpr: f64) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    let Ok(current) = w.outer_size() else { return };
    let scale = w.scale_factor().unwrap_or(1.0);

    // Physical throughout, because that is the only unit both sides agree on.
    // The page measures in CSS pixels and the window is configured in logical
    // ones; on a scaled display those differ, and passing one for the other is
    // what clipped the top option off the list.
    let min = (OVERLAY_MIN_HEIGHT as f64 * scale) as u32;
    let max = (OVERLAY_MAX_HEIGHT as f64 * scale) as u32;
    let height = height.clamp(min, max);

    // Width is sent only by the idle pill, and only because it has to be: an
    // idle window is *clickable*, and a transparent window intercepts clicks
    // across its whole rectangle. Left at the resting 520 px it would eat every
    // click in a band across the screen where nothing is drawn. Every other
    // state keeps the resting width, where full-width rows are what is wanted.
    let width = match width {
        Some(px) => {
            let min = (OVERLAY_MIN_WIDTH as f64 * scale) as u32;
            let max = (OVERLAY_SIZE.0 as f64 * scale) as u32;
            px.clamp(min, max)
        }
        None => current.width,
    };

    tracing::debug!(css, dpr, scale, height, width, current = current.height, "sizing overlay");
    if let Err(e) = w.set_size(tauri::PhysicalSize::new(width, height)) {
        tracing::warn!(?e, "could not resize the overlay");
        return;
    }
    // The window is positioned by its bottom edge, so this keeps the pill where
    // the user is already looking and grows the list upward into empty space.
    position_overlay(&w);
}

/// Answer the overlay's destination question by choosing option `index`.
///
/// Executes the command that was waiting rather than re-routing the transcript:
/// running the grammar a second time could land somewhere else entirely, and
/// the point of the question was that the *only* undecided part was this slot.
#[tauri::command]
fn answer_question(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    index: usize,
) -> Result<(), String> {
    // Taking the question consumes it, so a double click cannot save twice.
    let Some(answered) = state.questions.answer(index) else {
        // Expired, already answered, or superseded by a newer capture. Not an
        // error worth showing: the overlay is on its way out either way.
        tracing::debug!(index, "no question to answer");
        return Ok(());
    };
    let question::Answered {
        command_id,
        accepted,
        command: cmd,
        context: ctx,
    } = answered;

    // The verdict, before the work. This is the only place in the system where
    // the user says in so many words which answer was right, and it is what
    // ADR-0005 needs to stop hand-tuning the threshold that produced the
    // question in the first place.
    if let Err(e) = state
        .db
        .record_correction(command_id, accepted, None, &cmd.slots)
    {
        tracing::warn!(?e, "could not record the correction");
    }

    let out = memos_agent::execute(&state.db, &cmd, &ctx).map_err(|e| e.to_string())?;
    if matches!(out.kind, "save" | "note") {
        let _ = state.db.record_capture(&week_start());
        state.embeddings.nudge();
    }
    tracing::info!(summary = %out.summary, accepted, "answered");

    // Same receipt the spoken path produces, so an answered command and a
    // command that never needed asking look identical once done.
    let _ = app.emit_to(
        "overlay",
        "capture:result",
        transcription::CaptureResult::receipt(&cmd.transcript, out),
    );
    if let Some(w) = app.get_webview_window("overlay") {
        speak_only(&w);
        let fade = app.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(1_600));
            let _ = fade.emit_to("overlay", "capture:hide", ());
            std::thread::sleep(std::time::Duration::from_millis(140));
            if let Some(w) = fade.get_webview_window("overlay") {
                rest(&fade, &w);
            }
        });
    }
    Ok(())
}

/// Search from the Hub.
///
/// The same retrieval the voice command uses, so a query typed here and the
/// same query spoken cannot disagree — one implementation, two front doors.
#[tauri::command]
fn search(state: tauri::State<'_, AppState>, query: String, limit: usize) -> Vec<Item> {
    let vector = state.embeddings.embed_query(&query);
    let results = memos_retrieval::search_hybrid(
        &state.db,
        &query,
        vector.as_deref(),
        limit.clamp(1, 200),
    )
    .unwrap_or_default();

    let paths = collection_paths(&state.db);
    results
        .into_iter()
        .map(|r| {
            let mut item = Item::from(&r.item, &paths, &state.db);
            item.score = Some(r.score);
            item.why = Some(
                r.sources
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(" + "),
            );
            item
        })
        .collect()
}

/// The Library, or one collection of it.
#[tauri::command]
fn items(
    state: tauri::State<'_, AppState>,
    collection: Option<String>,
    limit: usize,
    offset: usize,
) -> Vec<Item> {
    let paths = collection_paths(&state.db);
    state
        .db
        .list_items(collection.as_deref(), limit.clamp(1, 500), offset)
        .unwrap_or_default()
        .iter()
        .map(|i| Item::from(i, &paths, &state.db))
        .collect()
}

#[tauri::command]
fn recent(state: tauri::State<'_, AppState>, limit: usize) -> Vec<Item> {
    let paths = collection_paths(&state.db);
    state
        .db
        .recent_items(limit.clamp(1, 200))
        .unwrap_or_default()
        .iter()
        .map(|i| Item::from(i, &paths, &state.db))
        .collect()
}

#[derive(serde::Serialize)]
struct CollectionRow {
    id: String,
    name: String,
    path: String,
    /// Nesting level, so the Hub can render the tree without rebuilding it.
    depth: usize,
    items: u32,
}

#[tauri::command]
fn collections(state: tauri::State<'_, AppState>) -> Vec<CollectionRow> {
    state
        .db
        .collections_with_counts()
        .unwrap_or_default()
        .into_iter()
        .map(|(c, items)| CollectionRow {
            id: c.id.to_string(),
            name: c.name,
            depth: c.path.matches('/').count(),
            path: c.path,
            items,
        })
        .collect()
}

/// How routing is actually going, from the correction log.
///
/// ADR-0003 makes Tier 0 coverage a product metric rather than an assumption:
/// if the grammar stops absorbing the majority, either people are phrasing
/// things differently than assumed or the grammar needs extending, and only
/// this table can tell you which. ADR-0006 makes showing it part of the deal
/// for keeping the table at all.
#[tauri::command]
fn routing_stats(state: tauri::State<'_, AppState>) -> memos_db::RoutingStats {
    state.db.routing_stats().unwrap_or_default()
}

/// Write the command log out where the user can read it.
///
/// Into the data directory rather than through a save dialog: it is one file,
/// it belongs beside the database it came from, and returning the path means
/// the UI can show exactly where it went instead of implying it went nowhere.
#[tauri::command]
fn export_command_log(state: tauri::State<'_, AppState>) -> Result<String, String> {
    let rows = state.db.export_commands(100_000).map_err(|e| e.to_string())?;
    let path = data_dir().join("command-log.json");
    let json = serde_json::to_string_pretty(&rows).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    tracing::info!(rows = rows.len(), path = %path.display(), "exported the command log");
    Ok(path.display().to_string())
}

/// Erase it. The other half of ADR-0006's bargain.
///
/// A log that records what the user said, with no way to delete it, is one they
/// were never really asked about. The corrections cascade with the commands, so
/// nothing is left behind holding what they asked to have forgotten.
#[tauri::command]
fn forget_command_log(state: tauri::State<'_, AppState>) -> Result<u32, String> {
    let n = state.db.forget_commands().map_err(|e| e.to_string())?;
    tracing::info!(commands = n, "erased the command log");
    Ok(n)
}

#[derive(serde::Serialize)]
struct Library {
    items: u32,
    collections: u32,
    this_week: u32,
}

#[tauri::command]
fn library_summary(state: tauri::State<'_, AppState>) -> Library {
    Library {
        items: state.db.item_count().unwrap_or(0),
        collections: state.db.collection_paths().unwrap_or_default().len() as u32,
        this_week: state.db.captures_this_week(&week_start()).unwrap_or(0),
    }
}

/// A task, flattened for the interface.
///
/// `about` is the memory the task was spoken alongside, resolved to its title
/// here rather than in the view: a task that reads "finish this" is meaningless
/// without it, and the Hub should not have to fetch a second list to find out
/// what "this" was.
#[derive(serde::Serialize)]
struct TaskRow {
    id: String,
    title: String,
    about: Option<String>,
    due_at: Option<String>,
    done: bool,
    created_at: String,
}

/// Everything on the list, open first.
///
/// Done tasks are included rather than filtered out. A task that vanishes the
/// instant it is ticked gives no confirmation that the tick landed, and undoing
/// a tick you cannot see is not something a user will attempt.
#[tauri::command]
fn tasks(state: tauri::State<'_, AppState>, limit: usize) -> Vec<TaskRow> {
    state
        .db
        .tasks(limit.clamp(1, 500))
        .unwrap_or_default()
        .into_iter()
        .map(|t| TaskRow {
            id: t.id.to_string(),
            title: t.title,
            about: t
                .item_id
                .and_then(|i| state.db.get_item(i).ok().flatten())
                .map(|i| i.title),
            due_at: t.due_at.map(|d| d.to_rfc3339()),
            done: t.status == "done",
            created_at: t.created_at.to_rfc3339(),
        })
        .collect()
}

/// Tick a task, or untick it.
#[tauri::command]
fn set_task_done(state: tauri::State<'_, AppState>, id: String, done: bool) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    state.db.set_task_done(id, done).map_err(|e| e.to_string())
}

/// How many are still open, for the nav badge.
#[tauri::command]
fn open_task_count(state: tauri::State<'_, AppState>) -> u32 {
    state.db.open_task_count().unwrap_or(0)
}

/// Reopen an item's source, and count the access.
///
/// Returns what it opened so the Hub can say so; `Ok(None)` means the item is a
/// note with no source, which is a normal outcome rather than a failure.
#[tauri::command]
fn open_item(state: tauri::State<'_, AppState>, id: String) -> Result<Option<String>, String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    let source = state.db.source_for_item(id).map_err(|e| e.to_string())?;
    state.db.record_access(id).map_err(|e| e.to_string())?;

    let target = source
        .as_ref()
        .and_then(|s| s.url.clone().or_else(|| s.file_path.clone()));
    match &target {
        Some(t) => open_externally(t)?,
        None => tracing::debug!(%id, "item has no source to open"),
    }
    Ok(target)
}

/// Hand a URL or path to the shell.
///
/// Guarded by scheme rather than trusted: everything here came from a captured
/// page, but `javascript:` and `file:` URLs handed to ShellExecute are a way to
/// turn a saved memory into code execution, and the guard costs nothing.
fn open_externally(target: &str) -> Result<(), String> {
    let t = target.trim();
    let is_web = t.starts_with("https://") || t.starts_with("http://");
    let is_local_file = std::path::Path::new(t).is_absolute() && std::path::Path::new(t).exists();
    if !is_web && !is_local_file {
        return Err(format!("refusing to open {t:?}"));
    }

    #[cfg(windows)]
    {
        // `cmd /c start` would need quoting rules that differ per shell;
        // `explorer` takes the argument verbatim and applies the user's own
        // default handler, which is the behaviour a user expects from "open".
        std::process::Command::new("explorer")
            .arg(t)
            // explorer.exe returns a non-zero exit code even on success, so the
            // status is deliberately not checked; a genuine failure surfaces as
            // a spawn error.
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        // `--` so a target that begins with a hyphen is an argument rather than
        // a flag to `open` itself. The scheme guard above already rejects
        // anything that is not http(s) or an existing absolute path.
        std::process::Command::new("open")
            .arg("--")
            .arg(t)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        let _ = t;
    }
    Ok(())
}

/// A knowledge item, flattened for the interface.
#[derive(serde::Serialize)]
struct Item {
    id: String,
    title: String,
    snippet: String,
    collection: Option<String>,
    source_url: Option<String>,
    captured_at: String,
    access_count: u32,
    /// Present only for search results.
    score: Option<f32>,
    why: Option<String>,
}

impl Item {
    fn from(
        i: &memos_core::KnowledgeItem,
        paths: &std::collections::HashMap<memos_core::Id, String>,
        db: &Db,
    ) -> Self {
        Item {
            id: i.id.to_string(),
            title: i.title.clone(),
            snippet: i.content.chars().take(240).collect(),
            collection: i.collection_id.and_then(|c| paths.get(&c).cloned()),
            source_url: match i.source_id {
                Some(_) => db.source_for_item(i.id).ok().flatten().and_then(|s| s.url),
                None => None,
            },
            captured_at: i.captured_at.to_rfc3339(),
            access_count: i.access_count,
            score: None,
            why: None,
        }
    }
}

fn collection_paths(db: &Db) -> std::collections::HashMap<memos_core::Id, String> {
    db.collections_with_counts()
        .unwrap_or_default()
        .into_iter()
        .map(|(c, _)| (c.id, c.path))
        .collect()
}

#[tauri::command]
fn capture_count(state: tauri::State<'_, AppState>) -> u32 {
    state.db.captures_this_week(&week_start()).unwrap_or(0)
}

/// ISO week start (Monday) as a date string — the key for the weekly meter.
fn week_start() -> String {
    use chrono::Datelike;
    let today = chrono::Utc::now().date_naive();
    let monday = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    monday.to_string()
}

pub fn data_dir() -> std::path::PathBuf {
    // Per-user application data, so two accounts on one machine get separate
    // databases with no application logic (§12). The platforms disagree only
    // about where that lives.
    //
    // The fallback is deliberately not `.`: a relative path resolves against
    // the working directory, and an app launched from Finder has `/` for a
    // working directory — so the database open failed with `CannotOpen` and the
    // app panicked before it drew anything. A home-relative path is wrong in
    // the same way on every platform, which is to say visible immediately.
    let base = if cfg!(windows) {
        std::env::var("APPDATA").map(std::path::PathBuf::from)
    } else {
        // ~/Library/Application Support — where a macOS user expects to find
        // an app's data, and where a Time Machine backup includes it.
        std::env::var("HOME").map(|h| {
            std::path::PathBuf::from(h)
                .join("Library")
                .join("Application Support")
        })
    };
    base.unwrap_or_else(|_| std::path::PathBuf::from("."))
        .join("PersonalMemoryOS")
}


/// The overlay's resting shape: tall enough for a list of results to grow
/// upward into, and click-through so none of that empty space is in the way.
const OVERLAY_SIZE: (u32, u32) = (520, 320);
/// Bounds on what the page may ask for. A measurement is a number from a
/// webview, and a webview mid-layout can report anything at all.
const OVERLAY_MIN_HEIGHT: u32 = 120;
const OVERLAY_MAX_HEIGHT: u32 = 620;
/// The idle pill is narrow, and the window shrinks to it. Small enough to hug
/// the pill, large enough that a bad measurement cannot produce a window too
/// small to see or click.
const OVERLAY_MIN_WIDTH: u32 = 90;

/// Make the overlay something the user can click.
///
/// Only ever while a question is on screen: a transparent window intercepts
/// clicks across its entire rectangle rather than where something is drawn, so
/// the rest of the time this window must stay out of the way. Reversed by
/// [`speak_only`].
///
/// The window is *not* sized here. The page measures itself once the options
/// are laid out and calls `size_overlay`, because the height depends on the
/// font Windows resolved, on display scaling and on how many options there are
/// — and a window shorter than its content does not scroll, it clips the top
/// away and leaves only the last row showing.
fn answerable<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    if let Err(e) = w.set_ignore_cursor_events(false) {
        tracing::warn!(?e, "overlay question will not be clickable");
    }
}

/// Back to a window you only ever speak to.
fn speak_only<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    let _ = w.set_ignore_cursor_events(true);
    let _ = w.set_size(tauri::LogicalSize::new(OVERLAY_SIZE.0, OVERLAY_SIZE.1));
    position_overlay(w);
}

/// Where the overlay goes when there is nothing to show.
///
/// Two resting states, and which one applies is the user's choice. With the
/// idle pill on the window never actually leaves — it shrinks back to the small
/// always-there pill, which is the entire point of that mode. With it off the
/// window hides, exactly as it did before the pill existed.
///
/// Note the asymmetry: hiding is safe to do here, but becoming *clickable* is
/// not. That waits until the page has laid the idle pill out and told us how
/// small the window may be — see `overlay_clickable`.
fn rest(app: &tauri::AppHandle, w: &tauri::WebviewWindow) {
    let idle = app
        .try_state::<AppState>()
        .map(|s| s.config.lock().idle_pill)
        .unwrap_or(false);
    if idle {
        let _ = app.emit_to("overlay", "capture:idle", ());
    } else {
        let _ = w.hide();
        speak_only(w);
    }
}

/// Let the page say when the overlay may be pointed at.
///
/// Driven from the page rather than from here because only the page knows when
/// its layout has settled. Turning this on while the window is still at its
/// resting 520x320 would swallow every click in that rectangle, so the idle
/// pill calls `size_overlay` first and this second — in that order, always.
#[tauri::command]
fn overlay_clickable(app: tauri::AppHandle, clickable: bool) {
    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    if let Err(e) = w.set_ignore_cursor_events(!clickable) {
        tracing::warn!(?e, clickable, "could not change overlay click handling");
    }
}

/// What the overlay should be doing when nothing is being captured.
#[derive(serde::Serialize, Clone, Copy)]
struct RestState {
    /// Whether to show the resting pill at all.
    idle_pill: bool,
    /// Whether the pill is anchored by its top edge, and so opens downward.
    top: bool,
}

/// Pulled by the page on load rather than pushed at it. An event emitted before
/// the webview has registered its listeners is simply dropped — and at startup
/// that is exactly the ordering, so the pill stayed invisible at `opacity: 0`
/// in a window that never shrank. Asking is race-free; being told is not.
#[tauri::command]
fn rest_state(state: tauri::State<'_, AppState>) -> RestState {
    let c = state.config.lock();
    RestState {
        idle_pill: c.idle_pill,
        top: c.pill_top,
    }
}

/// Remember where the user just dragged the pill to.
///
/// Reads the window's own rectangle rather than taking coordinates from the
/// page: the drag is performed by the window manager, so the window is the only
/// thing that knows where it ended up.
///
/// Which edge becomes the anchor is decided here, by where the pill landed. In
/// the top third of its display it anchors by its top and opens downward;
/// anywhere else it anchors by its bottom and opens upward. A third rather than
/// a half because opening upward is the better default — it is what the pill
/// does at its resting position — so the flip should need a deliberate move
/// toward the top edge, not merely crossing the middle of the screen.
#[tauri::command]
fn save_pill_anchor(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
) -> Result<RestState, String> {
    let Some(w) = app.get_webview_window("overlay") else {
        return Ok(rest_state(state));
    };
    let (Ok(pos), Ok(size)) = (w.outer_position(), w.outer_size()) else {
        return Ok(rest_state(state));
    };

    let centre_y = pos.y + size.height as i32 / 2;
    let top = w
        .current_monitor()
        .ok()
        .flatten()
        .map(|m| {
            let mp = m.position();
            let ms = m.size();
            centre_y < mp.y + ms.height as i32 / 3
        })
        .unwrap_or(false);

    let state_out = {
        let mut c = state.config.lock();
        c.pill_x = Some(pos.x + size.width as i32 / 2);
        c.pill_y = Some(if top { pos.y } else { pos.y + size.height as i32 });
        c.pill_top = top;
        c.save()?;
        RestState {
            idle_pill: c.idle_pill,
            top: c.pill_top,
        }
    };
    tracing::debug!(x = pos.x, y = pos.y, top, "pill anchored");
    Ok(state_out)
}

// ------------------------------------------------------------------- account

/// Who is signed in, and whether signing in is even on offer.
#[derive(serde::Serialize)]
struct Account {
    /// False in a build with no provider configured. The Hub hides the whole
    /// section rather than showing a button that cannot work.
    available: bool,
    /// Email sign-in needs the API rather than an OAuth client, so it can be
    /// available when Google is not, and the reverse.
    email_available: bool,
    #[serde(flatten)]
    identity: memos_auth::Identity,
}

#[tauri::command]
fn account(state: tauri::State<'_, AppState>) -> Account {
    Account {
        available: state.auth.is_configured(),
        email_available: state.auth.email_available(),
        identity: state.auth.identity(),
    }
}

/// Whether the sign-in screen has been shown and answered.
///
/// Separate from being signed in, and that difference is the whole point: the
/// screen is offered once, and skipping it is an answer. Without this the Hub
/// would present a sign-in wall on every launch to someone who has already
/// said no, which is how an optional account becomes a nag.
#[tauri::command]
fn sign_in_prompt_seen(state: tauri::State<'_, AppState>) -> bool {
    state.config.lock().sign_in_prompt_seen
}

#[tauri::command]
fn dismiss_sign_in_prompt(state: tauri::State<'_, AppState>) -> Result<(), String> {
    let mut cfg = state.config.lock();
    cfg.sign_in_prompt_seen = true;
    cfg.save()
}

/// The application's own OAuth credentials and backend, compiled in.
///
/// These identify *this app* to Google. They are identical for every install
/// and no user ever sees or supplies them — they come from `.env` in the
/// repository at build time (see `build.rs`). An empty client id means this
/// build was compiled without them, and sign-in is simply not offered.
///
/// A value in `config.json` still wins, so a running install can be pointed at
/// a different tenant without a rebuild. That is an escape hatch, not the path
/// anyone is expected to take.
fn built_in_provider() -> memos_auth::Provider {
    memos_auth::Provider {
        authorize_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
        token_url: "https://oauth2.googleapis.com/token".into(),
        userinfo_url: Some("https://openidconnect.googleapis.com/v1/userinfo".into()),
        client_id: env!("MEMOS_GOOGLE_CLIENT_ID").into(),
        client_secret: Some(env!("MEMOS_GOOGLE_CLIENT_SECRET").into())
            .filter(|s: &String| !s.is_empty()),
        scope: "openid email profile".into(),
    }
}

fn built_in_backend() -> memos_auth::Backend {
    memos_auth::Backend {
        api_url: env!("MEMOS_API_URL").into(),
    }
}

/// The provider this build should use: whatever `config.json` overrides, else
/// what was compiled in.
fn effective_provider(cfg: &config::Config) -> memos_auth::Provider {
    if cfg.auth.is_configured() {
        cfg.auth.clone()
    } else {
        built_in_provider()
    }
}

/// Run the sign-in flow, and resolve when the user comes back.
///
/// `spawn_blocking` because the flow is blocking by design: it opens a browser
/// and then waits on a person, for up to five minutes. Holding a Tauri worker
/// thread for that would starve every other command — including the ones the
/// capture path needs — so it goes to the pool that exists for exactly this.
#[tauri::command]
async fn sign_in(app: tauri::AppHandle) -> Result<Account, String> {
    let auth = app.state::<AppState>().auth.clone();
    let signed = tauri::async_runtime::spawn_blocking(move || {
        let identity = auth.sign_in();
        (auth.is_configured(), identity)
    })
    .await
    .map_err(|e| format!("sign-in did not run: {e}"))?;

    match signed {
        (available, Ok(identity)) => {
            tracing::info!(email = ?identity.email, "account connected");
            Ok(Account {
                available,
                email_available: app.state::<AppState>().auth.email_available(),
                identity,
            })
        }
        (_, Err(e)) => {
            // Logged whole, reported short. The user pressing cancel and the
            // provider being unreachable are the same non-event to them: they
            // are not signed in, and the app works exactly as it did.
            tracing::warn!(?e, "sign-in did not complete");
            Err(e.to_string())
        }
    }
}

/// Forget the session on this machine.
#[tauri::command]
fn sign_out(state: tauri::State<'_, AppState>) -> Result<Account, String> {
    state.auth.sign_out().map_err(|e| e.to_string())?;
    Ok(Account {
        available: state.auth.is_configured(),
        email_available: state.auth.email_available(),
        identity: state.auth.identity(),
    })
}

/// Send a one-time code to an email address.
#[tauri::command]
async fn email_start(app: tauri::AppHandle, email: String) -> Result<(), String> {
    let auth = app.state::<AppState>().auth.clone();
    tauri::async_runtime::spawn_blocking(move || auth.email_start(&email))
        .await
        .map_err(|e| format!("did not run: {e}"))?
        .map_err(|e| e.to_string())
}

/// Exchange a code for a session.
#[tauri::command]
async fn email_verify(
    app: tauri::AppHandle,
    email: String,
    code: String,
) -> Result<Account, String> {
    let auth = app.state::<AppState>().auth.clone();
    let identity = tauri::async_runtime::spawn_blocking(move || auth.email_verify(&email, &code))
        .await
        .map_err(|e| format!("did not run: {e}"))?
        .map_err(|e| e.to_string())?;

    let state = app.state::<AppState>();
    Ok(Account {
        available: state.auth.is_configured(),
        email_available: state.auth.email_available(),
        identity,
    })
}

/// Bring the Hub up from the overlay.
#[tauri::command]
fn open_hub(app: tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Turn the idle pill on or off, and act on it immediately.
///
/// Applied to the live window rather than only saved, because a preference that
/// needs a restart to take effect reads as one that did not work.
#[tauri::command]
fn set_idle_pill(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    enabled: bool,
) -> Result<Settings, String> {
    {
        let mut cfg = state.config.lock();
        cfg.idle_pill = enabled;
        cfg.save()?;
    }
    if let Some(w) = app.get_webview_window("overlay") {
        if enabled {
            let _ = w.show();
            let _ = app.emit_to("overlay", "capture:idle", ());
        } else {
            let _ = w.set_ignore_cursor_events(true);
            let _ = w.hide();
            speak_only(&w);
        }
    }
    Ok(settings_of(&state))
}

/// Let the overlay be clicked without ever taking focus.
///
/// `WS_EX_NOACTIVATE`. Without it, clicking an option would pull focus out of
/// the window the user was working in — and the whole premise of this interface
/// is that it does not interrupt what you were doing. The window is created
/// with `focus: false`, but that governs *showing* it, not clicking it.
#[cfg(windows)]
fn never_activates<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    use windows::Win32::Foundation::HWND;
    use windows::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    };

    let Ok(handle) = w.hwnd() else {
        tracing::warn!("no window handle; overlay may steal focus when clicked");
        return;
    };
    unsafe {
        let hwnd = HWND(handle.0 as _);
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style | WS_EX_NOACTIVATE.0 as isize);
    }
}

#[cfg(not(windows))]
fn never_activates<R: Runtime>(_w: &tauri::WebviewWindow<R>) {}

/// Place the overlay near the bottom-centre of whichever monitor the pointer is
/// on, so it appears where the user is actually working on a multi-monitor
/// setup rather than always on the primary display.
///
/// Called before every show, not only at startup: monitor layout and DPI can
/// change while the app sits in the tray for days.
fn position_overlay<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    let app = w.app_handle();
    let Ok(size) = w.outer_size() else { return };

    // A pill the user placed by hand wins over anything computed. The anchor is
    // an edge, not a corner: the window changes size constantly — 46px idle,
    // 520 wide mid-capture, taller again with results — and only by pinning the
    // edge the pill sits on does it stay put while everything else moves.
    let anchor = app
        .try_state::<AppState>()
        .and_then(|s| {
            let c = s.config.lock();
            match (c.pill_x, c.pill_y) {
                (Some(x), Some(y)) => Some((x, y, c.pill_top)),
                _ => None,
            }
        });

    if let Some((cx, cy, top)) = anchor {
        let x = cx - size.width as i32 / 2;
        let y = if top { cy } else { cy - size.height as i32 };
        // Clamped onto a monitor that actually exists. A display unplugged
        // since the pill was placed would otherwise strand it off-screen, where
        // it cannot be dragged back.
        let (x, y) = clamp_onto_a_monitor(w, x, y, size);
        if let Err(e) = w.set_position(PhysicalPosition::new(x, y)) {
            tracing::warn!(?e, "could not position overlay at its anchor");
        }
        return;
    }

    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| w.current_monitor().ok().flatten())
        .or_else(|| w.primary_monitor().ok().flatten());

    let Some(m) = monitor else {
        tracing::warn!("no monitor found; leaving overlay at its default position");
        return;
    };

    let mp = m.position();
    let ms = m.size();
    let x = mp.x + (ms.width as i32 - size.width as i32) / 2;
    // Roughly 14% up from the bottom edge — clear of the taskbar, still in the
    // lower field of view where the eye already is when typing.
    let y = mp.y + ms.height as i32 - size.height as i32 - (ms.height as i32 * 14 / 100);

    if let Err(e) = w.set_position(PhysicalPosition::new(x, y)) {
        tracing::warn!(?e, "could not position overlay");
    }
}

/// Keep a window rectangle on a display that exists.
///
/// Prefers the monitor the point already falls on and only falls back to the
/// primary when it falls on none — so a pill on a second screen stays on that
/// screen rather than being yanked to the middle of the main one.
fn clamp_onto_a_monitor<R: Runtime>(
    w: &tauri::WebviewWindow<R>,
    x: i32,
    y: i32,
    size: tauri::PhysicalSize<u32>,
) -> (i32, i32) {
    let app = w.app_handle();
    let m = app
        .monitor_from_point((x + size.width as i32 / 2) as f64, (y + size.height as i32 / 2) as f64)
        .ok()
        .flatten()
        .or_else(|| w.primary_monitor().ok().flatten());
    let Some(m) = m else { return (x, y) };

    let mp = m.position();
    let ms = m.size();
    // A margin, so a pill dragged flush to an edge stays visibly grabbable
    // rather than sitting half under the screen border.
    const EDGE: i32 = 2;
    let max_x = mp.x + ms.width as i32 - size.width as i32 - EDGE;
    let max_y = mp.y + ms.height as i32 - size.height as i32 - EDGE;
    (
        x.clamp(mp.x + EDGE, max_x.max(mp.x + EDGE)),
        y.clamp(mp.y + EDGE, max_y.max(mp.y + EDGE)),
    )
}

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,memos_desktop=debug,memos_db=debug".into()),
        )
        .with_target(false)
        .init();

    let db = Arc::new(
        Db::open(data_dir().join("memory.db")).expect("open local database"),
    );
    seed_if_empty(&db);

    let latency = Arc::new(LatencyTracker::default());

    // Opened before the window exists and never closed. This is the whole point
    // of stage 2: the hotkey must never pay a device-open cost, so by the time
    // any shortcut can fire there is already audio in the ring buffer.
    let audio = match AudioCapture::start() {
        Ok(a) => {
            hotkey::set_diag_path(data_dir().join("diag.log"));
            hotkey::diag(&format!(
                "microphone open: {} @ {} Hz, {} ch",
                a.device_name(),
                a.input_rate(),
                a.input_channels()
            ));
            Some(a)
        }
        Err(e) => {
            // Capture without audio is useless, but the Hub, the database and
            // the shortcut still work — so report it and carry on rather than
            // refusing to start.
            tracing::error!(?e, "microphone unavailable");
            hotkey::set_diag_path(data_dir().join("diag.log"));
            hotkey::diag(&format!("microphone UNAVAILABLE: {e}"));
            None
        }
    };

    let stt = Stt::new();
    // Started before the window exists, like the microphone: loading the model
    // costs ~220 ms and the first capture must not wait for it.
    let embeddings = Embeddings::new();
    embeddings.start(db.clone());
    let questions = Arc::new(question::Pending::default());

    // Tier 1, started before the window exists like the other two models — but
    // in a process of its own (ADR-0008). Loading costs ~1.3 s and prefills
    // several hundred tokens of prompt; the first capture must not be what
    // waits for that, and Tier 0 answers most commands without consulting it.
    let tier1 = router::Tier1::new();
    match router::find_model() {
        Some(path) => tier1.start(path, db.collection_paths().unwrap_or_default()),
        // Said through the router rather than only logged, so the Hub can
        // explain why unusual phrasings are not being understood instead of
        // leaving it as something the user has to notice for themselves.
        None => tier1.unavailable(&format!(
            "No router model. Fetch it: {} router",
            memos_core::scripts::FETCH_MODELS
        )),
    }

    let cfg = config::Config::load();
    let hold_ms = Arc::new(std::sync::atomic::AtomicU64::new(cfg.hold_threshold_ms));
    tracing::info!(
        hotkey = %cfg.hotkey,
        hold_ms = cfg.hold_threshold_ms,
        config = %config::Config::path().display(),
        "loaded configuration"
    );

    tauri::Builder::default()
        // Must be registered first. An always-on tray app launching twice is
        // not a hypothetical: WebView2 locks its user-data folder, so the
        // second instance comes up with no windows at all and the tray icon
        // just stops working. Focus the running Hub instead.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("second instance blocked; focusing the running Hub");
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.unminimize();
                let _ = w.set_focus();
            }
        }))
        .manage(AppState {
            db: db.clone(),
            latency: latency.clone(),
            audio: parking_lot::Mutex::new(audio),
            stt: stt.clone(),
            embeddings: embeddings.clone(),
            questions: questions.clone(),
            router: tier1.clone(),
            auth: Arc::new(memos_auth::Auth::new(
                effective_provider(&cfg),
                built_in_backend(),
                data_dir(),
            )),
            config: parking_lot::Mutex::new(cfg.clone()),
            hold_ms: hold_ms.clone(),
        })
        .invoke_handler(tauri::generate_handler![
            overlay_painted,
            latency_report,
            capture_count,
            hook_stats,
            get_settings,
            set_hotkey,
            mic_status,
            stt_status,
            capture_level,
            embed_status,
            router_status,
            routing_stats,
            export_command_log,
            forget_command_log,
            answer_question,
            size_overlay,
            overlay_clickable,
            rest_state,
            save_pill_anchor,
            account,
            sign_in,
            sign_out,
            sign_in_prompt_seen,
            dismiss_sign_in_prompt,
            email_start,
            email_verify,
            open_hub,
            set_idle_pill,
            search,
            items,
            recent,
            collections,
            library_summary,
            tasks,
            set_task_done,
            open_task_count,
            open_item
        ])
        .setup(move |app| {
            tray::install(app.handle())?;

            // The overlay is declared in tauri.conf.json with visible:false, so
            // by the time we get here it is already created, its webview is
            // loaded, and its first paint is done. Showing it later is one call
            // rather than a window construction — this *is* the 50 ms budget
            // (§4, stage 1). Constructing on demand would cost 200-400 ms.
            let overlay = app
                .get_webview_window("overlay")
                .expect("overlay window declared in tauri.conf.json");

            // The window is far taller than the pill so results can grow
            // upward into it. That leaves a large transparent rectangle over
            // whatever the user is working in, and a transparent window still
            // swallows clicks — so it is made click-through. The overlay is
            // something you speak to, never something you point at.
            if let Err(e) = overlay.set_ignore_cursor_events(true) {
                tracing::warn!(?e, "overlay will intercept clicks");
            }

            position_overlay(&overlay);

            // The idle pill, if the user wants one. Shown here rather than
            // declared visible in tauri.conf.json so the window still gets its
            // pre-warm — created hidden, webview loaded and first paint done —
            // before anything appears on screen. Showing it after that costs a
            // native show() and no webview construction, which is the same
            // trick the capture path relies on.
            // Only the native show() here. What the page draws is its own
            // decision, taken when it loads and asks `idle_pill_enabled` —
            // emitting at it now would land before its listeners exist.
            if app.state::<AppState>().config.lock().idle_pill {
                let _ = overlay.show();
            }

            // Vocabulary for whisper's decoder bias: the user's own collection
            // names. These are exactly the words a generic model gets wrong and
            // exactly the words the intent router depends on.
            let hints = Hints {
                vocabulary: db
                    .collection_paths()
                    .unwrap_or_default()
                    .iter()
                    .flat_map(|p| p.split('/').map(str::to_string).collect::<Vec<_>>())
                    .collect(),
            };

            // Results arrive on the worker thread. The overlay is replaced by
            // the transcript rather than dismissed on release, so the user sees
            // what was actually heard before it disappears.
            stt.attach_db(db.clone());
            stt.attach_embeddings(embeddings.clone());
            stt.attach_questions(questions.clone());
            stt.attach_router(tier1.clone());

            // Clicking an option must never pull focus out of whatever the user
            // was working in.
            never_activates(&overlay);

            let result_handle = app.handle().clone();
            stt.start(None, move |res| {
                let _ = result_handle.emit_to("overlay", "capture:result", res.clone());

                // A question is the one time the overlay is something you point
                // at rather than speak to, so for as long as one stands it stops
                // being click-through and shrinks to fit its options — a
                // transparent window swallows clicks across its whole rectangle,
                // and 520x320 of that over someone's work is not acceptable for
                // the sake of three rows.
                if res.ask.is_some() {
                    if let Some(w) = result_handle.get_webview_window("overlay") {
                        answerable(&w);
                    }
                    // Answering hides the overlay; expiry is what dismisses an
                    // unanswered one, and it is deliberately slower than the
                    // reading linger below.
                    let expire = result_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(question::LIFETIME);
                        if let Some(state) = expire.try_state::<AppState>() {
                            state.questions.clear();
                        }
                        if let Some(w) = expire.get_webview_window("overlay") {
                            let _ = expire.emit_to("overlay", "capture:hide", ());
                            std::thread::sleep(std::time::Duration::from_millis(140));
                            // Click-through again the moment the options are
                            // gone: an expired question must not leave a
                            // clickable rectangle sitting over the user's work.
                            let _ = w.set_ignore_cursor_events(true);
                            rest(&expire, &w);
                        }
                    });
                    return;
                }

                if let Some(w) = result_handle.get_webview_window("overlay") {
                    // Scale with how much there is to read. A fixed dwell either
                    // rushes a long transcript off the screen or leaves a short
                    // one loitering; roughly 45 ms per character tracks reading
                    // speed well enough. An empty capture gets the minimum:
                    // there is nothing to read, and lingering over a failure
                    // makes it feel worse than it is.
                    let results = res
                        .outcome
                        .as_ref()
                        .map(|o| o.results.len() as u64)
                        .unwrap_or(0);
                    let linger = if res.empty {
                        900
                    } else if results > 0 {
                        // A receipt is confirmation of something the user
                        // already knows they asked for; a result list is
                        // something they have to actually read. Roughly a
                        // second per result, and the ceiling rises with it.
                        (1600 + results * 900).min(9000)
                    } else {
                        (1200 + res.text.chars().count() as u64 * 45).min(5000)
                    };
                    let fade = result_handle.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(linger));
                        // Fade first, then hide once the transition has run —
                        // snapping a window out of existence reads as a glitch.
                        let _ = fade.emit_to("overlay", "capture:hide", ());
                        std::thread::sleep(std::time::Duration::from_millis(140));
                        rest(&fade, &w);
                    });
                }
            });

            let ring = app
                .state::<AppState>()
                .audio
                .lock()
                .as_ref()
                .map(|a| a.ring());

            let handle = app.handle().clone();
            let tracker = latency.clone();
            let hold_src = hold_ms.clone();
            let stt_worker = stt.clone();
            hotkey::diag(&format!(
                "--- startup: hotkey={} hold={}ms debug_keys={} ---",
                cfg.hotkey, cfg.hold_threshold_ms, cfg.debug_keys
            ));
            if cfg.debug_keys {
                let log = data_dir().join("keylog.txt");
                tracing::warn!(path = %log.display(), "key logging ENABLED (diagnostic)");
                hotkey::start_key_log(log);
            }
            let rx = hotkey::listen(cfg.chord());
            hotkey::report_health_after(std::time::Duration::from_secs(10));

            std::thread::Builder::new()
                .name("hotkey-dispatch".into())
                .spawn(move || {
                    let mut showing = false;
                    let mut capture_from: Option<memos_stt::Cursor> = None;
                    let mut pending_ctx: Option<memos_context::Pending> = None;
                    loop {
                        match rx.recv() {
                            Err(_) => break, // hook gone; app shutting down
                            Ok(ChordState::Released) => {
                                tracing::debug!("chord released");
                                if showing {
                                    showing = false;
                                    let released = std::time::Instant::now();
                                    let _ = handle.emit_to("overlay", "capture:end", ());

                                    match (ring.as_ref(), capture_from.take()) {
                                        (Some(r), Some(from)) => match r.read_from(from) {
                                            Some(audio) => {
                                                // Hand off and return immediately.
                                                // Blocking here would make the
                                                // dispatch thread miss the next
                                                // chord entirely.
                                                let ctx = pending_ctx
                                                    .take()
                                                    .map(|p| p.finish())
                                                    .unwrap_or_default();
                                                if !stt_worker.submit(
                                                    audio,
                                                    hints.clone(),
                                                    ctx,
                                                    released,
                                                ) {
                                                    let _ = overlay.hide();
                                                }
                                            }
                                            None => {
                                                // The start cursor aged out of the
                                                // ring — the chord was held past
                                                // 30 s. Say so rather than
                                                // transcribing the wrong audio.
                                                tracing::warn!("capture outran the audio buffer");
                                                pending_ctx = None;
                                                let _ = overlay.hide();
                                            }
                                        },
                                        _ => {
                                            pending_ctx = None;
                                            let _ = overlay.hide();
                                        }
                                    }
                                }
                            }
                            Ok(ChordState::Engaged) => {
                                tracing::debug!("chord engaged");
                                // Debounce. A single-modifier binding such as
                                // `rctrl` would otherwise fire during an
                                // ordinary Ctrl+C. Waiting here is free from M1
                                // onward: the microphone ring buffer already
                                // holds the audio spoken during the delay, so
                                // nothing the user says is lost.
                                let hold = std::time::Duration::from_millis(
                                    hold_src.load(std::sync::atomic::Ordering::SeqCst),
                                );
                                if !hold.is_zero() {
                                    match rx.recv_timeout(hold) {
                                        Ok(ChordState::Released) => continue, // a tap
                                        Ok(ChordState::Engaged) => {}
                                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                        Err(_) => break,
                                    }
                                }
                                // Clock starts at the decision to show, not at
                                // key-down, so the debounce is excluded from the
                                // stage-1 measurement it would otherwise dwarf.
                                tracker.begin();
                                showing = true;
                                // Rewind past the debounce plus a margin: speech
                                // starts fractionally before the chord registers.
                                // The audio is already buffered, so including it
                                // is free — and it is the difference between
                                // "ave this to React" and "save this to React".
                                // Start gathering context now, not at release.
                                // The user is about to speak for a few seconds;
                                // collection finishes well inside that window,
                                // so it costs nothing on the timeline.
                                pending_ctx = Some(memos_context::start(
                                    ContextPermissions::default(),
                                ));
                                capture_from = ring.as_ref().map(|r| {
                                    let lead = hold_src
                                        .load(std::sync::atomic::Ordering::SeqCst)
                                        as f32
                                        + 250.0;
                                    r.cursor_secs_ago(lead / 1000.0)
                                });
                                let t0 = std::time::Instant::now();
                                // Resting shape and click-through *before* the
                                // show. With the idle pill on, this window is
                                // currently small and clickable; a capture that
                                // began from that state would lay its transcript
                                // out inside a pill-sized window and intercept
                                // clicks while the user is talking. `speak_only`
                                // restores both, and is a no-op-ish pair of
                                // native calls when nothing changed.
                                speak_only(&overlay);
                                if let Err(e) = overlay.show() {
                                    tracing::error!(?e, "failed to show overlay");
                                }
                                // The Rust-side half of stage 1, logged
                                // separately from the painted-frame report. If
                                // the webview never reports, this still tells us
                                // what the native window cost.
                                let native_ms = t0.elapsed().as_secs_f64() * 1000.0;
                                tracing::info!("position+show(): {native_ms:.2} ms");
                                append_line(
                                    data_dir().join("latency.log"),
                                    &format!("native show: {native_ms:.2} ms"),
                                );
                                // Deliberately NOT set_focus(): stealing focus
                                // would yank the caret out of whatever the user
                                // is working in, defeating the point of an
                                // overlay that appears over their work.
                                let _ = handle.emit_to("overlay", "capture:begin", ());
                            }
                        }
                    }
                })
                .expect("spawn hotkey dispatch");

            tracing::info!(chord = %cfg.hotkey, "ready — hold the chord to test overlay latency");
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the Hub returns the app to the tray rather than exiting.
            // An always-on capture tool that quits when you close its window is
            // not always-on.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                if window.label() == "main" {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("run application");
}

/// A starter hierarchy, so the router grammar and the Hub have something real
/// to work with on first launch. Replaced by onboarding at M4.
fn seed_if_empty(db: &Db) {
    let existing = db.collection_paths().unwrap_or_default();
    if !existing.is_empty() {
        return;
    }
    // Mirrors the hierarchy in spec section 9, three levels deep, so the
    // grammar has realistic destinations to resolve against from first launch.
    /// One root and its children, each child with its own children.
    type Branch<'a> = (&'a str, &'a [(&'a str, &'a [&'a str])]);

    let tree: &[Branch] = &[
        (
            "Study",
            &[
                ("Programming", &["React", "TypeScript", "Python"][..]),
                ("AI", &[][..]),
                ("English", &[][..]),
            ][..],
        ),
        (
            "Life",
            &[("House", &[][..]), ("Garden", &[][..]), ("Finance", &[][..])][..],
        ),
        (
            "Career",
            &[
                ("Job Applications", &[][..]),
                ("Companies", &[][..]),
                ("Interviews", &[][..]),
            ][..],
        ),
    ];
    for (root, children) in tree {
        let Ok(parent) = db.create_collection(root, None) else {
            continue;
        };
        for (child, grandchildren) in *children {
            let Ok(mid) = db.create_collection(child, Some(parent.id)) else {
                continue;
            };
            for g in *grandchildren {
                let _ = db.create_collection(g, Some(mid.id));
            }
        }
    }
    tracing::info!("seeded starter collections");
}
