//! The local store: SQLite in WAL mode, with FTS5 for keyword retrieval.
//!
//! ADR-0001. This is the client's source of truth. A capture must commit here
//! within ~20 ms, offline, every time — everything else (embedding, sync, cloud
//! enrichment) happens afterwards and is allowed to fail.

pub mod actions;
pub mod commands;
mod migrate;
pub mod repo;
pub mod vectors;

use std::path::Path;

use parking_lot::Mutex;
use rusqlite::Connection;

pub use actions::{Task, Undone, UNDO_WINDOW};
pub use commands::{Correction, LoggedCommand, RoutingStats};
pub use migrate::SCHEMA_VERSION;
pub use vectors::VectorHit;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("migration: {0}")]
    Migration(String),
}

pub type DbResult<T> = Result<T, DbError>;

/// A handle to the local database.
///
/// One connection behind a mutex rather than a pool. SQLite in WAL mode allows
/// concurrent readers, but the capture path is a single short write and pooling
/// would add contention and complexity for no measurable gain at this size.
/// Revisit if background embedding ever contends with capture.
pub struct Db {
    conn: Mutex<Connection>,
    /// In-memory copy of every stored vector. A cache with a lifetime, not a
    /// second source of truth — SQLite remains durable, this only avoids
    /// re-reading it on every search.
    vectors: vectors::VectorIndex,
}

impl Db {
    /// Open (creating if absent) the database at `path`.
    pub fn open(path: impl AsRef<Path>) -> DbResult<Self> {
        let path = path.as_ref();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).ok();
        }
        let conn = Connection::open(path)?;
        Self::configure(&conn)?;
        migrate::run(&conn)?;
        tracing::info!(path = %path.display(), version = SCHEMA_VERSION, "database ready");
        Ok(Self {
            conn: Mutex::new(conn),
            vectors: Default::default(),
        })
    }

    /// In-memory database, for tests.
    pub fn open_in_memory() -> DbResult<Self> {
        let conn = Connection::open_in_memory()?;
        Self::configure(&conn)?;
        migrate::run(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
            vectors: Default::default(),
        })
    }

    pub(crate) fn index(&self) -> &vectors::VectorIndex {
        &self.vectors
    }

    fn configure(conn: &Connection) -> DbResult<()> {
        // WAL: readers never block the writer, which matters once the embedding
        // worker is reading while a capture is committing.
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // NORMAL rather than FULL: on WAL this is durable across application
        // crashes (only a host power loss can lose the last commits) and avoids
        // an fsync on the capture path. The right trade for this workload.
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 5000)?;
        conn.pragma_update(None, "temp_store", "MEMORY")?;
        // 64 MB page cache. Cheap relative to the model footprint and it keeps
        // hot search paths off disk.
        conn.pragma_update(None, "cache_size", -64_000)?;
        Ok(())
    }

    /// Run a closure with the connection.
    pub fn with<T>(&self, f: impl FnOnce(&Connection) -> DbResult<T>) -> DbResult<T> {
        let conn = self.conn.lock();
        f(&conn)
    }

    /// Run a closure inside a transaction, committing on `Ok`.
    pub fn transaction<T>(
        &self,
        f: impl FnOnce(&rusqlite::Transaction<'_>) -> DbResult<T>,
    ) -> DbResult<T> {
        let mut conn = self.conn.lock();
        let tx = conn.transaction()?;
        let out = f(&tx)?;
        tx.commit()?;
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opens_and_migrates() {
        let db = Db::open_in_memory().expect("open");
        let v: i64 = db
            .with(|c| Ok(c.query_row("PRAGMA user_version", [], |r| r.get(0))?))
            .unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn fts_triggers_keep_the_index_in_sync() {
        let db = Db::open_in_memory().unwrap();
        db.with(|c| {
            c.execute(
                "INSERT INTO knowledge_items
                   (id,title,content,captured_at,created_at,updated_at)
                 VALUES ('a','State as a Snapshot',
                         'State is a snapshot for each render',
                         '2026-09-07','2026-09-07','2026-09-07')",
                [],
            )?;
            Ok(())
        })
        .unwrap();

        let hits: i64 = db
            .with(|c| {
                Ok(c.query_row(
                    "SELECT count(*) FROM items_fts WHERE items_fts MATCH 'snapshot'",
                    [],
                    |r| r.get(0),
                )?)
            })
            .unwrap();
        assert_eq!(hits, 1, "insert trigger should have populated FTS");
    }
}
