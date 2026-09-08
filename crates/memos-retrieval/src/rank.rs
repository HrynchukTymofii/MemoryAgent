//! Signal re-ranking, applied after fusion.
//!
//! Spec section 11.6. Relevance is not only about text: an item saved this
//! morning is more likely to be the one being asked for than one saved a year
//! ago, and an item reopened twenty times matters more than one never revisited.
//!
//! These are *adjustments*, not the ranking. They are deliberately gentle —
//! recency that overwhelms relevance turns search into a reverse-chronological
//! list, which is exactly what the user could already get by scrolling.

use crate::SearchResult;

/// Weights for the post-fusion adjustments.
///
/// Hand-set for now. Section 7 of the brief calls for tuning these against the
/// correction log, which records which result the user actually opened — a real
/// relevance label collected for free. Until there is data, small and cautious.
#[derive(Debug, Clone, Copy)]
pub struct Signals {
    /// Weight on the cosine between query and item.
    ///
    /// The `w1·semantic_similarity` term of section 7. This is the signal
    /// fusion cannot express: reciprocal rank fusion sees only positions, so an
    /// item found at rank 1 by one retriever and an item found at rank 1 by the
    /// other are indistinguishable to it — even when the model scored one as a
    /// paraphrase and the other as barely related.
    pub semantic: f32,
    /// Weight on exponential recency decay.
    pub recency: f32,
    /// Half-life of that decay, in days.
    pub recency_half_life_days: f32,
    /// Weight on how often the item has been reopened.
    pub access: f32,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            // Larger than the others because it is the only one that speaks to
            // *what the query means*; still well under 1.0, so it adjusts the
            // fused order rather than replacing it.
            semantic: 0.40,
            // Small on purpose: enough to break ties between comparable
            // matches, not enough to float a weak match above a strong one.
            recency: 0.15,
            recency_half_life_days: 30.0,
            access: 0.10,
        }
    }
}

/// Apply the signals and re-sort.
pub fn rerank(results: Vec<SearchResult>) -> Vec<SearchResult> {
    rerank_with(results, Signals::default())
}

pub fn rerank_with(mut results: Vec<SearchResult>, s: Signals) -> Vec<SearchResult> {
    if results.is_empty() {
        return results;
    }
    let now = chrono::Utc::now();

    // Normalise the fusion score to 0..1 before adding adjustments, so the
    // weights mean the same thing regardless of how many retrievers ran or how
    // deep the candidate lists were.
    let max = results
        .iter()
        .map(|r| r.score)
        .fold(f32::MIN, f32::max)
        .max(1e-6);

    for r in results.iter_mut() {
        let base = r.score / max;

        let age_days =
            (now - r.item.captured_at).num_seconds().max(0) as f32 / 86_400.0;
        let recency = 0.5f32.powf(age_days / s.recency_half_life_days);

        // Logarithmic, not linear: the difference between never opened and
        // opened twice is meaningful; between forty and fifty times it is not.
        let access = (1.0 + r.item.access_count as f32).ln() / 5.0;

        // A keyword-only hit contributes nothing here rather than a penalty:
        // an exact term match is a good answer on its own, and the rare terms
        // it excels at — a product name, an error code — are exactly the ones
        // embeddings handle worst.
        let semantic = r.similarity.unwrap_or(0.0);

        r.score = base + s.semantic * semantic + s.recency * recency + s.access * access.min(1.0);
    }

    results.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.item.id.cmp(&b.item.id))
    });
    results
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::KnowledgeItem;

    fn result(title: &str, days_old: i64, accesses: u32, score: f32) -> SearchResult {
        let mut item = KnowledgeItem::capture(title, "body");
        item.captured_at = chrono::Utc::now() - chrono::Duration::days(days_old);
        item.access_count = accesses;
        SearchResult {
            item,
            score,
            sources: vec![("keyword", 1)],
            similarity: None,
        }
    }

    fn semantic(title: &str, score: f32, similarity: f32) -> SearchResult {
        SearchResult {
            similarity: Some(similarity),
            sources: vec![("vector", 1)],
            ..result(title, 0, 0, score)
        }
    }

    #[test]
    fn recency_breaks_a_tie() {
        let out = rerank(vec![
            result("old", 400, 0, 0.5),
            result("new", 0, 0, 0.5),
        ]);
        assert_eq!(out[0].item.title, "new");
    }

    #[test]
    fn recency_does_not_overturn_a_much_stronger_match() {
        // The failure this guards against: search collapsing into a
        // reverse-chronological list, which the user could get by scrolling.
        let out = rerank(vec![
            result("old but far more relevant", 400, 0, 1.0),
            result("new and barely relevant", 0, 0, 0.2),
        ]);
        assert_eq!(out[0].item.title, "old but far more relevant");
    }

    #[test]
    fn frequently_reopened_items_rise() {
        let out = rerank(vec![
            result("ignored", 10, 0, 0.5),
            result("revisited", 10, 30, 0.5),
        ]);
        assert_eq!(out[0].item.title, "revisited");
    }

    #[test]
    fn access_count_saturates() {
        // Forty opens should not outrank a genuinely better match.
        let out = rerank(vec![
            result("better match", 10, 0, 1.0),
            result("opened constantly", 10, 5000, 0.3),
        ]);
        assert_eq!(out[0].item.title, "better match");
    }

    #[test]
    fn an_empty_list_is_handled() {
        assert!(rerank(Vec::new()).is_empty());
    }

    #[test]
    fn ordering_is_deterministic() {
        let a = rerank(vec![result("a", 1, 0, 0.5), result("b", 1, 0, 0.5)]);
        let b = rerank(vec![result("a", 1, 0, 0.5), result("b", 1, 0, 0.5)]);
        assert_eq!(a.len(), b.len());
    }
    #[test]
    fn a_real_paraphrase_outranks_a_coincidental_term_match() {
        // The case that motivated the signal. Fusion put a keyword-only hit
        // level with a vector-only hit, because both were rank 1 in their own
        // list — but the model scored one at 0.62 and the other not at all.
        let out = rerank(vec![
            result("matched one common word", 0, 0, 1.0),
            semantic("says the same thing in other words", 0.95, 0.62),
        ]);
        assert_eq!(out[0].item.title, "says the same thing in other words");
    }

    #[test]
    fn similarity_does_not_overturn_a_decisively_better_match() {
        // A weak neighbour that scraped past the floor must not displace an
        // item both retrievers agreed on.
        let out = rerank(vec![
            result("found by both, decisively", 0, 0, 1.0),
            semantic("barely related", 0.4, 0.56),
        ]);
        assert_eq!(out[0].item.title, "found by both, decisively");
    }
}
