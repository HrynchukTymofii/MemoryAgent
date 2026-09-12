# A second shortcut that dictates: hold, speak, the text lands at the cursor

- apps/desktop/src-tauri/src/config.rs — `dictate_hotkey: Option<String>`, `dictate_chord()`
- apps/desktop/src-tauri/src/hotkey.rs — a second bindable chord; transitions name which one fired
- apps/desktop/src-tauri/src/inject.rs — new; type a transcript into the focused window (SendInput)
- apps/desktop/src-tauri/src/transcription.rs — `Mode` on the job and the result; dictation skips the router
- apps/desktop/src-tauri/src/main.rs — dispatch both chords; `set_dictate_hotkey`; inject on result
- apps/desktop/src/lib/api.ts — `dictate_hotkey` on Settings, `setDictateHotkey`
- apps/desktop/src/features/settings/Settings.tsx — `Shortcut` takes a binding; a second row for dictation
