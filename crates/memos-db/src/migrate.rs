//! Migrations.
//!
//! Deliberately hand-rolled rather than pulled from a crate: there are few of
//! them, they must run on a user's machine during an auto-update with no
//! recovery path, and a failure here loses somebody's memory. Explicit and
//! auditable beats convenient.

use rusqlite::Connection;

use crate::{DbError, DbResult};

/// Bump when adding a migration. `PRAGMA user_version` tracks the applied one.
pub const SCHEMA_VERSION: i64 = 6;

struct Migration {
    version: i64,
    name: &'static str,
    sql: &'static str,
}

const MIGRATIONS: &[Migration] = &[
    Migration {
        version: 1,
        name: "init",
        sql: include_str!("../migrations/001_init.sql"),
    },
    Migration {
        version: 2,
        name: "vectors",
        sql: include_str!("../migrations/002_vectors.sql"),
    },
    Migration {
        version: 3,
        name: "actions",
        sql: include_str!("../migrations/003_actions.sql"),
    },
    Migration {
        version: 4,
        name: "achievements",
        sql: include_str!("../migrations/004_achievements.sql"),
    },
    Migration {
        version: 5,
        name: "notes",
        sql: include_str!("../migrations/005_notes.sql"),
    },
    Migration {
        version: 6,
        name: "one_book",
        sql: include_str!("../migrations/006_one_book.sql"),
    },
];

pub fn run(conn: &Connection) -> DbResult<()> {
    let current: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;

    if current > SCHEMA_VERSION {
        // The user ran a newer build, then downgraded. Refuse rather than
        // guess: an older binary writing against a newer schema corrupts data.
        return Err(DbError::Migration(format!(
            "database is at version {current} but this build only understands {SCHEMA_VERSION}; \
             please update the application"
        )));
    }

    for m in MIGRATIONS.iter().filter(|m| m.version > current) {
        tracing::info!(version = m.version, name = m.name, "applying migration");
        // Each migration is one transaction: it either lands whole or not at
        // all, so a crash mid-update never leaves a half-migrated database.
        conn.execute_batch("BEGIN")?;
        match conn.execute_batch(m.sql) {
            Ok(()) => {
                conn.pragma_update(None, "user_version", m.version)?;
                conn.execute_batch("COMMIT")?;
            }
            Err(e) => {
                let _ = conn.execute_batch("ROLLBACK");
                return Err(DbError::Migration(format!(
                    "migration {} ({}) failed: {e}",
                    m.version, m.name
                )));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_are_idempotent() {
        let conn = Connection::open_in_memory().unwrap();
        run(&conn).unwrap();
        run(&conn).unwrap(); // second run must be a no-op
        let v: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
    }

    #[test]
    fn refuses_a_newer_database() {
        let conn = Connection::open_in_memory().unwrap();
        conn.pragma_update(None, "user_version", SCHEMA_VERSION + 5)
            .unwrap();
        assert!(run(&conn).is_err(), "must not downgrade a newer database");
    }
}
