# A call starting offers to take meeting notes, next to the pill

- apps/desktop/src-tauri/src/detect.rs — which app holds the microphone (CapabilityAccessManager registry); call apps, browsers only with a meeting window; a watcher that reports a call starting and ending; tests
- apps/desktop/src-tauri/Cargo.toml — `Win32_System_Registry`
- apps/desktop/src-tauri/src/meeting.rs — show the prompt under the pill on a call, hide it when the call ends or notes start; accept and dismiss commands
- apps/desktop/src-tauri/src/main.rs — start the watcher, register the commands
- apps/desktop/src-tauri/tauri.conf.json, capabilities/default.json — a hidden `prompt` window
- apps/desktop/prompt.html, apps/desktop/src/prompt.ts, apps/desktop/vite.config.ts — the prompt itself
