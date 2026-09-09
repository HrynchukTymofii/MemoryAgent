# Get the macOS bundle building and installed on this Mac

- (toolchain) — install rustup + stable, add `aarch64-apple-darwin`; `brew install cmake` for whisper.cpp
- rust-toolchain.toml — add the two Apple targets alongside the Windows one
- apps/desktop/src-tauri/tauri.conf.json — bundle targets per platform; add `icon.icns`
- apps/desktop/src-tauri/icons/icon.icns — generate from `icon.png`
- apps/desktop/src-tauri/src/main.rs — `data_dir()` was `%APPDATA%` or `.`; give it `~/Library/Application Support`
- scripts/build-macos.sh — allow an unsigned/development build when no Developer ID is in the keychain
- (verify) — `cargo build`, then `npm run tauri build`, install the `.app`, launch it, check the log
