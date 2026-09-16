# Start at Windows login, hidden to the tray

- apps/desktop/src-tauri/Cargo.toml — add `tauri-plugin-autostart`
- apps/desktop/src-tauri/src/config.rs — `start_at_login`, default off
- apps/desktop/src-tauri/src/main.rs — register the plugin, reconcile the
  registry to the config at startup, honour `--hidden`, `set_start_at_login`
- apps/desktop/src-tauri/tauri.conf.json — main window created hidden
- apps/desktop/src/lib/api.ts — `start_at_login` on `Settings`, `setStartAtLogin`
- apps/desktop/src/features/settings/Settings.tsx — the toggle
