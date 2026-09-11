# A document belongs to a leaf collection; parents are just the way there

- crates/memos-db/migrations/007_per_collection.sql — the global document goes
  if it is empty; kept, unattached, if somebody had written in it.
- crates/memos-db/src/notes.rs — back to `note_for_path` / `start_note` /
  `integrate_capture(collection, …)`; `integrate` matches the capture's title
  against the file's own `##` headings again.
- crates/memos-agent/src/execute.rs — the heading is the capture's title.
- apps/desktop/src-tauri/src/main.rs — `note(path)` / `start_note(path)`.
- apps/desktop/src/features/notes/{Knowledge.tsx → deleted, NoteEditor.tsx}
- apps/desktop/src/features/collections/Collections.tsx — the editor shows on a
  leaf and nowhere else; apps/desktop/src/hub/App.tsx drops the page.
