# "Save this screenshot" reads the text in a clipboard image

- crates/memos-context/Cargo.toml — WinRT features for clipboard, imaging and OCR
- crates/memos-context/src/windows_impl.rs — `clipboard_image_text()`: clipboard bitmap through Windows.Media.Ocr
- crates/memos-context/src/lib.rs — `image_text` on `Context`, first in `referent()`; `mentions_image()`; non-Windows stub
- crates/memos-context/examples/context_check.rs — show the image text
- crates/memos-agent/src/execute.rs — a save with image text records an `image` source; tests
- apps/desktop/src-tauri/src/transcription.rs — read the clipboard image only for a save that names an image
