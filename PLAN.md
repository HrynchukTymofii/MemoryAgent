# Meeting notes: the microphone and the PC's sound, transcribed into a document

- crates/memos-stt/src/capture.rs — `AudioCapture::loopback()`: the default output device; the stream stops when the capture is dropped
- apps/desktop/src-tauri/src/transcription.rs — `Stt::model()` so the recorder shares the loaded whisper
- apps/desktop/src-tauri/src/meeting.rs — recorder: cut each source at pauses, transcribe, keep "Me"/"Them" lines in time order in Documents/Meetings/<date>.md; open it on stop; tests
- apps/desktop/src-tauri/src/tray.rs — "Start meeting notes" / "Stop meeting notes"
- apps/desktop/src-tauri/src/main.rs — the recorder in `AppState`
