//! Repositories — the only way anything reads or writes the store.

use memos_core::{Collection, Id, KnowledgeItem, Source, SourceKind, SyncState};
use rusqlite::{params, Connection, OptionalExtension};

use crate::{Db, DbResult};

fn ts(t: &memos_core::Timestamp) -> String {
    t.to_rfc3339()
}

impl Db {
    /// Persist a capture and enqueue its follow-up work in **one transaction**.
    ///
    /// This is the write on the critical path (§4, stage 6). When it commits the
    /// user is told "Saved" — the embedding does not exist yet and nothing has
    /// been sent anywhere. That is what makes the acknowledgement honest and
    /// offline-safe: the outbox and the embed job are durable too, so they will
    /// happen, but the user never waits for them.
    /// `command` is the routing decision that caused this capture, when there
    /// was one. It rides on the audit row so that undoing the capture can be
    /// recorded as a verdict against the command rather than as an anonymous
    /// repair (ADR-0006).
    pub fn capture(&self, item: &KnowledgeItem, command: Option<Id>) -> DbResult<()> {
        self.transaction(|tx| {
            tx.execute(
                "INSERT INTO knowledge_items
                    (id, title, content, summary, collection_id, source_id,
                     captured_at, created_at, updated_at, access_count, sync_state)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,0,'pending')",
                params![
                    item.id.to_string(),
                    item.title,
                    item.content,
                    item.summary,
                    item.collection_id.map(|c| c.to_string()),
                    item.source_id.map(|s| s.to_string()),
                    ts(&item.captured_at),
                    ts(&item.created_at),
                    ts(&item.updated_at),
                ],
            )?;

            let now = ts(&memos_core::now());

            // Embedding is deferred, never inline.
            tx.execute(
                "INSERT INTO jobs (id, kind, payload, run_after, created_at)
                 VALUES (?1,'embed',?2,?3,?3)",
                params![
                    Id::new().to_string(),
                    serde_json::json!({ "item_id": item.id.to_string() }).to_string(),
                    now,
                ],
            )?;

            // Sync is deferred too, and may fail for days without the user noticing.
            tx.execute(
                "INSERT INTO outbox (id, entity, entity_id, op, created_at)
                 VALUES (?1,'knowledge_item',?2,'create',?3)",
                params![Id::new().to_string(), item.id.to_string(), now],
            )?;

            // Undo: the inverse of a create is a delete of this id.
            tx.execute(
                "INSERT INTO events
                    (id, kind, entity_id, payload, inverse, command_id, created_at)
                 VALUES (?1,'item.created',?2,'{}',?3,?4,?5)",
                params![
                    Id::new().to_string(),
                    item.id.to_string(),
                    serde_json::json!({ "op": "delete_item", "id": item.id.to_string() })
                        .to_string(),
                    command.map(|c| c.to_string()),
                    now,
                ],
            )?;

            Ok(())
        })
    }

    pub fn get_item(&self, id: Id) -> DbResult<Option<KnowledgeItem>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id,title,content,summary,collection_id,source_id,
                        captured_at,created_at,updated_at,last_accessed_at,
                        access_count,sync_state
                   FROM knowledge_items WHERE id = ?1",
                params![id.to_string()],
                row_to_item,
            )
            .optional()?)
        })
    }

    /// Keyword search over FTS5, ranked by BM25.
    ///
    /// Half of the hybrid retrieval in §7 — the vector half arrives at M2 and is
    /// fused with this via reciprocal rank fusion in a single query.
    pub fn search_keyword(&self, query: &str, limit: usize) -> DbResult<Vec<KnowledgeItem>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT k.id,k.title,k.content,k.summary,k.collection_id,k.source_id,
                        k.captured_at,k.created_at,k.updated_at,k.last_accessed_at,
                        k.access_count,k.sync_state
                   FROM items_fts f
                   JOIN knowledge_items k ON k.rowid = f.rowid
                  WHERE items_fts MATCH ?1
                  ORDER BY bm25(items_fts) LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![query, limit as i64], row_to_item)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn create_collection(&self, name: &str, parent: Option<Id>) -> DbResult<Collection> {
        self.transaction(|tx| {
            let path = match parent {
                Some(p) => {
                    let parent_path: String = tx.query_row(
                        "SELECT path FROM collections WHERE id = ?1",
                        params![p.to_string()],
                        |r| r.get(0),
                    )?;
                    format!("{parent_path}/{name}")
                }
                None => name.to_string(),
            };
            let c = Collection {
                id: Id::new(),
                parent_id: parent,
                name: name.to_string(),
                path,
                created_at: memos_core::now(),
            };
            tx.execute(
                "INSERT INTO collections (id,parent_id,name,path,created_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    c.id.to_string(),
                    c.parent_id.map(|p| p.to_string()),
                    c.name,
                    c.path,
                    ts(&c.created_at),
                ],
            )?;
            Ok(c)
        })
    }

    /// Rename a collection, and re-materialise every path beneath it.
    ///
    /// The path column is denormalised so the router grammar can be built from
    /// one query (§9), which means a rename is not one row's business: every
    /// descendant carries the old name inside its own path. Done in one
    /// transaction, because a half-renamed tree is a set of destinations the
    /// grammar would offer and the resolver could never find.
    pub fn rename_collection(&self, id: Id, name: &str) -> DbResult<String> {
        let name = name.trim().to_string();
        self.transaction(|tx| {
            let old: String = tx.query_row(
                "SELECT path FROM collections WHERE id = ?1",
                params![id.to_string()],
                |r| r.get(0),
            )?;
            let new = match old.rfind('/') {
                Some(cut) => format!("{}/{name}", &old[..cut]),
                None => name.clone(),
            };
            tx.execute(
                "UPDATE collections SET name = ?2, path = ?3 WHERE id = ?1",
                params![id.to_string(), name, new],
            )?;
            // The prefix match is anchored with the separator so `Life/Housing`
            // is not caught by a rename of `Life/House`.
            tx.execute(
                "UPDATE collections
                    SET path = ?2 || substr(path, length(?1) + 1)
                  WHERE path LIKE ?1 || '/%'",
                params![old, new],
            )?;
            Ok(new)
        })
    }

    /// Delete a collection and everything filed under it.
    ///
    /// Only the shelving goes. Child collections cascade, and the memories
    /// inside them fall back to unfiled rather than following the folder into
    /// the bin — deleting a name the user regrets must never be a way to lose
    /// the words they said.
    pub fn delete_collection(&self, id: Id) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "DELETE FROM collections WHERE id = ?1",
                params![id.to_string()],
            )?;
            Ok(())
        })
    }

    /// How many memories are filed in a collection or anywhere beneath it.
    ///
    /// What the confirmation before a delete has to state: the folders go, and
    /// this many memories come loose.
    pub fn count_in_subtree(&self, path: &str) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM knowledge_items k
                   JOIN collections c ON c.id = k.collection_id
                  WHERE c.path = ?1 OR c.path LIKE ?1 || '/%'",
                params![path],
                |r| r.get::<_, i64>(0),
            )? as u32)
        })
    }

    /// Look up a collection by its materialised path.
    ///
    /// Returns `Ok(None)` for an unknown path rather than an error: the caller
    /// files the item unsorted instead of losing the capture.
    pub fn collection_id_by_path(&self, path: &str) -> DbResult<Option<Id>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id FROM collections WHERE path = ?1",
                params![path],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|s| Id::parse(&s).ok()))
        })
    }

    /// Every collection path, for the router grammar.
    ///
    /// Regenerating the GBNF grammar from this is what stops a sub-billion
    /// parameter model from inventing a destination that does not exist
    /// (ADR-0003).
    pub fn collection_paths(&self) -> DbResult<Vec<String>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT path FROM collections ORDER BY path")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Captures used in the current week, for the free-tier meter (§16).
    pub fn captures_this_week(&self, week_start: &str) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT captures FROM usage WHERE week_start = ?1",
                params![week_start],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .unwrap_or(0) as u32)
        })
    }

    /// Increment the local counter. Local so capture still succeeds offline;
    /// reconciled on sync, tolerating slight over-run (§16).
    pub fn record_capture(&self, week_start: &str) -> DbResult<u32> {
        self.transaction(|tx| {
            tx.execute(
                "INSERT INTO usage (week_start, captures) VALUES (?1, 1)
                 ON CONFLICT(week_start) DO UPDATE SET captures = captures + 1",
                params![week_start],
            )?;
            Ok(tx.query_row(
                "SELECT captures FROM usage WHERE week_start = ?1",
                params![week_start],
                |r| r.get::<_, i64>(0),
            )? as u32)
        })
    }
    /// Record where an item came from.
    ///
    /// Written on the capture path so `OPEN` has something to reopen. Without
    /// it a saved page is only text: the user can find it again but not get
    /// back to it, which is half a memory.
    pub fn put_source(&self, s: &Source) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO sources (id,kind,url,domain,file_path,title,retrieved_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7)
                 ON CONFLICT(id) DO UPDATE SET
                     kind=excluded.kind, url=excluded.url, domain=excluded.domain,
                     file_path=excluded.file_path, title=excluded.title",
                params![
                    s.id.to_string(),
                    s.kind.as_str(),
                    s.url,
                    s.domain,
                    s.file_path,
                    s.title,
                    ts(&s.retrieved_at),
                ],
            )?;
            Ok(())
        })
    }

    pub fn source_for_item(&self, item: Id) -> DbResult<Option<Source>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT s.id,s.kind,s.url,s.domain,s.file_path,s.title,s.retrieved_at
                   FROM sources s
                   JOIN knowledge_items k ON k.source_id = s.id
                  WHERE k.id = ?1",
                params![item.to_string()],
                row_to_source,
            )
            .optional()?)
        })
    }

    /// Note that an item was opened.
    ///
    /// Feeds the access signal in re-ranking (§11.6) — and it is the only
    /// relevance label collected for free, because the result the user opens is
    /// the result that was right.
    pub fn record_access(&self, item: Id) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE knowledge_items
                    SET access_count = access_count + 1, last_accessed_at = ?2
                  WHERE id = ?1",
                params![item.to_string(), ts(&memos_core::now())],
            )?;
            Ok(())
        })
    }

    /// Most recently captured items, newest first. The Hub's landing screen.
    pub fn recent_items(&self, limit: usize) -> DbResult<Vec<KnowledgeItem>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id,title,content,summary,collection_id,source_id,
                        captured_at,created_at,updated_at,last_accessed_at,
                        access_count,sync_state
                   FROM knowledge_items
                  ORDER BY captured_at DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], row_to_item)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// A page of the Library, optionally narrowed to one collection.
    ///
    /// `collection` is a path, not an id: it is what the user sees and what the
    /// router speaks, and resolving it here keeps that translation in one place.
    /// An unknown path returns nothing rather than everything — a filter that
    /// silently stops filtering is worse than an empty list.
    pub fn list_items(
        &self,
        collection: Option<&str>,
        limit: usize,
        offset: usize,
    ) -> DbResult<Vec<KnowledgeItem>> {
        let collection_id = match collection {
            Some(path) => match self.collection_id_by_path(path)? {
                Some(id) => Some(id.to_string()),
                None => return Ok(Vec::new()),
            },
            None => None,
        };
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id,title,content,summary,collection_id,source_id,
                        captured_at,created_at,updated_at,last_accessed_at,
                        access_count,sync_state
                   FROM knowledge_items
                  WHERE (?1 IS NULL OR collection_id = ?1)
                  ORDER BY captured_at DESC LIMIT ?2 OFFSET ?3",
            )?;
            let rows =
                stmt.query_map(params![collection_id, limit as i64, offset as i64], row_to_item)?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Every collection with the number of items filed in it.
    ///
    /// The count comes from the same query rather than one per row: the
    /// Collections screen renders the whole tree at once, and a query per node
    /// is the classic way that screen gets slow as the tree grows.
    pub fn collections_with_counts(&self) -> DbResult<Vec<(Collection, u32)>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT c.id,c.parent_id,c.name,c.path,c.created_at,
                        (SELECT count(*) FROM knowledge_items k WHERE k.collection_id = c.id)
                   FROM collections c ORDER BY c.path",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok((
                    Collection {
                        id: Id::parse(&r.get::<_, String>(0)?).unwrap_or_default(),
                        parent_id: r
                            .get::<_, Option<String>>(1)?
                            .and_then(|s| Id::parse(&s).ok()),
                        name: r.get(2)?,
                        path: r.get(3)?,
                        created_at: chrono::DateTime::parse_from_rfc3339(
                            &r.get::<_, String>(4)?,
                        )
                        .map(|d| d.with_timezone(&chrono::Utc))
                        .unwrap_or_else(|_| chrono::Utc::now()),
                    },
                    r.get::<_, i64>(5)? as u32,
                ))
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    pub fn item_count(&self) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row("SELECT count(*) FROM knowledge_items", [], |r| {
                r.get::<_, i64>(0)
            })? as u32)
        })
    }
}

fn row_to_item(r: &rusqlite::Row<'_>) -> rusqlite::Result<KnowledgeItem> {
    let parse = |s: String| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now())
    };
    Ok(KnowledgeItem {
        id: Id::parse(&r.get::<_, String>(0)?).unwrap_or_default(),
        title: r.get(1)?,
        content: r.get(2)?,
        summary: r.get(3)?,
        collection_id: r
            .get::<_, Option<String>>(4)?
            .and_then(|s| Id::parse(&s).ok()),
        source_id: r
            .get::<_, Option<String>>(5)?
            .and_then(|s| Id::parse(&s).ok()),
        captured_at: parse(r.get(6)?),
        created_at: parse(r.get(7)?),
        updated_at: parse(r.get(8)?),
        last_accessed_at: r.get::<_, Option<String>>(9)?.map(parse),
        access_count: r.get::<_, i64>(10)? as u32,
        sync_state: match r.get::<_, String>(11)?.as_str() {
            "synced" => SyncState::Synced,
            "conflicted" => SyncState::Conflicted,
            _ => SyncState::Pending,
        },
    })
}

fn row_to_source(r: &rusqlite::Row<'_>) -> rusqlite::Result<Source> {
    Ok(Source {
        id: Id::parse(&r.get::<_, String>(0)?).unwrap_or_default(),
        // An unrecognised kind reads as a plain file rather than failing the
        // row: a newer build may have written a variant this one predates, and
        // losing the source entirely is worse than describing it loosely.
        kind: match r.get::<_, String>(1)?.as_str() {
            "webpage" => SourceKind::Webpage,
            "highlight" => SourceKind::Highlight,
            "voice_note" => SourceKind::VoiceNote,
            "pdf" => SourceKind::Pdf,
            "image" => SourceKind::Image,
            "manual_note" => SourceKind::ManualNote,
            "conversation" => SourceKind::Conversation,
            _ => SourceKind::File,
        },
        url: r.get(2)?,
        domain: r.get(3)?,
        file_path: r.get(4)?,
        title: r.get(5)?,
        retrieved_at: chrono::DateTime::parse_from_rfc3339(&r.get::<_, String>(6)?)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| chrono::Utc::now()),
    })
}

#[allow(dead_code)]
fn unused(_: &Connection) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_writes_item_job_outbox_and_undo_atomically() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("State as a Snapshot", "State is a snapshot per render");
        db.capture(&item, None).unwrap();

        let counts: (i64, i64, i64, i64) = db
            .with(|c| {
                Ok((
                    c.query_row("SELECT count(*) FROM knowledge_items", [], |r| r.get(0))?,
                    c.query_row("SELECT count(*) FROM jobs WHERE kind='embed'", [], |r| {
                        r.get(0)
                    })?,
                    c.query_row("SELECT count(*) FROM outbox", [], |r| r.get(0))?,
                    c.query_row("SELECT count(*) FROM events WHERE inverse IS NOT NULL", [], |r| {
                        r.get(0)
                    })?,
                ))
            })
            .unwrap();
        assert_eq!(counts, (1, 1, 1, 1), "one capture writes all four rows");
    }

    #[test]
    fn semantic_wording_still_found_by_keyword() {
        let db = Db::open_in_memory().unwrap();
        db.capture(&KnowledgeItem::capture(
            "State as a Snapshot",
            "State is a snapshot for each render",
        ), None)
        .unwrap();
        let hits = db.search_keyword("snapshot render", 10).unwrap();
        assert_eq!(hits.len(), 1);
    }

    #[test]
    fn collection_paths_are_materialised() {
        let db = Db::open_in_memory().unwrap();
        let study = db.create_collection("Study", None).unwrap();
        let prog = db.create_collection("Programming", Some(study.id)).unwrap();
        let react = db.create_collection("React", Some(prog.id)).unwrap();
        assert_eq!(react.path, "Study/Programming/React");
        assert_eq!(db.collection_paths().unwrap().len(), 3);
    }

    #[test]
    fn renaming_a_collection_rewrites_the_paths_below_it() {
        let db = Db::open_in_memory().unwrap();
        let life = db.create_collection("Life", None).unwrap();
        let house = db.create_collection("House", Some(life.id)).unwrap();
        db.create_collection("Router", Some(house.id)).unwrap();
        // The sibling that starts with the same letters: the prefix match has
        // to be anchored on the separator or this one moves too.
        db.create_collection("Housing", Some(life.id)).unwrap();

        db.rename_collection(house.id, "Home").unwrap();
        let paths = db.collection_paths().unwrap();
        assert!(paths.contains(&"Life/Home".to_string()), "{paths:?}");
        assert!(paths.contains(&"Life/Home/Router".to_string()), "{paths:?}");
        assert!(paths.contains(&"Life/Housing".to_string()), "{paths:?}");
    }

    #[test]
    fn deleting_a_collection_unfiles_its_memories_rather_than_erasing_them() {
        let db = Db::open_in_memory().unwrap();
        let life = db.create_collection("Life", None).unwrap();
        let house = db.create_collection("House", Some(life.id)).unwrap();

        let mut item = KnowledgeItem::capture("Router", "hold reset for ten seconds");
        item.collection_id = Some(house.id);
        db.capture(&item, None).unwrap();
        assert_eq!(db.count_in_subtree("Life").unwrap(), 1);

        db.delete_collection(life.id).unwrap();
        assert!(db.collection_paths().unwrap().is_empty(), "children go too");
        let kept = db.get_item(item.id).unwrap().expect("the memory survives");
        assert!(kept.collection_id.is_none(), "and comes loose");
    }

    #[test]
    fn weekly_counter_increments() {
        let db = Db::open_in_memory().unwrap();
        assert_eq!(db.captures_this_week("2026-09-07").unwrap(), 0);
        db.record_capture("2026-09-07").unwrap();
        db.record_capture("2026-09-07").unwrap();
        assert_eq!(db.captures_this_week("2026-09-07").unwrap(), 2);
    }
    #[test]
    fn a_source_survives_the_round_trip_and_is_reachable_from_its_item() {
        // OPEN depends on this: without a source row a saved page is text the
        // user can find but never get back to.
        let db = Db::open_in_memory().unwrap();
        let src = Source {
            id: Id::new(),
            kind: SourceKind::Webpage,
            url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            domain: Some("react.dev".into()),
            file_path: None,
            title: Some("State as a Snapshot".into()),
            retrieved_at: memos_core::now(),
        };
        db.put_source(&src).unwrap();

        let mut item = KnowledgeItem::capture("State as a Snapshot", "…");
        item.source_id = Some(src.id);
        db.capture(&item, None).unwrap();

        let got = db.source_for_item(item.id).unwrap().expect("source");
        assert_eq!(got.url.as_deref(), Some("https://react.dev/learn/state-as-a-snapshot"));
        assert_eq!(got.kind, SourceKind::Webpage);
    }

    #[test]
    fn an_item_with_no_source_asks_without_erroring() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("A thought", "no source");
        db.capture(&item, None).unwrap();
        assert!(db.source_for_item(item.id).unwrap().is_none());
    }

    #[test]
    fn opening_an_item_records_the_access() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("t", "c");
        db.capture(&item, None).unwrap();
        db.record_access(item.id).unwrap();
        db.record_access(item.id).unwrap();

        let got = db.get_item(item.id).unwrap().unwrap();
        assert_eq!(got.access_count, 2);
        assert!(got.last_accessed_at.is_some());
    }

    #[test]
    fn the_library_filters_by_collection_path() {
        let db = Db::open_in_memory().unwrap();
        let react = db.create_collection("React", None).unwrap();
        let mut filed = KnowledgeItem::capture("filed", "c");
        filed.collection_id = Some(react.id);
        db.capture(&filed, None).unwrap();
        db.capture(&KnowledgeItem::capture("unfiled", "c"), None).unwrap();

        assert_eq!(db.list_items(None, 10, 0).unwrap().len(), 2);
        assert_eq!(db.list_items(Some("React"), 10, 0).unwrap().len(), 1);
        // An unknown filter must return nothing, never everything.
        assert!(db.list_items(Some("Nope"), 10, 0).unwrap().is_empty());
    }

    #[test]
    fn collections_carry_their_item_counts() {
        let db = Db::open_in_memory().unwrap();
        let study = db.create_collection("Study", None).unwrap();
        db.create_collection("React", Some(study.id)).unwrap();
        let mut item = KnowledgeItem::capture("t", "c");
        item.collection_id = Some(study.id);
        db.capture(&item, None).unwrap();

        let all = db.collections_with_counts().unwrap();
        assert_eq!(all.len(), 2);
        let study_row = all.iter().find(|(c, _)| c.path == "Study").unwrap();
        assert_eq!(study_row.1, 1);
        assert_eq!(all.iter().find(|(c, _)| c.path == "Study/React").unwrap().1, 0);
    }

    #[test]
    fn recent_items_are_newest_first() {
        let db = Db::open_in_memory().unwrap();
        for t in ["one", "two", "three"] {
            let mut i = KnowledgeItem::capture(t, "c");
            // Same-millisecond captures would otherwise tie and order by
            // whatever SQLite felt like.
            i.captured_at = memos_core::now() + chrono::Duration::seconds(match t {
                "one" => 0,
                "two" => 1,
                _ => 2,
            });
            db.capture(&i, None).unwrap();
        }
        let recent = db.recent_items(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].title, "three");
        assert_eq!(recent[1].title, "two");
    }
}
