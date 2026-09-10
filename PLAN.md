# The document gets its own page, its full width, and a slash menu

- apps/desktop/src/features/notes/Knowledge.tsx — the page: one document, no
  chrome around it.
- apps/desktop/src/features/notes/slash.ts — the `/` trigger and the blocks it
  offers, each one something Markdown can hold.
- apps/desktop/src/features/notes/NoteEditor.tsx — full width, every heading
  level, to-do lists, and the menu's keyboard handling.
- apps/desktop/src/hub/App.tsx — Knowledge in the sidebar; Collections can send
  the reader to a heading.
- apps/desktop/src/features/collections/Collections.tsx — the editor comes out;
  "Read in the file" goes in.
- apps/desktop/src/lib/api.ts, styles/hub.css — `book`/`startBook`, the page.
