// Release builds must not open a console window behind the app.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod config;
mod detect;
mod embedding;
mod hotkey;
mod inject;
mod question;
mod latency;
mod meeting;
mod transcription;
mod tray;

use std::sync::Arc;

use embedding::Embeddings;
use hotkey::{ChordState, Mode};
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
    /// Sign-in. Present whether or not a provider is configured; an
    /// unconfigured one simply answers "not signed in" to everything, which is
    /// the same answer a signed-out user gets and needs no special casing.
    ///
    pub auth: Arc<memos_auth::Auth>,
    pub config: parking_lot::Mutex<config::Config>,
    /// Read by the dispatch thread on every engagement, so a changed debounce
    /// takes effect without a restart like the binding does.
    pub hold_ms: Arc<std::sync::atomic::AtomicU64>,
    pub meeting: meeting::Recorder,
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
    /// `None` when dictation is unbound, which is the default.
    dictate_hotkey: Option<String>,
    hold_threshold_ms: u64,
    debug_keys: bool,
    active_chord: String,
    /// Empty when nothing is bound. What the *hook* holds, not what the file
    /// says — the two differ when a hand-edited spec failed to parse, and that
    /// is precisely the case the screen has to be able to show.
    active_dictate_chord: String,
    idle_pill: bool,
}

/// One place that reads the config into the shape the Hub expects, so a field
/// added here cannot be forgotten by one of the three commands that return it.
fn settings_of(state: &tauri::State<'_, AppState>) -> Settings {
    let c = state.config.lock().clone();
    Settings {
        hotkey: c.hotkey,
        dictate_hotkey: c.dictate_hotkey,
        hold_threshold_ms: c.hold_threshold_ms,
        debug_keys: c.debug_keys,
        active_chord: hotkey::active_chord_label(),
        active_dictate_chord: hotkey::dictate_chord_label(),
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
    drop(cfg);

    Ok(settings_of(&state))
}

/// Bind, rebind, or unbind the dictation shortcut.
///
/// `None` — or an empty string, which is what a cleared field sends — turns
/// dictation off. Validated before anything is written, for the same reason
/// `set_hotkey` is: a rejected spec must leave the working state untouched.
#[tauri::command]
fn set_dictate_hotkey(
    state: tauri::State<'_, AppState>,
    spec: Option<String>,
) -> Result<Settings, String> {
    let spec = spec.filter(|s| !s.trim().is_empty());
    let chord = match &spec {
        Some(spec) => Some(config::Chord::parse(spec)?),
        None => None,
    };

    let mut cfg = state.config.lock();
    // Refused rather than accepted-and-ignored. The hook checks capture first
    // and stops there, so binding both to one gesture would leave a shortcut
    // that is configured, displayed, and silently dead.
    if let Some(c) = &chord {
        if c.same_gesture_as(&cfg.chord()) {
            return Err(format!(
                "{} is already the capture shortcut",
                cfg.chord().label()
            ));
        }
    }

    cfg.dictate_hotkey = spec;
    cfg.save()?;
    hotkey::set_dictate_chord(chord);
    hotkey::diag(&format!(
        "dictation shortcut set to {:?}",
        cfg.dictate_hotkey.as_deref().unwrap_or("(none)")
    ));
    drop(cfg);

    Ok(settings_of(&state))
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
    state: transcription::ModelState,
    detail: String,
}

/// Whether commands are being routed by the model or by the grammar.
///
/// Worth showing, and shown from the last thing that actually happened rather
/// than from configuration. A tier that is configured, reports itself fine and
/// silently declines every command is the least debuggable state this design
/// can produce — it has already happened once here, for three days, over a
/// stale binary nothing on screen mentioned.
#[tauri::command]
fn router_status(state: tauri::State<'_, AppState>) -> RouterStatus {
    let (state, detail) = state.stt.cloud_health();
    RouterStatus { state, detail }
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
    // Told the size rather than left to read it back: the resize above has not
    // landed yet on macOS.
    position_overlay_sized(&w, tauri::PhysicalSize::new(width, height));
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

    // The words were spoken on the capture this answers, and were counted
    // then. What is new is the outcome: a question answered with "yes, save
    // it" is the capture that command was always going to be, so the day's
    // capture tally moves here and the words do not.
    if matches!(out.kind, "save" | "note") {
        let _ = state
            .db
            .record_activity(0, 0, memos_db::ActivityKind::Capture);
    }
    announce(&app, &state.db.evaluate().unwrap_or_default());

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

/// Make a collection, under another one or at the top.
///
/// Addressed by path rather than id, because that is what the interface has in
/// its hand: the breadcrumb it is standing in. Returns the new path, which is
/// also a destination the router can file into from the next command onwards —
/// the collection list is read fresh on every one.
#[tauri::command]
fn create_collection(
    state: tauri::State<'_, AppState>,
    name: String,
    parent: Option<String>,
) -> Result<String, String> {
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("a collection needs a name".into());
    }
    if name.contains('/') {
        return Err("a name cannot contain a slash".into());
    }
    let parent_id = match parent.as_deref() {
        None => None,
        Some(path) => Some(
            state
                .db
                .collection_id_by_path(path)
                .map_err(|e| e.to_string())?
                .ok_or_else(|| format!("no collection at {path}"))?,
        ),
    };
    state
        .db
        .create_collection(&name, parent_id)
        .map(|c| c.path)
        .map_err(|e| e.to_string())
}

/// Rename one, and every path beneath it. Returns the new path.
#[tauri::command]
fn rename_collection(
    state: tauri::State<'_, AppState>,
    id: String,
    name: String,
) -> Result<String, String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    let name = name.trim().to_string();
    if name.is_empty() {
        return Err("a collection needs a name".into());
    }
    if name.contains('/') {
        return Err("a name cannot contain a slash".into());
    }
    state
        .db
        .rename_collection(id, &name)
        .map_err(|e| e.to_string())
}

/// Delete a collection and its children. The memories inside come loose.
#[tauri::command]
fn delete_collection(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    state.db.delete_collection(id).map_err(|e| e.to_string())
}

/// How many memories a delete would unfile, for the confirmation.
#[tauri::command]
fn collection_size(state: tauri::State<'_, AppState>, path: String) -> u32 {
    state.db.count_in_subtree(&path).unwrap_or(0)
}

/// A collection's document, flattened for the editor (ADR-0010).
#[derive(serde::Serialize)]
struct NoteRow {
    id: String,
    title: String,
    /// Markdown. The editor round-trips through this and hands it straight back.
    body: String,
    updated_at: String,
    /// When a person last edited it, as opposed to a capture growing it.
    edited_at: Option<String>,
    /// How many captures have been folded in — the document's own provenance.
    sources: u32,
}

impl NoteRow {
    fn of(db: &memos_db::Db, n: memos_db::Note) -> Self {
        NoteRow {
            sources: db.note_sources(n.id).unwrap_or(0),
            id: n.id.to_string(),
            title: n.title,
            body: n.body,
            updated_at: n.updated_at.to_rfc3339(),
            edited_at: n.edited_at.map(|d| d.to_rfc3339()),
        }
    }
}

/// The document for a collection, if anything has started it.
///
/// `None` is the ordinary answer for a collection nobody has captured into:
/// an empty file is not conjured because somebody opened a page.
#[tauri::command]
fn note(state: tauri::State<'_, AppState>, path: String) -> Option<NoteRow> {
    state
        .db
        .note_for_path(&path)
        .ok()
        .flatten()
        .map(|n| NoteRow::of(&state.db, n))
}

/// Begin one by hand, rather than waiting for a capture to begin it.
#[tauri::command]
fn start_note(state: tauri::State<'_, AppState>, path: String) -> Result<NoteRow, String> {
    let id = state
        .db
        .collection_id_by_path(&path)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no collection at {path}"))?;
    let name = path.rsplit('/').next().unwrap_or(&path);
    let n = state.db.start_note(id, name).map_err(|e| e.to_string())?;
    Ok(NoteRow::of(&state.db, n))
}

/// Store what the editor produced.
///
/// Whole-body, not a patch. The document is small, one machine writes it, and
/// a patch protocol between two halves of the same process would be a second
/// place for the text to be wrong.
#[tauri::command]
fn save_note(state: tauri::State<'_, AppState>, id: String, body: String) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    state.db.save_note(id, &body).map_err(|e| e.to_string())
}

/// Open a link from inside a note./// Open a link from inside a note./// Open a link from inside a note.
///
/// The editor is a document, not a browser: a link in it has to leave the app.
/// Guarded by the same scheme check as every other thing this app opens.
#[tauri::command]
fn open_url(url: String) -> Result<(), String> {
    open_externally(&url)
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

// --------------------------------------------------------------- achievements

/// The list behind the bell.
///
/// Evaluated on the way out as well as on the capture path. Some milestones —
/// a streak, a weekly recap — become true because a day passed, not because
/// anything was said, and an app that only checks after a command would not
/// notice a streak until the user had already broken it.
#[tauri::command]
fn notifications(state: tauri::State<'_, AppState>, limit: usize) -> Vec<memos_db::Notification> {
    let _ = state.db.evaluate();
    state.db.notifications(limit).unwrap_or_default()
}

#[tauri::command]
fn unread_notifications(state: tauri::State<'_, AppState>) -> u32 {
    state.db.unread_notifications().unwrap_or(0)
}

#[tauri::command]
fn mark_notifications_read(state: tauri::State<'_, AppState>) -> u32 {
    state.db.mark_notifications_read().unwrap_or(0)
}

#[tauri::command]
fn dismiss_notification(state: tauri::State<'_, AppState>, id: String) {
    if let Err(e) = state.db.dismiss_notification(&id) {
        tracing::warn!(?e, "could not dismiss the notification");
    }
}

#[tauri::command]
fn dismiss_all_notifications(state: tauri::State<'_, AppState>) -> u32 {
    state.db.dismiss_all_notifications().unwrap_or(0)
}

/// Tell the Hub about milestones that have just landed.
///
/// A separate event from the poll so a badge can appear the moment it is earned
/// rather than up to a second later. The Hub is often not open — this is a tray
/// app — and that is fine: the rows are already written, and the panel will
/// have them next time it is.
fn announce(app: &tauri::AppHandle, earned: &[memos_db::Notification]) {
    if earned.is_empty() {
        return;
    }
    let _ = app.emit_to("main", "notification:new", earned.to_vec());
}

// ----------------------------------------------------------------- referrals

/// What the referral screen needs, plus whether it can be shown at all.
///
/// `signed_in` is separate from an error because "sign in first" is not a
/// failure — it is the screen's other state, and rendering it as a red message
/// under a broken form would be reporting a problem the user does not have.
#[derive(serde::Serialize)]
struct Referrals {
    /// False when this build has no API configured; the screen says so.
    available: bool,
    signed_in: bool,
    status: Option<memos_auth::backend::ReferralStatus>,
    /// Set when the API was reachable and refused, or unreachable.
    error: Option<String>,
}

impl Referrals {
    fn offer(available: bool, signed_in: bool) -> Self {
        Referrals { available, signed_in, status: None, error: None }
    }
}

/// The caller's code, invites and rewards.
#[tauri::command]
async fn referral_status(app: tauri::AppHandle) -> Referrals {
    let auth = app.state::<AppState>().auth.clone();
    let available = auth.email_available();
    let signed_in = auth.has_api_session();
    if !available || !signed_in {
        return Referrals::offer(available, signed_in);
    }
    // Blocking HTTP, off the UI thread. Every call below does the same.
    match tauri::async_runtime::spawn_blocking(move || auth.referral_status()).await {
        Ok(Ok(status)) => Referrals {
            available,
            signed_in,
            status: Some(status),
            error: None,
        },
        Ok(Err(e)) => Referrals {
            available,
            signed_in,
            status: None,
            error: Some(e.to_string()),
        },
        Err(e) => Referrals {
            available,
            signed_in,
            status: None,
            error: Some(e.to_string()),
        },
    }
}

/// Be referred by somebody.
///
/// The error is returned rather than swallowed: every way this can fail is
/// something the user typed and can retype.
#[tauri::command]
async fn apply_referral(
    app: tauri::AppHandle,
    code: String,
) -> Result<memos_auth::backend::ReferralStatus, String> {
    let auth = app.state::<AppState>().auth.clone();
    let status = tauri::async_runtime::spawn_blocking(move || auth.apply_referral(&code))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;
    cache_entitlement(&app, status.pro_until, status.months_earned);
    Ok(status)
}

/// Mail an invite to each address.
#[tauri::command]
async fn send_invites(
    app: tauri::AppHandle,
    emails: Vec<String>,
) -> Result<memos_auth::backend::Invited, String> {
    let auth = app.state::<AppState>().auth.clone();
    tauri::async_runtime::spawn_blocking(move || auth.send_invites(&emails))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())
}

/// The plan, as this machine last understood it.
///
/// Read from the cache, never from the network. The sidebar draws this on every
/// render and the app has to work with no network at all — see `entitlement` in
/// `config.rs`.
#[tauri::command]
fn entitlement(state: tauri::State<'_, AppState>) -> memos_license::Entitlement {
    state.config.lock().entitlement.clone()
}

/// Ask the API to re-check the plan, and pay out a referral if it is due.
///
/// One call does both because they are the same question from the server's side:
/// here is how much this account has used the app, tell me what it is entitled
/// to. Called when the referral screen opens and after a milestone lands, not on
/// a timer — a background poll would be a request per user per interval to
/// discover a number that changes twice a year.
#[tauri::command]
async fn refresh_entitlement(app: tauri::AppHandle) -> Result<memos_license::Entitlement, String> {
    let state = app.state::<AppState>();
    let auth = state.auth.clone();
    if !auth.has_api_session() {
        return Ok(state.config.lock().entitlement.clone());
    }
    let words = state.db.totals().map(|t| t.words).unwrap_or(0);

    let progress = tauri::async_runtime::spawn_blocking(move || auth.report_progress(words))
        .await
        .map_err(|e| e.to_string())?
        .map_err(|e| e.to_string())?;

    if progress.qualified {
        tracing::info!("a referral qualified; the plan has been extended");
    }
    // `months_earned` is not on this response and must not be guessed at: the
    // cached figure stays until the referral screen fetches the real one.
    let held = app.state::<AppState>().config.lock().entitlement.months_earned;
    Ok(cache_entitlement(&app, progress.pro_until, held))
}

/// Write the plan down, so the meter is right the next time it is drawn.
fn cache_entitlement(
    app: &tauri::AppHandle,
    pro_until: Option<chrono::DateTime<chrono::Utc>>,
    months_earned: u32,
) -> memos_license::Entitlement {
    let state = app.state::<AppState>();
    let mut cfg = state.config.lock();
    cfg.entitlement = memos_license::Entitlement { pro_until, months_earned };
    let held = cfg.entitlement.clone();
    if let Err(e) = cfg.save() {
        // Not fatal. The plan is right for this run and will be re-fetched on
        // the next one; losing the cache costs a request, not an entitlement.
        tracing::warn!(?e, "could not cache the entitlement");
    }
    held
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
/// No link back to a memory. Tasks stopped being attached to whatever was
/// captured most recently, because that attachment was a guess — and a guess
/// rendered as provenance reads as a fact.
#[derive(serde::Serialize)]
struct TaskRow {
    id: String,
    title: String,
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

/// Reword a task.
///
/// Its own command rather than one `update_task` taking both fields: a Tauri
/// argument that is absent and one that is `null` both arrive as `None`, so a
/// single command could not tell "leave the deadline alone" from "clear it".
#[tauri::command]
fn rename_task(state: tauri::State<'_, AppState>, id: String, title: String) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    if title.trim().is_empty() {
        return Err("a task needs words".into());
    }
    state
        .db
        .update_task(id, Some(&title), None)
        .map_err(|e| e.to_string())
}

/// Set when a task is due, or clear it with `null`.
#[tauri::command]
fn set_task_due(
    state: tauri::State<'_, AppState>,
    id: String,
    due_at: Option<String>,
) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    let due = match due_at {
        None => None,
        Some(iso) => Some(
            chrono::DateTime::parse_from_rfc3339(&iso)
                .map_err(|e| e.to_string())?
                .with_timezone(&chrono::Utc),
        ),
    };
    state
        .db
        .update_task(id, None, Some(due))
        .map_err(|e| e.to_string())
}

/// Take a task off the list.
#[tauri::command]
fn delete_task(state: tauri::State<'_, AppState>, id: String) -> Result<(), String> {
    let id = memos_core::Id::parse(&id).map_err(|e| e.to_string())?;
    state.db.delete_task(id).map_err(|e| e.to_string())
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

/// Where the models shipped inside the application live.
///
/// Read-only, and the same for every user on the machine — the opposite of
/// [`data_dir`], which is per-user and writable. This is what makes a fresh
/// install work without running a fetch script: the smallest speech model, the
/// embedding model and the ONNX Runtime are in the installer.
///
/// Resolved from the executable rather than through Tauri, because the models
/// start loading before the window exists and there is no `AppHandle` yet. The
/// two layouts Tauri produces are the two branches below; `None` when the
/// executable cannot be located, which leaves every lookup exactly as it was.
pub fn bundle_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    if cfg!(target_os = "macos") {
        // PersonalMemoryOS.app/Contents/MacOS/<exe> -> Contents/Resources.
        // Only inside a real bundle: a bare `cargo run` binary has no such
        // sibling, and the development paths already cover that case.
        let resources = dir.parent()?.join("Resources");
        resources.is_dir().then_some(resources)
    } else {
        // Windows and Linux: resources are laid down beside the binary.
        Some(dir.to_path_buf())
    }
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
    let size = tauri::LogicalSize::new(OVERLAY_SIZE.0, OVERLAY_SIZE.1);
    let _ = w.set_size(size);
    // Converted here rather than read back, for the same reason: on macOS the
    // resize above is still in flight.
    let scale = w.scale_factor().unwrap_or(1.0);
    position_overlay_sized(w, size.to_physical(scale));
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
/// Type a dictated transcript into the focused window and dismiss the overlay.
///
/// Typing happens on the worker thread rather than off it: `SendInput` costs
/// microseconds, and every millisecond between releasing the key and seeing the
/// words is a millisecond the feature feels slower than the hosted tool it is
/// replacing.
fn dictated(app: &tauri::AppHandle, res: &transcription::CaptureResult) {
    if !res.text.trim().is_empty() {
        match inject::type_text(&res.text) {
            Ok(n) => tracing::info!(chars = n, "dictated"),
            Err(e) => {
                // Nothing else can report this: the transcript went nowhere and
                // the overlay is already on its way out.
                tracing::warn!(%e, "dictation could not be typed");
                hotkey::diag(&format!("dictation FAILED: {e}"));
            }
        }
    }

    let Some(w) = app.get_webview_window("overlay") else {
        return;
    };
    // No dwell on a transcript the user can already read in their own document.
    // An empty capture is the exception — "nothing heard" is the only thing the
    // overlay has to say, and it has to stay up long enough to be read.
    let linger = if res.empty { 900 } else { 0 };
    let fade = app.clone();
    std::thread::spawn(move || {
        if linger > 0 {
            std::thread::sleep(std::time::Duration::from_millis(linger));
        }
        let _ = fade.emit_to("overlay", "capture:hide", ());
        std::thread::sleep(std::time::Duration::from_millis(140));
        rest(&fade, &w);
    });
}

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
        // `pill_top` only means anything once the pill has been dragged
        // somewhere. Until then the answer is the resting position, which is the
        // top of the screen — and reading the stored `false` instead would lay
        // the page out to open upward from a pill sitting at the top edge, where
        // there is nothing to open into.
        top: if c.pill_x.is_some() && c.pill_y.is_some() {
            c.pill_top
        } else {
            true
        },
    }
}

/// Remember where the user just dragged the pill to.
///
/// Reads the window's own rectangle rather than taking coordinates from the
/// page: the drag is performed by the window manager, so the window is the only
/// thing that knows where it ended up.
///
/// Which edge becomes the anchor is decided here, by where the pill landed. In
/// the bottom third of its display it anchors by its bottom and opens upward;
/// anywhere else it anchors by its top and opens downward. Two thirds rather
/// than a half because opening downward is the default — it is what the pill
/// does at its resting position at the top of the screen — so the flip should
/// need a deliberate move toward the bottom edge, not merely crossing the
/// middle.
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
            centre_y < mp.y + (ms.height as i32 * 2 / 3)
        })
        .unwrap_or(true);

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

/// Let the overlay appear over a full-screen application.
///
/// `alwaysOnTop` raises the window's *level*, which settles what it sits above
/// within a space and does nothing about which spaces it appears in. A
/// full-screen application on macOS is not a maximised window — it is its own
/// space, and an ordinary window belongs to the one it was created in. So the
/// overlay was correct, on top, and on a different desktop: invisible over
/// full-screen Code or Chrome, and perfectly fine over either one windowed.
///
/// Two behaviours are needed and they do different jobs:
///
/// - `CanJoinAllSpaces` — appear on whichever space is current, rather than
///   dragging the user back to the one the app started in.
/// - `FullScreenAuxiliary` — be allowed into another application's full-screen
///   space at all. Without this the first flag alone still stops at the edge of
///   a full-screen app, which is exactly the reported symptom.
///
/// The level goes up to the status-item level as well. Floating is above
/// ordinary windows but below the things a full-screen app puts over itself,
/// and being on the right space but underneath is the same result as not being
/// there.
///
/// Must run on the main thread: these are AppKit setters on a window, and the
/// setup closure this is called from is the main thread.
/// Turn the overlay's window into a non-activating panel.
///
/// The collection-behaviour flags decide how a window behaves across spaces
/// *once it is allowed there*. They do not decide admission. A full-screen
/// application's space admits panels from other applications and does not admit
/// ordinary windows, which is why Spotlight and its kin are all panels, and why
/// the flags alone left the pill stuck on the desktop it started on.
///
/// ## Why this is a class swap, and why that is not reckless here
///
/// Tauri does not create panels, so the window has to be changed into one after
/// the fact. `object_setClass` on a live window is a blunt instrument with one
/// real hazard: if the new class needs more storage than was allocated for the
/// old one, every access past the end is memory corruption.
///
/// So the sizes are compared and the swap is refused if it would grow the
/// object. This is not belt-and-braces — `tao`'s window class adds a
/// `focusable` ivar to `NSWindow`, so it is *larger* than `NSPanel`, and the
/// swap shrinks the object rather than growing it. The check is what turns that
/// from a thing I believe into a thing the program verifies before it acts.
///
/// What is lost with the old class is `tao`'s two overrides:
///
/// - `canBecomeKeyWindow`, which returned a stored flag. A non-activating panel
///   answers this correctly by construction, and better: it can take a click
///   without activating the application, which is the property the overlay
///   needed anyway and got on Windows through `WS_EX_NOACTIVATE`.
/// - `sendEvent:`, which forwarded a background drag. The pill is dragged by
///   the page through Tauri's own command, not by the window background, so
///   nothing depended on it.
#[cfg(target_os = "macos")]
fn become_nonactivating_panel(ptr: *mut std::ffi::c_void) {
    use objc2::runtime::{AnyClass, AnyObject};
    use objc2::ClassType;
    use objc2_app_kit::{NSPanel, NSWindowStyleMask};

    let object = ptr as *mut AnyObject;
    let panel_class: &AnyClass = NSPanel::class();

    // SAFETY: the pointer is a live NSWindow handed over by Tauri.
    let current = unsafe { (*object).class() };
    if current == panel_class {
        return; // already done
    }

    let (have, want) = (current.instance_size(), panel_class.instance_size());
    if want > have {
        crate::hotkey::diag(&format!(
            "overlay: NOT converting to a panel — NSPanel needs {want} bytes, \
             the window has {have}. It will not show over full-screen apps."
        ));
        return;
    }

    // SAFETY: NSPanel is a subclass of NSWindow, so every message the window
    // already answers it still answers; and the size check above guarantees the
    // new class fits inside the existing allocation.
    unsafe { objc2::ffi::object_setClass(object, panel_class as *const AnyClass as *const _) };

    let panel: &NSPanel = unsafe { &*(ptr as *const NSPanel) };
    // The bit that means "take clicks without bringing the application
    // forward". It exists only on panels, which is the other half of why this
    // conversion is necessary rather than merely convenient.
    panel.setStyleMask(panel.styleMask() | NSWindowStyleMask::NonactivatingPanel);
    panel.setFloatingPanel(true);
    // Take key status only when something actually needs typing into, so
    // showing the overlay never pulls the caret out of the user's editor.
    panel.setBecomesKeyOnlyIfNeeded(true);
    // A panel hides itself when its application deactivates unless told not to,
    // and this application is *always* the inactive one — that is the entire
    // premise of an overlay you speak to while working somewhere else.
    panel.setHidesOnDeactivate(false);

    crate::hotkey::diag(&format!(
        "overlay: converted to NSPanel ({have} -> {want} bytes), styleMask={:#x}",
        panel.styleMask().0
    ));
}

#[cfg(target_os = "macos")]
fn floats_over_fullscreen<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    use objc2_app_kit::{NSWindow, NSWindowCollectionBehavior};

    let Ok(ptr) = w.ns_window() else {
        tracing::warn!("no NSWindow; the overlay will not show over full-screen apps");
        return;
    };
    if ptr.is_null() {
        return;
    }

    // SAFETY: `ns_window()` hands back this window's NSWindow, and this runs on
    // the main thread, which is the only thread AppKit permits these on.
    // Become a panel first, because that is what decides admission; the flags
    // below only decide behaviour once admitted.
    become_nonactivating_panel(ptr);

    let window: &NSWindow = unsafe { &*(ptr as *const NSWindow) };

    // Stationary is deliberately *not* set alongside CanJoinAllSpaces. It means
    // "does not participate in Spaces switching", which is the opposite of
    // following the user onto a full-screen space, and the two together are a
    // contradiction the window server resolves in the direction we do not want.
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary,
    );
    // NSStatusWindowLevel. Named by value because objc2 exposes the levels as
    // plain integers, and this is the level the menu bar's own items use.
    window.setLevel(25);

    // Read back rather than assume. Setting a collection behaviour is silent
    // whether or not it takes, and "the overlay still is not over my editor" is
    // impossible to act on without knowing which half failed — the flags, or
    // what macOS did with them.
    crate::hotkey::diag(&format!(
        "overlay window: level={} collectionBehavior={:#x}",
        window.level(),
        window.collectionBehavior().0
    ));
}

#[cfg(not(target_os = "macos"))]
fn floats_over_fullscreen<R: Runtime>(_w: &tauri::WebviewWindow<R>) {}

/// Place the overlay near the bottom-centre of whichever monitor the pointer is
/// on, so it appears where the user is actually working on a multi-monitor
/// setup rather than always on the primary display.
///
/// Called before every show, not only at startup: monitor layout and DPI can
/// change while the app sits in the tray for days.
fn position_overlay<R: Runtime>(w: &tauri::WebviewWindow<R>) {
    let Ok(size) = w.outer_size() else { return };
    position_overlay_sized(w, size)
}

/// Place the overlay, told what size it is about to be.
///
/// The size is passed in rather than read back because `set_size` does not take
/// effect immediately on macOS — it is dispatched to the main thread, so
/// `outer_size()` on the next line still returns the *previous* size. Every
/// caller here resizes and then positions, so the read was of a stale value and
/// the window was centred as though it were still its old shape.
///
/// Measured, before the fix: the 90x120 idle pill was placed at (595, 633) on a
/// 1710x1107 display, where bottom-centre is (810, 832) — out by exactly half
/// the difference between the pill and the resting 520x320 window. It looked
/// like a positioning bug and was a synchronisation one.
///
/// Windows resizes synchronously, which is why this never showed up there.
fn position_overlay_sized<R: Runtime>(w: &tauri::WebviewWindow<R>, size: tauri::PhysicalSize<u32>) {
    let app = w.app_handle();

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
    // Roughly 5% down from the top edge. Far enough to clear the menu bar and
    // the notch on the displays that have one, close enough that the pill reads
    // as belonging to the top of the screen rather than floating in the middle.
    //
    // The window is anchored by its top edge here, so a result list grows
    // downward into empty space and the pill itself never moves.
    let y = mp.y + (ms.height as i32 * 5 / 100);

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

    let cfg = config::Config::load();

    // The router (ADR-0011). Nothing loads and nothing starts: it is an HTTP
    // call made when a command needs one. What it costs is a key, and without
    // one the app falls back to the grammar for everything.
    match memos_cloud::Cloud::new(cfg.api_key()) {
        Some(cloud) => {
            stt.attach_cloud(Arc::new(cloud));
            hotkey::diag(&format!("router ready: {}", memos_cloud::MODEL));
        }
        // In the diagnostic log beside the other three subsystems, because "is
        // the router up?" was the one question that log could not answer.
        None => hotkey::diag("router unavailable: no API key (config.json: anthropic_api_key)"),
    }
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
            auth: Arc::new(memos_auth::Auth::new(
                effective_provider(&cfg),
                built_in_backend(),
                data_dir(),
            )),
            config: parking_lot::Mutex::new(cfg.clone()),
            hold_ms: hold_ms.clone(),
            meeting: meeting::Recorder::default(),
        })
        .invoke_handler(tauri::generate_handler![
            overlay_painted,
            meeting::meeting_status,
            meeting::meeting_start,
            meeting::meeting_stop,
            meeting::meetings,
            meeting::meeting_text,
            meeting::open_meeting,
            meeting::summarize_meeting,
            meeting::meeting_prompt_accept,
            meeting::meeting_prompt_dismiss,
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
            create_collection,
            rename_collection,
            delete_collection,
            collection_size,
            note,
            start_note,
            save_note,
            open_url,
            library_summary,
            tasks,
            set_task_done,
            rename_task,
            set_task_due,
            delete_task,
            open_task_count,
            open_item,
            notifications,
            unread_notifications,
            mark_notifications_read,
            dismiss_notification,
            dismiss_all_notifications,
            referral_status,
            apply_referral,
            send_invites,
            entitlement,
            set_dictate_hotkey,
            refresh_entitlement
        ])
        .setup(move |app| {
            tray::install(app.handle())?;
            meeting::offer_on_calls(app.handle().clone());

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
            // The same Auth the interface uses, shared rather than rebuilt: it
            // owns the HTTP client and the API's base URL, and dictation wants
            // both. It does not want the session — shaping takes none.
            stt.attach_api(app.state::<AppState>().auth.clone());

            // Clicking an option must never pull focus out of whatever the user
            // was working in.
            never_activates(&overlay);
            // And the overlay is useless if it cannot appear over the editor or
            // the browser the user is actually looking at, which on this
            // platform is usually full-screen.
            floats_over_fullscreen(&overlay);

            let result_handle = app.handle().clone();
            stt.start(None, move |res| {
                let _ = result_handle.emit_to("overlay", "capture:result", res.clone());

                // Dictation ends here. The words go into the application the
                // user was already in — which still has the caret, because the
                // overlay deliberately never takes focus — and there is no
                // receipt to read, nothing was saved, and nothing was earned.
                if res.mode == Mode::Dictate {
                    dictated(&result_handle, &res);
                    return;
                }

                announce(&result_handle, &res.earned);

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
                "--- startup: hotkey={} dictate={} hold={}ms debug_keys={} ---",
                cfg.hotkey,
                // What the hook was actually handed, not what the file says.
                // The two differ when a hand-edited spec failed to parse, and
                // "I bound it and nothing happens" is exactly the report that
                // cannot be answered without knowing which.
                match cfg.dictate_chord() {
                    Some(c) => c.label().to_string(),
                    None => "(unbound)".into(),
                },
                cfg.hold_threshold_ms,
                cfg.debug_keys
            ));
            if cfg.debug_keys {
                let log = data_dir().join("keylog.txt");
                tracing::warn!(path = %log.display(), "key logging ENABLED (diagnostic)");
                hotkey::start_key_log(log);
            }
            // Said at startup, beside the models and the shortcut, because it
            // decides the same kind of question they do. Without Accessibility
            // every context lookup answers "nothing" — a saved memory keeps the
            // words and loses what they were about — and that is indisplayable
            // from an application that genuinely exposes nothing. It is also a
            // separate grant from Input Monitoring, which is the mistake this
            // line exists to catch.
            #[cfg(target_os = "macos")]
            {
                hotkey::diag(if memos_context::macos_impl::is_trusted() {
                    "Accessibility granted — window, selection and URL will be captured"
                } else {
                    "Accessibility DENIED — captures will have no context. \
                     System Settings > Privacy & Security > Accessibility"
                });
                // The same walk a capture does, but not now: at startup the
                // application that just launched is the frontmost one, and
                // reading our own window says nothing about whether we can read
                // anybody else's. Ten seconds in, the user is back in whatever
                // they were doing, which is the case that matters.
                //
                // "Granted" and "works" turned out to be different things, and
                // the AXError codes are the only place the difference shows.
                std::thread::spawn(|| {
                    std::thread::sleep(std::time::Duration::from_secs(10));
                    hotkey::diag(&format!("  ax: {}", memos_context::macos_impl::probe()));
                });
            }

            let rx = hotkey::listen(cfg.chord(), cfg.dictate_chord());
            hotkey::report_health_after(std::time::Duration::from_secs(10));

            std::thread::Builder::new()
                .name("hotkey-dispatch".into())
                .spawn(move || {
                    let mut showing = false;
                    let mut capture_from: Option<memos_stt::Cursor> = None;
                    let mut pending_ctx: Option<memos_context::Pending> = None;
                    // What a long hold has said so far. The ring keeps only the
                    // last 30 s, so a hold that outlasts it is drained into here
                    // as it goes rather than read back in one piece at release.
                    let mut held: Vec<f32> = Vec::new();
                    loop {
                        let event = if showing {
                            match rx.recv_timeout(std::time::Duration::from_secs(10)) {
                                Ok(event) => Ok(event),
                                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
                                    if let (Some(r), Some(at)) = (ring.as_ref(), capture_from.as_mut()) {
                                        if let Some(audio) = r.drain(at) {
                                            held.extend(audio);
                                        }
                                    }
                                    continue;
                                }
                                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(()),
                            }
                        } else {
                            rx.recv().map_err(|_| ())
                        };
                        match event {
                            Err(_) => break, // hook gone; app shutting down
                            Ok(ChordState::Released(mode)) => {
                                tracing::debug!("chord released");
                                hotkey::diag(&format!("chord RELEASED (showing={showing})"));
                                if showing {
                                    showing = false;
                                    let released = std::time::Instant::now();
                                    let _ = handle.emit_to("overlay", "capture:end", ());

                                    let so_far = std::mem::take(&mut held);
                                    match (ring.as_ref(), capture_from.take()) {
                                        (Some(r), Some(from)) => match r.read_from(from) {
                                            Some(tail) => {
                                                let mut audio = so_far;
                                                audio.extend(tail);
                                                // Hand off and return immediately.
                                                // Blocking here would make the
                                                // dispatch thread miss the next
                                                // chord entirely.
                                                let ctx = pending_ctx
                                                    .take()
                                                    .map(|p| p.finish())
                                                    .unwrap_or_default();
                                                // What "save this" will actually
                                                // have to work with. An empty
                                                // context is declined further
                                                // down with "nothing to save
                                                // here", and that message cannot
                                                // distinguish a permission that
                                                // was never granted from a page
                                                // that exposes nothing — but
                                                // these four fields can.
                                                hotkey::diag(&format!(
                                                    "context: app={:?} title={:?} url={:?} selection={} chars",
                                                    ctx.active_application.as_deref().unwrap_or("-"),
                                                    ctx.active_window_title.as_deref().unwrap_or("-"),
                                                    ctx.current_url.as_deref().unwrap_or("-"),
                                                    ctx.selected_text.as_deref().map(str::len).unwrap_or(0),
                                                ));
                                                // Nothing at all came back, so
                                                // ask the API why. Only in this
                                                // case: the walk is a handful of
                                                // extra calls and there is no
                                                // reason to pay for them on a
                                                // capture that worked.
                                                #[cfg(target_os = "macos")]
                                                if ctx.active_application.is_none()
                                                    && ctx.active_window_title.is_none()
                                                {
                                                    hotkey::diag(&format!(
                                                        "  why: {}",
                                                        memos_context::macos_impl::probe()
                                                    ));
                                                }
                                                if !stt_worker.submit(
                                                    audio,
                                                    // Collection names bias the
                                                    // decoder towards the words
                                                    // a command is made of. In
                                                    // dictation the user is
                                                    // writing prose, and that
                                                    // bias is just a thumb on
                                                    // the scale for the wrong
                                                    // vocabulary.
                                                    match mode {
                                                        Mode::Capture => hints.clone(),
                                                        Mode::Dictate => Hints::default(),
                                                    },
                                                    ctx,
                                                    mode,
                                                    released,
                                                ) {
                                                    let _ = overlay.hide();
                                                }
                                            }
                                            None => {
                                                // The cursor aged out of the ring
                                                // between drains, which only a
                                                // stalled dispatch thread could
                                                // cause. Say so rather than
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
                            Ok(ChordState::Engaged(mode)) => {
                                tracing::debug!("chord engaged");
                                // In the diagnostic log too, not only at debug
                                // level: in a packaged app there is no console
                                // to read, and "I press the shortcut and
                                // nothing happens" is unanswerable without
                                // knowing whether the chord ever matched.
                                hotkey::diag("chord ENGAGED");
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
                                        Ok(ChordState::Released(_)) => continue, // a tap
                                        Ok(ChordState::Engaged(_)) => {}
                                        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                                        Err(_) => break,
                                    }
                                }
                                // Clock starts at the decision to show, not at
                                // key-down, so the debounce is excluded from the
                                // stage-1 measurement it would otherwise dwarf.
                                tracker.begin();
                                showing = true;
                                held.clear();
                                // Rewind past the debounce plus a margin: speech
                                // starts fractionally before the chord registers.
                                // The audio is already buffered, so including it
                                // is free — and it is the difference between
                                // "ave this to React" and "save this to React".
                                // Start gathering context now, not at release.
                                // The user is about to speak for a few seconds;
                                // collection finishes well inside that window,
                                // so it costs nothing on the timeline.
                                // Not for dictation. Context is what a command
                                // is resolved against — "save this" needs to know
                                // what "this" is — and dictation resolves nothing.
                                // Reading window titles and selections to type a
                                // sentence would be collecting it for no reason.
                                pending_ctx = match mode {
                                    Mode::Capture => Some(memos_context::start(
                                        ContextPermissions::default(),
                                    )),
                                    Mode::Dictate => None,
                                };
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
                                // What the window actually became, not what it
                                // was asked to become. "The pill did not
                                // expand" has two very different causes — the
                                // chord never fired, or it fired and the window
                                // stayed pill-sized — and only one line tells
                                // them apart.
                                if let (Ok(sz), Ok(pos)) =
                                    (overlay.outer_size(), overlay.outer_position())
                                {
                                    hotkey::diag(&format!(
                                        "overlay shown: {}x{} at ({}, {})",
                                        sz.width, sz.height, pos.x, pos.y
                                    ));
                                }
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
