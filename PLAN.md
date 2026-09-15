# Meeting notes: the PC's sound is not heard as "Me", and Claude writes a summary

- apps/desktop/src-tauri/src/meeting.rs — silence the microphone wherever the PC's sound was playing; tests
- crates/memos-cloud/src/summary.rs, lib.rs — `Cloud::summarize(transcript)`
- apps/desktop/src-tauri/src/transcription.rs — `Stt::cloud()`
- apps/desktop/src-tauri/src/meeting.rs — summarize when recording stops and on request; the summary sits above the transcript in the document; tests
- apps/desktop/src-tauri/src/main.rs — register `summarize_meeting`
- apps/desktop/src/lib/api.ts, src/features/meetings/Meetings.tsx, src/styles/hub.css — the summary, and a Summarize button
