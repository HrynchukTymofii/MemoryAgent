//! The notification centre's store, and the evaluation that fills it.
//!
//! `evaluate` is the whole of it: read the counters, ask the ladder what is
//! newly true, and write both the award and the notification in one
//! transaction. Everything else here is the reading side.
//!
//! Stored rather than derived, which is the difference between this and the
//! bell it replaces. A notification that is computed fresh on every render has
//! nowhere to record that it has been seen, so it re-announces itself forever;
//! these have a `read_at`, and they survive a restart.

use memos_core::achieve;
use memos_core::Id;
use rusqlite::{params, OptionalExtension};

use crate::{Db, DbResult};

/// A notification, as the interface will draw it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct Notification {
    pub id: String,
    /// `milestone` | `nudge` | `alert`.
    pub kind: String,
    /// The achievement behind it, when there was one.
    pub code: Option<String>,
    pub title: String,
    pub body: String,
    /// Page to open on click.
    pub goto: Option<String>,
    pub created_at: String,
    pub read: bool,
}

impl Db {
    /// Award whatever has just become true, and return it.
    ///
    /// Called after anything that could move a counter. Cheap enough to run on
    /// that path: five aggregate queries over small tables and a scan of one
    /// row per active day. The insert is `OR IGNORE` on the achievement's
    /// primary key, so two evaluations racing each other cannot both award the
    /// same milestone — the loser writes no notification either, because the
    /// notification is only written when the award actually landed.
    pub fn evaluate(&self) -> DbResult<Vec<Notification>> {
        let totals = self.totals()?;
        let streak = self.streak()?;
        let recap = self.due_recap()?;
        let already = self.awarded()?;

        let newly = achieve::earned(&totals, &streak, recap.as_ref(), &already);
        if newly.is_empty() {
            return Ok(Vec::new());
        }

        let now = memos_core::now().to_rfc3339();
        self.transaction(|tx| {
            let mut out = Vec::new();
            for e in newly {
                let landed = tx.execute(
                    "INSERT OR IGNORE INTO achievements (code, progress, earned_at)
                     VALUES (?1, ?2, ?3)",
                    params![e.code, e.progress as i64, now],
                )?;
                if landed == 0 {
                    // Someone else got there first. Not an error, and not a
                    // second notification.
                    continue;
                }
                let id = Id::new().to_string();
                tx.execute(
                    "INSERT INTO notifications
                        (id, kind, code, title, body, goto, created_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7)",
                    params![id, e.kind.as_str(), e.code, e.title, e.body, e.goto, now],
                )?;
                out.push(Notification {
                    id,
                    kind: e.kind.as_str().to_string(),
                    code: Some(e.code),
                    title: e.title,
                    body: e.body,
                    goto: e.goto.map(str::to_string),
                    created_at: now.clone(),
                    read: false,
                });
            }
            Ok(out)
        })
    }

    /// Every milestone code already awarded.
    fn awarded(&self) -> DbResult<std::collections::HashSet<String>> {
        self.with(|c| {
            let mut stmt = c.prepare("SELECT code FROM achievements")?;
            let mut set = std::collections::HashSet::new();
            for row in stmt.query_map([], |r| r.get::<_, String>(0))? {
                set.insert(row?);
            }
            Ok(set)
        })
    }

    /// The list behind the bell: newest first, dismissed ones gone.
    pub fn notifications(&self, limit: usize) -> DbResult<Vec<Notification>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT id, kind, code, title, body, goto, created_at, read_at
                   FROM notifications
                  WHERE dismissed_at IS NULL
                  ORDER BY created_at DESC
                  LIMIT ?1",
            )?;
            let rows = stmt.query_map(params![limit as i64], |r| {
                Ok(Notification {
                    id: r.get(0)?,
                    kind: r.get(1)?,
                    code: r.get(2)?,
                    title: r.get(3)?,
                    body: r.get(4)?,
                    goto: r.get(5)?,
                    created_at: r.get(6)?,
                    read: r.get::<_, Option<String>>(7)?.is_some(),
                })
            })?;
            Ok(rows.collect::<Result<Vec<_>, _>>()?)
        })
    }

    /// How many are unread. The number on the bell.
    pub fn unread_notifications(&self) -> DbResult<u32> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT count(*) FROM notifications
                  WHERE read_at IS NULL AND dismissed_at IS NULL",
                [],
                |r| r.get::<_, i64>(0),
            )? as u32)
        })
    }

    /// Mark everything currently unread as read. Returns how many changed.
    ///
    /// All of them at once rather than one per row as it scrolls past: opening
    /// the panel is the act of reading it, and a per-row rule leaves a badge
    /// showing for the two items below the fold that the user never wanted.
    pub fn mark_notifications_read(&self) -> DbResult<u32> {
        let now = memos_core::now().to_rfc3339();
        self.with(|c| {
            Ok(c.execute(
                "UPDATE notifications SET read_at = ?1 WHERE read_at IS NULL",
                params![now],
            )? as u32)
        })
    }

    /// Take one off the list for good.
    ///
    /// The achievement row stays. Dismissing the note that you crossed ten
    /// thousand words does not un-cross them, and it must not let the milestone
    /// be announced a second time.
    pub fn dismiss_notification(&self, id: &str) -> DbResult<()> {
        let now = memos_core::now().to_rfc3339();
        self.with(|c| {
            c.execute(
                "UPDATE notifications SET dismissed_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    /// Clear the list. Dismisses everything currently visible.
    pub fn dismiss_all_notifications(&self) -> DbResult<u32> {
        let now = memos_core::now().to_rfc3339();
        self.with(|c| {
            Ok(c.execute(
                "UPDATE notifications SET dismissed_at = ?1 WHERE dismissed_at IS NULL",
                params![now],
            )? as u32)
        })
    }

    /// When a given milestone was earned, if it has been.
    pub fn earned_at(&self, code: &str) -> DbResult<Option<String>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT earned_at FROM achievements WHERE code = ?1",
                params![code],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
        })
    }
}

/// A notification that did not come from the ladder.
///
/// The backend going quiet is worth the same panel as a milestone, but it is
/// not an achievement and it must not be stored: it is true or it is not, and a
/// stored copy would still be sitting there tomorrow claiming the app is
/// offline. The interface builds these each render, from live state.
impl Notification {
    pub fn alert(id: &str, title: &str, body: &str, goto: Option<&str>) -> Self {
        Notification {
            id: id.to_string(),
            kind: "alert".to_string(),
            code: None,
            title: title.to_string(),
            body: body.to_string(),
            goto: goto.map(str::to_string),
            created_at: memos_core::now().to_rfc3339(),
            read: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stats::ActivityKind;

    #[test]
    fn the_first_capture_is_announced_once() {
        let db = Db::open_in_memory().unwrap();
        db.record_activity(6, 4_000, ActivityKind::Capture).unwrap();

        let first = db.evaluate().unwrap();
        assert!(first.iter().any(|n| n.code.as_deref() == Some("captures-1")));

        // Same state, evaluated again — and the panel does not grow.
        let again = db.evaluate().unwrap();
        assert!(again.is_empty(), "re-announced: {again:?}");
        assert_eq!(db.notifications(50).unwrap().len(), first.len());
    }

    #[test]
    fn unread_falls_to_zero_when_the_panel_is_opened() {
        let db = Db::open_in_memory().unwrap();
        db.record_activity(150, 60_000, ActivityKind::Capture).unwrap();
        db.evaluate().unwrap();

        assert!(db.unread_notifications().unwrap() > 0);
        db.mark_notifications_read().unwrap();
        assert_eq!(db.unread_notifications().unwrap(), 0);
        assert!(db.notifications(50).unwrap().iter().all(|n| n.read));
    }

    #[test]
    fn dismissing_hides_the_note_and_keeps_the_award() {
        let db = Db::open_in_memory().unwrap();
        db.record_activity(120, 60_000, ActivityKind::Capture).unwrap();
        db.evaluate().unwrap();

        let one = db.notifications(50).unwrap()[0].id.clone();
        db.dismiss_notification(&one).unwrap();
        assert!(db.notifications(50).unwrap().iter().all(|n| n.id != one));

        // And it stays awarded, so it cannot come round again.
        assert!(db.evaluate().unwrap().is_empty());
        assert!(db.earned_at("captures-1").unwrap().is_some());
    }
}
