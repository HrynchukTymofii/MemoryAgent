//! User configuration, and the chord parser behind the capture shortcut.
//!
//! The shortcut must be user-configurable, and not only as a preference. Other
//! always-on voice tools install their own low-level keyboard hooks, and hooks
//! are a *chain* — every installed hook sees every keystroke. Two apps bound to
//! the same chord both activate, both open an overlay, and both record the
//! microphone. Nothing errors; the result is simply useless. A user hitting that
//! needs to be able to rebind in seconds, without a rebuild.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Bit positions for the modifier keys we track. Left and right are distinct so
/// a chord can bind one side specifically — "right Ctrl held" is a good
/// push-to-talk trigger precisely because nothing else claims it.
pub mod bits {
    pub const LCTRL: u32 = 1 << 0;
    pub const RCTRL: u32 = 1 << 1;
    pub const LALT: u32 = 1 << 2;
    pub const RALT: u32 = 1 << 3;
    pub const LSHIFT: u32 = 1 << 4;
    pub const RSHIFT: u32 = 1 << 5;
    pub const LWIN: u32 = 1 << 6;
    pub const RWIN: u32 = 1 << 7;

    pub const CTRL: u32 = LCTRL | RCTRL;
    pub const ALT: u32 = LALT | RALT;
    pub const SHIFT: u32 = LSHIFT | RSHIFT;
    pub const WIN: u32 = LWIN | RWIN;
}

/// A parsed shortcut: which modifier families must be held, on which side, plus
/// an optional non-modifier key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Chord {
    /// Each entry is a mask; the chord requires at least one bit of each to be
    /// held. `ctrl` becomes `LCTRL|RCTRL`; `rctrl` becomes just `RCTRL`.
    required: Vec<u32>,
    /// Union of the families involved, used to reject supersets.
    families: u32,
    key: Option<u32>,
    label: String,
}

impl Chord {
    /// Parse `"ctrl+win"`, `"rctrl"`, `"ctrl+alt+space"`, or a bare key `"f9"`.
    ///
    /// A bare non-modifier key is a legitimate binding, not a degenerate case.
    /// Modifier keys can be swallowed by another low-level hook earlier in the
    /// chain — which is invisible and unfixable from here — and a dedicated key
    /// such as F9 or a mouse-adjacent key routes around the problem entirely.
    pub fn parse(spec: &str) -> Result<Self, String> {
        let mut required = Vec::new();
        let mut families = 0u32;
        let mut key = None;

        for part in spec.split('+').map(|p| p.trim().to_ascii_lowercase()) {
            let (mask, family) = match part.as_str() {
                "ctrl" | "control" => (bits::CTRL, bits::CTRL),
                "lctrl" => (bits::LCTRL, bits::CTRL),
                "rctrl" => (bits::RCTRL, bits::CTRL),
                "alt" => (bits::ALT, bits::ALT),
                "lalt" => (bits::LALT, bits::ALT),
                "ralt" => (bits::RALT, bits::ALT),
                "shift" => (bits::SHIFT, bits::SHIFT),
                "lshift" => (bits::LSHIFT, bits::SHIFT),
                "rshift" => (bits::RSHIFT, bits::SHIFT),
                "win" | "super" | "meta" => (bits::WIN, bits::WIN),
                "lwin" => (bits::LWIN, bits::WIN),
                "rwin" => (bits::RWIN, bits::WIN),
                other => {
                    if key.is_some() {
                        return Err(format!("more than one non-modifier key in '{spec}'"));
                    }
                    key = Some(parse_key(other)?);
                    continue;
                }
            };
            required.push(mask);
            families |= family;
        }

        if required.is_empty() && key.is_none() {
            return Err(format!("'{spec}' binds nothing"));
        }

        Ok(Chord {
            required,
            families,
            key,
            label: spec.trim().to_string(),
        })
    }

    pub fn label(&self) -> &str {
        &self.label
    }

    /// Whether two bindings are satisfied by exactly the same keys.
    ///
    /// Not `PartialEq`, which compares the spelling the user typed: `win+ctrl`
    /// and `ctrl+win` are two strings for one gesture, and binding both would
    /// make the second one unreachable rather than merely redundant — the hook
    /// checks capture first and stops there.
    pub fn same_gesture_as(&self, other: &Chord) -> bool {
        if self.families != other.families || self.key != other.key {
            return false;
        }
        let mut mine = self.required.clone();
        let mut theirs = other.required.clone();
        mine.sort_unstable();
        theirs.sort_unstable();
        mine == theirs
    }

    /// Whether the chord is satisfied by the currently held keys.
    ///
    /// Modifier families must match **exactly**. Holding Ctrl+Shift+Win does not
    /// satisfy `ctrl+win` — without that rule, a chord fires in the middle of
    /// unrelated shortcuts the user is pressing in some other application.
    pub fn matches(&self, held_mods: u32, held_key: Option<u32>) -> bool {
        let held_families = family_mask(held_mods);
        if held_families != self.families {
            return false;
        }
        if !self.required.iter().all(|m| held_mods & m != 0) {
            return false;
        }
        match self.key {
            Some(k) => held_key == Some(k),
            // A modifier-only chord requires that no ordinary key is down.
            // Without this, `rctrl` fires in the middle of Right-Ctrl+C.
            None => held_key.is_none(),
        }
    }
}

fn family_mask(held: u32) -> u32 {
    let mut m = 0;
    for f in [bits::CTRL, bits::ALT, bits::SHIFT, bits::WIN] {
        if held & f != 0 {
            m |= f;
        }
    }
    m
}

fn parse_key(name: &str) -> Result<u32, String> {
    Ok(match name {
        "space" => 0x20,
        "tab" => 0x09,
        "enter" | "return" => 0x0D,
        "esc" | "escape" => 0x1B,
        "capslock" => 0x14,
        "`" | "backquote" | "grave" => 0xC0,
        s if s.len() == 1 && s.chars().next().unwrap().is_ascii_alphanumeric() => {
            s.chars().next().unwrap().to_ascii_uppercase() as u32
        }
        s if s.starts_with('f') && s[1..].parse::<u32>().is_ok() => {
            let n: u32 = s[1..].parse().unwrap();
            if !(1..=24).contains(&n) {
                return Err(format!("unknown key '{name}'"));
            }
            0x70 + n - 1
        }
        _ => return Err(format!("unknown key '{name}'")),
    })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// e.g. "ctrl+win", "rctrl", "ctrl+alt+space".
    pub hotkey: String,

    /// A second chord that dictates instead of capturing: hold it, speak, and
    /// the transcript is typed into whatever field the caret is already in. No
    /// router, no memory written, nothing leaving the machine.
    ///
    /// `None` by default, and deliberately unbound rather than given a
    /// plausible default. Dictation types into the user's *other* application,
    /// so a chord they did not choose is a chord that one day puts a sentence
    /// into a document they were not dictating into.
    #[serde(default)]
    pub dictate_hotkey: Option<String>,

    /// How long the chord must be held before capture begins.
    ///
    /// Guards against incidental presses — a single-modifier binding like
    /// `rctrl` would otherwise fire during an ordinary Ctrl+C. From M1 this is
    /// genuinely free: the microphone ring buffer is always running, so the
    /// audio spoken during the delay is already recorded and nothing is lost.
    pub hold_threshold_ms: u64,

    /// Append every key transition to `keylog.txt` beside this file. Diagnostic
    /// only — it records everything you type, so it stays off by default.
    #[serde(default)]
    pub debug_keys: bool,

    /// The plan, as the API last reported it.
    ///
    /// Cached rather than fetched on demand, because the meter in the sidebar
    /// is drawn on every render and the app is expected to work with no
    /// network at all. A user who earned a month of Pro and then got on a plane
    /// is still on it; asking the server before drawing a progress bar would
    /// mean they were not.
    ///
    /// Trusted locally. It is a date this machine's own app wrote down, and
    /// anybody willing to edit it could as easily edit the binary that reads
    /// it — what it guards is a free month of a product with no price yet, and
    /// the rules that matter are the ones in the database.
    #[serde(default)]
    pub entitlement: memos_license::Entitlement,

    /// Keep a small pill on screen when nothing is being captured.
    ///
    /// On by default: a shortcut with no visible affordance is a shortcut
    /// people forget they have, and the pill is the only thing telling them the
    /// app is running at all. It is a preference rather than a fixed behaviour
    /// because an always-on-top window is genuinely unwelcome over a game or a
    /// full-screen video, and that is not a judgement to make for someone.
    #[serde(default = "yes")]
    pub idle_pill: bool,

    /// Where the user dragged the resting pill to, in absolute physical screen
    /// coordinates: `pill_x` is the horizontal centre, `pill_y` the edge it is
    /// anchored by. Absolute rather than monitor-relative because the pill is
    /// placed against a physical spot on a physical desk — "the top right of my
    /// second screen" — and a fraction of a monitor stops meaning that as soon
    /// as the layout changes. Out-of-range values are clamped back onto a real
    /// monitor at load, so an unplugged display cannot strand it off-screen.
    #[serde(default)]
    pub pill_x: Option<i32>,
    #[serde(default)]
    pub pill_y: Option<i32>,

    /// Whether that edge is the pill's top rather than its bottom.
    ///
    /// Set when the pill is dragged into the upper part of a display, and it
    /// flips which way its panel opens. A list that grows upward from a pill
    /// near the top edge grows straight off the screen.
    #[serde(default)]
    pub pill_top: bool,

    /// Where sign-in goes, if anywhere.
    ///
    /// Empty in a checked-out build, and an empty provider means the Hub offers
    /// no sign-in at all rather than a button that fails when pressed. Filling
    /// this in is a deployment step, not a code change — see the Account
    /// section of the README.
    #[serde(default)]
    pub auth: memos_auth::Provider,

    /// The key the router uses, if this copy has one.
    ///
    /// In the config file rather than baked in at build time, because it is the
    /// user's own key and their own bill — and in the data directory rather
    /// than the repository, because a key in a checked-out file is a key in
    /// somebody's git history eventually. `ANTHROPIC_API_KEY` in the
    /// environment wins over it, which is what makes a development machine
    /// usable without writing the key to disk at all.
    #[serde(default)]
    pub anthropic_api_key: Option<String>,

    /// Whether the sign-in screen has been shown and answered once.
    ///
    /// Skipping is an answer, and it has to be remembered. An optional account
    /// that asks again on every launch is not optional, it is a nag with a
    /// close button.
    #[serde(default)]
    pub sign_in_prompt_seen: bool,
}

fn yes() -> bool {
    true
}

impl Default for Config {
    fn default() -> Self {
        Self {
            // Matches the documented product default. Users who already run
            // another tool on this chord will need to rebind — which is the
            // entire reason this file exists.
            hotkey: "ctrl+win".into(),
            dictate_hotkey: None,
            hold_threshold_ms: 120,
            debug_keys: false,
            // Free until the API says otherwise. A fresh install that assumed
            // Pro would give it away to anyone who deleted this file.
            entitlement: memos_license::Entitlement::free(),
            idle_pill: true,
            pill_x: None,
            pill_y: None,
            pill_top: false,
            auth: memos_auth::Provider::default(),
            anthropic_api_key: None,
            sign_in_prompt_seen: false,
        }
    }
}

/// `ANTHROPIC_API_KEY` from a `.env` beside the running binary, or above it.
///
/// The same outward walk `build.rs` does, from the executable rather than from
/// the manifest directory — this code has no manifest at run time. An installed
/// copy has no `.env` anywhere near it and simply finds nothing, which is the
/// correct answer there: an installed copy's key belongs in `config.json`.
fn dotenv_key() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    // target/debug/memos-desktop.exe -> target/debug -> target -> <root>, and
    // the working directory too, because `tauri dev` runs the app from
    // `apps/desktop/src-tauri` and a developer's `.env` is above that.
    let from_exe = exe.ancestors().skip(1).take(4).map(PathBuf::from);
    let from_cwd = std::env::current_dir()
        .ok()
        .into_iter()
        .flat_map(|d| d.ancestors().take(4).map(PathBuf::from).collect::<Vec<_>>());

    for dir in from_exe.chain(from_cwd) {
        if let Some(key) = std::fs::read_to_string(dir.join(".env"))
            .ok()
            .and_then(|text| key_from(&text))
        {
            return Some(key);
        }
    }
    None
}

/// `ANTHROPIC_API_KEY` out of the text of a `.env`.
///
/// Its own function so the parsing is testable without a filesystem: a key
/// silently missed because of a quote or a comment is a router that reports
/// itself absent while the user is looking straight at the line that sets it.
fn key_from(text: &str) -> Option<String> {
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        // `split_once` and not `split`: a value may contain '=' even though
        // these keys do not, and quietly truncating a credential is the worst
        // kind of bug to have written here.
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() != "ANTHROPIC_API_KEY" {
            continue;
        }
        // An inline comment is not part of the value, and neither are the
        // quotes a shell would have stripped.
        let value = value.split('#').next().unwrap_or("").trim();
        let value = value.trim_matches('"').trim_matches('\'').trim();
        if !value.is_empty() {
            return Some(value.to_string());
        }
    }
    None
}

impl Config {
    /// The key to route with: the environment, then a development `.env`, then
    /// this file.
    ///
    /// The middle one exists because `.env` at the repository root is where a
    /// developer will put it — `build.rs` reads that file, so it is already
    /// "the file you put credentials in" for this project. But `build.rs` runs
    /// at *build* time and bakes its values in with `env!`, which is right for
    /// a client id that is the same for every copy and wrong for a key that
    /// bills somebody. So this reads it again, here, at run time, and the key
    /// never enters the binary.
    ///
    /// Returning `None` is a supported state and not an error: the app runs
    /// without a router, on the grammar, exactly as it did before there was
    /// one — and the Hub says so rather than leaving the user to discover it
    /// one unusual phrasing at a time.
    pub fn api_key(&self) -> Option<String> {
        std::env::var("ANTHROPIC_API_KEY")
            .ok()
            .or_else(dotenv_key)
            .or_else(|| self.anthropic_api_key.clone())
            .map(|k| k.trim().to_string())
            .filter(|k| !k.is_empty())
    }

    pub fn path() -> PathBuf {
        crate::data_dir().join("config.json")
    }

    /// Load, writing defaults on first run. Never fails: a malformed config
    /// falls back to defaults with a warning rather than preventing startup.
    pub fn load() -> Self {
        let path = Self::path();
        match std::fs::read_to_string(&path) {
            Ok(raw) => {
                // Strip a UTF-8 BOM. Notepad and PowerShell's `Set-Content
                // -Encoding utf8` both write one, serde_json rejects it, and the
                // failure is invisible: the app silently reverts to defaults, so
                // an edited shortcut never takes effect and the user has no way
                // to tell why. Costs one line; saves an unfalsifiable bug report.
                let text = raw.trim_start_matches('\u{feff}');
                match serde_json::from_str::<Config>(text) {
                    Ok(c) => c,
                    Err(e) => {
                        // Loud, and to a file: the stdout warning is
                        // block-buffered and lost when the process is killed
                        // rather than exiting cleanly.
                        let msg = format!(
                            "config.json is INVALID ({e}) - falling back to defaults, \
                             your settings are being IGNORED"
                        );
                        tracing::error!("{msg}");
                        crate::hotkey::diag(&msg);
                        Config::default()
                    }
                }
            }
            Err(_) => {
                let c = Config::default();
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let _ = std::fs::write(
                    &path,
                    serde_json::to_string_pretty(&c).unwrap_or_default(),
                );
                tracing::info!(path = %path.display(), "wrote default config");
                c
            }
        }
    }

    /// Persist, without a BOM and pretty-printed so it stays hand-editable.
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, json).map_err(|e| e.to_string())
    }

    pub fn chord(&self) -> Chord {
        Chord::parse(&self.hotkey).unwrap_or_else(|e| {
            tracing::warn!("{e}; falling back to ctrl+win");
            Chord::parse("ctrl+win").expect("fallback chord is valid")
        })
    }

    /// The dictation chord, if one is bound and parses.
    ///
    /// A bad spec falls back to *unbound*, not to a default the way `chord`
    /// does. The capture shortcut must exist or the app is unreachable; this
    /// one is an extra, and silently binding something the user did not write
    /// would be worse than leaving it off.
    pub fn dictate_chord(&self) -> Option<Chord> {
        let spec = self.dictate_hotkey.as_deref()?;
        match Chord::parse(spec) {
            Ok(c) => Some(c),
            Err(e) => {
                tracing::warn!("{e}; dictation shortcut left unbound");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    /// The line the user actually edits, in every shape a `.env` takes.
    #[test]
    fn the_key_is_read_out_of_a_dotenv() {
        use super::key_from;
        assert_eq!(key_from("ANTHROPIC_API_KEY=sk-ant-abc").as_deref(), Some("sk-ant-abc"));
        assert_eq!(key_from("  ANTHROPIC_API_KEY = sk-ant-abc  ").as_deref(), Some("sk-ant-abc"));
        assert_eq!(key_from("ANTHROPIC_API_KEY=\"sk-ant-abc\"").as_deref(), Some("sk-ant-abc"));
        assert_eq!(
            key_from("MEMOS_API_URL=
ANTHROPIC_API_KEY=sk-ant-abc  # mine
").as_deref(),
            Some("sk-ant-abc")
        );
        // The commented-out example line in .env.example must not read as a key.
        assert_eq!(key_from("# ANTHROPIC_API_KEY=sk-ant-abc"), None);
        // And neither must the empty one it ships with, or the app would think
        // it had a key and fail every command instead of saying it has none.
        assert_eq!(key_from("ANTHROPIC_API_KEY="), None);
        assert_eq!(key_from("ANTHROPIC_API_KEY=   "), None);
    }

    use super::*;

    #[test]
    fn parses_modifier_only_chord() {
        let c = Chord::parse("ctrl+win").unwrap();
        assert!(c.matches(bits::LCTRL | bits::LWIN, None));
        assert!(c.matches(bits::RCTRL | bits::RWIN, None));
        assert!(!c.matches(bits::LCTRL, None));
    }

    #[test]
    fn rejects_supersets() {
        // Holding Ctrl+Shift+Win while using some other app's shortcut must not
        // trigger a capture.
        let c = Chord::parse("ctrl+win").unwrap();
        assert!(!c.matches(bits::LCTRL | bits::LWIN | bits::LSHIFT, None));
    }

    #[test]
    fn side_specific_binding() {
        let c = Chord::parse("rctrl").unwrap();
        assert!(c.matches(bits::RCTRL, None));
        assert!(!c.matches(bits::LCTRL, None), "left Ctrl must not fire rctrl");
    }

    #[test]
    fn chord_with_key() {
        let c = Chord::parse("ctrl+alt+space").unwrap();
        assert!(c.matches(bits::LCTRL | bits::LALT, Some(0x20)));
        assert!(!c.matches(bits::LCTRL | bits::LALT, None));
        assert!(!c.matches(bits::LCTRL | bits::LALT, Some(0x41)));
    }

    #[test]
    fn a_held_key_blocks_a_modifier_only_chord() {
        // Correct behaviour — `rctrl` must not fire during Ctrl+C. It is also
        // why the hook has to clear `held_key` reliably: one phantom stuck key
        // makes this chord permanently unsatisfiable, and the shortcut dies
        // silently with nothing logged.
        let c = Chord::parse("rctrl").unwrap();
        assert!(c.matches(bits::RCTRL, None));
        assert!(!c.matches(bits::RCTRL, Some(0x08)), "stale Backspace must block");
        assert!(!c.matches(bits::RCTRL, Some(0x43)), "Ctrl+C must not fire it");
    }

    #[test]
    fn one_gesture_is_recognised_through_two_spellings() {
        // The dictation binding is refused when it collides with capture, and
        // the collision that matters is of gestures, not of strings: the hook
        // checks capture first, so the second binding would be dead rather than
        // duplicated.
        let a = Chord::parse("ctrl+win").unwrap();
        assert!(a.same_gesture_as(&Chord::parse("win+ctrl").unwrap()));
        assert!(a.same_gesture_as(&Chord::parse("  CTRL + WIN ").unwrap()));
        assert!(!a.same_gesture_as(&Chord::parse("ctrl+shift").unwrap()));
        // Same modifiers, different key — two usable bindings, not one.
        let b = Chord::parse("ctrl+alt+space").unwrap();
        assert!(!b.same_gesture_as(&Chord::parse("ctrl+alt+z").unwrap()));
        // A side-specific chord is not the loose one it is a subset of.
        assert!(!Chord::parse("rctrl")
            .unwrap()
            .same_gesture_as(&Chord::parse("ctrl").unwrap()));
    }

    #[test]
    fn a_dictation_binding_is_optional_and_never_guessed() {
        // A bad spec unbinds dictation rather than falling back to something,
        // which is the opposite of what `chord` does for capture: an app with no
        // capture shortcut is unreachable, an app with no dictation shortcut is
        // just an app without dictation.
        let mut cfg = Config::default();
        assert!(cfg.dictate_chord().is_none(), "off until asked for");
        cfg.dictate_hotkey = Some("shift+z".into());
        assert_eq!(cfg.dictate_chord().unwrap().label(), "shift+z");
        cfg.dictate_hotkey = Some("ctrl+nonsense".into());
        assert!(cfg.dictate_chord().is_none(), "an unparseable spec binds nothing");
        // And capture still has its fallback, unaffected.
        assert_eq!(cfg.chord().label(), "ctrl+win");
    }

    #[test]
    fn bare_key_chord_is_allowed() {
        // Needed when another hook is eating modifiers: a plain key still gets
        // through, so the shortcut can route around the interference.
        let c = Chord::parse("f9").unwrap();
        assert!(c.matches(0, Some(0x78)), "F9 alone should engage");
        assert!(!c.matches(0, None));
        assert!(!c.matches(bits::LCTRL, Some(0x78)), "Ctrl+F9 is a different chord");
    }

    #[test]
    fn rejects_empty_spec() {
        assert!(Chord::parse("").is_err());
    }

    #[test]
    fn unknown_key_is_an_error() {
        assert!(Chord::parse("ctrl+nonsense").is_err());
    }
}
