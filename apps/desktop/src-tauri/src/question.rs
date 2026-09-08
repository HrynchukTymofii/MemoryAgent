//! A question the router could not answer for itself.
//!
//! Tier 0 refuses to guess a destination when two candidates are
//! indistinguishable (ADR-0005). That refusal is only worth anything if the
//! user can *answer* — a question with no way to reply is just a slower way of
//! failing, which is exactly what it was until now.
//!
//! The command is kept whole while it waits. Re-routing the transcript on the
//! answer would run the grammar a second time and could land somewhere else
//! entirely; what is stored here is the decision already made, missing one slot.

use std::time::{Duration, Instant};

use memos_agent::Candidate;
use memos_context::Context;
use memos_core::{Confidence, Id, Intent, RoutedCommand, Slots, Tier};
use parking_lot::Mutex;

/// How long an unanswered question stands.
///
/// Long enough to read three options and move a mouse; short enough that a
/// question you walked away from cannot be answered by accident by the next
/// person to touch the machine. When it lapses nothing was captured — the same
/// outcome as saying nothing at all.
pub const LIFETIME: Duration = Duration::from_secs(12);

/// What is being waited on.
pub struct Question {
    /// The row this question is about, in the correction log. The answer is a
    /// verdict on a prediction that was already recorded — not a new command —
    /// so it has to be able to find it again (ADR-0006).
    pub command_id: Id,
    pub intent: Intent,
    pub transcript: String,
    pub slots: Slots,
    pub context: Context,
    pub candidates: Vec<Candidate>,
    asked_at: Instant,
}

impl Question {
    fn expired(&self) -> bool {
        self.asked_at.elapsed() > LIFETIME
    }
}

/// A question the user settled: what to run, and what that says about the
/// prediction it replaced.
pub struct Answered {
    pub command_id: Id,
    pub accepted: bool,
    pub command: RoutedCommand,
    pub context: Context,
}

/// The one outstanding question, if any.
///
/// One, not a queue: a second capture supersedes the first. Stacking questions
/// would mean answering them in an order the user cannot see.
#[derive(Default)]
pub struct Pending(Mutex<Option<Question>>);

impl Pending {
    pub fn ask(
        &self,
        command_id: Id,
        intent: Intent,
        transcript: String,
        slots: Slots,
        context: Context,
        candidates: Vec<Candidate>,
    ) {
        *self.0.lock() = Some(Question {
            command_id,
            intent,
            transcript,
            slots,
            context,
            candidates,
            asked_at: Instant::now(),
        });
    }

    pub fn clear(&self) {
        *self.0.lock() = None;
    }

    /// Take the question and turn choice `index` into a command to execute.
    ///
    /// Taking rather than reading: an answer consumes the question, so a double
    /// click cannot save the same capture twice.
    pub fn answer(&self, index: usize) -> Option<Answered> {
        let question = self.0.lock().take()?;
        if question.expired() {
            // Nothing is recorded. A question nobody answered is not a
            // rejection, and logging it as one would calibrate the confidence
            // thresholds against a verdict the user never gave (ADR-0006).
            tracing::info!("question expired before it was answered");
            return None;
        }
        let chosen = question.candidates.get(index)?;

        let mut slots = question.slots.clone();
        slots.collection = Some(chosen.path.clone());

        Some(Answered {
            command_id: question.command_id,
            // Index 0 is what the ranking already believed. Picking it means
            // the question was unnecessary — which is exactly as useful to
            // record as picking anything else (ADR-0005).
            accepted: index == 0,
            command: RoutedCommand {
                transcript: question.transcript,
                intent: question.intent,
                slots,
                // The user picked it. There is nothing left to be uncertain
                // about, and recording anything less would misreport why this
                // command executed when the correction log reads it back.
                confidence: Confidence::CERTAIN,
                tier: Tier::Grammar,
                routing_ms: 0,
            },
            context: question.context,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidates() -> Vec<Candidate> {
        vec![
            Candidate { path: "Study/Programming/React".into(), score: 0.28 },
            Candidate { path: "Study/Programming".into(), score: 0.14 },
        ]
    }

    fn pending() -> Pending {
        let p = Pending::default();
        p.ask(
            Id::new(),
            Intent::Save,
            "save this to the react programming database".into(),
            Slots::default(),
            Context::default(),
            candidates(),
        );
        p
    }

    #[test]
    fn answering_fills_the_slot_that_was_missing() {
        let p = pending();
        let a = p.answer(0).expect("answered");
        assert_eq!(a.command.intent, Intent::Save);
        assert_eq!(a.command.slots.collection.as_deref(), Some("Study/Programming/React"));
        assert_eq!(a.command.transcript, "save this to the react programming database");
    }

    /// The verdict the correction log stores. Picking the top candidate says
    /// the ranking was right and the question was the mistake; picking any
    /// other says the opposite, and both are worth knowing.
    #[test]
    fn which_option_was_picked_is_the_verdict() {
        assert!(pending().answer(0).expect("answered").accepted);
        let second = pending().answer(1).expect("answered");
        assert!(!second.accepted);
        assert_eq!(second.command.slots.collection.as_deref(), Some("Study/Programming"));
    }

    #[test]
    fn a_question_can_only_be_answered_once() {
        // Two clicks on one option must not save the capture twice.
        let p = pending();
        assert!(p.answer(0).is_some());
        assert!(p.answer(0).is_none());
    }

    #[test]
    fn an_option_that_does_not_exist_answers_nothing() {
        let p = pending();
        assert!(p.answer(9).is_none());
        // ...and the question is consumed rather than left half-answered.
        assert!(p.answer(0).is_none());
    }

    #[test]
    fn answering_nothing_is_answering_nothing() {
        let p = Pending::default();
        assert!(p.answer(0).is_none());
    }
}
