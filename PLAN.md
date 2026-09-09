# macOS prep (compiles on Windows, port finishes on the Mac)

Put the Windows-only code behind traits so only one file is missing on macOS.

- `crates/memos-context/src/lib.rs` — platform trait, pick impl by cfg
- `crates/memos-context/src/macos_impl.rs` — new: stub returning empty context
- `apps/desktop/src-tauri/src/hotkey.rs` — split: shared state vs Windows hook
- `apps/desktop/src-tauri/src/hotkey_macos.rs` — new: stub, no-op hook
- `apps/desktop/src-tauri/tauri.conf.json` — macOS bundle + entitlements
- `docs/adr/0009-macos-port.md` — what is left to write on the Mac
