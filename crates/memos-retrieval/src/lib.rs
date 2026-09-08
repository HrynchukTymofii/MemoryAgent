//! Hybrid retrieval.
//!
//! Spec section 11. Keyword search and vector search answer different
//! questions: BM25 is precise about exact terms — product names, acronyms,
//! URLs, people — and blind to meaning; embeddings are the reverse. Neither is
//! sufficient, which is why this is a fusion rather than a choice.
//!
//! Vector similarity is one input, never the whole system (Principle 6).

pub mod fuse;
pub mod rank;

use memos_core::{Id, KnowledgeItem};
use memos_db::{Db, DbResult};

pub use fuse::{rrf, Fused, Provenance, Ranking, RRF_K};
pub use rank::{rerank, Signals};

/// A search result, with the reasoning attached.
#[derive(Debug, Clone)]
pub struct SearchResult {
    pub item: KnowledgeItem,
    pub score: f32,
    /// Which retrievers found it and at what rank.
    pub sources: Provenance,
    /// Cosine to the query, when the vector retriever saw it.
    ///
    /// Fusion works on ranks and throws magnitudes away, which is what makes it
    /// robust across incomparable scoring scales — but it also means "barely
    /// above the floor" and "near-identical" fuse the same. Section 7 of the
    /// brief scores `w1·semantic_similarity` for exactly that reason, so the
    /// number is carried through to re-ranking rather than discarded.
    pub similarity: Option<f32>,
}

/// How many candidates each retriever contributes before fusion.
///
/// Wider than the number shown, deliberately: fusion and re-ranking can only
/// reorder what they are given, so a result that neither retriever surfaced in
/// its top slice can never be recovered.
const CANDIDATE_DEPTH: usize = 50;

/// The cosine below which a neighbour is not a match.
///
/// Measured against bge-small-en-v1.5, whose scores are compressed into a
/// narrow band: a genuine paraphrase lands around 0.70-0.75 and two unrelated
/// sentences still score 0.43-0.48, because both are English prose. A floor at
/// 0.55 sits in the gap.
///
/// It matters most on a small library, which is every library on day one: with
/// fewer items than `CANDIDATE_DEPTH` an unfiltered scan returns *everything*,
/// and fusion then reads the tail — items the model scored as unrelated — as
/// corroborating evidence.
const SIMILARITY_FLOOR: f32 = 0.55;

/// Search with keywords only.
///
/// The fallback when no embedding model is loaded, and the path that still
/// works on a machine that has never embedded anything.
pub fn search_keyword(db: &Db, query: &str, limit: usize) -> DbResult<Vec<SearchResult>> {
    let items = db.search_keyword(&sanitise_fts(query), limit)?;
    Ok(items
        .into_iter()
        .enumerate()
        .map(|(i, item)| SearchResult {
            item,
            score: 1.0 / (i as f32 + 1.0),
            sources: vec![("keyword", i + 1)],
            similarity: None,
        })
        .collect())
}

/// Search with both retrievers, fused.
pub fn search_hybrid(
    db: &Db,
    query: &str,
    query_vector: Option<&[f32]>,
    limit: usize,
) -> DbResult<Vec<SearchResult>> {
    let keyword_items = db.search_keyword(&sanitise_fts(query), CANDIDATE_DEPTH)?;
    let keyword_ids: Vec<Id> = keyword_items.iter().map(|i| i.id).collect();

    let vector_hits = match query_vector {
        Some(v) => db.search_vector(v, CANDIDATE_DEPTH, SIMILARITY_FLOOR)?,
        None => Vec::new(),
    };
    let vector_ids: Vec<Id> = vector_hits.iter().map(|h| h.item_id).collect();

    let rankings = [
        Ranking {
            name: "keyword",
            ids: &keyword_ids,
            weight: 1.0,
        },
        Ranking {
            name: "vector",
            ids: &vector_ids,
            weight: 1.0,
        },
    ];
    let fused = rrf(&rankings);

    // Hydrate only what survived fusion. Keyword hits are already loaded; the
    // rest are fetched individually, which is a handful of primary-key lookups
    // rather than a second full scan.
    let mut out = Vec::with_capacity(fused.len().min(limit));
    for f in fused.into_iter().take(limit) {
        let item = match keyword_items.iter().find(|i| i.id == f.item_id) {
            Some(i) => Some(i.clone()),
            None => db.get_item(f.item_id)?,
        };
        if let Some(item) = item {
            out.push(SearchResult {
                similarity: vector_hits
                    .iter()
                    .find(|h| h.item_id == item.id)
                    .map(|h| h.similarity),
                item,
                score: f.score,
                sources: f.sources,
            });
        }
    }
    Ok(rerank(out))
}

/// Words carried by every sentence, which therefore distinguish none of them.
///
/// This is not a general-purpose stop list — it is the function words of spoken
/// English, plus the framing verbs that open a spoken query ("where did I put",
/// "what was the"). Nothing here is ever the reason one memory is the right
/// answer and another is not.
const STOPWORDS: &[&str] = &[
    "a", "about", "after", "again", "all", "am", "an", "and", "any", "are", "as", "at", "away",
    "back", "be", "because", "been", "before", "being", "but", "by", "can", "could", "did", "do",
    "does", "doesn", "doing", "don", "down", "each", "for", "from", "get", "give", "go", "had",
    "has", "have", "he", "her", "here", "hers", "him", "his", "how", "i", "if", "in", "into", "is",
    "it", "its", "just", "know", "like", "me", "mine", "more", "most", "much", "must", "my",
    "need", "no", "not", "now", "of", "off", "on", "one", "only", "or", "other", "our", "out",
    "over", "put", "really", "right", "said", "same", "say", "see", "she", "should", "so", "some",
    "something", "still", "such", "than", "that", "the", "their", "them", "then", "there", "these",
    "they", "thing", "things", "this", "those", "to", "too", "up", "us", "use", "very", "want",
    "was", "way", "we", "well", "went", "were", "what", "when", "where", "which", "while", "who",
    "why", "will", "with", "would", "you", "your", "yours",
];

fn is_stopword(t: &str) -> bool {
    STOPWORDS.binary_search(&t).is_ok()
}

/// Escape a spoken phrase for FTS5.
///
/// FTS5's query syntax treats quotes, `*`, `^`, `NEAR`, `AND`, `OR` and `NOT`
/// as operators. A transcript is prose, not a query language: an apostrophe in
/// "don't" or a stray `-` would otherwise be a syntax error rather than a
/// search. Every surviving term is quoted and OR-ed, so partial matches rank.
///
/// **Function words are dropped before the OR.** This is the difference between
/// keyword search working and not working on spoken input. "Why doesn't my
/// component see the new value straight away" is two content words and nine
/// that appear in every document ever written; OR-ing all eleven makes BM25
/// rank by how many common words a document happens to contain, which is noise
/// that then outvotes the vector side during fusion.
fn sanitise_fts(query: &str) -> String {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .filter(|t| !t.is_empty())
        .map(str::to_lowercase)
        .collect();

    let mut terms: Vec<String> = words
        .iter()
        .filter(|t| !is_stopword(t))
        .map(|t| format!("\"{t}\""))
        .collect();

    // A query made entirely of function words is unusual but real — "what was
    // that about" — and searching for nothing is worse than searching for the
    // words the user actually said.
    if terms.is_empty() {
        terms = words.iter().map(|t| format!("\"{t}\"")).collect();
    }
    if terms.is_empty() {
        // FTS5 rejects an empty MATCH. A token that matches nothing is a
        // cleaner "no results" than an error the caller has to special-case.
        return "\"\"".into();
    }
    terms.join(" OR ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::KnowledgeItem;

    fn seeded() -> Db {
        let db = Db::open_in_memory().unwrap();
        for (t, c) in [
            ("State as a Snapshot", "State is a snapshot for each render"),
            ("Customer Acquisition Cost", "CAC measures marketing spend per user"),
            ("Router config", "The router configuration is on the fridge"),
        ] {
            db.capture(&KnowledgeItem::capture(t, c)).unwrap();
        }
        db
    }

    #[test]
    fn keyword_search_finds_exact_terms() {
        let db = seeded();
        let r = search_keyword(&db, "snapshot", 5).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.title, "State as a Snapshot");
    }

    #[test]
    fn spoken_punctuation_does_not_break_the_query() {
        // A transcript is prose. Unescaped, "don't" and a stray hyphen are FTS5
        // syntax errors rather than searches.
        let db = seeded();
        for q in ["don't", "state - snapshot", "CAC?", "NOT router", "a*"] {
            assert!(
                search_keyword(&db, q, 5).is_ok(),
                "query {q:?} must not be a syntax error"
            );
        }
    }

    #[test]
    fn an_empty_query_returns_nothing_rather_than_erroring() {
        let db = seeded();
        assert!(search_keyword(&db, "   ", 5).unwrap().is_empty());
    }

    #[test]
    fn hybrid_without_vectors_still_works() {
        // The path on a machine that has never embedded anything.
        let db = seeded();
        let r = search_hybrid(&db, "snapshot", None, 5).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sources[0].0, "keyword");
    }

    #[test]
    fn vector_only_matches_are_recovered() {
        // The point of the hybrid: a semantic hit sharing no keywords with the
        // query still surfaces.
        let db = Db::open_in_memory().unwrap();
        let a = KnowledgeItem::capture("Snapshot", "State is a snapshot per render");
        db.capture(&a).unwrap();
        let mut v = vec![0.0f32; 384];
        v[7] = 1.0;
        db.put_embedding(a.id, "test", &v).unwrap();

        // A query with no lexical overlap at all.
        let r = search_hybrid(&db, "zzzz", Some(&v), 5).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].item.id, a.id);
        assert_eq!(r[0].sources[0].0, "vector");
    }
    #[test]
    fn function_words_are_not_searched_for() {
        // The failure this prevents: OR-ing "the", "my" and "new" makes BM25
        // rank by how ordinary a document's vocabulary is.
        let q = sanitise_fts("why doesn't my component see the new value straight away");
        assert!(q.contains("\"component\""));
        assert!(q.contains("\"value\""));
        assert!(!q.contains("\"the\""));
        assert!(!q.contains("\"my\""));
        assert!(!q.contains("\"see\""));
    }

    #[test]
    fn a_query_of_only_function_words_still_searches_for_them() {
        let q = sanitise_fts("what was that about");
        assert!(q.contains("\"about\""), "{q}");
    }

    #[test]
    fn the_stop_list_is_sorted_for_binary_search() {
        let mut sorted = STOPWORDS.to_vec();
        sorted.sort_unstable();
        assert_eq!(sorted, STOPWORDS, "STOPWORDS must stay sorted");
        sorted.dedup();
        assert_eq!(sorted.len(), STOPWORDS.len(), "no duplicates");
    }

    #[test]
    fn common_words_no_longer_drag_in_every_item() {
        let db = seeded();
        // "the" appears in two of the three seeded items; on its own it must
        // not constitute a search.
        let r = search_keyword(&db, "the router", 5).unwrap();
        assert_eq!(r.len(), 1, "only the item that is actually about a router");
        assert_eq!(r[0].item.title, "Router config");
    }
}
