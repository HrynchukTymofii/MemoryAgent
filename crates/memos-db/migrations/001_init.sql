-- 001_init — the local store.
--
-- SQLite is the client's source of truth (ADR-0001). Postgres exists in the
-- cloud tier with the same shape, but a capture must succeed offline and
-- instantly, so nothing here may depend on a network.

------------------------------------------------------------------- structure

CREATE TABLE collections (
    id          TEXT PRIMARY KEY,
    parent_id   TEXT REFERENCES collections(id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    -- Materialised path. Denormalised because the router grammar needs every
    -- valid destination on every command; a recursive CTE on the hot path
    -- would be wasted work.
    path        TEXT NOT NULL UNIQUE,
    sort_order  INTEGER NOT NULL DEFAULT 0,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_collections_parent ON collections(parent_id);

CREATE TABLE sources (
    id           TEXT PRIMARY KEY,
    kind         TEXT NOT NULL,
    url          TEXT,
    domain       TEXT,
    file_path    TEXT,
    title        TEXT,
    retrieved_at TEXT NOT NULL
);
CREATE INDEX idx_sources_domain ON sources(domain);
CREATE INDEX idx_sources_url    ON sources(url);

CREATE TABLE knowledge_items (
    id               TEXT PRIMARY KEY,
    title            TEXT NOT NULL,
    content          TEXT NOT NULL,
    summary          TEXT,
    collection_id    TEXT REFERENCES collections(id) ON DELETE SET NULL,
    source_id        TEXT REFERENCES sources(id)     ON DELETE SET NULL,
    captured_at      TEXT NOT NULL,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    last_accessed_at TEXT,
    access_count     INTEGER NOT NULL DEFAULT 0,
    metadata         TEXT NOT NULL DEFAULT '{}',
    sync_state       TEXT NOT NULL DEFAULT 'pending'
);
CREATE INDEX idx_items_collection ON knowledge_items(collection_id);
CREATE INDEX idx_items_captured   ON knowledge_items(captured_at DESC);
CREATE INDEX idx_items_sync       ON knowledge_items(sync_state) WHERE sync_state <> 'synced';

CREATE TABLE tags (
    id     TEXT PRIMARY KEY,
    name   TEXT NOT NULL UNIQUE,
    colour TEXT
);

-- Many-to-many on purpose: tagging must never pull an item out of its
-- collection. Nothing gets trapped in one folder (spec §9).
CREATE TABLE knowledge_tags (
    item_id TEXT NOT NULL REFERENCES knowledge_items(id) ON DELETE CASCADE,
    tag_id  TEXT NOT NULL REFERENCES tags(id)            ON DELETE CASCADE,
    PRIMARY KEY (item_id, tag_id)
);
CREATE INDEX idx_knowledge_tags_tag ON knowledge_tags(tag_id);

CREATE TABLE relationships (
    from_id    TEXT NOT NULL REFERENCES knowledge_items(id) ON DELETE CASCADE,
    to_id      TEXT NOT NULL REFERENCES knowledge_items(id) ON DELETE CASCADE,
    kind       TEXT NOT NULL,
    weight     REAL NOT NULL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    PRIMARY KEY (from_id, to_id, kind)
);
CREATE INDEX idx_rel_to ON relationships(to_id);

------------------------------------------------------------------- actions

CREATE TABLE tasks (
    id         TEXT PRIMARY KEY,
    item_id    TEXT REFERENCES knowledge_items(id) ON DELETE SET NULL,
    title      TEXT NOT NULL,
    due_at     TEXT,
    status     TEXT NOT NULL DEFAULT 'open',
    priority   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_tasks_due ON tasks(due_at) WHERE status = 'open';

CREATE TABLE reminders (
    id         TEXT PRIMARY KEY,
    item_id    TEXT REFERENCES knowledge_items(id) ON DELETE CASCADE,
    fire_at    TEXT NOT NULL,
    recurrence TEXT,
    status     TEXT NOT NULL DEFAULT 'pending',
    created_at TEXT NOT NULL
);
CREATE INDEX idx_reminders_fire ON reminders(fire_at) WHERE status = 'pending';

------------------------------------------------- the personalization asset

-- Every command, whatever tier handled it. This is the most valuable table in
-- the system (ADR-0006): it is the few-shot corpus, the router evaluation set,
-- the confidence calibration data, and the Tier 0 coverage metric.
CREATE TABLE commands (
    id          TEXT PRIMARY KEY,
    transcript  TEXT NOT NULL,
    context     TEXT NOT NULL DEFAULT '{}',
    intent      TEXT NOT NULL,
    slots       TEXT NOT NULL DEFAULT '{}',
    tier        TEXT NOT NULL,
    conf_score  REAL,
    conf_margin REAL,
    latency_ms  INTEGER,
    created_at  TEXT NOT NULL
);
CREATE INDEX idx_commands_created ON commands(created_at DESC);
CREATE INDEX idx_commands_tier    ON commands(tier);

CREATE TABLE corrections (
    command_id       TEXT PRIMARY KEY REFERENCES commands(id) ON DELETE CASCADE,
    corrected_intent TEXT,
    corrected_slots  TEXT NOT NULL DEFAULT '{}',
    accepted         INTEGER NOT NULL,
    corrected_at     TEXT NOT NULL
);

---------------------------------------------------------------- machinery

-- Replaces Redis on the client (ADR-0001): background jobs, drained by Tokio
-- workers rather than a separate service the user would have to install.
CREATE TABLE jobs (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    run_after  TEXT NOT NULL,
    attempts   INTEGER NOT NULL DEFAULT 0,
    state      TEXT NOT NULL DEFAULT 'queued',
    last_error TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_jobs_ready ON jobs(run_after) WHERE state = 'queued';

-- Written in the same transaction as the capture it describes, which is what
-- makes "acknowledged in under 1 s, offline-safe" true (§11).
CREATE TABLE outbox (
    id         TEXT PRIMARY KEY,
    entity     TEXT NOT NULL,
    entity_id  TEXT NOT NULL,
    op         TEXT NOT NULL,
    payload    TEXT NOT NULL DEFAULT '{}',
    attempts   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_outbox_created ON outbox(created_at);

-- Append-only audit. Every executed action records its inverse here, which is
-- what makes undo cheap — and cheap undo is what lets the confidence
-- thresholds be aggressive (ADR-0005).
CREATE TABLE events (
    id         TEXT PRIMARY KEY,
    kind       TEXT NOT NULL,
    entity_id  TEXT,
    payload    TEXT NOT NULL DEFAULT '{}',
    inverse    TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_events_created ON events(created_at DESC);

-- Local capture counter. Increments locally so capture still succeeds offline
-- and reconciles on sync; slight over-run is the correct way to be wrong.
CREATE TABLE usage (
    week_start TEXT PRIMARY KEY,
    captures   INTEGER NOT NULL DEFAULT 0,
    synced_at  TEXT
);

CREATE TABLE meta (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

---------------------------------------------------------- keyword retrieval

-- FTS5 gives BM25. sqlite-vec supplies the vector half at M2; the two are
-- fused with reciprocal rank fusion in a single in-process query (§7).
CREATE VIRTUAL TABLE items_fts USING fts5(
    title,
    content,
    summary,
    content='knowledge_items',
    content_rowid='rowid',
    tokenize='unicode61 remove_diacritics 2'
);

CREATE TRIGGER items_fts_ai AFTER INSERT ON knowledge_items BEGIN
    INSERT INTO items_fts(rowid, title, content, summary)
    VALUES (new.rowid, new.title, new.content, new.summary);
END;

CREATE TRIGGER items_fts_ad AFTER DELETE ON knowledge_items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, content, summary)
    VALUES ('delete', old.rowid, old.title, old.content, old.summary);
END;

CREATE TRIGGER items_fts_au AFTER UPDATE ON knowledge_items BEGIN
    INSERT INTO items_fts(items_fts, rowid, title, content, summary)
    VALUES ('delete', old.rowid, old.title, old.content, old.summary);
    INSERT INTO items_fts(rowid, title, content, summary)
    VALUES (new.rowid, new.title, new.content, new.summary);
END;
