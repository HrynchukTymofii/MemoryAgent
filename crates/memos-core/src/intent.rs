use serde::{Deserialize, Serialize};

use crate::{Id, Timestamp};

/// The complete action space. This enum is the single source of truth: the GBNF
/// grammar handed to the local router is generated from it, so the model
/// structurally cannot emit an intent that does not appear here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Intent {
    /// Capture the current context into memory.
    Save,
    /// A standalone note with no source attached.
    Note,
    /// Retrieve by meaning, keyword, time or relationship.
    Search,
    /// Browse a collection or tag.
    Show,
    /// Reopen an original source.
    Open,
    /// Refile an existing item.
    Move,
    /// Attach a tag.
    Tag,
    /// Create a task, optionally linked to an item.
    Task,
    /// Make a new collection, optionally under an existing one.
    ///
    /// Filing something somewhere that does not exist yet is one command to a
    /// person and two to the store, so the action has to exist for the second
    /// half to be sayable at all.
    CreateCollection,
    /// Create a time-based reminder.
    Reminder,
    /// Filesystem work: create, move, rename, download.
    FileOperation,
    /// Multi-step external research. Tier 2.
    Research,
    /// Answer grounded in the user's own material. Tier 2.
    Explain,
    /// Schedule and prioritise. Tier 2.
    Plan,
    /// Generate recall exercises. Tier 2.
    Learn,
    /// Take back the last thing that happened.
    ///
    /// Not in [`Intent::ALL`], and so never in the Tier 1 grammar. Undo is a
    /// fixed phrase the grammar recognises exactly; a model that could *guess*
    /// it would be a model that can delete a memory on a misheard word.
    Undo,
    /// Nothing actionable was recognised.
    Unknown,
}

impl Intent {
    /// Every intent that has a name, in a stable order.
    ///
    /// This is the round-trip set — what `from_name` will parse back out of a
    /// log or a model's JSON. It is **not** what the router may emit: that is
    /// `memos_llm::grammar::ROUTABLE`, a much smaller list, and the difference
    /// is what keeps `UNDO` parseable in the correction log while remaining
    /// impossible for a model to produce.
    pub const ALL: &'static [Intent] = &[
        Intent::Save,
        Intent::Note,
        Intent::Search,
        Intent::Show,
        Intent::Open,
        Intent::Move,
        Intent::Tag,
        Intent::Task,
        Intent::CreateCollection,
        Intent::Reminder,
        Intent::FileOperation,
        Intent::Research,
        Intent::Explain,
        Intent::Plan,
        Intent::Learn,
        Intent::Undo,
    ];

    pub fn as_str(&self) -> &'static str {
        match self {
            Intent::Save => "SAVE",
            Intent::Note => "NOTE",
            Intent::Search => "SEARCH",
            Intent::Show => "SHOW",
            Intent::Open => "OPEN",
            Intent::Move => "MOVE",
            Intent::Tag => "TAG",
            Intent::Task => "TASK",
            Intent::CreateCollection => "CREATE_COLLECTION",
            Intent::Reminder => "REMINDER",
            Intent::FileOperation => "FILE_OPERATION",
            Intent::Research => "RESEARCH",
            Intent::Explain => "EXPLAIN",
            Intent::Plan => "PLAN",
            Intent::Learn => "LEARN",
            Intent::Undo => "UNDO",
            Intent::Unknown => "UNKNOWN",
        }
    }

    /// The inverse of [`Intent::as_str`], case-insensitively.
    ///
    /// Named `from_name` rather than `from_str` so it cannot be mistaken for
    /// the standard trait method, which returns a `Result` and is reached
    /// through an import.
    ///
    /// Liberal about case on purpose. `as_str` shouts (`SAVE`) because that is
    /// how intents read in a log, while anything JSON-shaped writes them
    /// lowercase — the router's grammar among them. Accepting both means the
    /// two conventions can never drift into a silent parse failure.
    pub fn from_name(s: &str) -> Option<Intent> {
        let s = s.trim();
        Intent::ALL
            .iter()
            .copied()
            .find(|i| i.as_str().eq_ignore_ascii_case(s))
    }

    /// The lowercase form, as it appears in JSON and in the router grammar.
    pub fn token(&self) -> String {
        self.as_str().to_ascii_lowercase()
    }

    /// Whether executing this intent can destroy or overwrite user data.
    ///
    /// ADR-0005: irreversible actions confirm at *any* confidence. This is the
    /// predicate that enforces it, so the rule cannot be forgotten at a call site.
    pub fn is_irreversible(&self) -> bool {
        matches!(self, Intent::FileOperation)
    }

    /// Whether this intent needs cloud reasoning. Tier 2 work is allowed to take
    /// seconds; everything else is on the sub-second capture path.
    pub fn requires_reasoning(&self) -> bool {
        matches!(
            self,
            Intent::Research | Intent::Explain | Intent::Plan | Intent::Learn
        )
    }
}

/// Which tier produced a routing decision. Recorded on every command so Tier 0
/// coverage is a measurable product metric rather than an assumption.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Deterministic grammar. No model ran.
    Grammar,
    /// Local constrained-decoding router.
    LocalRouter,
    /// Cloud reasoning model.
    Cloud,
}

/// Extracted parameters. All optional — the router fills what the command
/// actually specified and nothing more.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Slots {
    pub collection: Option<String>,
    pub title: Option<String>,
    pub query: Option<String>,
    pub tags: Vec<String>,
    pub due_at: Option<Timestamp>,
    pub target_path: Option<String>,
    pub source_url: Option<String>,
}

/// Confidence is computed by us, never self-reported by a model (ADR-0005).
///
/// `margin` is the load-bearing field: two candidates at 0.51 and 0.49 are
/// ambiguous because they are *indistinguishable*, not because either score is
/// low. A threshold on `score` alone would miss exactly that case.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Confidence {
    /// Mean token probability across the slot tokens of the constrained decode.
    pub logprob: f32,
    /// score(top-1) − score(top-2) among candidate resolutions.
    pub margin: f32,
    /// Historical acceptance rate for this intent, for this user.
    pub prior: f32,
}

impl Confidence {
    /// Deterministic Tier 0 match: the grammar either parsed or it did not.
    pub const CERTAIN: Confidence = Confidence {
        logprob: 1.0,
        margin: 1.0,
        prior: 1.0,
    };

    /// Combined score. Weights are placeholders until calibrated against the
    /// correction log — see ADR-0005 and ADR-0006.
    pub fn score(&self) -> f32 {
        (0.4 * self.logprob + 0.4 * self.margin + 0.2 * self.prior).clamp(0.0, 1.0)
    }

    /// Whether to execute silently or ask a narrow question.
    ///
    /// The margin gate is separate and stricter on purpose: a confident-looking
    /// score with two indistinguishable destinations must still ask.
    ///
    /// **Not yet consulted on any path.** The constants below were picked
    /// before there was anything to pick them from, and the first real
    /// measurements say they are wrong — see
    /// `the_threshold_has_not_earned_its_constants`. ADR-0005 is explicit that
    /// thresholds come from logged outcomes rather than hand-tuning, so this
    /// waits for the correction log to hold enough of them.
    pub fn should_execute(&self, intent: Intent) -> bool {
        if intent.is_irreversible() {
            return false;
        }
        self.margin >= 0.15 && self.score() >= 0.70
    }
}

/// A transcript that has been routed but not yet executed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutedCommand {
    /// Identity, assigned where the routing happens rather than where it is
    /// stored. One command is one row in the correction log, one set of events
    /// in the audit, and one thing an undo can point back at — and all three
    /// have to agree on which command they mean.
    pub id: Id,
    pub transcript: String,
    pub intent: Intent,
    pub slots: Slots,
    pub confidence: Confidence,
    pub tier: Tier,
    /// Transcript-to-decision time. Feeds the latency budget in §4.
    pub routing_ms: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Measured, not invented: these are what the Tier 1 constrained decode
    /// reported for nine ordinary commands, all of which it routed **correctly**
    /// (`cargo run -p memos-llm --features local --example route_check`).
    ///
    /// They are here because they say something the code cannot say for itself:
    /// the 0.70 gate in `should_execute` would refuse one of the nine, and that
    /// one was right. A second scrapes through at 0.72. That is not a case for
    /// nudging the constant — nine correct answers say nothing about where the
    /// wrong ones sit — it is the reason
    /// ADR-0005 insists the thresholds be calibrated from the correction log
    /// instead of chosen. This test is what will notice if someone changes them
    /// before that data exists.
    const MEASURED: &[(&str, f32, f32)] = &[
        ("save this page to the react programming database", 1.00, 0.99),
        ("add this to my react notes", 1.00, 0.98),
        ("stick this in with the python stuff", 1.00, 1.00),
        ("file this under job applications", 1.00, 1.00),
        ("keep this for the garden", 0.90, 0.21),
        ("I want to remember this for my interviews", 0.99, 0.94),
        ("what did I read about hooks last week", 1.00, 1.00),
        ("pull up that typescript thing", 0.99, 0.98),
        ("jot down that the bins go out on tuesday", 0.96, 0.34),
    ];

    #[test]
    fn the_threshold_has_not_earned_its_constants() {
        let refused: Vec<&str> = MEASURED
            .iter()
            .filter(|(_, logprob, margin)| {
                !Confidence {
                    logprob: *logprob,
                    margin: *margin,
                    prior: 1.0,
                }
                .should_execute(Intent::Save)
            })
            .map(|(phrase, _, _)| *phrase)
            .collect();

        assert_eq!(
            refused,
            vec!["keep this for the garden"],
            "the measured distribution moved; recheck the gate against it"
        );
    }

    /// The same numbers, read the other way: free text is not a choice between
    /// alternatives, so its margins are narrow by construction. "the bins go out
    /// on tuesday" has many plausible continuations at every token and no wrong
    /// one; a collection path has a handful and exactly one right one.
    ///
    /// A single global margin threshold therefore cannot be right for both —
    /// which is why ADR-0005 asks for per-intent thresholds, and why this stays
    /// an observation in a test rather than a constant somewhere.
    #[test]
    fn free_text_margins_are_narrower_than_a_choice_between_collections() {
        let note = MEASURED.iter().find(|(p, ..)| p.starts_with("jot down")).unwrap();
        let filed = MEASURED.iter().find(|(p, ..)| p.starts_with("file this")).unwrap();
        assert!(note.2 < filed.2, "a title should be a narrower call than a destination");
        // ...and yet the model was confident about the tokens themselves.
        assert!(note.1 > 0.9);
    }

    #[test]
    fn every_intent_survives_a_round_trip_through_its_name() {
        // The router parses an intent back out of JSON. A variant whose name
        // does not round-trip would be routable by the grammar and rejected by
        // the parser - visible only as commands that mysteriously never run.
        for intent in Intent::ALL {
            assert_eq!(Intent::from_name(intent.as_str()), Some(*intent));
            assert_eq!(Intent::from_name(&intent.token()), Some(*intent));
        }
        assert_eq!(Intent::from_name("  save  "), Some(Intent::Save));
        assert_eq!(Intent::from_name("file_operation"), Some(Intent::FileOperation));
        assert_eq!(Intent::from_name("nonsense"), None);
    }

    #[test]
    fn irreversible_intents_never_auto_execute() {
        // Even a perfect score must not auto-run a destructive file operation.
        assert!(!Confidence::CERTAIN.should_execute(Intent::FileOperation));
        assert!(Confidence::CERTAIN.should_execute(Intent::Save));
    }

    #[test]
    fn low_margin_blocks_execution_despite_high_score() {
        // The "put this with the house stuff" case: both candidates plausible.
        let c = Confidence {
            logprob: 0.95,
            margin: 0.02,
            prior: 0.9,
        };
        assert!(
            !c.should_execute(Intent::Save),
            "indistinguishable candidates must ask, not guess"
        );
    }
}
