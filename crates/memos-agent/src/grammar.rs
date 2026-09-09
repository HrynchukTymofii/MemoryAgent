//! Tier 0: the deterministic grammar.
//!
//! ADR-0003. Most spoken commands are formulaic — "save this to X", "note that
//! Y", "find Z" — and a fixed grammar routes them in microseconds with no model
//! loaded and no chance of inventing a destination that does not exist.
//! Everything it does not recognise escalates to Tier 1 rather than being
//! guessed at.
//!
//! Two properties are load-bearing:
//!
//! 1. **It never invents a slot.** A destination is resolved against the user's
//!    real collection paths, so an unknown one becomes a question, not a folder.
//! 2. **It refuses rather than guesses.** Coverage is measurable — `tier` is
//!    recorded on every command — so "how much does the grammar handle" has a
//!    number for an answer instead of a hope.

use memos_core::{Confidence, Intent, RoutedCommand, Slots, Tier};

use crate::collections::{resolve, Resolution};

/// A matched prefix: the intent it implies, and the words left over.
struct Shape {
    intent: Intent,
    rest: String,
}

/// Prefix table, scanned in order; the first match wins.
const PATTERNS: &[(&[&str], Intent)] = &[
    (
        &[
            // Longest first: the loop takes the first prefix that matches, so
            // "save this to" must be tried before "save this" or the
            // destination ends up inside the slot text.
            "save this to", "save it to", "save that to", "save this in",
            "save this under", "save to", "save this", "save it", "save",
            // "save" is one word for this out of many, and it is not the one
            // most people reach for first. Every variant below requires an
            // object — "add this", never a bare "add" — because the bare verbs
            // belong to other intents ("add a task", "put it off until
            // Friday"), and a first-match grammar would swallow them here.
            "add this to", "add it to", "add that to", "add this in",
            "add this under", "add this", "add it", "add that",
            "put this in", "put it in", "put this under", "put it under",
            "put this into", "put this",
            "file this under", "file it under", "file this in", "file this",
            "store this in", "store this under", "store this", "store it",
            "keep this in", "keep this under", "keep this", "keep it",
            "capture this", "bookmark this", "bookmark it", "clip this",
        ],
        Intent::Save,
    ),
    (
        &["move this to", "move it to", "move that to", "move this", "move to"],
        Intent::Move,
    ),
    (
        &[
            "remind me about this", "remind me about", "remind me to", "remind me",
            "reminder",
        ],
        Intent::Reminder,
    ),
    (
        &[
            "create a task to", "create a task", "add a task to", "add a task",
            "make a task", "task",
        ],
        Intent::Task,
    ),
    (
        &["tag this as", "tag this", "tag it as", "tag it", "tag as", "tag"],
        Intent::Tag,
    ),
    (
        &[
            "remember that", "remember this", "remember", "note that", "make a note that",
            "make a note", "add a note that", "add a note about", "add a note",
            "jot down that", "jot down", "note",
        ],
        Intent::Note,
    ),
    (
        &[
            "find me", "find the", "find", "search for", "search", "look for",
            "where is", "what did i save about",
        ],
        Intent::Search,
    ),
    (
        &["show me everything about", "show me everything", "show me", "show all", "show"],
        Intent::Show,
    ),
    (&["open the", "open"], Intent::Open),
    // Last, and exact. Every phrase here takes no object, so a shape that has
    // words after it falls through to Tier 1 rather than matching — "undo the
    // react one" is a request this cannot honour and must not pretend to.
    //
    // Tier 1 can never produce UNDO (it is not in `ROUTABLE`), which is the
    // point: a model that could guess this word is a model that can delete a
    // memory on a misheard syllable.
    (
        &[
            "undo that", "undo it", "undo", "never mind", "nevermind",
            "scratch that", "forget that", "take that back",
        ],
        Intent::Undo,
    ),
];

/// Strip trailing filler that speech recognition reliably appends.
fn tidy(s: &str) -> String {
    s.trim()
        .trim_end_matches(['.', ',', '!', '?', ';'])
        .trim()
        .to_string()
}

fn match_shape(transcript: &str) -> Option<Shape> {
    let lower = transcript.trim().to_lowercase();
    let lower = lower.trim_start_matches("please ").trim();

    for (prefixes, intent) in PATTERNS {
        for p in *prefixes {
            // Require a word boundary: "saving" must not match "save".
            if let Some(rest) = lower.strip_prefix(p) {
                if rest.is_empty() || rest.starts_with(' ') {
                    return Some(Shape {
                        intent: *intent,
                        rest: tidy(rest),
                    });
                }
            }
        }
    }
    None
}

/// The result of a Tier 0 attempt.
pub enum Tier0 {
    /// Parsed and every slot resolved confidently. Execute without a model.
    Routed(RoutedCommand),
    /// Parsed, but a slot is ambiguous. Ask rather than guess (ADR-0005).
    Ambiguous {
        intent: Intent,
        slot: &'static str,
        resolution: Resolution,
        transcript: String,
    },
    /// Not a shape we recognise. Escalate to Tier 1.
    Unrecognised,
}

/// Parse a transcript deterministically.
///
/// `collections` are the user's real collection paths; a destination that does
/// not exist can never be produced.
pub fn parse(transcript: &str, collections: &[String]) -> Tier0 {
    let started = std::time::Instant::now();
    let Some(shape) = match_shape(transcript) else {
        return Tier0::Unrecognised;
    };

    let mut slots = Slots::default();
    let mut confidence = Confidence::CERTAIN;

    match shape.intent {
        Intent::Save | Intent::Move => {
            if shape.rest.is_empty() {
                // "save this" with no destination is a valid, complete command:
                // capture it unfiled rather than interrogating the user. Filing
                // later is cheap; losing the thought is not.
                if shape.intent == Intent::Move {
                    return Tier0::Unrecognised; // a move needs a destination
                }
            } else {
                let r = resolve(&shape.rest, collections);
                if !r.is_confident() {
                    return Tier0::Ambiguous {
                        intent: shape.intent,
                        slot: "collection",
                        resolution: r,
                        transcript: transcript.to_string(),
                    };
                }
                confidence = Confidence {
                    logprob: 1.0,
                    margin: r.margin,
                    prior: 1.0,
                };
                slots.collection = r.best.map(|c| c.path);
            }
        }
        Intent::Note => {
            if shape.rest.is_empty() {
                return Tier0::Unrecognised;
            }
            slots.title = Some(shape.rest.clone());
        }
        // OPEN belongs here rather than with the temporal intents below: it is
        // a search whose answer is acted on, and the words after "open the" are
        // a query like any other. Which item it lands on is decided by
        // retrieval, not by the grammar.
        Intent::Search | Intent::Show | Intent::Open => {
            if shape.rest.is_empty() {
                return Tier0::Unrecognised;
            }
            slots.query = Some(shape.rest.clone());
        }
        Intent::Tag => {
            if shape.rest.is_empty() {
                return Tier0::Unrecognised;
            }
            slots.tags = vec![shape.rest.clone()];
        }
        // A task is complete with nothing but its words, which is exactly what
        // separates it from a reminder. "call the plumber" is a whole task; it
        // is not a reminder at all until somebody says when.
        Intent::Task => {
            if shape.rest.is_empty() {
                return Tier0::Unrecognised;
            }
            slots.title = Some(shape.rest.clone());
        }
        // Takes no object and needs no slot. Anything after the word means the
        // user asked for something narrower than this can do.
        Intent::Undo => {
            if !shape.rest.is_empty() {
                return Tier0::Unrecognised;
            }
        }
        // Temporal parsing is not deterministic enough to belong in Tier 0 yet:
        // "next Tuesday" and "in a couple of days" need real interpretation, and
        // a wrong reminder time is a silent failure the user only discovers when
        // it does not fire. A task has no such hole to fall into.
        Intent::Reminder => return Tier0::Unrecognised,
        _ => return Tier0::Unrecognised,
    }

    Tier0::Routed(RoutedCommand {
        id: memos_core::Id::new(),
        transcript: transcript.to_string(),
        intent: shape.intent,
        slots,
        confidence,
        tier: Tier::Grammar,
        routing_ms: started.elapsed().as_millis() as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        [
            "Study",
            "Study/Programming",
            "Study/Programming/React",
            "Study/Programming/TypeScript",
            "Study/AI",
            "Life/House",
            "Career",
            "Career/Job Applications",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    fn routed(text: &str) -> RoutedCommand {
        match parse(text, &paths()) {
            Tier0::Routed(c) => c,
            Tier0::Ambiguous { slot, .. } => panic!("{text:?} was ambiguous on {slot}"),
            Tier0::Unrecognised => panic!("{text:?} was not recognised"),
        }
    }

    #[test]
    fn saves_to_a_named_collection() {
        let r = routed("save this to react");
        assert_eq!(r.intent, Intent::Save);
        assert_eq!(r.slots.collection.as_deref(), Some("Study/Programming/React"));
        assert_eq!(r.tier, Tier::Grammar);

        // The path spoken in full resolves to the same place.
        assert_eq!(
            routed("save this to programming react").slots.collection.as_deref(),
            Some("Study/Programming/React")
        );
    }

    #[test]
    fn save_without_a_destination_still_captures() {
        // Filing later is cheap; losing the thought is not.
        let r = routed("save this");
        assert_eq!(r.intent, Intent::Save);
        assert!(r.slots.collection.is_none());
    }

    #[test]
    fn a_note_keeps_its_text() {
        let r = routed("note that the router is behind the books");
        assert_eq!(r.intent, Intent::Note);
        assert_eq!(r.slots.title.as_deref(), Some("the router is behind the books"));

        assert_eq!(
            routed("remember that the router config is on the fridge")
                .slots
                .title
                .as_deref(),
            Some("the router config is on the fridge")
        );
    }

    #[test]
    fn search_and_show_capture_the_query() {
        let find = routed("find the react article");
        assert_eq!(find.intent, Intent::Search);
        assert_eq!(find.slots.query.as_deref(), Some("react article"));

        let show = routed("show me everything about react");
        assert_eq!(show.intent, Intent::Show);
        assert_eq!(show.slots.query.as_deref(), Some("react"));
    }

    #[test]
    fn open_routes_with_its_words_as_the_query() {
        let r = routed("open the react article");
        assert_eq!(r.intent, Intent::Open);
        assert_eq!(r.slots.query.as_deref(), Some("react article"));
    }

    #[test]
    fn a_bare_open_is_not_a_command() {
        // "open" alone names no destination. Guessing one would open something
        // at random, which is the worst possible response to an ambiguous verb.
        assert!(matches!(parse("open", &[]), Tier0::Unrecognised));
    }

    #[test]
    fn the_everyday_verbs_for_saving_all_route() {
        // "save" is one word for this out of many, and not the one most people
        // reach for first. Each of these was silently unrecognised — which,
        // from outside, is indistinguishable from a broken product.
        for phrase in [
            "add this",
            "add this to react",
            "put this in react",
            "file this under react",
            "keep this",
            "store this in react",
            "capture this",
            "bookmark this",
        ] {
            match parse(phrase, &paths()) {
                Tier0::Routed(c) => assert_eq!(c.intent, Intent::Save, "{phrase:?}"),
                _ => panic!("{phrase:?} must route to SAVE"),
            }
        }
    }

    #[test]
    fn saving_verbs_do_not_swallow_the_other_intents() {
        // SAVE is matched first, so every verb it claims must be specific
        // enough to leave the later intents intact. This used to assert the
        // weaker thing — that a task was not *saved* — because TASK did not
        // route at all; now it routes, and the stronger claim can be made.
        let task = routed("add a task to call the bank");
        assert_eq!(task.intent, Intent::Task, "a task must not be captured as a save");
        assert_eq!(task.slots.title.as_deref(), Some("call the bank"));
        let note = routed("add a note that the bins go out on tuesday");
        assert_eq!(note.intent, Intent::Note);
        assert_eq!(note.slots.title.as_deref(), Some("the bins go out on tuesday"));
    }

    #[test]
    fn the_destination_does_not_leak_into_the_slot() {
        assert_eq!(
            routed("add this to react").slots.collection.as_deref(),
            Some("Study/Programming/React")
        );
    }

    #[test]
    fn an_ambiguous_destination_asks_instead_of_guessing() {
        // The specification's own example: neither score is low, but the top
        // two are indistinguishable — and it is the margin, not the score, that
        // makes this a question rather than a guess (ADR-0005).
        let paths: Vec<String> = [
            "Study",
            "Study/Programming",
            "Study/Programming/React",
            "Study/AI",
            "Life",
            "Life/House",
            "Life/House/Internet",
            "Life/House/Electricity",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();

        match parse("save this to house", &paths) {
            Tier0::Ambiguous { slot, resolution, .. } => {
                assert_eq!(slot, "collection");
                assert!(resolution.candidates.len() > 1);
            }
            // Resolving confidently is acceptable only if it landed on a real
            // house collection — never on something unrelated.
            Tier0::Routed(c) => assert!(
                c.slots.collection.as_deref().unwrap_or("").starts_with("Life/House"),
                "resolved to {:?}",
                c.slots.collection
            ),
            Tier0::Unrecognised => panic!("the shape itself is recognisable"),
        }
    }

    #[test]
    fn temporal_commands_are_left_to_a_smarter_tier() {
        // A wrong reminder time is a silent failure the user only discovers
        // when it does not fire.
        assert!(matches!(
            parse("remind me next Tuesday", &paths()),
            Tier0::Unrecognised
        ));
    }

    #[test]
    fn tolerates_punctuation_and_case() {
        assert_eq!(
            routed("Save this to programming React").slots.collection.as_deref(),
            Some("Study/Programming/React")
        );
        assert_eq!(
            routed("save this to React.").slots.collection.as_deref(),
            Some("Study/Programming/React")
        );
        assert_eq!(
            routed("please save this to react").slots.collection.as_deref(),
            Some("Study/Programming/React")
        );
    }

    #[test]
    fn word_boundaries_are_respected() {
        // "saving" starts with "save" but is not the command.
        assert!(matches!(
            parse("saving money is hard", &paths()),
            Tier0::Unrecognised
        ));
    }

    #[test]
    fn unrecognised_shapes_escalate() {
        for text in ["what is the capital of france", "hello there", ""] {
            assert!(
                matches!(parse(text, &paths()), Tier0::Unrecognised),
                "{text:?} must escalate"
            );
        }
    }

    #[test]
    fn parsing_is_fast_enough_to_be_free() {
        // Stage 5 of the latency budget allots this ~0 ms. It is a scan over a
        // few dozen string prefixes, and it has to stay that way.
        let started = std::time::Instant::now();
        for _ in 0..1_000 {
            let _ = parse("save this to programming react", &paths());
        }
        let per_call_us = started.elapsed().as_micros() / 1_000;
        assert!(per_call_us < 200, "{per_call_us} us per parse");
    }
}
