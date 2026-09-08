//! Reciprocal Rank Fusion.
//!
//! Two retrievers, two rankings, one result list. RRF is used rather than a
//! weighted sum of scores because **BM25 and cosine similarity are not on the
//! same scale and never will be**: BM25 is unbounded and corpus-dependent,
//! cosine is bounded to [-1, 1]. Normalising them against each other means
//! inventing a mapping that shifts as the corpus grows, and quietly re-tunes
//! itself as the user saves more.
//!
//! RRF sidesteps that entirely by discarding the scores and keeping only the
//! *ranks*, which are comparable by construction.

use std::collections::HashMap;

use memos_core::Id;

/// The RRF damping constant.
///
/// 60 is the value from the original paper and is what nearly every
/// implementation uses. It controls how sharply rank position is discounted: a
/// smaller k makes the top result dominate, a larger one flattens the
/// contribution across the list.
pub const RRF_K: f32 = 60.0;

/// One retriever's opinion: item ids, best first.
pub struct Ranking<'a> {
    pub name: &'static str,
    pub ids: &'a [Id],
    /// Relative influence. Equal weights mean each retriever gets an equal say.
    pub weight: f32,
}

/// A fused result, with enough detail to explain itself.
#[derive(Debug, Clone, PartialEq)]
pub struct Fused {
    pub item_id: Id,
    pub score: f32,
    /// Which retrievers found it, and at what rank. Kept because "why did this
    /// rank here" is a question worth being able to answer — of the ranking
    /// signals in section 11.6, provenance is the one that builds trust.
    pub sources: Provenance,
}

/// Which retrievers found an item, and at what rank.
pub type Provenance = Vec<(&'static str, usize)>;

/// Fuse rankings by reciprocal rank.
///
/// An item found by *both* retrievers outranks one found by either alone, even
/// if it was second in each. That is the whole point: agreement between two
/// independent methods is stronger evidence than a high score from one.
pub fn rrf(rankings: &[Ranking<'_>]) -> Vec<Fused> {
    let mut acc: HashMap<Id, (f32, Provenance)> = HashMap::new();

    for r in rankings {
        for (i, id) in r.ids.iter().enumerate() {
            let rank = i + 1;
            let contribution = r.weight / (RRF_K + rank as f32);
            let e = acc.entry(*id).or_insert((0.0, Vec::new()));
            e.0 += contribution;
            e.1.push((r.name, rank));
        }
    }

    let mut out: Vec<Fused> = acc
        .into_iter()
        .map(|(item_id, (score, sources))| Fused {
            item_id,
            score,
            sources,
        })
        .collect();

    out.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            // Deterministic tie-break, so identical queries return identical
            // orders between runs.
            .then_with(|| a.item_id.cmp(&b.item_id))
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(n: usize) -> Vec<Id> {
        (0..n).map(|_| Id::new()).collect()
    }

    #[test]
    fn agreement_beats_a_single_strong_hit() {
        // The central property. `b` is second in both rankings; `a` is first in
        // one and absent from the other. Two independent retrievers agreeing is
        // stronger evidence than one being confident.
        let v = ids(3);
        let (a, b, c) = (v[0], v[1], v[2]);
        let keyword = [a, b];
        let vector = [c, b];

        let out = rrf(&[
            Ranking { name: "keyword", ids: &keyword, weight: 1.0 },
            Ranking { name: "vector", ids: &vector, weight: 1.0 },
        ]);
        assert_eq!(out[0].item_id, b, "the item both retrievers found must win");
        assert_eq!(out[0].sources.len(), 2);
    }

    #[test]
    fn rank_order_within_one_retriever_is_preserved() {
        let v = ids(3);
        let out = rrf(&[Ranking { name: "only", ids: &v, weight: 1.0 }]);
        assert_eq!(out.iter().map(|f| f.item_id).collect::<Vec<_>>(), v);
    }

    #[test]
    fn weights_shift_the_balance() {
        let v = ids(2);
        let (a, b) = (v[0], v[1]);
        // Each retriever ranks a different item first.
        let out = rrf(&[
            Ranking { name: "keyword", ids: &[a], weight: 0.2 },
            Ranking { name: "vector", ids: &[b], weight: 1.0 },
        ]);
        assert_eq!(out[0].item_id, b, "the heavier retriever should win");
    }

    #[test]
    fn an_empty_ranking_contributes_nothing() {
        let v = ids(2);
        let out = rrf(&[
            Ranking { name: "keyword", ids: &v, weight: 1.0 },
            Ranking { name: "vector", ids: &[], weight: 1.0 },
        ]);
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn no_rankings_yields_no_results() {
        assert!(rrf(&[]).is_empty());
    }

    #[test]
    fn ordering_is_deterministic() {
        let v = ids(4);
        let a = rrf(&[Ranking { name: "k", ids: &v, weight: 1.0 }]);
        let b = rrf(&[Ranking { name: "k", ids: &v, weight: 1.0 }]);
        assert_eq!(a, b);
    }

    #[test]
    fn every_result_records_where_it_came_from() {
        let v = ids(2);
        let out = rrf(&[
            Ranking { name: "keyword", ids: &v, weight: 1.0 },
            Ranking { name: "vector", ids: &v[..1], weight: 1.0 },
        ]);
        let top = &out[0];
        assert!(top.sources.iter().any(|(n, _)| *n == "keyword"));
        assert!(top.sources.iter().any(|(n, _)| *n == "vector"));
    }
}
