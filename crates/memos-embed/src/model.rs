//! Which model, and how its output is pooled.
//!
//! Kept separate from the runtime because the pooling strategy is a property of
//! the *model*, not of ONNX: getting it wrong produces vectors that look
//! plausible, normalise correctly, and rank badly — a failure that no type
//! system catches and no crash reveals.

/// Description of an embedding model.
#[derive(Debug, Clone, Copy)]
pub struct ModelSpec {
    pub id: &'static str,
    pub dim: usize,
    /// Tokens beyond this are truncated. Captures are short; a long article
    /// would be chunked before reaching here.
    pub max_tokens: usize,
    pub pooling: Pooling,
    /// Some models were trained with an instruction prefix on the *query* side
    /// only. Applying it to stored documents as well quietly degrades recall.
    pub query_prefix: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Pooling {
    /// Take the [CLS] token. What the BGE family was trained with.
    Cls,
    /// Average over non-padding tokens. What sentence-transformers models use.
    Mean,
}

/// `bge-small-en-v1.5` — 384 dimensions, ~33 MB quantised.
///
/// Chosen for the size/quality point rather than raw benchmark position: it is
/// small enough to hold resident without argument and strong enough that the
/// reranker, not the embedder, becomes the quality bottleneck.
pub const BGE_SMALL_EN_V15: ModelSpec = ModelSpec {
    id: "bge-small-en-v1.5",
    dim: 384,
    max_tokens: 512,
    pooling: Pooling::Cls,
    // The BGE authors recommend this prefix for retrieval queries and none for
    // the passages being searched.
    query_prefix: Some("Represent this sentence for searching relevant passages: "),
};

/// Whether text is being embedded to be stored or to search with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    Document,
    Query,
}

impl ModelSpec {
    /// Apply the model's role-specific prefix, if any.
    pub fn prepare<'a>(&self, text: &'a str, role: Role) -> std::borrow::Cow<'a, str> {
        match (role, self.query_prefix) {
            (Role::Query, Some(p)) => std::borrow::Cow::Owned(format!("{p}{text}")),
            _ => std::borrow::Cow::Borrowed(text),
        }
    }
}

/// L2-normalise in place, so cosine similarity is a dot product.
pub fn normalise(v: &mut [f32]) {
    let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 1e-12 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn query_gets_the_prefix_and_documents_do_not() {
        let s = BGE_SMALL_EN_V15;
        assert!(s.prepare("react state", Role::Query).starts_with("Represent this"));
        assert_eq!(s.prepare("react state", Role::Document), "react state");
    }

    #[test]
    fn normalisation_produces_unit_length() {
        let mut v = vec![3.0, 4.0];
        normalise(&mut v);
        let len: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((len - 1.0).abs() < 1e-6);
    }

    #[test]
    fn normalising_a_zero_vector_does_not_divide_by_zero() {
        let mut v = vec![0.0, 0.0];
        normalise(&mut v);
        assert_eq!(v, vec![0.0, 0.0]);
    }
}
