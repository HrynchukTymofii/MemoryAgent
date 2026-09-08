//! Prove the embeddings carry meaning, not just tokens.
//!
//!   cargo run -p memos-embed --features onnx --example embed_check
use memos_embed::model::Role;
use memos_embed::{cosine, find_model_dir, Embedder, OnnxEmbedder};
use std::time::Instant;

fn main() {
    let dir = match find_model_dir(None, std::path::Path::new(".")) {
        Some(d) => d,
        None => {
            println!("No embedding model. Run: scripts/fetch-models.ps1 embedding");
            std::process::exit(1);
        }
    };
    let t0 = Instant::now();
    let e = OnnxEmbedder::load(&dir).expect("load embedder");
    println!("model {} ({} dims), loaded in {} ms\n", e.model_id(), e.dim(), t0.elapsed().as_millis());

    // The spec's own example: the saved note and the query share no keywords.
    let stored = "State is a snapshot for each render";
    let docs = [
        stored,
        "Customer acquisition cost measures marketing spend per new user",
        "The router configuration is taped inside the cabinet door",
    ];

    let t1 = Instant::now();
    let dv = e.embed(&docs).expect("embed docs");
    let per = t1.elapsed().as_millis() as f32 / docs.len() as f32;
    println!("embedded {} documents in {} ms ({:.1} ms each)\n", docs.len(), t1.elapsed().as_millis(), per);

    let query = "find what I saved about state not changing during rendering";
    let qv = e.embed_as(&[query], Role::Query).expect("embed query");

    println!("query: {query:?}\n");
    let mut scored: Vec<(f32, &str)> = docs
        .iter()
        .enumerate()
        .map(|(i, d)| (cosine(&qv[0], &dv[i]), *d))
        .collect();
    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());

    for (score, doc) in &scored {
        let marker = if *doc == stored { "  <- the one we wanted" } else { "" };
        println!("  {score:.3}  {doc}{marker}");
    }

    let winner = scored[0].1;
    println!();
    if winner == stored {
        println!("PASS - semantic match won with no shared keywords");
        println!("       margin over runner-up: {:.3}", scored[0].0 - scored[1].0);
    } else {
        println!("FAIL - expected the React note to win");
        std::process::exit(1);
    }
}
