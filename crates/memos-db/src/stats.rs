//! The counters behind the achievements: what was done, and on which days.
//!
//! One row per local day in `daily_activity`, written on the routing path and
//! read by everything that makes a claim about the user's history. Lifetime
//! totals are sums over that table rather than separate running counters —
//! there is one number for each fact and no second copy to drift.
//!
//! Local days, not UTC. See the migration: a streak is a claim about days as
//! the person lived them, and dictating at 11pm and again at 1am is one day's
//! work everywhere except in a database.

use chrono::{Datelike, Duration, Local, NaiveDate};
use memos_core::achieve::{Recap, Streak, Totals};
use rusqlite::params;

use crate::{Db, DbResult};

/// What a command did, for the purposes of counting it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActivityKind {
    /// It saved something. Counts towards captures as well as commands.
    Capture,
    /// It searched, answered, undid, moved, tagged. Words still count.
    Other,
}

/// Today, where the user is sitting.
pub fn today() -> String {
    Local::now().date_naive().to_string()
}

/// The ISO week a date falls in, as `2026-W36`.
///
/// Zero-padded so the strings sort in calendar order, which is what makes
/// "the most recent recap" a `max()` rather than a parse.
fn iso_week(d: NaiveDate) -> String {
    let w = d.iso_week();
    format!("{}-W{:02}", w.year(), w.week())
}

/// How many words a transcript is.
///
/// Whitespace-separated, which is wrong for Chinese and Japanese and right for
/// everything the STT model currently ships for. When those land this becomes a
/// per-language count; until then a character-based rule would make every
/// English total wrong to fix a case that cannot yet occur.
pub fn count_words(transcript: &str) -> u32 {
    transcript.split_whitespace().filter(|w| !w.is_empty()).count() as u32
}

impl Db {
    /// Record one command against today.
    ///
    /// Called for every transcript that gets routed, whatever it turned out to
    /// mean — a search is use of the app and belongs in the streak, even though
    /// it saved nothing.
    pub fn record_activity(
        &self,
        words: u32,
        speech_ms: u32,
        kind: ActivityKind,
    ) -> DbResult<()> {
        let day = today();
        let captures = i64::from(kind == ActivityKind::Capture);
        self.with(|c| {
            c.execute(
                "INSERT INTO daily_activity (day, words, captures, commands, speech_ms)
                 VALUES (?1, ?2, ?3, 1, ?4)
                 ON CONFLICT(day) DO UPDATE SET
                     words     = words     + excluded.words,
                     captures  = captures  + excluded.captures,
                     commands  = commands  + excluded.commands,
                     speech_ms = speech_ms + excluded.speech_ms",
                params![day, words as i64, captures, speech_ms as i64],
            )?;
            Ok(())
        })
    }

    /// Everything the ladder is measured against.
    ///
    /// `items` and `collections` come from the live tables because they are
    /// statements about what exists now; words, captures and days come from
    /// `daily_activity` because they are statements about what happened, and
    /// deleting a memory does not un-say it.
    pub fn totals(&self) -> DbResult<Totals> {
        self.with(|c| {
            let (words, captures, commands, days, speech_ms) = c.query_row(
                "SELECT coalesce(sum(words),0), coalesce(sum(captures),0),
                        coalesce(sum(commands),0), count(*), coalesce(sum(speech_ms),0)
                   FROM daily_activity",
                [],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                        r.get::<_, i64>(3)?,
                        r.get::<_, i64>(4)?,
                    ))
                },
            )?;
            let items: i64 = c.query_row("SELECT count(*) FROM knowledge_items", [], |r| r.get(0))?;
            let collections: i64 =
                c.query_row("SELECT count(*) FROM collections", [], |r| r.get(0))?;
            let tasks_done: i64 = c.query_row(
                "SELECT count(*) FROM tasks WHERE status = 'done'",
                [],
                |r| r.get(0),
            )?;

            Ok(Totals {
                words: words as u64,
                captures: captures as u64,
                commands: commands as u64,
                items: items as u32,
                collections: collections as u32,
                tasks_done: tasks_done as u32,
                days_active: days as u32,
                speech_ms: speech_ms as u64,
            })
        })
    }

    /// Current streak, longest ever, best day, and how long the last silence
    /// before today ran.
    ///
    /// One pass over every active day, newest first. The table holds one row
    /// per day the app was used, so this is a few hundred rows after a year —
    /// small enough that a scan beats keeping a maintained counter that can be
    /// wrong.
    pub fn streak(&self) -> DbResult<Streak> {
        let today = Local::now().date_naive();
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT day, words FROM daily_activity WHERE commands > 0 ORDER BY day DESC",
            )?;
            let mut days: Vec<(NaiveDate, u32)> = Vec::new();
            for row in stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })? {
                let (day, words) = row?;
                // A day that will not parse is a row from a corrupted write,
                // not a reason to have no streak at all.
                if let Ok(d) = day.parse::<NaiveDate>() {
                    days.push((d, words.max(0) as u32));
                }
            }

            let best_day = days.iter().map(|(_, w)| *w).max().unwrap_or(0);

            // The current run. It may end today or yesterday: a streak is not
            // broken until a day has actually been missed, and at 9am nobody
            // has missed today yet.
            let mut current = 0u32;
            if let Some((newest, _)) = days.first() {
                let gap = (today - *newest).num_days();
                if gap <= 1 {
                    current = 1;
                    let mut prev = *newest;
                    for (d, _) in days.iter().skip(1) {
                        if (prev - *d).num_days() == 1 {
                            current += 1;
                            prev = *d;
                        } else {
                            break;
                        }
                    }
                }
            }

            // The longest run anywhere in the history.
            let mut longest = 0u32;
            let mut run = 0u32;
            let mut prev: Option<NaiveDate> = None;
            for (d, _) in &days {
                run = match prev {
                    Some(p) if (p - *d).num_days() == 1 => run + 1,
                    _ => 1,
                };
                longest = longest.max(run);
                prev = Some(*d);
            }

            // Days of silence before today's first command. Measured from the
            // most recent day that is not today, so it does not reset the
            // moment today's row is written.
            let quiet_before = days
                .iter()
                .find(|(d, _)| *d < today)
                .map(|(d, _)| ((today - *d).num_days() - 1).max(0) as u32)
                .unwrap_or(0);

            Ok(Streak {
                current,
                longest: longest.max(current),
                best_day,
                quiet_before,
            })
        })
    }

    /// Last week's figures, but only on the day the summary is due.
    ///
    /// Returns `None` on every day but Monday, and on a Monday whose previous
    /// week was silent. The caller does not decide when a recap happens; this
    /// does, because the answer depends on the local calendar and that is
    /// already open here.
    pub fn due_recap(&self) -> DbResult<Option<Recap>> {
        let today = Local::now().date_naive();
        if today.weekday() != chrono::Weekday::Mon {
            return Ok(None);
        }
        let start = today - Duration::days(7);
        let end = today - Duration::days(1);
        let week = iso_week(start);

        self.with(|c| {
            let (words, captures, days) = c.query_row(
                "SELECT coalesce(sum(words),0), coalesce(sum(captures),0), count(*)
                   FROM daily_activity
                  WHERE day >= ?1 AND day <= ?2 AND commands > 0",
                params![start.to_string(), end.to_string()],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )?;
            if days == 0 {
                return Ok(None);
            }
            Ok(Some(Recap {
                iso_week: week,
                words: words as u64,
                captures: captures as u64,
                days: days as u32,
            }))
        })
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    /// Write a day directly. The public path can only ever write *today*,
    /// which is exactly what makes streaks untestable through it.
    fn day(db: &Db, date: &str, words: u32, captures: u32) {
        db.with(|c| {
            c.execute(
                "INSERT INTO daily_activity (day, words, captures, commands, speech_ms)
                 VALUES (?1, ?2, ?3, 1, 60000)",
                params![date, words as i64, captures as i64],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn ago(n: i64) -> String {
        (Local::now().date_naive() - Duration::days(n)).to_string()
    }

    #[test]
    fn counts_words_by_whitespace() {
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   "), 0);
        assert_eq!(count_words("save this to work"), 4);
        assert_eq!(count_words("  spaced   out \n lines "), 3);
    }

    #[test]
    fn activity_accumulates_into_one_row_a_day() {
        let db = Db::open_in_memory().unwrap();
        db.record_activity(10, 5_000, ActivityKind::Capture).unwrap();
        db.record_activity(15, 7_000, ActivityKind::Other).unwrap();

        let t = db.totals().unwrap();
        assert_eq!(t.words, 25);
        assert_eq!(t.captures, 1);
        assert_eq!(t.commands, 2);
        assert_eq!(t.days_active, 1);
        assert_eq!(t.speech_ms, 12_000);
    }

    #[test]
    fn a_streak_runs_up_to_today() {
        let db = Db::open_in_memory().unwrap();
        for d in 0..4 {
            day(&db, &ago(d), 100, 1);
        }
        let s = db.streak().unwrap();
        assert_eq!(s.current, 4);
        assert_eq!(s.longest, 4);
    }

    #[test]
    fn yesterday_still_counts_as_unbroken() {
        let db = Db::open_in_memory().unwrap();
        day(&db, &ago(1), 100, 1);
        day(&db, &ago(2), 100, 1);
        assert_eq!(db.streak().unwrap().current, 2);
    }

    #[test]
    fn a_missed_day_ends_the_run_but_not_the_record() {
        let db = Db::open_in_memory().unwrap();
        // A five-day run, a gap, then today.
        for d in 5..10 {
            day(&db, &ago(d), 100, 1);
        }
        day(&db, &ago(0), 100, 1);

        let s = db.streak().unwrap();
        assert_eq!(s.current, 1, "the old run is over");
        assert_eq!(s.longest, 5, "but it happened");
        assert_eq!(s.quiet_before, 4, "days silent before today");
    }

    #[test]
    fn the_best_day_is_the_biggest_one() {
        let db = Db::open_in_memory().unwrap();
        day(&db, &ago(3), 120, 1);
        day(&db, &ago(2), 1_800, 4);
        day(&db, &ago(1), 40, 1);
        assert_eq!(db.streak().unwrap().best_day, 1_800);
    }

    #[test]
    fn an_iso_week_is_padded_and_sortable() {
        assert_eq!(iso_week("2026-01-05".parse().unwrap()), "2026-W02");
        assert_eq!(iso_week("2026-09-07".parse().unwrap()), "2026-W37");
    }
}
