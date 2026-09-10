# Ship the models inside the installer, so a fresh install works with no script

- scripts/fetch-models.ps1, .sh — a `bundle` target: fetch tiny.en, the
  embedding model and the ONNX Runtime, stage into src-tauri/resources/
- apps/desktop/src-tauri/tauri.conf.json — bundle that directory
- crates/memos-stt/src/transcribe.rs — find_model also looks in a bundled dir
- crates/memos-embed/src/lib.rs — the same for find_model_dir and
  use_bundled_runtime
- apps/desktop/src-tauri/src/{main,transcription,embedding}.rs — pass Tauri's
  resource_dir into all three
- crates/memos-stt/examples/{transcribe_check,wav_check}.rs — the new argument
- .gitignore — the staged weights
- README.md — first run no longer needs a script
