//! Meeting notes.
//!
//! Two tracks, transcribed separately: the microphone is "Me" and what the
//! machine plays is "Them". Keeping them apart is what names the speakers — no
//! diarisation, just two sources that were never mixed.
//!
//! Each track is cut at a pause and each piece goes through the same whisper
//! the shortcuts use. The document is rewritten after every piece, in the order
//! things were said, so a crash or a quit loses at most the sentence in flight.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use memos_stt::{AudioCapture, Cursor, Hints, RingBuffer, Speech, Vad, SAMPLE_RATE};
use parking_lot::Mutex;
use tauri::{AppHandle, Manager, Runtime};

use crate::transcription::Stt;

/// What the recording thread is told, through one atomic.
const RUNNING: u8 = 0;
const STOP: u8 = 1;
/// Stopped from the tray, where the only way to see the result is the file.
const STOP_AND_OPEN: u8 = 2;

/// A piece is not cut before this, so a breath does not become a line.
const MIN_SECS: f32 = 3.0;
/// Nor allowed to grow past this, so a monologue still reaches the page while
/// it is happening and a piece stays inside one whisper window.
const MAX_SECS: f32 = 25.0;
/// How far apart a line and the far side's words can be for the line to count
/// as their echo.
const ECHO_WINDOW: Duration = Duration::from_secs(15);

const ME: &str = "Me";
const THEM: &str = "Them";

/// The pause that ends a piece. Longer than dictation's: a meeting is not
/// waiting on the result, and cutting mid-thought splits sentences.
const PAUSE_MS: u32 = 700;

#[derive(Default)]
pub struct Recorder {
    session: Mutex<Option<Session>>,
}

struct Session {
    signal: Arc<AtomicU8>,
    path: PathBuf,
    started: Instant,
}

impl Recorder {
    pub fn is_recording(&self) -> bool {
        self.session.lock().is_some()
    }

    /// Start recording into a new document in `dir`, and return its path.
    ///
    /// Fails only when there is nothing to record at all. A machine whose
    /// output cannot be recorded still gets the microphone, and the document
    /// says so at the top.
    pub fn start(
        &self,
        stt: Arc<Stt>,
        mic: Option<Arc<RingBuffer>>,
        dir: &Path,
    ) -> Result<PathBuf, String> {
        let mut session = self.session.lock();
        if session.is_some() {
            return Err("already recording".into());
        }

        let mut notices = Vec::new();
        let loopback = match AudioCapture::loopback() {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!(error = %e, "the PC's sound cannot be recorded");
                notices.push(format!(
                    "The PC's sound could not be recorded ({e}), so only the microphone is here."
                ));
                None
            }
        };
        if mic.is_none() {
            if loopback.is_none() {
                return Err("no microphone and no output device to record".into());
            }
            notices.push("No microphone, so only the PC's sound is here.".into());
        }

        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let now = chrono::Local::now();
        let path = free_name(dir, &format!("Meeting {}", now.format("%Y-%m-%d %H-%M")));
        let title = format!("Meeting — {}", now.format("%A %-d %B %Y, %H:%M"));
        std::fs::write(&path, render(&title, &notices, &[])).map_err(|e| e.to_string())?;

        let mut tracks = Vec::new();
        if let Some(ring) = mic {
            tracks.push(Track::new(ME, ring));
        }
        if let Some(c) = &loopback {
            tracks.push(Track::new(THEM, c.ring()));
        }

        let signal = Arc::new(AtomicU8::new(RUNNING));
        let doc = Doc {
            path: path.clone(),
            title,
            notices,
            lines: Vec::new(),
        };
        let flag = signal.clone();
        std::thread::Builder::new()
            .name("meeting".into())
            .spawn(move || {
                // Held here so the output stream lives exactly as long as the
                // recording does.
                let _loopback = loopback;
                record(stt, tracks, doc, flag);
            })
            .map_err(|e| e.to_string())?;

        *session = Some(Session {
            signal,
            path: path.clone(),
            started: Instant::now(),
        });
        tracing::info!(path = %path.display(), "meeting notes started");
        Ok(path)
    }

    /// Stop recording. Returns at once; the last pieces are transcribed on the
    /// recording thread, which then opens the document if `open`.
    pub fn stop(&self, open: bool) {
        if let Some(s) = self.session.lock().take() {
            s.signal
                .store(if open { STOP_AND_OPEN } else { STOP }, Ordering::SeqCst);
        }
    }
}

fn record(stt: Arc<Stt>, mut tracks: Vec<Track>, mut doc: Doc, signal: Arc<AtomicU8>) {
    let started = Instant::now();
    let mut warned_no_model = false;
    let open = loop {
        let told = signal.load(Ordering::SeqCst);
        let stopping = told != RUNNING;
        if !stopping {
            std::thread::sleep(Duration::from_millis(500));
        }

        let now = started.elapsed();
        // Every track read before any is cut, so a microphone piece is checked
        // against everything the PC played up to the same moment.
        for track in &mut tracks {
            track.pull(now);
        }
        let mut pieces = Vec::new();
        for track in &mut tracks {
            pieces.extend(track.cut(now, stopping));
        }
        let playing = tracks.iter().find_map(|t| t.playing.as_ref());

        for mut piece in pieces {
            if piece.who == ME {
                if let Some(playing) = playing {
                    playing.silence(piece.at, &mut piece.audio);
                    let mut vad = Vad::default();
                    vad.push(&piece.audio);
                    // All of it was the PC coming back through the microphone.
                    if !vad.heard_speech() {
                        continue;
                    }
                }
            }
            let Some(model) = stt.model() else {
                if !warned_no_model {
                    warned_no_model = true;
                    doc.notices.push(
                        "The speech model is not loaded, so some of this was not transcribed."
                            .into(),
                    );
                }
                continue;
            };
            match model.transcribe(&piece.audio, &Hints::default()) {
                Ok(t) => {
                    let text = without_annotations(&t.text);
                    if !text.is_empty() {
                        doc.lines.push(Line {
                            at: piece.at,
                            who: piece.who,
                            text,
                        });
                    }
                }
                Err(e) => tracing::warn!(error = %e, "a meeting piece was not transcribed"),
            }
        }
        doc.lines.sort_by_key(|l| l.at);
        if let Err(e) = std::fs::write(&doc.path, render(&doc.title, &doc.notices, &doc.lines)) {
            tracing::error!(error = %e, path = %doc.path.display(), "could not write the meeting notes");
        }

        if stopping {
            break told == STOP_AND_OPEN;
        }
    };
    tracing::info!(lines = doc.lines.len(), path = %doc.path.display(), "meeting notes finished");
    if open {
        if let Err(e) = crate::open_externally(&doc.path.to_string_lossy()) {
            tracing::warn!(error = %e, "could not open the meeting notes");
        }
    }
}

/// Where meeting documents go: a folder of their own in Documents, where
/// somebody looking for the file would look.
fn folder<R: Runtime>(app: &AppHandle<R>) -> PathBuf {
    app.path()
        .document_dir()
        .unwrap_or_else(|_| crate::data_dir())
        .join("Meetings")
}

/// Start recording, from the tray or the Hub, and bring the other in line.
pub fn start<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let state = app.state::<crate::AppState>();
    let mic = state.audio.lock().as_ref().map(|a| a.ring());
    let path = state.meeting.start(state.stt.clone(), mic, &folder(app))?;
    crate::tray::meeting_changed(app, true);
    // However it was started, the offer to start it has been answered.
    hide_prompt(app);
    Ok(path)
}

/// Offer to take notes whenever a call starts, and withdraw the offer when it
/// ends.
pub fn offer_on_calls(app: AppHandle) {
    crate::detect::watch(move |call| match call {
        Some(name) => {
            crate::hotkey::diag(&format!("call detected: {name}"));
            if !app.state::<crate::AppState>().meeting.is_recording() {
                show_prompt(&app, name);
            }
        }
        None => hide_prompt(&app),
    });
}

/// Logical pixels from the pill's edge to the prompt: the idle pill's height
/// and a gap.
const PILL_CLEARANCE: f64 = 56.0;

fn show_prompt<R: Runtime>(app: &AppHandle<R>, call: &str) {
    use tauri::Emitter;
    let Some(w) = app.get_webview_window("prompt") else {
        return;
    };
    place_prompt(app, &w);
    let _ = w.emit_to("prompt", "prompt:call", call);
    if let Err(e) = w.show() {
        tracing::warn!(?e, "could not show the meeting prompt");
    }
}

fn hide_prompt<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("prompt") {
        let _ = w.hide();
    }
}

/// Just below the pill, or just above it when the pill sits at the bottom.
fn place_prompt<R: Runtime>(app: &AppHandle<R>, w: &tauri::WebviewWindow<R>) {
    let Ok(size) = w.outer_size() else { return };
    let gap = (PILL_CLEARANCE * w.scale_factor().unwrap_or(1.0)) as i32;
    let anchor = app.try_state::<crate::AppState>().and_then(|s| {
        let c = s.config.lock();
        c.pill_x.zip(c.pill_y).map(|(x, y)| (x, y, c.pill_top))
    });
    let (cx, y) = match anchor {
        Some((x, y, true)) => (x, y + gap),
        Some((x, y, false)) => (x, y - gap - size.height as i32),
        // Where the overlay puts an unmoved pill: top centre, 5% down.
        None => {
            let monitor = app
                .cursor_position()
                .ok()
                .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
                .or_else(|| w.primary_monitor().ok().flatten());
            let Some(m) = monitor else { return };
            (
                m.position().x + m.size().width as i32 / 2,
                m.position().y + m.size().height as i32 * 5 / 100 + gap,
            )
        }
    };
    let (x, y) = crate::clamp_onto_a_monitor(w, cx - size.width as i32 / 2, y, size);
    let _ = w.set_position(tauri::PhysicalPosition::new(x, y));
}

/// "Take notes" on the prompt.
#[tauri::command]
pub fn meeting_prompt_accept(app: AppHandle) -> Result<(), String> {
    hide_prompt(&app);
    start(&app).map(|_| ())
}

/// "Not now", or the prompt timing out. It comes back with the next call.
#[tauri::command]
pub fn meeting_prompt_dismiss(app: AppHandle) {
    hide_prompt(&app);
}

pub fn stop<R: Runtime>(app: &AppHandle<R>, open: bool) {
    app.state::<crate::AppState>().meeting.stop(open);
    crate::tray::meeting_changed(app, false);
}

/// A document in the meetings folder, by file name alone. A path from the
/// interface is never joined as given: `..\` would reach any file on the disk.
fn document<R: Runtime>(app: &AppHandle<R>, name: &str) -> Result<PathBuf, String> {
    if !is_document_name(name) {
        return Err(format!("not a meeting: {name:?}"));
    }
    Ok(folder(app).join(name))
}

fn is_document_name(name: &str) -> bool {
    name.ends_with(".md") && !name.contains(['/', '\\', ':']) && !name.starts_with('.')
}

#[derive(serde::Serialize)]
pub struct MeetingStatus {
    recording: bool,
    /// The document being written, while recording.
    name: Option<String>,
    elapsed_secs: u64,
}

#[derive(serde::Serialize)]
pub struct MeetingFile {
    name: String,
    /// Last written, RFC 3339.
    modified: String,
}

#[tauri::command]
pub fn meeting_status(state: tauri::State<'_, crate::AppState>) -> MeetingStatus {
    match &*state.meeting.session.lock() {
        Some(s) => MeetingStatus {
            recording: true,
            name: s.path.file_name().map(|n| n.to_string_lossy().into_owned()),
            elapsed_secs: s.started.elapsed().as_secs(),
        },
        None => MeetingStatus {
            recording: false,
            name: None,
            elapsed_secs: 0,
        },
    }
}

/// Resolves to the new document's file name.
#[tauri::command]
pub fn meeting_start(app: AppHandle) -> Result<String, String> {
    let path = start(&app)?;
    Ok(path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default())
}

/// The Hub is already showing the transcript, so nothing is opened.
#[tauri::command]
pub fn meeting_stop(app: AppHandle) {
    stop(&app, false);
}

/// Past meetings, newest first.
#[tauri::command]
pub fn meetings(app: AppHandle) -> Vec<MeetingFile> {
    let Ok(entries) = std::fs::read_dir(folder(&app)) else {
        // No folder is no meetings yet, not a failure.
        return Vec::new();
    };
    let mut files: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let modified = e.metadata().ok()?.modified().ok()?;
            is_document_name(&name).then_some((modified, name))
        })
        .collect();
    files.sort_by(|a, b| b.0.cmp(&a.0));
    files
        .into_iter()
        .map(|(modified, name)| MeetingFile {
            name,
            modified: chrono::DateTime::<chrono::Utc>::from(modified).to_rfc3339(),
        })
        .collect()
}

#[tauri::command]
pub fn meeting_text(app: AppHandle, name: String) -> Result<String, String> {
    std::fs::read_to_string(document(&app, &name)?).map_err(|e| e.to_string())
}

#[tauri::command]
pub fn open_meeting(app: AppHandle, name: String) -> Result<(), String> {
    crate::open_externally(&document(&app, &name)?.to_string_lossy())
}

struct Doc {
    path: PathBuf,
    title: String,
    notices: Vec<String>,
    lines: Vec<Line>,
}

struct Line {
    /// Since the recording began.
    at: Duration,
    who: &'static str,
    text: String,
}

struct Piece {
    at: Duration,
    who: &'static str,
    audio: Vec<f32>,
}

/// One source, read from its ring as it fills.
struct Track {
    who: &'static str,
    ring: Arc<RingBuffer>,
    read: Cursor,
    vad: Vad,
    pending: Vec<f32>,
    /// When the pending audio began, by the clock rather than by counting
    /// samples: an output device delivers nothing while nothing plays, so its
    /// sample count runs behind the meeting.
    began: Option<Duration>,
    spoke: bool,
    paused: bool,
    /// On the PC's track only: when it was making sound.
    playing: Option<Playing>,
}

impl Track {
    fn new(who: &'static str, ring: Arc<RingBuffer>) -> Self {
        Self {
            who,
            read: ring.cursor(),
            ring,
            vad: Vad::new(PAUSE_MS),
            pending: Vec::new(),
            began: None,
            spoke: false,
            paused: false,
            playing: (who == THEM).then(Playing::default),
        }
    }

    fn pull(&mut self, now: Duration) {
        let audio = match self.ring.drain(&mut self.read) {
            Some(audio) => audio,
            // Fell a whole ring behind, which a very slow transcription could
            // do. What aged out is gone; carry on from what is still there.
            None => {
                tracing::warn!(
                    who = self.who,
                    "meeting audio outran the buffer; a gap was skipped"
                );
                self.read = self.ring.oldest();
                return;
            }
        };
        if audio.is_empty() {
            return;
        }
        let start = now.saturating_sub(Duration::from_secs_f32(secs(&audio)));
        if self.began.is_none() {
            self.began = Some(start);
        }
        if let Some(playing) = &mut self.playing {
            playing.mark(start, &audio);
        }
        match self.vad.push(&audio) {
            Speech::Silence => {}
            Speech::Speaking => {
                self.spoke = true;
                self.paused = false;
            }
            Speech::Ended => {
                self.spoke = true;
                self.paused = true;
            }
        }
        self.pending.extend(audio);

        // Until somebody speaks, keep only a moment of lead-in, so a piece
        // starts near its first word instead of being cut at the maximum
        // halfway through a sentence that began after a long silence.
        let keep = SAMPLE_RATE as usize / 2;
        if !self.spoke && self.pending.len() > keep * 4 {
            let drop = self.pending.len() - keep;
            self.pending.drain(..drop);
            self.began = Some(now.saturating_sub(Duration::from_secs_f32(secs(&self.pending))));
        }
    }

    /// The pending audio as a piece, if it is time to cut one. Silence is
    /// dropped rather than cut: whisper invents words for it.
    fn cut(&mut self, now: Duration, stopping: bool) -> Option<Piece> {
        let began = self.began?;
        let length = secs(&self.pending);
        let elapsed = now.saturating_sub(began).as_secs_f32();
        if !should_cut(length, elapsed, self.paused, stopping) {
            return None;
        }
        let audio = std::mem::take(&mut self.pending);
        let spoke = std::mem::take(&mut self.spoke);
        self.began = None;
        self.paused = false;
        spoke.then_some(Piece {
            at: began,
            who: self.who,
            audio,
        })
    }
}

/// Length of one slot in [`Playing`].
const SLOT_MS: u64 = 20;
const SLOT_LEN: usize = SAMPLE_RATE as usize * SLOT_MS as usize / 1000;
/// Level above which the PC counts as playing. Its track is the digital signal
/// itself, so silence there is near zero rather than room tone.
const PLAYING_LEVEL: f32 = 0.003;
/// How long after the PC made a sound the microphone can still be hearing it:
/// the trip out of the speakers, the room's reverb, and the two devices'
/// buffers disagreeing about when "now" was.
const ECHO_TAIL_MS: u64 = 400;
/// And how far the other way, for the same disagreement.
const ECHO_LEAD_MS: u64 = 200;

/// When the PC was making sound, in slots since the recording began.
///
/// Without headphones the microphone hears the call as well as the user, and
/// whisper turns that into "Me" saying a garbled version of what they said —
/// garbled enough that comparing words does not catch it. So the microphone
/// is silenced wherever the PC was playing, before whisper hears it. The cost
/// is the user's own words spoken over the other side at the same moment.
#[derive(Default)]
struct Playing {
    slots: Vec<bool>,
}

impl Playing {
    fn mark(&mut self, start: Duration, audio: &[f32]) {
        let first = start.as_millis() as u64 / SLOT_MS;
        for (i, frame) in audio.chunks(SLOT_LEN).enumerate() {
            if level(frame) > PLAYING_LEVEL {
                let slot = (first + i as u64) as usize;
                if self.slots.len() <= slot {
                    self.slots.resize(slot + 1, false);
                }
                self.slots[slot] = true;
            }
        }
    }

    /// Whether the PC was audible close enough to `at` for the microphone to be
    /// hearing it then.
    fn near(&self, at: Duration) -> bool {
        let ms = at.as_millis() as u64;
        let from = ms.saturating_sub(ECHO_TAIL_MS) / SLOT_MS;
        let to = (ms + ECHO_LEAD_MS) / SLOT_MS;
        (from..=to).any(|s| self.slots.get(s as usize).copied().unwrap_or(false))
    }

    /// Zero the microphone audio that began at `start` wherever the PC was
    /// audible.
    fn silence(&self, start: Duration, audio: &mut [f32]) {
        for (i, frame) in audio.chunks_mut(SLOT_LEN).enumerate() {
            if self.near(start + Duration::from_millis(i as u64 * SLOT_MS)) {
                frame.fill(0.0);
            }
        }
    }
}

fn level(frame: &[f32]) -> f32 {
    if frame.is_empty() {
        return 0.0;
    }
    (frame.iter().map(|v| v * v).sum::<f32>() / frame.len() as f32).sqrt()
}

/// `length` is the audio held, `elapsed` the time since it began — they differ
/// for an output device that went quiet.
fn should_cut(length: f32, elapsed: f32, paused: bool, stopping: bool) -> bool {
    if length <= 0.0 {
        return false;
    }
    stopping
        || length >= MAX_SECS
        || (paused && elapsed >= MIN_SECS)
        || (!paused && elapsed >= MAX_SECS)
}

fn secs(audio: &[f32]) -> f32 {
    audio.len() as f32 / SAMPLE_RATE as f32
}

/// Whisper's `*typing*` and `*laughs*`. The bracketed kind is already gone.
fn without_annotations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut inside = false;
    for c in text.chars() {
        match c {
            '*' => inside = !inside,
            _ if !inside => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether a line from the microphone is the far side coming back out of the
/// speakers. Without headphones the microphone hears every question too, and
/// the transcript would carry each one twice — once garbled, and as "Me".
///
/// Judged on words rather than sound: most of the line was said by them, just
/// now. Decided when the document is written, so an echo transcribed before
/// the words it echoes still disappears once they arrive.
fn is_echo(line: &Line, lines: &[Line]) -> bool {
    if line.who != ME {
        return false;
    }
    let spoken = words(&line.text);
    if spoken.is_empty() {
        return false;
    }
    let heard: std::collections::HashSet<String> = lines
        .iter()
        .filter(|l| l.who == THEM)
        .filter(|l| {
            let gap = if l.at > line.at {
                l.at - line.at
            } else {
                line.at - l.at
            };
            gap <= ECHO_WINDOW
        })
        .flat_map(|l| words(&l.text))
        .collect();
    let shared = spoken.iter().filter(|w| heard.contains(*w)).count();
    shared * 10 >= spoken.len() * 6
}

fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric() && c != '\'')
        .filter(|w| !w.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// The document. Consecutive lines from one speaker read as one paragraph under
/// the time it began.
fn render(title: &str, notices: &[String], lines: &[Line]) -> String {
    let mut out = format!("# {title}\n\n");
    for n in notices {
        out.push_str(&format!("_{n}_\n\n"));
    }
    let mut last: Option<&str> = None;
    for line in lines.iter().filter(|l| !is_echo(l, lines)) {
        if last == Some(line.who) {
            // Replace the paragraph break with a space.
            out.truncate(out.trim_end().len());
            out.push(' ');
        } else {
            let s = line.at.as_secs();
            out.push_str(&format!("**{:02}:{:02} {}:** ", s / 60, s % 60, line.who));
        }
        out.push_str(line.text.trim());
        out.push_str("\n\n");
        last = Some(line.who);
    }
    out
}

/// `stem.md`, or `stem (2).md` and on if that is taken — a second meeting in
/// the same minute must not overwrite the first.
fn free_name(dir: &Path, stem: &str) -> PathBuf {
    let mut path = dir.join(format!("{stem}.md"));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stem} ({n}).md"));
        n += 1;
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(at: u64, who: &'static str, text: &str) -> Line {
        Line {
            at: Duration::from_secs(at),
            who,
            text: text.into(),
        }
    }

    #[test]
    fn the_microphone_is_silenced_while_the_pc_plays_and_just_after() {
        let mut playing = Playing::default();
        // The PC plays from 1.0 s to 2.0 s.
        playing.mark(Duration::from_secs(1), &vec![0.2; SAMPLE_RATE as usize]);

        // Four seconds of microphone from 0.0 s.
        let mut mic = vec![0.5; SAMPLE_RATE as usize * 4];
        playing.silence(Duration::ZERO, &mut mic);
        let at = |secs: f32| mic[(secs * SAMPLE_RATE as f32) as usize];

        assert_eq!(at(0.5), 0.5, "before the PC played");
        assert_eq!(at(1.5), 0.0, "while it played");
        assert_eq!(at(2.2), 0.0, "its echo, just after");
        assert_eq!(at(3.0), 0.5, "well after");
    }

    #[test]
    fn a_silent_pc_silences_nothing() {
        let mut playing = Playing::default();
        playing.mark(Duration::ZERO, &vec![0.0; SAMPLE_RATE as usize * 3]);
        let mut mic = vec![0.5; SAMPLE_RATE as usize * 3];
        playing.silence(Duration::ZERO, &mut mic);
        assert!(mic.iter().all(|&v| v == 0.5));
    }

    #[test]
    fn a_pause_cuts_only_after_the_minimum() {
        assert!(!should_cut(1.0, 1.0, true, false));
        assert!(should_cut(4.0, 4.0, true, false));
        assert!(!should_cut(10.0, 10.0, false, false));
    }

    #[test]
    fn a_monologue_is_cut_at_the_maximum() {
        assert!(should_cut(MAX_SECS, MAX_SECS, false, false));
    }

    #[test]
    fn a_quiet_output_device_still_gets_its_last_words_out() {
        // Two seconds of audio, then the far side stopped playing anything.
        assert!(should_cut(2.0, 5.0, true, false));
    }

    #[test]
    fn stopping_cuts_whatever_is_left_but_not_nothing() {
        assert!(should_cut(0.5, 0.5, false, true));
        assert!(!should_cut(0.0, 0.0, false, true));
    }

    #[test]
    fn speakers_alternate_and_runs_join() {
        let doc = render(
            "Meeting",
            &[],
            &[
                line(5, "Them", "Tell me about yourself."),
                line(9, "Me", "Sure."),
                line(75, "Me", "I have built..."),
                line(90, "Them", "Great."),
            ],
        );
        assert_eq!(
            doc,
            "# Meeting\n\n\
             **00:05 Them:** Tell me about yourself.\n\n\
             **00:09 Me:** Sure. I have built...\n\n\
             **01:30 Them:** Great.\n\n"
        );
    }

    #[test]
    fn the_far_side_heard_through_the_microphone_is_not_me() {
        // What an end-to-end run on laptop speakers actually produced.
        let doc = render(
            "Meeting",
            &[],
            &[
                line(
                    0,
                    THEM,
                    "Thanks for joining. Can you walk me through your last role?",
                ),
                line(2, ME, "Can you walk me through your nostrils? sure."),
                line(10, THEM, "and why are you interested in this position?"),
                line(10, ME, "and quite often interested in this position."),
                line(14, ME, "My last role was at a payments company."),
            ],
        );
        assert_eq!(
            doc,
            "# Meeting\n\n\
             **00:00 Them:** Thanks for joining. Can you walk me through your last role? \
             and why are you interested in this position?\n\n\
             **00:14 Me:** My last role was at a payments company.\n\n"
        );
    }

    #[test]
    fn the_same_words_a_minute_later_are_not_an_echo() {
        let lines = [
            line(0, THEM, "Why this position?"),
            line(60, ME, "Why this position?"),
        ];
        assert!(!is_echo(&lines[1], &lines));
    }

    #[test]
    fn annotations_are_dropped() {
        assert_eq!(
            without_annotations("in this position. *typing*"),
            "in this position."
        );
        assert_eq!(without_annotations("*laughs*"), "");
    }

    #[test]
    fn only_a_plain_file_name_names_a_meeting() {
        assert!(is_document_name("Meeting 2026-09-14 14-30.md"));
        assert!(!is_document_name("..\\..\\secrets.md"));
        assert!(!is_document_name("C:notes.md"));
        assert!(!is_document_name("Meeting.txt"));
    }

    #[test]
    fn notices_come_before_the_transcript() {
        let doc = render(
            "Meeting",
            &["No microphone.".into()],
            &[line(0, "Them", "Hi")],
        );
        assert!(doc.starts_with("# Meeting\n\n_No microphone._\n\n**00:00 Them:** Hi"));
    }
}
