# Dictation is shaped by our own server, not by Anthropic

- cloud/app/shape.py — new; load Qwen3-0.6B through llama-cpp-python, format a transcript
- cloud/app/main.py — `POST /v1/dictation/shape`
- cloud/requirements.txt — llama-cpp-python
- cloud/tests/test_shape.py — prompt and reply handling, no model needed
- crates/memos-auth/src/backend.rs — `shape()` against MEMOS_API_URL
- crates/memos-cloud/src/lib.rs — drop `Cloud::shape`; dictation no longer goes to Anthropic
- crates/memos-cloud/tests/conversation.rs — drop its two tests
- apps/desktop/src-tauri/src/transcription.rs — shape through the backend
- apps/desktop/src-tauri/src/main.rs — hand the worker the configured backend
