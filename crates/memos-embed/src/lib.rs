//! Turning text into vectors.
//!
//! Spec section 12. Embeddings are *one* retrieval signal, not the database
//! (Principle 6) — they supply the semantic half of the hybrid search in
//! section 11, and the keyword half already works without them.
//!
//! ## Why this is never on the critical path
//!
//! A capture is acknowledged the moment the SQLite transaction commits, before
//! any vector exists. Embedding runs on a background worker draining the `jobs`
//! table and backfills. That is what makes stage 6 of the latency budget 20 ms
//! instead of 20 ms plus a model run — and it is why an embedding failure
//! degrades search quality rather than losing a memory.

pub mod model;
#[cfg(feature = "onnx")]
mod onnx;

use std::path::{Path, PathBuf};

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("model files not found under {0}")]
    ModelMissing(PathBuf),
    #[error("failed to load embedding model: {0}")]
    Load(String),
    #[error("embedding failed: {0}")]
    Run(String),
    #[error("built without the `onnx` feature")]
    Unavailable,
}

/// A text embedder.
///
/// Batched on purpose: the background worker embeds a queue, and running one
/// text at a time wastes most of the throughput a transformer offers.
pub trait Embedder: Send + Sync {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// Vector width. Stored alongside every vector so a model change is
    /// detectable rather than silently producing nonsense comparisons.
    fn dim(&self) -> usize;

    /// Identifier recorded with each vector, e.g. `bge-small-en-v1.5`.
    fn model_id(&self) -> &str;

    fn embed_one(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(self.embed(&[text])?.into_iter().next().unwrap_or_default())
    }
}

/// Cosine similarity of two unit-length vectors.
///
/// The embedder normalises its output, so this is a plain dot product. It is
/// kept separate anyway because a future provider may not normalise, and a
/// silently un-normalised vector would make every score subtly wrong rather
/// than obviously broken.
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    if a.len() != b.len() || a.is_empty() {
        return 0.0;
    }
    let (mut dot, mut na, mut nb) = (0.0f32, 0.0f32, 0.0f32);
    for i in 0..a.len() {
        dot += a[i] * b[i];
        na += a[i] * a[i];
        nb += b[i] * b[i];
    }
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

/// Locate the model directory.
///
/// Checked in order so the repository layout works during development and the
/// installed layout works in production, with no build-time switch.
pub fn find_model_dir(explicit: Option<&Path>, data_dir: &Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(p) = explicit {
        candidates.push(p.to_path_buf());
    }
    candidates.push(data_dir.join("models/embedding"));
    candidates.push(PathBuf::from("models/embedding"));
    candidates.push(PathBuf::from("../../models/embedding"));
    candidates.push(PathBuf::from("../../../models/embedding"));
    candidates
        .into_iter()
        .find(|p| p.join("model.onnx").exists() && p.join("tokenizer.json").exists())
}

/// Locate the ONNX Runtime DLL we ship, and point `ort` at it.
///
/// Not optional on Windows. `load-dynamic` searches by bare name, and Windows
/// carries its own `onnxruntime.dll` (1.17.1) in System32, which loads first and
/// then fails against the 1.20 API this build requires — with an error about a
/// missing symbol rather than about the wrong file being found. An explicit
/// absolute path is the only reliable answer.
///
/// An existing `ORT_DYLIB_PATH` always wins: somebody who set it meant it.
pub fn use_bundled_runtime(data_dir: &Path) -> Option<PathBuf> {
    const DYLIB: &str = if cfg!(windows) {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    };

    if let Ok(existing) = std::env::var("ORT_DYLIB_PATH") {
        if !existing.trim().is_empty() {
            return Some(PathBuf::from(existing));
        }
    }

    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            // Installed layout: the DLL sits beside the binary.
            candidates.push(dir.join(DYLIB));
            candidates.push(dir.join("runtime").join(DYLIB));
        }
    }
    candidates.push(data_dir.join("runtime").join(DYLIB));
    // Development layout, from a crate directory or the workspace root.
    for prefix in ["runtime", "../runtime", "../../runtime", "../../../runtime"] {
        candidates.push(PathBuf::from(prefix).join(DYLIB));
    }

    let found = candidates.into_iter().find(|p| p.exists())?;
    let absolute = std::fs::canonicalize(&found).unwrap_or(found);
    // A UNC prefix from canonicalize confuses LoadLibrary on some Windows
    // builds; the plain path is what the loader expects.
    let cleaned = absolute
        .to_string_lossy()
        .trim_start_matches(r"\\?\")
        .to_string();
    std::env::set_var("ORT_DYLIB_PATH", &cleaned);
    tracing::info!(path = %cleaned, "using bundled ONNX Runtime");
    Some(PathBuf::from(cleaned))
}

#[cfg(feature = "onnx")]
pub use onnx::OnnxEmbedder;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_vectors_are_maximally_similar() {
        let v = vec![0.1, -0.4, 0.9];
        assert!((cosine(&v, &v) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn opposite_vectors_are_minimally_similar() {
        assert!((cosine(&[1.0, 0.0], &[-1.0, 0.0]) + 1.0).abs() < 1e-5);
    }

    #[test]
    fn orthogonal_vectors_score_zero() {
        assert!(cosine(&[1.0, 0.0], &[0.0, 1.0]).abs() < 1e-5);
    }

    #[test]
    fn mismatched_dimensions_score_zero_rather_than_panicking() {
        // A model change mid-database must degrade search, not crash it.
        assert_eq!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]), 0.0);
        assert_eq!(cosine(&[], &[]), 0.0);
    }

    #[test]
    fn a_zero_vector_scores_zero() {
        assert_eq!(cosine(&[0.0, 0.0], &[1.0, 1.0]), 0.0);
    }

    #[test]
    fn an_explicit_runtime_path_is_respected() {
        // Somebody who sets ORT_DYLIB_PATH is overriding us on purpose.
        std::env::set_var("ORT_DYLIB_PATH", "C:/somewhere/onnxruntime.dll");
        let got = use_bundled_runtime(Path::new("."));
        assert_eq!(got, Some(PathBuf::from("C:/somewhere/onnxruntime.dll")));
        std::env::remove_var("ORT_DYLIB_PATH");
    }
}
