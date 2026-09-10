-- 005_notes — the document a collection actually is (ADR-0010).
--
-- A capture stays exactly what it was: append-only, undoable, indexed. This
-- adds the thing a person reads — one Markdown file per collection that
-- captures are integrated into, and that they may edit by hand.

CREATE TABLE notes (
    id            TEXT PRIMARY KEY,
    -- Nullable, and that is the point: deleting a collection must not destroy
    -- the document that grew inside it. The note comes loose, as its memories
    -- do. UNIQUE because a collection is a page, not a folder of pages.
    collection_id TEXT UNIQUE REFERENCES collections(id) ON DELETE SET NULL,
    title         TEXT NOT NULL,
    body          TEXT NOT NULL DEFAULT '',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    -- Last edit by a person, as opposed to by a capture. What separates a note
    -- that was written from one that merely accumulated.
    edited_at     TEXT
);

-- Where a capture was integrated. The link the note's own text does not carry
-- yet: provenance in the file is a date and a source, and this is how the
-- record answers "which capture put that paragraph there".
ALTER TABLE knowledge_items ADD COLUMN note_id TEXT REFERENCES notes(id) ON DELETE SET NULL;
CREATE INDEX idx_items_note ON knowledge_items(note_id);
