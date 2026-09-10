# A collection has one note that captures grow, instead of a pile of rows

- docs/adr/0010-notes-are-the-knowledge-base.md — the decision: the note is
  what you read, the capture is its provenance; merges only ever append.
- docs/architecture.md — a section pointing at it.
- crates/memos-db/migrations/005_notes.sql — `notes`, and `note_id` on items.
- crates/memos-db/src/notes.rs — read, save, and `integrate()`: the pure
  heading-merge function, with its tests.
- crates/memos-db/src/{lib,migrate}.rs — register both.
- crates/memos-agent/src/execute.rs — SAVE and NOTE integrate after capturing.
