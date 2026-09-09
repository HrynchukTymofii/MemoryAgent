# Make the macOS build do the things it currently only pretends to do

- apps/desktop/src-tauri/src/hotkey.rs — CGEventTap `imp`, keycodes translated
  into the Windows VK numbering `Chord` already speaks
- crates/memos-context/src/macos_impl.rs — app, title, selection, URL, clipboard
  via the Accessibility API; `page_text` deliberately left out
- crates/memos-context/src/url.rs — the URL helpers, now shared not Windows-only
- crates/memos-embed/src/onnx.rs — refuse without a runtime instead of panicking
- scripts/fetch-models.sh — models, ONNX Runtime, and `install` into the app
- scripts/build-router.sh — llama.cpp sidecar, into the bundle, re-signed
- crates/memos-core/src/lib.rs — name the script the reader can actually run
