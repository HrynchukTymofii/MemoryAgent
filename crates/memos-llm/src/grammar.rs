//! GBNF: the grammar the router is not allowed to leave.
//!
//! ADR-0003 makes one structural claim, and this module is where it either
//! holds or does not: **a sub-billion-parameter model cannot emit invalid JSON,
//! an unknown intent, or a collection that does not exist.** Not because it was
//! asked nicely in a prompt, but because llama.cpp masks every token that would
//! leave the grammar before sampling. A 0.6B model asked politely will invent a
//! folder called "Work Stuff" sooner or later; one decoding under this grammar
//! cannot, no matter how confused it is.
//!
//! Two consequences worth stating:
//!
//! 1. **The grammar is regenerated from the user's real collections.** It is
//!    data, not a constant, and a stale one silently blocks a valid
//!    destination — so it is rebuilt whenever the collection set changes.
//! 2. **Slots are constrained per intent.** A `search` cannot carry a
//!    collection and a `save` cannot carry a query, because the alternatives
//!    are written per intent rather than as one bag of optional keys. That
//!    removes a whole class of nonsense the parser would otherwise have to
//!    reject after the fact.

use memos_core::Intent;

/// The intents Tier 1 is allowed to produce.
///
/// Not every variant of [`Intent`]: the reasoning intents belong to Tier 2, and
/// offering them here would let a small model route a research question to
/// itself. `Unknown` is absent on purpose — a model that cannot classify
/// something should say the closest thing it can, and the confidence score is
/// what decides whether we act on it (ADR-0005).
pub const ROUTABLE: &[Intent] = &[
    Intent::Save,
    Intent::Note,
    Intent::Search,
    Intent::Show,
    Intent::Open,
    Intent::Move,
    Intent::Tag,
];

/// Longest free-text slot the model may emit, in characters.
///
/// A bound rather than a nicety: an unbounded string rule lets a looping model
/// generate until the context runs out, and that turns a 200 ms route into a
/// multi-second stall on exactly the commands that were already hard.
const MAX_TEXT: usize = 160;

/// Build the grammar for one user's collections.
///
/// `collections` are materialised paths — `Study/Programming/React`. An empty
/// list is valid: the destination slot simply disappears from the grammar,
/// which is the correct behaviour for a library with nowhere to file anything
/// yet, and much better than a rule that can match nothing.
pub fn build(collections: &[String]) -> String {
    let mut out = String::new();
    let has_collections = !collections.is_empty();

    // A move with nowhere to move to is not a command. The parser would reject
    // it, but a grammar that can *express* a meaningless command has already
    // given up the property this module exists for — so the intent is dropped
    // entirely rather than left to be caught downstream.
    let routable: Vec<Intent> = ROUTABLE
        .iter()
        .copied()
        .filter(|i| has_collections || *i != Intent::Move)
        .collect();

    let intents: Vec<String> = routable.iter().map(|i| rule_name(*i)).collect();
    out.push_str(&format!("root ::= \"{{\" ws ({}) ws \"}}\"\n", intents.join(" | ")));

    for intent in &routable {
        let name = rule_name(*intent);
        let key = format!("\"\\\"intent\\\"\" ws \":\" ws \"\\\"{}\\\"\"", intent.token());

        // Which slot each intent may carry, and whether it must.
        let tail = match intent {
            // A destination is optional: "save this" with nowhere named is a
            // complete command, and inventing a folder to satisfy the grammar
            // would be worse than filing it unsorted.
            Intent::Save | Intent::Move if has_collections => {
                " (ws \",\" ws collection-slot)?".to_string()
            }
            Intent::Save | Intent::Move => String::new(),
            // Retrieval without a query is not a command at all.
            Intent::Search | Intent::Show | Intent::Open => " ws \",\" ws query-slot".to_string(),
            Intent::Note => " ws \",\" ws title-slot".to_string(),
            Intent::Tag => " ws \",\" ws tag-slot".to_string(),
            _ => String::new(),
        };
        out.push_str(&format!("{name} ::= {key}{tail}\n"));
    }

    if has_collections {
        out.push_str("collection-slot ::= \"\\\"collection\\\"\" ws \":\" ws collection\n");
        // The whole point: literal alternatives, so no other value can be
        // sampled. Sorted and de-duplicated for a stable grammar — an unstable
        // one would invalidate the cached prefix on every rebuild.
        let mut paths: Vec<&String> = collections.iter().collect();
        paths.sort();
        paths.dedup();
        let alternatives: Vec<String> = paths.iter().map(|p| json_literal(p)).collect();
        out.push_str(&format!("collection ::= {}\n", alternatives.join(" | ")));
    }

    out.push_str("query-slot ::= \"\\\"query\\\"\" ws \":\" ws text\n");
    out.push_str("title-slot ::= \"\\\"title\\\"\" ws \":\" ws text\n");
    out.push_str("tag-slot ::= \"\\\"tag\\\"\" ws \":\" ws text\n");
    out.push_str(&format!(
        "text ::= \"\\\"\" ( [^\"\\\\\\x7F\\x00-\\x1F] | \"\\\\\" [\"\\\\bfnrt] ){{0,{MAX_TEXT}}} \"\\\"\"\n"
    ));
    // Whitespace is allowed but never required, and never newlines: a model
    // that can emit a newline can emit an unbounded run of them.
    out.push_str("ws ::= [ ]?\n");

    out
}

fn rule_name(intent: Intent) -> String {
    format!("cmd-{}", intent.token())
}

/// A path as a GBNF literal containing a JSON string.
///
/// Two levels of escaping, which is exactly where this goes wrong if written by
/// eye. The model must emit `"Say \"hi\""` — JSON-escaped — and to say that in
/// GBNF, every one of those characters is escaped again for the literal.
fn json_literal(value: &str) -> String {
    let mut json = String::from("\"");
    for ch in value.chars() {
        match ch {
            '"' => json.push_str("\\\""),
            '\\' => json.push_str("\\\\"),
            c if (c as u32) < 0x20 => json.push(' '),
            c => json.push(c),
        }
    }
    json.push('"');

    // Now the same string as a GBNF literal.
    let mut literal = String::from("\"");
    for ch in json.chars() {
        match ch {
            '"' => literal.push_str("\\\""),
            '\\' => literal.push_str("\\\\"),
            c => literal.push(c),
        }
    }
    literal.push('"');
    literal
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<String> {
        ["Study", "Study/Programming/React", "Life/House"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn every_routable_intent_has_a_rule() {
        let g = build(&paths());
        for intent in ROUTABLE {
            assert!(
                g.contains(&format!("cmd-{} ::=", intent.token())),
                "{} has no rule:\n{g}",
                intent.as_str()
            );
        }
    }

    #[test]
    fn collections_appear_as_literal_alternatives() {
        // The structural guarantee: a destination the user does not have cannot
        // be sampled, because it is not in the grammar to sample.
        let g = build(&paths());
        assert!(g.contains(r#""\"Study/Programming/React\"""#), "{g}");
        assert!(g.contains(r#""\"Life/House\"""#), "{g}");
        assert!(!g.contains("Work Stuff"));
    }

    #[test]
    fn a_library_with_no_collections_has_no_destination_slot() {
        // Not a rule that matches nothing — no rule at all, or the grammar
        // would be unsatisfiable the moment a save wanted a destination.
        let g = build(&[]);
        assert!(!g.contains("collection"), "{g}");
        assert!(g.contains("cmd-save ::="));
    }

    #[test]
    fn a_move_is_unroutable_when_there_is_nowhere_to_move_to() {
        // The parser would reject it anyway, but a grammar that can express a
        // meaningless command has given up the property this module exists for.
        let g = build(&[]);
        assert!(!g.contains("cmd-move"), "{g}");
        // ...and it comes back as soon as there is a destination.
        assert!(build(&paths()).contains("cmd-move ::="));
    }

    #[test]
    fn retrieval_requires_a_query_and_cannot_take_a_collection() {
        let g = build(&paths());
        let search = g
            .lines()
            .find(|l| l.starts_with("cmd-search ::="))
            .expect("a search rule");
        assert!(search.contains("query-slot"), "{search}");
        assert!(!search.contains("collection"), "{search}");
        // ...and it is not optional. "find" with nothing to find is not a
        // command, and a grammar that allows it invites the model to produce it.
        assert!(!search.contains("?"), "{search}");
    }

    #[test]
    fn saving_may_omit_the_destination() {
        let g = build(&paths());
        let save = g
            .lines()
            .find(|l| l.starts_with("cmd-save ::="))
            .expect("a save rule");
        assert!(save.contains("collection-slot)?"), "{save}");
    }

    #[test]
    fn a_quote_in_a_collection_name_is_escaped_for_both_layers() {
        // The case that breaks a grammar written by eye: the model has to emit
        // JSON-escaped text, and the grammar has to quote that escaping again.
        let g = build(&[r#"Say "hi""#.to_string(), r"Back\slash".to_string()]);
        assert!(g.contains(r#""\"Say \\\"hi\\\"\"""#), "{g}");
        assert!(g.contains(r#""\"Back\\\\slash\"""#), "{g}");
    }

    #[test]
    fn the_grammar_is_stable_across_rebuilds() {
        // An unstable grammar invalidates the cached prefix on every rebuild,
        // and that cache is the difference between 200 ms and 400 ms per route.
        let shuffled = vec![
            "Life/House".to_string(),
            "Study".to_string(),
            "Study/Programming/React".to_string(),
        ];
        assert_eq!(build(&paths()), build(&shuffled));
    }

    #[test]
    fn duplicate_collections_appear_once() {
        let g = build(&["Study".into(), "Study".into()]);
        assert_eq!(g.matches(r#""\"Study\"""#).count(), 1, "{g}");
    }

    #[test]
    fn free_text_is_bounded() {
        // An unbounded string rule lets a looping model generate until the
        // context runs out, turning a 200 ms route into a multi-second stall.
        let g = build(&paths());
        assert!(g.contains(&format!("{{0,{MAX_TEXT}}}")), "{g}");
    }
}
