use serde::{Deserialize, Serialize};

use crate::Timestamp;

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
    /// Nothing actionable was recognised.
    Unknown,
}

impl Intent {
    /// Every variant, in a stable order. Used to generate the router grammar.
    pub const ALL: &'static [Intent] = &[
        Intent::Save,
        Intent::Note,
        Intent::Search,
        Intent::Show,
        Intent::Open,
        Intent::Move,
        Intent::Tag,
        Intent::Task,
        Intent::Reminder,
        Intent::FileOperation,
        Intent::Research,
        Intent::Explain,
        Intent::Plan,
        Intent::Learn,
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
            Intent::Reminder => "REMINDER",
            Intent::FileOperation => "FILE_OPERATION",
            Intent::Research => "RESEARCH",
            Intent::Explain => "EXPLAIN",
            Intent::Plan => "PLAN",
            Intent::Learn => "LEARN",
            Intent::Unknown => "UNKNOWN",
        }
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
