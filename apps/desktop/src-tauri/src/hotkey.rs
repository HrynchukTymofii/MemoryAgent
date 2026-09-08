//! Global push-to-talk hotkey via a low-level keyboard hook.
//!
//! ## Why a hook rather than `RegisterHotKey`
//!
//! The interaction is *hold a chord and speak* — often modifier-only, always
//! with separate press and release events. `RegisterHotKey` can express
//! neither: it requires modifiers plus a non-modifier key, and it only reports
//! a press. A `WH_KEYBOARD_LL` hook sees every key transition system-wide.
//!
//! ## Hooks are a chain, and that has consequences
//!
//! Windows calls every installed low-level hook for every keystroke. A hook may
//! pass the event on or swallow it, and swallowing modifiers system-wide would
//! break Ctrl+C for the whole machine — so essentially every well-behaved tool
//! passes them through, and therefore **every such tool sees every chord**.
//!
//! Two applications bound to the same chord both fire. Nothing errors; the user
//! simply gets two overlays and two microphone streams. That is unfixable from
//! our side, which is why the binding is configuration rather than a constant.
//!
//! ## The constraint that shapes this code
//!
//! The hook procedure runs on the installing thread, **inside the input path of
//! every keystroke on the machine**. Exceed `LowLevelHooksTimeout` (300 ms by
//! default) and Windows silently unhooks us with no error. So the procedure
//! updates two atomics and pushes to a channel. Debouncing, timing and all
//! policy happen on the dispatch side.

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::OnceLock;

use crate::config::{bits, Chord};

/// Raw chord transitions from the hook. Held-duration policy is applied later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordState {
    Engaged,
    Released,
}

static TX: OnceLock<Sender<ChordState>> = OnceLock::new();
/// The active binding, swappable at runtime so Settings can change the shortcut
/// without a restart. Read once per keystroke inside the hook, so it must stay
/// cheap — an uncontended `parking_lot` read lock is a few nanoseconds, and the
/// write side runs only when a human clicks Save.
static CHORD: OnceLock<parking_lot::RwLock<Chord>> = OnceLock::new();

/// Replace the active binding. Takes effect on the very next keystroke.
pub fn set_chord(c: Chord) {
    match CHORD.get() {
        Some(lock) => *lock.write() = c,
        None => {
            let _ = CHORD.set(parking_lot::RwLock::new(c));
        }
    }
    // A stale engagement from the previous binding would otherwise leave the
    // overlay stuck open, since its release transition can no longer fire.
    ENGAGED.store(0, Ordering::SeqCst);
}

/// Label of the active binding, for the interface.
pub fn active_chord_label() -> String {
    CHORD
        .get()
        .map(|l| l.read().label().to_string())
        .unwrap_or_default()
}
static HELD_MODS: AtomicU32 = AtomicU32::new(0);
/// The non-modifier key currently down, or 0. Only one is tracked — a chord
/// never needs two.
static HELD_KEY: AtomicU32 = AtomicU32::new(0);
static ENGAGED: AtomicU32 = AtomicU32::new(0);
/// Every key transition the hook has seen. If this stays at zero while you type,
/// the hook is installed but not receiving — which is a different problem from a
/// chord that never matches.
static HOOK_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Modifier transitions specifically. The decisive number: if this stays at zero
/// while `events` climbs, ordinary keys reach us but modifiers do not — meaning
/// something ahead of us in the hook chain is swallowing them, which no amount
/// of fixing our own code can address.
static MOD_EVENTS: AtomicU64 = AtomicU64::new(0);
/// Transitions carrying LLKHF_INJECTED — synthetic input rather than a real key.
static INJECTED_EVENTS: AtomicU64 = AtomicU64::new(0);

/// Virtual-key code of the most recent key pressed, for diagnostics. Shows what
/// a given physical key actually reports on this machine, which is the only way
/// to settle "why doesn't my shortcut fire".
static LAST_VK: AtomicU32 = AtomicU32::new(0);

/// One raw key transition, for the diagnostic log.
#[derive(Debug, Clone, Copy)]
pub struct KeyTrace {
    pub vk: u32,
    pub down: bool,
    pub mods: u32,
    pub held_key: u32,
    pub matched: bool,
}

static TRACE: OnceLock<Sender<KeyTrace>> = OnceLock::new();
static DIAG: OnceLock<std::path::PathBuf> = OnceLock::new();

/// Append a line to the diagnostic log.
///
/// Startup failures used to vanish: stdout is block-buffered once redirected, so
/// anything logged before the process is killed is simply lost. A hook that
/// fails to install is exactly the failure you most need to see, and it was the
/// one guaranteed to be invisible.
pub fn diag(msg: &str) {
    use std::io::Write;
    if let Some(p) = DIAG.get() {
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(p) {
            let _ = writeln!(f, "[{}] {msg}", chrono::Local::now().format("%H:%M:%S%.3f"));
        }
    }
}

pub fn set_diag_path(p: std::path::PathBuf) {
    if let Some(d) = p.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = DIAG.set(p);
}

fn trace(t: KeyTrace) {
    if let Some(tx) = TRACE.get() {
        let _ = tx.send(t);
    }
}

/// Append every key transition to `keylog.txt`.
///
/// Formatting and file I/O happen on this thread, never in the hook — the hook
/// only sends a `Copy` struct down a channel. A `format!` inside the input path
/// of every keystroke on the machine would be exactly the kind of work that
/// trips `LowLevelHooksTimeout`.
pub fn start_key_log(path: std::path::PathBuf) {
    use std::io::Write;
    let (tx, rx) = channel::<KeyTrace>();
    if TRACE.set(tx).is_err() {
        return;
    }
    std::thread::Builder::new()
        .name("key-log".into())
        .spawn(move || {
            if let Some(d) = path.parent() {
                let _ = std::fs::create_dir_all(d);
            }
            // Append, never truncate. The dev watcher restarts this process on
            // every code change, and a truncating open silently discards the
            // very evidence the log exists to capture.
            let mut f = match std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(&path)
            {
                Ok(f) => f,
                Err(e) => {
                    tracing::warn!(?e, "could not open key log");
                    return;
                }
            };
            let _ = writeln!(
                f,
                "
=== session started {} ===
vk    name            edge  mods      key   match",
                chrono::Local::now().format("%H:%M:%S")
            );
            for t in rx {
                let _ = writeln!(
                    f,
                    "0x{:02X}  {:<14}  {:<4}  {:08b}  0x{:02X}  {}",
                    t.vk,
                    vk_name(t.vk),
                    if t.down { "down" } else { "up" },
                    t.mods,
                    t.held_key,
                    if t.matched { "YES" } else { "-" }
                );
                let _ = f.flush();
            }
        })
        .ok();
}

fn vk_name(vk: u32) -> &'static str {
    match vk {
        0xA0 => "LShift", 0xA1 => "RShift",
        0xA2 => "LCtrl",  0xA3 => "RCtrl",
        0xA4 => "LAlt",   0xA5 => "RAlt",
        0x5B => "LWin",   0x5C => "RWin",
        0x10 => "Shift*", 0x11 => "Ctrl*", 0x12 => "Alt*",
        0x08 => "Backspace", 0x20 => "Space", 0x0D => "Enter",
        0x09 => "Tab", 0x1B => "Esc", 0x14 => "CapsLock",
        _ => "",
    }
}

/// Live hook state, for the diagnostics tile in the Hub.
#[derive(Debug, Clone, Copy, serde::Serialize)]
pub struct HookStats {
    pub events: u64,
    pub mod_events: u64,
    pub injected_events: u64,
    pub held_mods: u32,
    pub held_key: u32,
    pub last_vk: u32,
    pub engaged: bool,
}

pub fn stats() -> HookStats {
    HookStats {
        events: HOOK_EVENTS.load(Ordering::Relaxed),
        mod_events: MOD_EVENTS.load(Ordering::Relaxed),
        injected_events: INJECTED_EVENTS.load(Ordering::Relaxed),
        held_mods: HELD_MODS.load(Ordering::Relaxed),
        held_key: HELD_KEY.load(Ordering::Relaxed),
        last_vk: LAST_VK.load(Ordering::Relaxed),
        engaged: ENGAGED.load(Ordering::Relaxed) == 1,
    }
}

#[cfg(windows)]
mod imp {
    use super::*;
    use windows::Win32::Foundation::{HINSTANCE, LPARAM, LRESULT, WPARAM};
    use windows::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
    use windows::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, SetWindowsHookExW, TranslateMessage,
        UnhookWindowsHookEx, HHOOK, KBDLLHOOKSTRUCT, MSG, WH_KEYBOARD_LL, WM_KEYDOWN, WM_KEYUP,
        WM_SYSKEYDOWN, WM_SYSKEYUP,
    };

    /// Map a virtual-key code to the modifier bits it sets.
    ///
    /// Low-level hooks normally report the side-specific codes (VK_LCONTROL /
    /// VK_RCONTROL), but the generic ones can also arrive — from injected input,
    /// remote desktop, and some keyboard drivers and remappers. Treating a
    /// generic code as an ordinary key would be silently fatal: it would land in
    /// HELD_KEY, so a modifier-only chord could never match and the shortcut
    /// would appear completely dead. Map both, with the generic form counting as
    /// either side.
    fn modifier_bit(vk: u32) -> Option<u32> {
        Some(match vk {
            0xA2 => bits::LCTRL,
            0xA3 => bits::RCTRL,
            0x11 => bits::CTRL, // VK_CONTROL, side unknown
            0xA4 => bits::LALT,
            0xA5 => bits::RALT,
            0x12 => bits::ALT, // VK_MENU
            0xA0 => bits::LSHIFT,
            0xA1 => bits::RSHIFT,
            0x10 => bits::SHIFT,
            0x5B => bits::LWIN,
            0x5C => bits::RWIN,
            _ => return None,
        })
    }

    /// Runs on every keystroke system-wide. Must stay trivial — see module docs.
    unsafe extern "system" fn hook_proc(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
        // This runs inside the input path of every keystroke on the machine.
        // Unwinding across an `extern "system"` boundary is undefined behaviour,
        // and a dead hook thread makes Windows wait out LowLevelHooksTimeout on
        // every key — which degrades typing system-wide, for every application.
        // Swallow any panic and keep the chain intact.
        let _ = std::panic::catch_unwind(|| observe(code, wparam, lparam));
        CallNextHookEx(HHOOK::default(), code, wparam, lparam)
    }

    /// Is this virtual key physically down right now, according to the OS?
    fn key_down(vk: u32) -> bool {
        unsafe { (GetAsyncKeyState(vk as i32) as u16 & 0x8000) != 0 }
    }

    /// Read the modifier state from the OS.
    ///
    /// Accumulating state from hook events alone drifts the moment one event is
    /// missed, and it *will* be missed — keys held while the app starts, a key
    /// released into an elevated window, a remapper synthesising input. A
    /// drifted bit is unrecoverable and makes the chord impossible to satisfy.
    ///
    /// But the OS is not authoritative for the event being processed *right
    /// now*: a low-level hook runs **before** the key state is committed, so
    /// during a key-down callback `GetAsyncKeyState` may still report the key
    /// up, and during key-up it may still report it down. Modifiers never
    /// auto-repeat, so a missed key-down edge gives no second chance — the chord
    /// would simply never engage.
    ///
    /// Hence the split: the OS supplies truth for every *other* key, and the
    /// caller overlays the in-flight event on top.
    fn current_mods() -> u32 {
        let mut m = 0;
        for (vk, bit) in [
            (0xA2, bits::LCTRL),
            (0xA3, bits::RCTRL),
            (0xA4, bits::LALT),
            (0xA5, bits::RALT),
            (0xA0, bits::LSHIFT),
            (0xA1, bits::RSHIFT),
            (0x5B, bits::LWIN),
            (0x5C, bits::RWIN),
        ] {
            if key_down(vk) {
                m |= bit;
            }
        }
        m
    }

    unsafe fn observe(code: i32, wparam: WPARAM, lparam: LPARAM) {
        if code >= 0 {
            HOOK_EVENTS.fetch_add(1, Ordering::Relaxed);
            let kb = &*(lparam.0 as *const KBDLLHOOKSTRUCT);
            let msg = wparam.0 as u32;
            let down = msg == WM_KEYDOWN || msg == WM_SYSKEYDOWN;
            let up = msg == WM_KEYUP || msg == WM_SYSKEYUP;

            if down || up {
                let vk = kb.vkCode;
                if down {
                    LAST_VK.store(vk, Ordering::Relaxed);
                }

                let is_modifier = modifier_bit(vk);
                if is_modifier.is_some() {
                    MOD_EVENTS.fetch_add(1, Ordering::Relaxed);
                }
                // LLKHF_INJECTED — set for synthetic input. Recorded, never
                // filtered on: a remapper's output is legitimate input.
                if kb.flags.0 & 0x10 != 0 {
                    INJECTED_EVENTS.fetch_add(1, Ordering::Relaxed);
                }

                // OS truth for every other key, then this event applied on top —
                // the OS has not committed it yet.
                let mut mods = current_mods();
                if let Some(bit) = is_modifier {
                    if down {
                        mods |= bit;
                    } else {
                        mods &= !bit;
                    }
                }
                HELD_MODS.store(mods, Ordering::SeqCst);

                // Same rule for the non-modifier key. Checking the OS for the key
                // in flight is what wedged this before: on its own key-up the OS
                // still reported it down, so it was never cleared and stayed
                // "held" forever, making every modifier-only chord unsatisfiable.
                let stale = HELD_KEY.load(Ordering::SeqCst);
                if stale != 0 && stale != vk && !key_down(stale) {
                    HELD_KEY.store(0, Ordering::SeqCst);
                }
                if is_modifier.is_none() {
                    if down {
                        HELD_KEY.store(vk, Ordering::SeqCst);
                    } else if stale == vk {
                        HELD_KEY.store(0, Ordering::SeqCst);
                    }
                }

                if let Some(chord_lock) = CHORD.get() {
                    let key = match HELD_KEY.load(Ordering::SeqCst) {
                        0 => None,
                        k => Some(k),
                    };
                    let now_matching = chord_lock.read().matches(mods, key);
                    trace(KeyTrace {
                        vk,
                        down,
                        mods,
                        held_key: key.unwrap_or(0),
                        matched: now_matching,
                    });

                    // compare_exchange rather than a store: key repeat fires
                    // WM_KEYDOWN continuously while a key is held, and we must
                    // emit exactly one transition per chord, not one per repeat.
                    if now_matching {
                        if ENGAGED
                            .compare_exchange(0, 1, Ordering::SeqCst, Ordering::SeqCst)
                            .is_ok()
                        {
                            super::emit(ChordState::Engaged);
                        }
                    } else if ENGAGED
                        .compare_exchange(1, 0, Ordering::SeqCst, Ordering::SeqCst)
                        .is_ok()
                    {
                        super::emit(ChordState::Released);
                    }
                }
            }
        }
    }

    pub fn install() {
        // A low-level hook needs a message pump on its own thread, and that
        // thread must not be the UI thread — blocking it stalls the hook and
        // gets us silently unhooked.
        std::thread::Builder::new()
            .name("hotkey-hook".into())
            .spawn(|| unsafe {
                let module = match GetModuleHandleW(None) {
                    Ok(m) => m,
                    Err(e) => {
                        tracing::error!(?e, "GetModuleHandleW failed; hotkey unavailable");
                        super::diag(&format!("GetModuleHandleW FAILED: {e:?}"));
                        return;
                    }
                };
                let hook = match SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(hook_proc),
                    HINSTANCE(module.0),
                    0,
                ) {
                    Ok(h) => {
                        super::diag("SetWindowsHookExW OK — hook installed");
                        h
                    }
                    Err(e) => {
                        tracing::error!(?e, "SetWindowsHookExW failed; hotkey unavailable");
                        super::diag(&format!("SetWindowsHookExW FAILED: {e:?}"));
                        return;
                    }
                };

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                super::diag("message pump EXITED — hook thread ending");
                let _ = UnhookWindowsHookEx(hook);
            })
            .expect("spawn hotkey thread");
    }
}

#[cfg(not(windows))]
mod imp {
    /// macOS lands at M7 and needs a CGEventTap plus Accessibility permission.
    /// The channel contract is platform-neutral, so only this module changes.
    pub fn install() {
        tracing::warn!("global hotkey not implemented on this platform yet");
    }
}

fn emit(state: ChordState) {
    if let Some(tx) = TX.get() {
        // Send errors are ignored deliberately: if the receiver is gone the app
        // is shutting down, and this runs inside the system input path where
        // panicking would be far worse than dropping an event.
        let _ = tx.send(state);
    }
}

/// Install the hook for `chord` and return the raw transition stream.
pub fn listen(chord: Chord) -> Receiver<ChordState> {
    let (tx, rx) = channel();
    let _ = TX.set(tx);
    tracing::info!(
        chord = chord.label(),
        "keyboard hook installed — hold this chord to capture"
    );
    set_chord(chord);
    imp::install();
    rx
}

/// Report hook health after startup, so a silently dead hook is visible in the
/// diagnostic log rather than only as "nothing happens when I press the key".
pub fn report_health_after(delay: std::time::Duration) {
    std::thread::spawn(move || {
        std::thread::sleep(delay);
        let s = stats();
        diag(&format!(
            "health: events={} mod_events={} injected={} (0 events means the hook is installed but receiving nothing)",
            s.events, s.mod_events, s.injected_events
        ));
    });
}
