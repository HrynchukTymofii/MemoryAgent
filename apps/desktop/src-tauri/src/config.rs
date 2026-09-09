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

    /// Keep a small pill on screen when nothing is being captured.
    ///
    /// On by default: a shortcut with no visible affordance is a shortcut
    /// people forget they have, and the pill is the only thing telling them the
    /// app is running at all. It is a preference rather than a fixed behaviour
    /// because an always-on-top window is genuinely unwelcome over a game or a
    /// full-screen video, and that is not a judgement to make for someone.
    #[serde(default = "yes")]
    pub idle_pill: bool,
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
            hold_threshold_ms: 120,
            debug_keys: false,
            idle_pill: true,
        }
    }
}

impl Config {
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
}

#[cfg(test)]
mod tests {
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
