//! The M2 claim, end to end: capture, embed, retrieve.
//!
//!   cargo run -p memos-embed --features onnx --example search_check
//!
//! Every query here is phrased the way somebody would actually ask months
//! later — sharing few or no words with what was saved. Keyword search alone
//! answers some of them and is helpless on the rest; the point of the hybrid is
//! that it answers both kinds without being told which kind it is looking at.

use memos_core::KnowledgeItem;
use memos_db::Db;
use memos_embed::{find_model_dir, Embedder, OnnxEmbedder};
use memos_retrieval::{search_hybrid, search_keyword};
use std::time::Instant;

/// What a week of ordinary use might leave behind.
const CORPUS: &[(&str, &str)] = &[
    (
        "State as a Snapshot",
        "State is a snapshot for each render. Setting it schedules a re-render \
         rather than changing the variable you already read.",
    ),
    (
        "Customer Acquisition Cost",
        "CAC is total sales and marketing spend divided by new customers won in \
         the same period.",
    ),
    (
        "Router config",
        "The router configuration card is taped inside the cabinet door in the \
         hallway.",
    ),
    (
        "Postgres connection pooling",
        "PgBouncer in transaction mode; each request checks a connection out and \
         returns it at commit.",
    ),
    (
        "Sourdough hydration",
        "80% hydration gives an open crumb but the dough is slack and needs \
         stretch and folds rather than kneading.",
    ),
    (
        "Deadline for the grant",
        "The Horizon application closes on the 14th of November and needs two \
         letters of support.",
    ),
];

/// Query, and the title that should come back first.
const QUERIES: &[(&str, &str)] = &[
    // No shared content word with the item at all.
    (
        "why doesn't my component see the new value straight away",
        "State as a Snapshot",
    ),
    // Spoken paraphrase of an acronym never spelled out in the query.
    (
        "how much it costs us to win someone new",
        "Customer Acquisition Cost",
    ),
    // The keyword half should carry this one — an exact, rare term.
    ("pgbouncer", "Postgres connection pooling"),
    // Ordinary recall, phrased as a question.
    ("where did I put the wifi details", "Router config"),
    ("when is the funding application due", "Deadline for the grant"),
];

fn main() {
    let Some(dir) = find_model_dir(None, std::path::Path::new(".")) else {
        println!("No embedding model. Run: scripts/fetch-models.ps1 embedding");
        std::process::exit(1);
    };
    let embedder = OnnxEmbedder::load(&dir).expect("load embedder");
    let db = Db::open_in_memory().expect("open database");

    // Capture first, embed second — the order the product uses, and the reason
    // a capture is acknowledged in milliseconds.
    let mut ids = Vec::new();
    for (title, content) in CORPUS {
        let item = KnowledgeItem::capture(*title, *content);
        db.capture(&item).expect("capture");
        ids.push(item.id);
    }
    println!("captured {} items", ids.len());

    // The backfill, exactly as the worker does it: one batch, document role.
    let pending = db.items_awaiting_embedding(64).expect("queue");
    assert_eq!(pending.len(), CORPUS.len(), "every capture is queued");

    let t = Instant::now();
    let texts: Vec<&str> = pending.iter().map(|(_, t)| t.as_str()).collect();
    let vectors = embedder.embed(&texts).expect("embed batch");
    let embed_ms = t.elapsed().as_millis();
    for ((id, _), v) in pending.iter().zip(&vectors) {
        db.put_embedding(*id, embedder.model_id(), v).expect("store");
        db.mark_embedded(*id).expect("settle job");
    }
    println!(
        "embedded {} items in {} ms ({:.1} ms each), {} still queued\n",
        vectors.len(),
        embed_ms,
        embed_ms as f32 / vectors.len() as f32,
        db.items_awaiting_embedding(64).unwrap().len()
    );

    let mut hybrid_hits = 0;
    let mut keyword_hits = 0;

    for (query, expected) in QUERIES {
        let t = Instant::now();
        let qv = embedder
            .embed_as(&[query], memos_embed::model::Role::Query)
            .expect("embed query");
        let query_ms = t.elapsed().as_millis();

        let t = Instant::now();
        let results = search_hybrid(&db, query, Some(&qv[0]), 3).expect("search");
        let search_ms = t.elapsed().as_micros() as f32 / 1000.0;

        // The same query with the vector half switched off, for contrast.
        let keyword_only = search_keyword(&db, query, 3).expect("keyword search");
        let keyword_top = keyword_only.first().map(|r| r.item.title.as_str());
        if keyword_top == Some(*expected) {
            keyword_hits += 1;
        }

        let top = results.first().map(|r| r.item.title.as_str());
        let ok = top == Some(*expected);
        if ok {
            hybrid_hits += 1;
        }

        println!("  {:?}", query);
        println!(
            "    {} {}  (embed {} ms, search {:.2} ms)",
            if ok { "->" } else { "XX" },
            top.unwrap_or("nothing"),
            query_ms,
            search_ms
        );
        for r in results.iter().take(3) {
            let why: Vec<&str> = r.sources.iter().map(|(n, _)| *n).collect();
            println!("       {:.4}  {:<32} {}", r.score, r.item.title, why.join(" + "));
        }
        println!(
            "       keyword alone: {}\n",
            keyword_top.unwrap_or("nothing")
        );
    }

    println!(
        "hybrid  {}/{}\nkeyword {}/{}",
        hybrid_hits,
        QUERIES.len(),
        keyword_hits,
        QUERIES.len()
    );
    if hybrid_hits == QUERIES.len() {
        println!("\nPASS - every query found its item");
    } else {
        println!("\nFAIL - {} queries missed", QUERIES.len() - hybrid_hits);
        std::process::exit(1);
    }
}
