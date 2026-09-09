-- 003_actions — undo, and the intents that act on a memory that already exists.
--
-- Nothing here is new storage. `events` has carried an `inverse` for every
-- capture since 001; `tasks`, `tags` and `knowledge_tags` have been sitting
-- empty for as long. What was missing was the two columns that make an event
-- undoable *once* and attributable to the command that caused it.

-- Undo has to be idempotent. Without this, saying "undo" twice would find the
-- same event and try to delete an item that is already gone — and worse, a
-- second "undo" would look like it worked.
ALTER TABLE events ADD COLUMN undone_at TEXT;

-- Which command produced this event. An undone capture is the clearest verdict
-- a user can give (ADR-0005), and this is the link that lets it be recorded
-- against the right row in the correction log.
--
-- No foreign key on purpose: `events` is an append-only audit and must survive
-- the user erasing their command log (ADR-0006). A dangling id here resolves to
-- "no verdict to record", which is the correct outcome.
ALTER TABLE events ADD COLUMN command_id TEXT;

-- The lookup undo performs, and the only one: the newest event that can still
-- be reversed.
CREATE INDEX idx_events_undoable ON events(created_at DESC)
    WHERE inverse IS NOT NULL AND undone_at IS NULL;

-- Tasks are listed newest-first in the Hub. `idx_tasks_due` covers the open-by-
-- deadline query; this covers the one the user actually looks at.
CREATE INDEX idx_tasks_created ON tasks(created_at DESC);
