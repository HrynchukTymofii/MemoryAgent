-- 002_vectors — the semantic half of hybrid retrieval.
--
-- Vectors live in an ordinary table and the nearest-neighbour scan runs in
-- Rust, not in a SQLite extension.
--
-- The brief specified sqlite-vec, and the published crate is broken: its source
-- includes a file its own package omits, so it cannot compile at all. But the
-- substitution costs little, because sqlite-vec brute-forces the scan too — it
-- has no ANN index below a very large corpus. Doing the same scan in Rust drops
-- a C toolchain dependency, drops an extension-registration ordering hazard,
-- and is directly testable.
--
-- What is genuinely given up: fusing keyword and vector results inside one SQL
-- statement. Fusion now happens in Rust over two result sets, which is a
-- handful of microseconds at this size. Revisit when the corpus approaches
-- 100k items, where an ANN index starts to matter.

CREATE TABLE item_vectors (
    item_id    TEXT PRIMARY KEY REFERENCES knowledge_items(id) ON DELETE CASCADE,
    -- Which model produced it, so a model change is detectable and the affected
    -- rows can be re-embedded rather than silently compared across
    -- incompatible vector spaces.
    model      TEXT NOT NULL,
    dim        INTEGER NOT NULL,
    -- Little-endian f32s. Compact and directly reinterpretable.
    vector     BLOB NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_item_vectors_model ON item_vectors(model);
