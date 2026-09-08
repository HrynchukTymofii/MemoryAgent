//! Resolving a spoken phrase to a collection.
//!
//! "programming react" has to become `Study/Programming/React`. This is where
//! most of a capture's ambiguity lives, and where the margin that drives the
//! confidence system (ADR-0005) actually comes from: the question is never
//! "how good is the best match" but "is the best match distinguishable from the
//! second".

use serde::Serialize;

/// A candidate destination and how well it matched.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Candidate {
    pub path: String,
    pub score: f32,
}

/// The outcome of resolving a phrase.
#[derive(Debug, Clone, Serialize)]
pub struct Resolution {
    pub best: Option<Candidate>,
    /// score(top-1) − score(top-2). Zero when there is only one candidate,
    /// which counts as unambiguous.
    pub margin: f32,
    /// Everything that scored, best first — the list a disambiguation prompt
    /// offers the user.
    pub candidates: Vec<Candidate>,
}

impl Resolution {
    /// Confident enough to act without asking.
    ///
    /// Both conditions matter. A high score with a low margin is the
    /// `House/Internet` vs `House/Electricity` case: plausible, and precisely
    /// the moment to ask rather than guess.
    pub fn is_confident(&self) -> bool {
        self.best.as_ref().is_some_and(|b| b.score >= 0.6) && self.margin >= 0.15
    }
}

/// Words that carry no meaning for matching but are always present in speech.
const FILLER: &[&str] = &[
    "the", "a", "an", "my", "to", "in", "into", "under", "on", "of", "for", "please", "stuff",
    "folder", "collection", "section",
];

fn normalise(s: &str) -> Vec<String> {
    s.split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(|w| w.to_lowercase())
        .filter(|w| !FILLER.contains(&w.as_str()))
        .collect()
}

/// Score a phrase against one collection path.
///
/// Returns 0.0 when nothing matches, so a caller can drop non-candidates
/// without a separate threshold.
fn score(query: &[String], path: &str) -> f32 {
    if query.is_empty() {
        return 0.0;
    }
    let segments: Vec<String> = path.split('/').map(|s| s.to_lowercase()).collect();
    let path_words: Vec<String> = segments
        .iter()
        .flat_map(|s| normalise(s))
        .collect();
    if path_words.is_empty() {
        return 0.0;
    }

    let leaf = segments.last().cloned().unwrap_or_default();
    let joined = query.join(" ");

    // Exact leaf match is the strongest signal: people name the leaf, not the
    // path. "save this to react" means React, not Study.
    if leaf == joined {
        return 1.0;
    }

    let matched = query.iter().filter(|w| path_words.contains(w)).count();
    if matched == 0 {
        return 0.0;
    }

    // Every spoken word appears somewhere in the path.
    if matched == query.len() {
        // Prefer the shallowest path that accounts for all of them: "programming
        // react" should pick Study/Programming/React over a deeper
        // Study/Programming/React/Hooks that also contains both words.
        let depth_penalty = (path_words.len() as f32 - query.len() as f32).max(0.0) * 0.06;
        let leaf_bonus = if query.iter().any(|w| leaf.contains(w.as_str())) {
            0.12
        } else {
            0.0
        };
        return (0.85 + leaf_bonus - depth_penalty).clamp(0.3, 0.99);
    }

    // Partial overlap. Scaled by how much of the phrase was accounted for, so a
    // one-word hit against a three-word phrase stays well below the ask
    // threshold rather than sneaking past it.
    0.55 * (matched as f32 / query.len() as f32)
}

/// Resolve a spoken phrase against the known collection paths.
pub fn resolve(phrase: &str, paths: &[String]) -> Resolution {
    let query = normalise(phrase);
    let mut candidates: Vec<Candidate> = paths
        .iter()
        .map(|p| Candidate {
            path: p.clone(),
            score: score(&query, p),
        })
        .filter(|c| c.score > 0.0)
        .collect();

    candidates.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Deterministic ordering for equal scores, so the same command does
            // not resolve differently between runs.
            .then_with(|| a.path.cmp(&b.path))
    });

    let margin = match candidates.as_slice() {
        [] => 0.0,
        [_only] => 1.0,
        [a, b, ..] => a.score - b.score,
    };

    Resolution {
        best: candidates.first().cloned(),
        margin,
        candidates: candidates.into_iter().take(4).collect(),
    }
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
            "Life",
            "Life/House",
            "Life/House/Internet",
            "Life/House/Electricity",
            "Career",
            "Career/Job Applications",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn resolves_a_leaf_by_name() {
        let r = resolve("react", &paths());
        assert!(r.is_confident());
        assert_eq!(r.best.as_ref().unwrap().path, "Study/Programming/React");
    }

    #[test]
    fn resolves_a_multi_word_phrase() {
        let r = resolve("programming react", &paths());
        assert!(r.is_confident());
        assert_eq!(r.best.as_ref().unwrap().path, "Study/Programming/React");
    }

    #[test]
    fn ignores_filler_words() {
        let r = resolve("the react folder please", &paths());
        assert_eq!(r.best.as_ref().unwrap().path, "Study/Programming/React");
    }

    /// The spec's disambiguation example: siblings under House, with no
    /// collection named House itself.
    fn sibling_paths() -> Vec<String> {
        [
            "Life",
            "Life/House/Internet",
            "Life/House/Electricity",
            "Life/House/Automation",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect()
    }

    #[test]
    fn ambiguous_phrase_is_not_confident() {
        // "put this with the house stuff": three plausible destinations, none
        // distinguishable. The margin is what catches this — every candidate
        // scores identically, so no threshold on the top score alone would.
        let r = resolve("house", &sibling_paths());
        assert!(
            !r.is_confident(),
            "house matches three siblings; margin was {} over {:?}",
            r.margin,
            r.candidates
        );
        assert_eq!(r.candidates.len(), 3);
        assert_eq!(r.margin, 0.0, "identical scores must produce zero margin");
    }

    #[test]
    fn an_exact_collection_name_is_not_ambiguous() {
        // If a collection is literally named House, saying "house" means it —
        // even though its children also match. This is the counterpart to the
        // test above and the reason margin beats a raw score threshold.
        let r = resolve("house", &paths());
        assert!(r.is_confident());
        assert_eq!(r.best.as_ref().unwrap().path, "Life/House");
    }

    #[test]
    fn an_unknown_phrase_resolves_to_nothing() {
        let r = resolve("fertilizers", &paths());
        assert!(r.best.is_none());
        assert!(!r.is_confident());
    }

    #[test]
    fn a_sole_match_is_unambiguous() {
        let r = resolve("typescript", &paths());
        assert!(r.is_confident());
        assert_eq!(r.best.as_ref().unwrap().path, "Study/Programming/TypeScript");
    }

    #[test]
    fn ordering_is_deterministic() {
        // Equal scores must not reorder between runs, or the same command would
        // file into different places on different days.
        let a = resolve("house", &sibling_paths());
        let b = resolve("house", &sibling_paths());
        let pa: Vec<_> = a.candidates.iter().map(|c| &c.path).collect();
        let pb: Vec<_> = b.candidates.iter().map(|c| &c.path).collect();
        assert_eq!(pa, pb);
    }
}
