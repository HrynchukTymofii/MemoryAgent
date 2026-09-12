# Dictation goes through the router, so a spoken list arrives as a list

- crates/memos-cloud/src/lib.rs — `Cloud::shape`: one no-tool call that formats a raw transcript
- apps/desktop/src-tauri/src/transcription.rs — dictation shapes before it returns; any failure keeps the raw words
- apps/desktop/src/features/settings/Settings.tsx — the blurb no longer claims dictation stays local
- apps/desktop/src/features/help/Shortcuts.tsx — same correction
- apps/desktop/src-tauri/src/inject.rs — module doc: the transcript is shaped remotely first
