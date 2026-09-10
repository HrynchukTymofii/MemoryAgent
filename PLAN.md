# The note gets an editor, and the collection page is built around it

- apps/desktop/src-tauri/src/main.rs — `note`, `start_note`, `save_note`,
  `open_url`.
- crates/memos-db/src/notes.rs — `start_note`, for a document a person begins
  before any capture has.
- apps/desktop/src/features/notes/NoteEditor.tsx — TipTap, Markdown in and out,
  autosave, and a notice rather than a reload when a capture lands mid-sentence.
- apps/desktop/src/features/collections/Collections.tsx — the note above the
  captures, which are demoted to where the note came from.
- apps/desktop/src/lib/api.ts, styles/hub.css, package.json — wrappers, prose
  styles, the editor dependency.
