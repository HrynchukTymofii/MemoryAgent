//! Vector storage and nearest-neighbour search.
//!
//! The scan is a brute-force cosine over every stored vector, held in memory.
//! At 384 dimensions and 10k items that is 3.8M floats — about 15 MB and a few
//! milliseconds to sweep. sqlite-vec would do the same brute-force scan; it
//! only starts to win once the corpus is large enough to want an ANN index,
//! which the brief puts around 100k items.
//!
//! The index is loaded once and updated on write, so a search never reads from
//! disk. SQLite remains the durable store; this is a cache with a lifetime,
//! not a second source of truth.

use memos_core::Id;
use parking_lot::RwLock;
use rusqlite::params;

use crate::{Db, DbResult};

/// A vector search hit.
#[derive(Debug, Clone, PartialEq)]
pub struct VectorHit {
    pub item_id: Id,
    /// Cosine similarity, -1..1. Vectors are unit length, so this is a dot
    /// product.
    pub similarity: f32,
}

/// In-memory copy of every stored vector.
#[derive(Default)]
pub struct VectorIndex {
    entries: RwLock<Vec<(Id, Vec<f32>)>>,
    loaded: RwLock<bool>,
}

fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|f| f.to_le_bytes()).collect()
}

fn from_bytes(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]]))
        .collect()
}

/// Dot product of two equal-length vectors.
///
/// Both sides are unit length by construction (the embedder normalises), so
/// this *is* cosine similarity. Length mismatch returns 0 rather than panicking:
/// a mid-database model change must degrade ranking, not crash search.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() {
        return 0.0;
    }
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

impl Db {
    /// Store an item's embedding, replacing any previous one.
    pub fn put_embedding(&self, item: Id, model: &str, vector: &[f32]) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO item_vectors (item_id, model, dim, vector, created_at)
                 VALUES (?1,?2,?3,?4,?5)
                 ON CONFLICT(item_id) DO UPDATE SET
                     model=excluded.model, dim=excluded.dim,
                     vector=excluded.vector, created_at=excluded.created_at",
                params![
                    item.to_string(),
                    model,
                    vector.len() as i64,
                    to_bytes(vector),
                    memos_core::now().to_rfc3339()
                ],
            )?;
            Ok(())
        })?;
        self.index().upsert(item, vector.to_vec());
        Ok(())
    }

    /// Nearest neighbours to a query vector, best first.
    ///
    /// `min_similarity` is not an optimisation. Every corpus has a nearest
    /// neighbour, so an unfiltered kNN scan returns the whole library ranked by
    /// how *least unrelated* each item is — and a caller that fuses rankings
    /// then reads that tail as evidence. Below the floor the honest answer is
    /// "the vector side found nothing", which is a thing this system is allowed
    /// to say.
    pub fn search_vector(
        &self,
        query: &[f32],
        limit: usize,
        min_similarity: f32,
    ) -> DbResult<Vec<VectorHit>> {
        self.ensure_index_loaded()?;
        Ok(self.index().nearest(query, limit, min_similarity))
    }

    fn ensure_index_loaded(&self) -> DbResult<()> {
        if *self.index().loaded.read() {
            return Ok(());
        }
        let rows: Vec<(Id, Vec<f32>)> = self.with(|c| {
            let mut stmt = c.prepare("SELECT item_id, vector FROM item_vectors")?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?))
            })?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter_map(|(id, b)| Id::parse(&id).ok().map(|i| (i, from_bytes(&b))))
                .collect())
        })?;
        let idx = self.index();
        *idx.entries.write() = rows;
        *idx.loaded.write() = true;
        tracing::debug!(count = idx.entries.read().len(), "vector index loaded");
        Ok(())
    }

    /// Items with no embedding yet, oldest first.
    ///
    /// This is what the background worker drains. Captures are acknowledged
    /// before they are embedded, so a non-empty queue is the normal state of a
    /// healthy system, not a backlog.
    pub fn items_awaiting_embedding(&self, limit: usize) -> DbResult<Vec<(Id, String)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT k.id, k.title || ' ' || k.content
                   FROM knowledge_items k
                   LEFT JOIN item_vectors v ON v.item_id = k.id
                  WHERE v.item_id IS NULL
                  ORDER BY k.created_at
                  LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?;
            Ok(rows
                .filter_map(|r| r.ok())
                .filter_map(|(id, text)| Id::parse(&id).ok().map(|i| (i, text)))
                .collect())
        })
    }

    /// Close out the `embed` jobs for an item the worker has just vectorised.
    ///
    /// The queue the worker actually reads is `items_awaiting_embedding`, which
    /// derives from the absence of a vector and is therefore self-healing: a
    /// job row lost to a crash costs nothing. These rows are still settled so
    /// the table does not grow without bound, and so a stuck item is visible as
    /// a job with attempts on it rather than as silence.
    pub fn mark_embedded(&self, item: Id) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE jobs SET state = 'done'
                  WHERE kind = 'embed' AND state = 'queued'
                    AND json_extract(payload, '$.item_id') = ?1",
                params![item.to_string()],
            )?;
            Ok(())
        })
    }

    /// Record that an item could not be embedded.
    ///
    /// Left `queued`: the next pass retries it. `attempts` is what distinguishes
    /// a transient failure from a poison item, and it is the only signal the
    /// Hub has that the backlog is stuck rather than merely long.
    pub fn embedding_failed(&self, item: Id, error: &str) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE jobs SET attempts = attempts + 1, last_error = ?2
                  WHERE kind = 'embed' AND state = 'queued'
                    AND json_extract(payload, '$.item_id') = ?1",
                params![item.to_string(), error],
            )?;
            Ok(())
        })
    }

    pub fn embedding_count(&self) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row("SELECT count(*) FROM item_vectors", [], |r| {
                r.get::<_, i64>(0)
            })? as u32)
        })
    }
}

impl VectorIndex {
    fn upsert(&self, id: Id, vector: Vec<f32>) {
        let mut e = self.entries.write();
        match e.iter_mut().find(|(i, _)| *i == id) {
            Some(slot) => slot.1 = vector,
            None => e.push((id, vector)),
        }
    }

    fn nearest(&self, query: &[f32], limit: usize, min_similarity: f32) -> Vec<VectorHit> {
        let e = self.entries.read();
        let mut hits: Vec<VectorHit> = e
            .iter()
            .map(|(id, v)| VectorHit {
                item_id: *id,
                similarity: dot(query, v),
            })
            // Zero means an orthogonal vector or a dimension mismatch; neither
            // is a result. Above that, the caller's floor decides.
            .filter(|h| h.similarity > 0.0 && h.similarity >= min_similarity)
            .collect();
        hits.sort_by(|a, b| {
            b.similarity
                .partial_cmp(&a.similarity)
                .unwrap_or(std::cmp::Ordering::Equal)
                // Stable order for ties, so identical queries rank identically
                // between runs.
                .then_with(|| a.item_id.cmp(&b.item_id))
        });
        hits.truncate(limit);
        hits
    }
}

/// Scale a vector to unit length. Test-side only: the embedder does this for
/// real vectors, and a test that skipped it would be measuring dot products
/// that are not cosines.
#[cfg(test)]
fn normalise(v: &mut [f32]) {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        for x in v.iter_mut() {
            *x /= n;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::KnowledgeItem;

    /// Deterministic unit vectors, for testing the plumbing without a model.
    fn unit(dim: usize, hot: usize) -> Vec<f32> {
        let mut v = vec![0.0; dim];
        v[hot % dim] = 1.0;
        v
    }

    #[test]
    fn stores_and_retrieves_nearest_neighbours() {
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("A", "first");
        let b = KnowledgeItem::capture("B", "second");
        db.capture(&a, None).unwrap();
        db.capture(&b, None).unwrap();

        db.put_embedding(a.id, "test", &unit(384, 0)).unwrap();
        db.put_embedding(b.id, "test", &unit(384, 1)).unwrap();

        let hits = db.search_vector(&unit(384, 0), 5, 0.0).unwrap();
        assert_eq!(hits.len(), 1, "the orthogonal vector should not score");
        assert_eq!(hits[0].item_id, a.id);
        assert!((hits[0].similarity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn re_embedding_replaces_rather_than_duplicates() {
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("A", "first");
        db.capture(&a, None).unwrap();
        db.put_embedding(a.id, "test", &unit(384, 0)).unwrap();
        db.put_embedding(a.id, "test-v2", &unit(384, 5)).unwrap();

        assert_eq!(db.embedding_count().unwrap(), 1);
        let hits = db.search_vector(&unit(384, 5), 5, 0.0).unwrap();
        assert_eq!(hits.len(), 1);
        assert!((hits[0].similarity - 1.0).abs() < 1e-5);
    }

    #[test]
    fn vectors_survive_a_reopen() {
        // The in-memory index is a cache; SQLite is the durable store.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        let id = {
            let db = Db::open(&path).unwrap();
            let a = KnowledgeItem::capture("A", "first");
            db.capture(&a, None).unwrap();
            db.put_embedding(a.id, "test", &unit(384, 3)).unwrap();
            a.id
        };
        let db = Db::open(&path).unwrap();
        let hits = db.search_vector(&unit(384, 3), 5, 0.0).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item_id, id);
    }

    #[test]
    fn the_queue_reflects_what_still_needs_embedding() {
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("A", "first");
        let b = KnowledgeItem::capture("B", "second");
        db.capture(&a, None).unwrap();
        db.capture(&b, None).unwrap();

        assert_eq!(db.items_awaiting_embedding(10).unwrap().len(), 2);
        db.put_embedding(a.id, "test", &unit(384, 0)).unwrap();

        let pending = db.items_awaiting_embedding(10).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, b.id);
        // Both title and content must be queued, or search matches only one.
        assert!(pending[0].1.contains('B') && pending[0].1.contains("second"));
    }

    #[test]
    fn a_dimension_mismatch_degrades_rather_than_panicking() {
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("A", "first");
        db.capture(&a, None).unwrap();
        db.put_embedding(a.id, "old-model", &unit(128, 0)).unwrap();
        // Querying with a different width must not crash.
        assert!(db.search_vector(&unit(384, 0), 5, 0.0).unwrap().is_empty());
    }
    #[test]
    fn embedding_an_item_settles_its_job() {
        let db = Db::open_in_memory().unwrap();
        let item = memos_core::KnowledgeItem::capture("t", "c");
        db.capture(&item, None).unwrap();

        let queued = |db: &Db| -> i64 {
            db.with(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM jobs WHERE kind='embed' AND state='queued'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap()
        };
        assert_eq!(queued(&db), 1);

        db.put_embedding(item.id, "test", &[1.0, 0.0]).unwrap();
        db.mark_embedded(item.id).unwrap();
        assert_eq!(queued(&db), 0);
    }

    #[test]
    fn a_failed_embedding_stays_queued_and_counts_the_attempt() {
        let db = Db::open_in_memory().unwrap();
        let item = memos_core::KnowledgeItem::capture("t", "c");
        db.capture(&item, None).unwrap();
        db.embedding_failed(item.id, "model unavailable").unwrap();

        let (attempts, state): (i64, String) = db
            .with(|c| {
                Ok(c.query_row(
                    "SELECT attempts, state FROM jobs WHERE kind='embed'",
                    [],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )?)
            })
            .unwrap();
        assert_eq!(attempts, 1);
        assert_eq!(state, "queued", "a retry must still be possible");
        // And it is still in the worker's queue, which is what actually matters.
        assert_eq!(db.items_awaiting_embedding(10).unwrap().len(), 1);
    }
    #[test]
    fn the_floor_keeps_the_tail_of_the_scan_out_of_the_results() {
        // Every corpus has a nearest neighbour. Without a floor, "no semantic
        // match" is indistinguishable from "here is the least unrelated item".
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("A", "first");
        db.capture(&a, None).unwrap();
        let mut v = unit(384, 0);
        v[1] = 0.4; // a weak partial match to the query below
        normalise(&mut v);
        db.put_embedding(a.id, "test", &v).unwrap();

        let q = unit(384, 0);
        assert_eq!(db.search_vector(&q, 5, 0.0).unwrap().len(), 1);
        assert!(
            db.search_vector(&q, 5, 0.99).unwrap().is_empty(),
            "a weak neighbour must not be reported as a match"
        );
    }
}
