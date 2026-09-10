//! Acting on a memory that already exists — and taking it back.
//!
//! ADR-0005 makes a bargain: the system is allowed to act without asking
//! *because* every action is cheap to reverse. "Every executed command writes
//! an inverse operation" is the sentence that buys the aggressive thresholds,
//! and until now nothing collected on it — `events.inverse` has been written
//! since the first commit and never read.
//!
//! ## Undo is a verdict, not just a repair
//!
//! An undone capture is the clearest thing a user can say about a command:
//! that was not what I wanted. ADR-0006 needs verdicts and, since Tier 1
//! started resolving the ambiguity that used to produce questions, the
//! disambiguation prompt has almost stopped supplying them. This is the other
//! source — and the more honest one, because it costs the user nothing to give.
//!
//! So an event carries the id of the command that caused it, and undoing it
//! records `accepted = false` against that row.
//!
//! ## Why it is bounded in time
//!
//! Undo reverses the most recent reversible action, within
//! [`UNDO_WINDOW`]. Beyond that it says there is nothing recent to undo, and
//! that refusal is deliberate: "undo" is a word about the thing you just did,
//! and a system that hears it an hour later and deletes a memory you have
//! forgotten saving has done something you cannot get back. Deliberate deletion
//! belongs in the library, where you can see what you are deleting.

use std::time::Duration;

use memos_core::{Id, KnowledgeItem, Timestamp};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Serialize;

use crate::{Db, DbError, DbResult};

/// How far back "undo" reaches.
///
/// Ten minutes. Long enough to cover noticing a mistake, reading the receipt
/// and saying so; short enough that the word can never reach something the user
/// has stopped thinking about.
pub const UNDO_WINDOW: Duration = Duration::from_secs(10 * 60);

/// What an undo reversed.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Undone {
    /// Phrased for a receipt: `the memory "State as a Snapshot"`.
    pub what: String,
    /// The command that caused the thing being undone, when it is still known.
    /// This is what turns a repair into a verdict (ADR-0006).
    pub command_id: Option<Id>,
}

/// A task, as the Hub shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Task {
    pub id: Id,
    pub title: String,
    pub item_id: Option<Id>,
    pub due_at: Option<Timestamp>,
    pub status: String,
    pub created_at: Timestamp,
}

impl Db {
    /// Reverse the most recent reversible action.
    ///
    /// `None` means there was nothing recent enough to reverse — which is also
    /// what a second "undo" gets, because the first one marked the event.
    /// Idempotence matters here more than usual: an undo that silently found
    /// the same event twice would report success and do nothing.
    pub fn undo_last(&self) -> DbResult<Option<Undone>> {
        let cutoff = (memos_core::now()
            - chrono::Duration::from_std(UNDO_WINDOW).unwrap_or(chrono::Duration::minutes(10)))
        .to_rfc3339();

        self.transaction(|tx| {
            let row: Option<(String, String, Option<String>)> = tx
                .query_row(
                    "SELECT id, inverse, command_id FROM events
                      WHERE inverse IS NOT NULL AND undone_at IS NULL AND created_at >= ?1
                      ORDER BY created_at DESC LIMIT 1",
                    params![cutoff],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
                )
                .optional()?;
            let Some((event_id, inverse, command_id)) = row else {
                return Ok(None);
            };

            let what = apply_inverse(tx, &inverse)?;

            // Marked inside the same transaction that applied it. An inverse
            // that ran and an event that still looks undoable is the one state
            // this must never be left in.
            tx.execute(
                "UPDATE events SET undone_at = ?2 WHERE id = ?1",
                params![event_id, memos_core::now().to_rfc3339()],
            )?;

            Ok(Some(Undone {
                what,
                command_id: command_id.as_deref().and_then(|s| Id::parse(s).ok()),
            }))
        })
    }

    /// Refile an item, and record how to put it back.
    pub fn move_item(&self, item: Id, collection: Option<Id>) -> DbResult<()> {
        self.transaction(|tx| {
            let previous: Option<Option<String>> = tx
                .query_row(
                    "SELECT collection_id FROM knowledge_items WHERE id = ?1",
                    params![item.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            // The item is gone, or was never there. Not an error the capture
            // path should care about; the caller reports "nothing to move".
            let Some(previous) = previous else {
                return Ok(());
            };

            tx.execute(
                "UPDATE knowledge_items SET collection_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![
                    item.to_string(),
                    collection.map(|c| c.to_string()),
                    memos_core::now().to_rfc3339()
                ],
            )?;
            record_event(
                tx,
                "item.moved",
                Some(item),
                None,
                serde_json::json!({ "op": "move_item", "id": item.to_string(), "collection": previous }),
            )
        })
    }

    /// Attach a tag, creating it if this is the first time it has been used.
    ///
    /// Tagging never moves an item out of its collection — the join table is
    /// many-to-many precisely so nothing is trapped in one folder (§9).
    /// Returns whether the tag was newly attached; re-tagging is a no-op rather
    /// than an error, and writes no event, so undo does not remove a tag this
    /// command did not add.
    pub fn tag_item(&self, item: Id, tag: &str) -> DbResult<bool> {
        let tag = tag.trim().to_string();
        if tag.is_empty() {
            return Ok(false);
        }
        self.transaction(|tx| {
            let exists: Option<String> = tx
                .query_row(
                    "SELECT id FROM knowledge_items WHERE id = ?1",
                    params![item.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if exists.is_none() {
                return Ok(false);
            }

            tx.execute(
                "INSERT OR IGNORE INTO tags (id, name) VALUES (?1, ?2)",
                params![Id::new().to_string(), tag],
            )?;
            let tag_id: String = tx.query_row(
                "SELECT id FROM tags WHERE name = ?1",
                params![tag],
                |r| r.get(0),
            )?;

            let added = tx.execute(
                "INSERT OR IGNORE INTO knowledge_tags (item_id, tag_id) VALUES (?1, ?2)",
                params![item.to_string(), tag_id],
            )?;
            if added == 0 {
                return Ok(false);
            }

            record_event(
                tx,
                "item.tagged",
                Some(item),
                None,
                serde_json::json!({
                    "op": "untag_item",
                    "id": item.to_string(),
                    "tag_id": tag_id,
                }),
            )?;
            Ok(true)
        })
    }

    /// Every tag on an item, alphabetically.
    pub fn tags_for_item(&self, item: Id) -> DbResult<Vec<String>> {
        self.with(|c| {
            let mut q = c.prepare(
                "SELECT t.name FROM tags t
                   JOIN knowledge_tags kt ON kt.tag_id = t.id
                  WHERE kt.item_id = ?1
                  ORDER BY t.name",
            )?;
            let rows = q.query_map(params![item.to_string()], |r| r.get::<_, String>(0))?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// Create a task.
    ///
    /// `item` links it to a memory when the command was spoken about one; a
    /// task can perfectly well stand alone, which is the difference between
    /// this and a reminder.
    pub fn create_task(
        &self,
        title: &str,
        item: Option<Id>,
        due_at: Option<Timestamp>,
    ) -> DbResult<Task> {
        let task = Task {
            id: Id::new(),
            title: title.trim().to_string(),
            item_id: item,
            due_at,
            status: "open".into(),
            created_at: memos_core::now(),
        };
        self.transaction(|tx| {
            tx.execute(
                "INSERT INTO tasks (id, item_id, title, due_at, status, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6)",
                params![
                    task.id.to_string(),
                    task.item_id.map(|i| i.to_string()),
                    task.title,
                    task.due_at.map(|d| d.to_rfc3339()),
                    task.status,
                    task.created_at.to_rfc3339(),
                ],
            )?;
            record_event(
                tx,
                "task.created",
                Some(task.id),
                None,
                serde_json::json!({ "op": "delete_task", "id": task.id.to_string() }),
            )
        })?;
        Ok(task)
    }

    /// Tasks, open first, then by deadline, then newest first.
    ///
    /// A dated task outranks an undated one whatever its age: the list is read
    /// top-down to find out what is next, and "next" means the nearest
    /// deadline, not the most recent thought.
    ///
    /// One query rather than two lists: a done task that vanishes the instant
    /// it is ticked gives no confirmation that the tick landed.
    pub fn tasks(&self, limit: usize) -> DbResult<Vec<Task>> {
        self.with(|c| {
            let mut q = c.prepare(
                "SELECT id, item_id, title, due_at, status, created_at FROM tasks
                  ORDER BY (status = 'open') DESC,
                           (due_at IS NULL) ASC, due_at ASC,
                           created_at DESC
                  LIMIT ?1",
            )?;
            let rows = q.query_map(params![limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, Option<String>>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })?;
            let mut out = Vec::new();
            for row in rows {
                let (id, item_id, title, due_at, status, created_at) = row?;
                out.push(Task {
                    id: Id::parse(&id).unwrap_or_default(),
                    item_id: item_id.as_deref().and_then(|s| Id::parse(s).ok()),
                    title,
                    due_at: due_at.as_deref().and_then(parse_ts),
                    status,
                    created_at: parse_ts(&created_at).unwrap_or_else(memos_core::now),
                });
            }
            Ok(out)
        })
    }

    /// Rewrite a task's words, its deadline, or both.
    ///
    /// The deadline is passed as `Some(None)` to clear it and `None` to leave
    /// it alone — a task can lose its due date without losing its title, and
    /// one nullable argument cannot say which of those was meant.
    pub fn update_task(
        &self,
        id: Id,
        title: Option<&str>,
        due_at: Option<Option<Timestamp>>,
    ) -> DbResult<()> {
        self.with(|c| {
            if let Some(title) = title {
                c.execute(
                    "UPDATE tasks SET title = ?2 WHERE id = ?1",
                    params![id.to_string(), title.trim()],
                )?;
            }
            if let Some(due) = due_at {
                c.execute(
                    "UPDATE tasks SET due_at = ?2 WHERE id = ?1",
                    params![id.to_string(), due.map(|d| d.to_rfc3339())],
                )?;
            }
            Ok(())
        })
    }

    /// Take a task off the list for good.
    ///
    /// No audit row, and so nothing for `undo_last` to reverse: the button that
    /// calls this is next to the task it deletes, under a cursor, with a
    /// confirmation in front of it. Voice undo exists because a spoken command
    /// acts before you can stop it — a click does not.
    pub fn delete_task(&self, id: Id) -> DbResult<()> {
        self.with(|c| {
            c.execute("DELETE FROM tasks WHERE id = ?1", params![id.to_string()])?;
            Ok(())
        })
    }

    /// Tick a task, or un-tick it.
    pub fn set_task_done(&self, id: Id, done: bool) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE tasks SET status = ?2 WHERE id = ?1",
                params![id.to_string(), if done { "done" } else { "open" }],
            )?;
            Ok(())
        })
    }

    /// How many tasks are still open. For the Hub's nav badge.
    pub fn open_task_count(&self) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM tasks WHERE status = 'open'",
                [],
                |r| r.get(0),
            )?)
        })
    }

    /// The memory a bare "this" refers to, once the thing on screen is no
    /// longer the subject.
    ///
    /// `MOVE` and `TAG` act on a memory that already exists, and the user is
    /// almost always talking about the one they just made — "save this to
    /// react… no, move it to typescript". Newest capture, not newest access: a
    /// search you ran a moment ago changed nothing, and treating it as the
    /// referent would make the word mean different things depending on what you
    /// happened to do last.
    pub fn most_recent_capture(&self) -> DbResult<Option<KnowledgeItem>> {
        Ok(self.recent_items(1)?.into_iter().next())
    }
}

fn parse_ts(s: &str) -> Option<Timestamp> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.with_timezone(&chrono::Utc))
}

/// Append one audit row, with the operation that reverses it.
fn record_event(
    tx: &Transaction<'_>,
    kind: &str,
    entity: Option<Id>,
    command: Option<Id>,
    inverse: serde_json::Value,
) -> DbResult<()> {
    tx.execute(
        "INSERT INTO events (id, kind, entity_id, payload, inverse, command_id, created_at)
         VALUES (?1,?2,?3,'{}',?4,?5,?6)",
        params![
            Id::new().to_string(),
            kind,
            entity.map(|e| e.to_string()),
            inverse.to_string(),
            command.map(|c| c.to_string()),
            memos_core::now().to_rfc3339(),
        ],
    )?;
    Ok(())
}

/// Run one recorded inverse, and say what it undid.
///
/// Every operation named here is one this crate writes. An unrecognised one is
/// an error rather than a skip: it means a build wrote an inverse this build
/// cannot reverse, and quietly marking it undone would lose the change with no
/// trace of having done so.
fn apply_inverse(tx: &Transaction<'_>, inverse: &str) -> DbResult<String> {
    let inverse: serde_json::Value = serde_json::from_str(inverse)?;
    let op = inverse.get("op").and_then(|v| v.as_str()).unwrap_or("");
    let id = inverse.get("id").and_then(|v| v.as_str()).unwrap_or("");

    match op {
        "delete_item" => {
            let title: Option<String> = tx
                .query_row(
                    "SELECT title FROM knowledge_items WHERE id = ?1",
                    params![id],
                    |r| r.get(0),
                )
                .optional()?;
            // Everything hanging off the item — its vector, its tags, its FTS
            // row — goes with it, by cascade and by trigger.
            tx.execute("DELETE FROM knowledge_items WHERE id = ?1", params![id])?;
            Ok(match title {
                Some(t) => format!("the memory \u{201c}{}\u{201d}", truncate(&t, 60)),
                None => "that memory".into(),
            })
        }
        "move_item" => {
            let collection = inverse.get("collection").and_then(|v| v.as_str());
            tx.execute(
                "UPDATE knowledge_items SET collection_id = ?2, updated_at = ?3 WHERE id = ?1",
                params![id, collection, memos_core::now().to_rfc3339()],
            )?;
            Ok("that move".into())
        }
        "untag_item" => {
            let tag_id = inverse.get("tag_id").and_then(|v| v.as_str()).unwrap_or("");
            tx.execute(
                "DELETE FROM knowledge_tags WHERE item_id = ?1 AND tag_id = ?2",
                params![id, tag_id],
            )?;
            Ok("that tag".into())
        }
        "delete_task" => {
            let title: Option<String> = tx
                .query_row("SELECT title FROM tasks WHERE id = ?1", params![id], |r| {
                    r.get(0)
                })
                .optional()?;
            tx.execute("DELETE FROM tasks WHERE id = ?1", params![id])?;
            Ok(match title {
                Some(t) => format!("the task \u{201c}{}\u{201d}", truncate(&t, 60)),
                None => "that task".into(),
            })
        }
        other => Err(DbError::Migration(format!(
            "no way to undo {other:?} — the event was written by a different build"
        ))),
    }
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max.saturating_sub(1)).collect();
    format!("{}\u{2026}", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::KnowledgeItem;

    fn db() -> Db {
        Db::open_in_memory().expect("open")
    }

    fn capture(db: &Db, title: &str, command: Option<Id>) -> KnowledgeItem {
        let item = KnowledgeItem::capture(title, format!("{title} body"));
        db.capture(&item, command).unwrap();
        item
    }

    #[test]
    fn undo_removes_the_last_capture_and_says_what_it_removed() {
        let db = db();
        capture(&db, "State as a Snapshot", None);
        let second = capture(&db, "Router config", None);

        let undone = db.undo_last().unwrap().expect("something to undo");
        assert!(undone.what.contains("Router config"), "{}", undone.what);
        assert!(db.get_item(second.id).unwrap().is_none());
        // ...and only the last one.
        assert_eq!(db.recent_items(10).unwrap().len(), 1);
    }

    /// The failure this must never have: a second undo that reports success and
    /// changes nothing, or worse, reaches back and removes something else.
    #[test]
    fn undo_consumes_the_event_it_used() {
        let db = db();
        let only = capture(&db, "State as a Snapshot", None);
        assert!(db.undo_last().unwrap().is_some());
        assert!(db.undo_last().unwrap().is_none());
        assert!(db.get_item(only.id).unwrap().is_none());
    }

    /// The link that turns a repair into a verdict (ADR-0006).
    #[test]
    fn undo_names_the_command_that_caused_it() {
        let db = db();
        let command = Id::new();
        capture(&db, "State as a Snapshot", Some(command));
        assert_eq!(db.undo_last().unwrap().unwrap().command_id, Some(command));
    }

    /// "undo" is a word about the thing you just did. Anything older is not
    /// something the system should be able to delete on one misheard syllable.
    #[test]
    fn undo_does_not_reach_past_its_window() {
        let db = db();
        let item = capture(&db, "State as a Snapshot", None);
        db.with(|c| {
            let long_ago = (memos_core::now() - chrono::Duration::hours(2)).to_rfc3339();
            c.execute("UPDATE events SET created_at = ?1", params![long_ago])?;
            Ok(())
        })
        .unwrap();

        assert!(db.undo_last().unwrap().is_none());
        assert!(db.get_item(item.id).unwrap().is_some(), "still there");
    }

    #[test]
    fn a_move_can_be_put_back() {
        let db = db();
        let study = db.create_collection("Study", None).unwrap();
        let life = db.create_collection("Life", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "body");
        db.capture(&item, None).unwrap();
        db.move_item(item.id, Some(study.id)).unwrap();
        db.move_item(item.id, Some(life.id)).unwrap();

        assert_eq!(db.get_item(item.id).unwrap().unwrap().collection_id, Some(life.id));
        db.undo_last().unwrap().expect("undid the second move");
        assert_eq!(db.get_item(item.id).unwrap().unwrap().collection_id, Some(study.id));
        db.undo_last().unwrap().expect("undid the first move");
        assert_eq!(db.get_item(item.id).unwrap().unwrap().collection_id, None);
    }

    #[test]
    fn tagging_is_idempotent_and_undoable() {
        let db = db();
        let item = KnowledgeItem::capture("Hooks", "body");
        db.capture(&item, None).unwrap();

        assert!(db.tag_item(item.id, "react").unwrap());
        // Already tagged: nothing happens, and — the part that matters — no
        // event, so a later undo cannot strip a tag this command did not add.
        assert!(!db.tag_item(item.id, "react").unwrap());
        assert_eq!(db.tags_for_item(item.id).unwrap(), vec!["react"]);

        db.undo_last().unwrap().expect("undid the tag");
        assert!(db.tags_for_item(item.id).unwrap().is_empty());
        // The capture is what is left to undo, which proves the second tag
        // wrote nothing.
        assert!(db.undo_last().unwrap().is_some());
        assert!(db.undo_last().unwrap().is_none());
    }

    #[test]
    fn tags_and_moves_leave_the_item_where_the_other_put_it() {
        let db = db();
        let study = db.create_collection("Study", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "body");
        db.capture(&item, None).unwrap();
        db.move_item(item.id, Some(study.id)).unwrap();
        db.tag_item(item.id, "react").unwrap();

        // A tag never moves an item out of its collection (§9).
        assert_eq!(db.get_item(item.id).unwrap().unwrap().collection_id, Some(study.id));
        assert_eq!(db.tags_for_item(item.id).unwrap(), vec!["react"]);
    }

    #[test]
    fn tasks_list_open_first_and_can_be_ticked() {
        let db = db();
        let first = db.create_task("call the plumber", None, None).unwrap();
        let second = db.create_task("renew the domain", None, None).unwrap();

        db.set_task_done(second.id, true).unwrap();
        let listed = db.tasks(10).unwrap();
        assert_eq!(listed.len(), 2);
        assert_eq!(listed[0].id, first.id, "open tasks come first");
        assert_eq!(listed[1].status, "done");
        assert_eq!(db.open_task_count().unwrap(), 1);
    }

    #[test]
    fn a_dated_task_is_listed_before_an_undated_one() {
        let db = db();
        // Created last, so age alone would put it at the bottom.
        db.create_task("someday, learn the guitar", None, None).unwrap();
        let due = db
            .create_task(
                "renew the domain",
                None,
                Some(memos_core::now() + chrono::Duration::days(2)),
            )
            .unwrap();
        assert_eq!(db.tasks(10).unwrap()[0].id, due.id);
    }

    #[test]
    fn a_task_can_be_reworded_and_dated_and_undated() {
        let db = db();
        let t = db.create_task("call the plumer", None, None).unwrap();
        let when = memos_core::now() + chrono::Duration::days(1);

        db.update_task(t.id, Some("call the plumber"), Some(Some(when)))
            .unwrap();
        let row = db.tasks(10).unwrap().remove(0);
        assert_eq!(row.title, "call the plumber");
        assert!(row.due_at.is_some());

        // Clearing the deadline must leave the words alone, which is why the
        // argument is a `Some(None)` and not a bare `None`.
        db.update_task(t.id, None, Some(None)).unwrap();
        let row = db.tasks(10).unwrap().remove(0);
        assert_eq!(row.title, "call the plumber");
        assert!(row.due_at.is_none());
    }

    #[test]
    fn a_deleted_task_leaves_nothing_behind() {
        let db = db();
        let t = db.create_task("call the plumber", None, None).unwrap();
        db.delete_task(t.id).unwrap();
        assert!(db.tasks(10).unwrap().is_empty());
        assert_eq!(db.open_task_count().unwrap(), 0);
    }

    #[test]
    fn a_task_can_be_undone() {
        let db = db();
        db.create_task("call the plumber", None, None).unwrap();
        let undone = db.undo_last().unwrap().expect("undid the task");
        assert!(undone.what.contains("call the plumber"), "{}", undone.what);
        assert!(db.tasks(10).unwrap().is_empty());
    }

    /// Acting on nothing is not an error — the caller says "nothing to move",
    /// which is a better receipt than a failure the user cannot act on.
    #[test]
    fn acting_on_an_item_that_is_gone_changes_nothing() {
        let db = db();
        let ghost = Id::new();
        db.move_item(ghost, None).unwrap();
        assert!(!db.tag_item(ghost, "react").unwrap());
        assert!(db.undo_last().unwrap().is_none(), "no event was written");
    }

    #[test]
    fn this_means_the_newest_capture() {
        let db = db();
        capture(&db, "State as a Snapshot", None);
        let newest = capture(&db, "Router config", None);
        assert_eq!(db.most_recent_capture().unwrap().unwrap().id, newest.id);
    }
}
