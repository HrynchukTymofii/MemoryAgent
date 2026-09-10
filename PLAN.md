# One document for everything, with the collection path as its outline

- crates/memos-db/migrations/006_one_book.sql — the per-collection notes fold
  into a single document; items repoint at it.
- crates/memos-db/src/notes.rs — `integrate` takes a heading trail and walks or
  creates `#`/`##`/`###` down to it; `book()` / `ensure_book()` replace the
  per-collection lookups.
- crates/memos-agent/src/execute.rs — a capture passes its collection path as
  that trail.
- apps/desktop/src-tauri/src/main.rs — `book`, `start_book`, `save_note`.
