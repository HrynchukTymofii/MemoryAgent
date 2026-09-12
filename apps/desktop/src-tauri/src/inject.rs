//! Typing a transcript into whatever application already has the caret.
//!
//! The last step of dictation mode: the words go where the user was already
//! working, and nowhere else. Nothing is stored and nothing is routed, and —
//! unlike every hosted dictation tool — nothing is counted.
//!
//! What arrives here has already been through the shaping pass on our own API,
//! so it carries the speaker's punctuation and layout rather than whisper's
//! flat run of words. That pass is the one part of dictation that leaves this
//! process; everything below this line is local.
//!
//! ## Why synthesised keystrokes rather than the clipboard
//!
//! Pasting is the obvious trick and it is faster for long text, but it destroys
//! whatever the user had copied, and this codebase already treats the clipboard
//! as something that "frequently holds passwords" (see `ContextPermissions`).
//! Borrowing it to deliver a sentence is not a trade worth making for the
//! milliseconds it saves.
//!
//! ## Why the overlay never takes focus
//!
//! It already does not — `set_focus` is deliberately not called on the capture
//! window — and dictation is the feature that makes that decision load-bearing.
//! Synthesised input goes to the focused window, so the target is still the
//! editor the user was typing in when they pressed the chord.

/// Type `text` into the focused window.
///
/// Best-effort by nature: there is no acknowledgement from the other side, and
/// an application that ignores synthesised input will ignore this too. Returns
/// the number of characters handed to the OS, or an error worth logging.
pub fn type_text(text: &str) -> Result<usize, String> {
    if text.is_empty() {
        return Ok(0);
    }
    imp::type_text(text)
}

#[cfg(windows)]
mod imp {
    use windows::Win32::UI::Input::KeyboardAndMouse::{
        SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS,
        KEYEVENTF_KEYUP, KEYEVENTF_UNICODE, VIRTUAL_KEY, VK_RETURN,
    };

    /// Characters per `SendInput` call.
    ///
    /// One call for the whole transcript would be simpler, but `SendInput` is
    /// atomic against other threads' input: a very long block holds the input
    /// queue and makes the machine feel stalled. A few hundred characters is
    /// imperceptible and bounds that.
    const CHUNK: usize = 256;

    pub fn type_text(text: &str) -> Result<usize, String> {
        // UTF-16 rather than chars: `KEYEVENTF_UNICODE` carries one code unit
        // per event, and a surrogate pair is delivered as its two halves in
        // order. Iterating by `char` would silently drop anything outside the
        // BMP — rare in dictation, but wrong for free.
        let units: Vec<u16> = text.encode_utf16().collect();
        let mut inputs: Vec<INPUT> = Vec::with_capacity(CHUNK * 2);
        let mut sent = 0usize;

        for chunk in units.chunks(CHUNK) {
            inputs.clear();
            for &unit in chunk {
                // A newline arrives as U+000A, which as a *character* is not
                // what any text field does with Enter. Send the key instead.
                if unit == b'\n' as u16 {
                    inputs.push(key(VK_RETURN, 0, KEYBD_EVENT_FLAGS(0)));
                    inputs.push(key(VK_RETURN, 0, KEYEVENTF_KEYUP));
                } else if unit == b'\r' as u16 {
                    // Half of a CRLF: the LF beside it already sent the key.
                    continue;
                } else {
                    inputs.push(key(VIRTUAL_KEY(0), unit, KEYEVENTF_UNICODE));
                    inputs.push(key(
                        VIRTUAL_KEY(0),
                        unit,
                        KEYEVENTF_UNICODE | KEYEVENTF_KEYUP,
                    ));
                }
            }
            if inputs.is_empty() {
                continue;
            }

            let n = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
            if n as usize != inputs.len() {
                // Blocked partway rather than never started. The usual cause is
                // UIPI: an elevated window will not accept input from a process
                // running at a lower integrity level, and there is nothing this
                // side can do about it except say so.
                return Err(format!(
                    "SendInput delivered {n} of {} events — the focused window is \
                     probably running elevated",
                    inputs.len()
                ));
            }
            sent += chunk.len();
        }

        Ok(sent)
    }

    fn key(vk: VIRTUAL_KEY, scan: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use core_graphics::event::{CGEvent, CGEventTapLocation};
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};

    /// Code units per event.
    ///
    /// `CGEventKeyboardSetUnicodeString` takes a whole string, but long ones are
    /// unreliable in practice — some applications read only the first few units
    /// of an event. Small groups are what every tool doing this settles on.
    const CHUNK: usize = 16;

    pub fn type_text(text: &str) -> Result<usize, String> {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
            .map_err(|_| "could not create an event source".to_string())?;

        let units: Vec<u16> = text.encode_utf16().collect();
        for chunk in units.chunks(CHUNK) {
            // A keyboard event with no key: the virtual-key code is ignored once
            // a unicode string is attached, and the string is what gets typed.
            let down = CGEvent::new_keyboard_event(source.clone(), 0, true)
                .map_err(|_| "could not create a keyboard event".to_string())?;
            down.set_string_from_utf16_unchecked(chunk);
            down.post(CGEventTapLocation::HID);

            let up = CGEvent::new_keyboard_event(source.clone(), 0, false)
                .map_err(|_| "could not create a keyboard event".to_string())?;
            up.set_string_from_utf16_unchecked(chunk);
            up.post(CGEventTapLocation::HID);
        }

        Ok(units.len())
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
mod imp {
    pub fn type_text(_text: &str) -> Result<usize, String> {
        Err("dictation is not implemented on this platform".into())
    }
}
