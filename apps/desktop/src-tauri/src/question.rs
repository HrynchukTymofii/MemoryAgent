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
use memos_core::{Confidence, Intent, RoutedCommand, Slots, Tier};
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

/// The one outstanding question, if any.
///
/// One, not a queue: a second capture supersedes the first. Stacking questions
/// would mean answering them in an order the user cannot see.
#[derive(Default)]
pub struct Pending(Mutex<Option<Question>>);

impl Pending {
    pub fn ask(
        &self,
        intent: Intent,
        transcript: String,
        slots: Slots,
        context: Context,
        candidates: Vec<Candidate>,
    ) {
        *self.0.lock() = Some(Question {
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
    pub fn answer(&self, index: usize) -> Option<(RoutedCommand, Context)> {
        let question = self.0.lock().take()?;
        if question.expired() {
            tracing::info!("question expired before it was answered");
            return None;
        }
        let chosen = question.candidates.get(index)?;

        let mut slots = question.slots.clone();
        slots.collection = Some(chosen.path.clone());

        Some((
            RoutedCommand {
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
            question.context,
        ))
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
        let (cmd, _) = p.answer(0).expect("answered");
        assert_eq!(cmd.intent, Intent::Save);
        assert_eq!(cmd.slots.collection.as_deref(), Some("Study/Programming/React"));
        assert_eq!(cmd.transcript, "save this to the react programming database");
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
