# Meeting summaries on request, automatic only when switched on

- apps/desktop/src-tauri/src/config.rs — `auto_summarize_meetings`, off by default
- apps/desktop/src-tauri/src/main.rs — in `Settings`; `set_auto_summarize` command
- apps/desktop/src-tauri/src/meeting.rs — summarize on stop only when the setting is on
- apps/desktop/src/lib/api.ts, src/features/settings/Settings.tsx — the switch
