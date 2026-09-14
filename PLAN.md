# A "Meeting notes" tab in the Hub sidebar

- apps/desktop/src-tauri/src/meeting.rs — commands: status, start, stop, past meetings, read one, open one
- apps/desktop/src-tauri/src/tray.rs — the tray label follows the recorder, wherever it was started
- apps/desktop/src-tauri/src/main.rs — register the commands
- apps/desktop/src/lib/api.ts — the meeting calls
- apps/desktop/src/features/meetings/Meetings.tsx — start/stop, live transcript, past meetings
- apps/desktop/src/hub/App.tsx — "Meeting notes" under Tools
- apps/desktop/src/styles/hub.css — transcript styles
