//! The correction log — every command, and what the user made of it.
//!
//! ADR-0006 calls this the most valuable table in the system, and the reason is
//! that it is four things at once:
//!
//! - the few-shot corpus, once kNN retrieval over past commands lands;
//! - the evaluation set for any change to the router;
//! - the calibration data behind ADR-0005's `prior`, so confidence thresholds
//!   come from observed acceptance rather than a hand-tuned constant;
//! - the Tier 0 coverage metric, which is a product number: if the grammar
//!   stops absorbing the majority, either the phrasings changed or the grammar
//!   needs extending, and nothing else in the system can tell you which.
//!
//! ## What counts as a correction
//!
//! A row in `corrections` means **the user gave a verdict**, and there is
//! exactly one place today where they do: the disambiguation question. They
//! were shown candidates, ranked, and picked one.
//!
//! - picked the top-ranked candidate → `accepted = 1`. The system's own best
//!   guess was right, and asking cost the user a click it did not need to.
//! - picked any other → `accepted = 0`, and the choice is the correction.
//!
//! That distinction is the whole point. It makes "should we have asked?"
//! measurable, which is what ADR-0005 needs to stop guessing at thresholds.
//!
//! **Silence is not a verdict.** A command that executed without a question
//! writes no correction row, and neither does a question that expired. Counting
//! either as acceptance would fill the table with agreement nobody expressed
//! and calibrate the thresholds against it. What is knowable from a `commands`
//! row with no correction is exactly what happened: nobody was asked, or nobody
//! answered.

use memos_core::{Id, Intent, RoutedCommand, Slots, Tier};
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

use crate::{Db, DbResult};

/// One command, as the log sees it.
#[derive(Debug, Clone, Serialize)]
pub struct LoggedCommand {
    pub id: Id,
    pub transcript: String,
    /// The situation, without its contents — see `Context::digest`.
    pub context: serde_json::Value,
    pub intent: Intent,
    pub slots: Slots,
    pub tier: Tier,
    pub conf_score: Option<f32>,
    pub conf_margin: Option<f32>,
    pub latency_ms: Option<u32>,
    pub created_at: String,
    /// Present only when the user gave a verdict.
    pub correction: Option<Correction>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Correction {
    /// The system's own top choice was what the user wanted.
    pub accepted: bool,
    pub corrected_intent: Option<Intent>,
    pub corrected_slots: Slots,
    pub corrected_at: String,
}

/// How routing is going, over the whole log.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize)]
pub struct RoutingStats {
    pub total: u32,
    /// Routed by the grammar alone. The number ADR-0003 says to watch: below
    /// ~40% of commands, something has drifted.
    pub tier0: u32,
    /// Escalated to the local router and routed there.
    pub tier1: u32,
    /// Neither tier could route it. The grammar's to-do list.
    pub unrouted: u32,
    /// Questions the user answered, either way.
    pub answered: u32,
    /// ...of which the system's own top candidate was already right.
    pub accepted: u32,
}

impl RoutingStats {
    /// Share of commands the grammar handled with no model at all.
    pub fn tier0_coverage(&self) -> Option<f32> {
        (self.total > 0).then(|| self.tier0 as f32 / self.total as f32)
    }
}

impl Db {
    /// Record a routing decision, whichever tier made it and whether or not it
    /// executed.
    ///
    /// Called before execution, not after, and for refusals too. A transcript
    /// neither tier could route is not a failure to be discarded — it is the
    /// single most useful row in the table, because it is the one that says
    /// which phrasing the grammar does not cover yet.
    pub fn log_command(
        &self,
        cmd: &RoutedCommand,
        context: &impl Serialize,
        latency_ms: u32,
    ) -> DbResult<Id> {
        let id = cmd.id;
        let context = serde_json::to_string(context)?;
        let slots = serde_json::to_string(&cmd.slots)?;
        self.with(|c| {
            c.execute(
                "INSERT INTO commands
                    (id, transcript, context, intent, slots, tier,
                     conf_score, conf_margin, latency_ms, created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)",
                params![
                    id.to_string(),
                    cmd.transcript,
                    context,
                    cmd.intent.as_str(),
                    slots,
                    tier_name(cmd.tier),
                    cmd.confidence.score() as f64,
                    cmd.confidence.margin as f64,
                    latency_ms,
                    memos_core::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })?;
        Ok(id)
    }

    /// Record what the user decided about a command they were asked about.
    ///
    /// `accepted` is not "did this work" — it is "was the system's own top
    /// choice the one they picked". A correction that agrees with the ranking
    /// is evidence the question was unnecessary, which is as useful as evidence
    /// that it was needed.
    pub fn record_correction(
        &self,
        command_id: Id,
        accepted: bool,
        corrected_intent: Option<Intent>,
        corrected_slots: &Slots,
    ) -> DbResult<()> {
        let slots = serde_json::to_string(corrected_slots)?;
        self.with(|c| {
            // Replace rather than fail: a question can only be answered once,
            // but a row that somehow arrives twice should settle on the later
            // verdict instead of poisoning the write with an error the capture
            // path would have to handle.
            c.execute(
                "INSERT OR REPLACE INTO corrections
                    (command_id, corrected_intent, corrected_slots, accepted, corrected_at)
                 VALUES (?1,?2,?3,?4,?5)",
                params![
                    command_id.to_string(),
                    corrected_intent.map(|i| i.as_str()),
                    slots,
                    accepted as i32,
                    memos_core::now().to_rfc3339(),
                ],
            )?;
            Ok(())
        })
    }

    /// Historical acceptance rate for one intent — ADR-0005's `prior`.
    ///
    /// `None` until there is enough of it to mean anything. A single answered
    /// question is not a rate, and letting one click move a confidence
    /// threshold would make the system's behaviour feel arbitrary in exactly
    /// the first week the user is deciding whether to trust it.
    pub fn accept_rate(&self, intent: Intent, minimum: u32) -> DbResult<Option<f32>> {
        self.with(|c| {
            let row: Option<(u32, u32)> = c
                .query_row(
                    "SELECT count(*), coalesce(sum(cr.accepted), 0)
                       FROM corrections cr
                       JOIN commands c ON c.id = cr.command_id
                      WHERE c.intent = ?1",
                    params![intent.as_str()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            Ok(match row {
                Some((n, accepted)) if n >= minimum && n > 0 => {
                    Some(accepted as f32 / n as f32)
                }
                _ => None,
            })
        })
    }

    /// Where routing stands, for Settings and for anyone changing the grammar.
    pub fn routing_stats(&self) -> DbResult<RoutingStats> {
        self.with(|c| {
            let mut s = RoutingStats::default();
            let mut q = c.prepare(
                "SELECT tier, intent, count(*) FROM commands GROUP BY tier, intent",
            )?;
            let rows = q.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, u32>(2)?))
            })?;
            for row in rows {
                let (tier, intent, n) = row?;
                s.total += n;
                // An unroutable transcript is recorded against whichever tier
                // last looked at it, so the intent is what separates a decision
                // from a refusal — not the tier.
                if intent == Intent::Unknown.as_str() {
                    s.unrouted += n;
                } else if tier == tier_name(Tier::Grammar) {
                    s.tier0 += n;
                } else if tier == tier_name(Tier::LocalRouter) {
                    s.tier1 += n;
                }
            }
            let (answered, accepted): (u32, u32) = c.query_row(
                "SELECT count(*), coalesce(sum(accepted), 0) FROM corrections",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            s.answered = answered;
            s.accepted = accepted;
            Ok(s)
        })
    }

    /// The whole log, newest first.
    ///
    /// ADR-0006 requires this to be exportable *and* deletable, because it is a
    /// record of what the user said. Both halves are the price of keeping it.
    pub fn export_commands(&self, limit: usize) -> DbResult<Vec<LoggedCommand>> {
        self.with(|c| {
            let mut q = c.prepare(
                "SELECT c.id, c.transcript, c.context, c.intent, c.slots, c.tier,
                        c.conf_score, c.conf_margin, c.latency_ms, c.created_at,
                        cr.accepted, cr.corrected_intent, cr.corrected_slots, cr.corrected_at
                   FROM commands c
                   LEFT JOIN corrections cr ON cr.command_id = c.id
                  ORDER BY c.created_at DESC
                  LIMIT ?1",
            )?;
            let rows = q.query_map(params![limit as i64], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, String>(5)?,
                    r.get::<_, Option<f64>>(6)?,
                    r.get::<_, Option<f64>>(7)?,
                    r.get::<_, Option<u32>>(8)?,
                    r.get::<_, String>(9)?,
                    r.get::<_, Option<i32>>(10)?,
                    r.get::<_, Option<String>>(11)?,
                    r.get::<_, Option<String>>(12)?,
                    r.get::<_, Option<String>>(13)?,
                ))
            })?;

            let mut out = Vec::new();
            for row in rows {
                let (
                    id,
                    transcript,
                    context,
                    intent,
                    slots,
                    tier,
                    score,
                    margin,
                    latency,
                    created_at,
                    accepted,
                    corrected_intent,
                    corrected_slots,
                    corrected_at,
                ) = row?;
                out.push(LoggedCommand {
                    id: Id::parse(&id).unwrap_or_else(|_| Id::new()),
                    transcript,
                    context: serde_json::from_str(&context).unwrap_or(serde_json::Value::Null),
                    intent: Intent::from_name(&intent).unwrap_or(Intent::Unknown),
                    slots: serde_json::from_str(&slots).unwrap_or_default(),
                    tier: tier_from_name(&tier),
                    conf_score: score.map(|v| v as f32),
                    conf_margin: margin.map(|v| v as f32),
                    latency_ms: latency,
                    created_at,
                    correction: accepted.map(|a| Correction {
                        accepted: a != 0,
                        corrected_intent: corrected_intent
                            .as_deref()
                            .and_then(Intent::from_name),
                        corrected_slots: corrected_slots
                            .as_deref()
                            .and_then(|s| serde_json::from_str(s).ok())
                            .unwrap_or_default(),
                        corrected_at: corrected_at.unwrap_or_default(),
                    }),
                });
            }
            Ok(out)
        })
    }

    /// Erase the log. Returns how many commands went with it.
    ///
    /// `corrections` cascades on the foreign key, so one delete is the whole
    /// thing — there is no second table left holding what the user asked to
    /// have forgotten.
    pub fn forget_commands(&self) -> DbResult<u32> {
        self.with(|c| {
            let n: u32 = c.query_row("SELECT count(*) FROM commands", [], |r| r.get(0))?;
            c.execute("DELETE FROM commands", [])?;
            Ok(n)
        })
    }
}

fn tier_name(t: Tier) -> &'static str {
    match t {
        Tier::Grammar => "grammar",
        Tier::LocalRouter => "local_router",
        Tier::Cloud => "cloud",
    }
}

fn tier_from_name(s: &str) -> Tier {
    match s {
        "local_router" => Tier::LocalRouter,
        "cloud" => Tier::Cloud,
        _ => Tier::Grammar,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::Confidence;

    fn db() -> Db {
        Db::open_in_memory().expect("open")
    }

    fn cmd(intent: Intent, tier: Tier, collection: Option<&str>) -> RoutedCommand {
        RoutedCommand {
            id: Id::new(),
            transcript: "save this to react".into(),
            intent,
            slots: Slots {
                collection: collection.map(str::to_string),
                ..Slots::default()
            },
            confidence: Confidence::CERTAIN,
            tier,
            routing_ms: 4,
        }
    }

    #[test]
    fn a_command_round_trips_through_the_log() {
        let db = db();
        let id = db
            .log_command(
                &cmd(Intent::Save, Tier::Grammar, Some("Study/Programming/React")),
                &serde_json::json!({ "active_application": "chrome.exe" }),
                4,
            )
            .unwrap();

        let logged = db.export_commands(10).unwrap();
        assert_eq!(logged.len(), 1);
        let row = &logged[0];
        assert_eq!(row.id, id);
        assert_eq!(row.intent, Intent::Save);
        assert_eq!(row.tier, Tier::Grammar);
        assert_eq!(row.slots.collection.as_deref(), Some("Study/Programming/React"));
        assert_eq!(row.context["active_application"], "chrome.exe");
        // Nobody was asked, so nobody agreed. The absence is the record.
        assert!(row.correction.is_none());
    }

    #[test]
    fn a_verdict_attaches_to_the_command_it_is_about() {
        let db = db();
        let id = db
            .log_command(&cmd(Intent::Save, Tier::Grammar, None), &(), 4)
            .unwrap();
        let chosen = Slots {
            collection: Some("Study/Programming".into()),
            ..Slots::default()
        };
        db.record_correction(id, false, None, &chosen).unwrap();

        let row = &db.export_commands(10).unwrap()[0];
        let correction = row.correction.as_ref().expect("a verdict");
        assert!(!correction.accepted);
        assert_eq!(correction.corrected_slots.collection.as_deref(), Some("Study/Programming"));
    }

    /// The number ADR-0005 wants instead of a hand-tuned constant — and the
    /// refusal to produce one before it means anything.
    #[test]
    fn the_accept_rate_waits_until_it_is_a_rate() {
        let db = db();
        for accepted in [true, true, false, true] {
            let id = db
                .log_command(&cmd(Intent::Save, Tier::Grammar, None), &(), 4)
                .unwrap();
            db.record_correction(id, accepted, None, &Slots::default()).unwrap();
        }
        assert_eq!(db.accept_rate(Intent::Save, 4).unwrap(), Some(0.75));
        // Not enough evidence yet, and a threshold that moves on one click is
        // worse than one that does not move at all.
        assert_eq!(db.accept_rate(Intent::Save, 10).unwrap(), None);
        // A different intent has its own history, and no borrowing.
        assert_eq!(db.accept_rate(Intent::Note, 1).unwrap(), None);
    }

    /// Tier 0 coverage is a product metric (ADR-0003), so it has to count the
    /// refusals too — a grammar that routes everything it accepts and refuses
    /// half of what it hears is not at 100%.
    #[test]
    fn coverage_counts_what_the_grammar_could_not_do() {
        let db = db();
        for _ in 0..6 {
            db.log_command(&cmd(Intent::Save, Tier::Grammar, None), &(), 4).unwrap();
        }
        for _ in 0..2 {
            db.log_command(&cmd(Intent::Save, Tier::LocalRouter, None), &(), 600)
                .unwrap();
        }
        for _ in 0..2 {
            db.log_command(&cmd(Intent::Unknown, Tier::LocalRouter, None), &(), 600)
                .unwrap();
        }

        let s = db.routing_stats().unwrap();
        assert_eq!(s.total, 10);
        assert_eq!(s.tier0, 6);
        assert_eq!(s.tier1, 2);
        assert_eq!(s.unrouted, 2);
        assert_eq!(s.tier0_coverage(), Some(0.6));
    }

    /// The other half of ADR-0006's bargain: a log the user cannot erase is one
    /// they were never really asked about.
    #[test]
    fn forgetting_takes_the_verdicts_with_it() {
        let db = db();
        let id = db
            .log_command(&cmd(Intent::Save, Tier::Grammar, None), &(), 4)
            .unwrap();
        db.record_correction(id, true, None, &Slots::default()).unwrap();

        assert_eq!(db.forget_commands().unwrap(), 1);
        assert!(db.export_commands(10).unwrap().is_empty());
        assert_eq!(db.routing_stats().unwrap(), RoutingStats::default());
        // The cascade is the point: no orphaned verdict for a command that no
        // longer exists.
        assert_eq!(db.accept_rate(Intent::Save, 1).unwrap(), None);
    }
}
