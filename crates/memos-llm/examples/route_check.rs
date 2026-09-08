//! Does the local router actually route?
//!
//!   cargo run -p memos-llm --features local --example route_check
//!
//! Every phrase here is one Tier 0 refuses or has to ask about — that is the
//! whole job of Tier 1, and a router that only handles what the grammar already
//! handled would be 610 MB of nothing.

use std::time::Instant;

use memos_llm::runner::Router;
use memos_llm::ModelState;

const STARTER: &[&str] = &[
    "Career",
    "Career/Companies",
    "Career/Interviews",
    "Career/Job Applications",
    "Life",
    "Life/Finance",
    "Life/Garden",
    "Life/House",
    "Study",
    "Study/AI",
    "Study/English",
    "Study/Programming",
    "Study/Programming/Python",
    "Study/Programming/React",
    "Study/Programming/TypeScript",
];

/// Transcript, and the destination or intent a person would expect.
const CASES: &[(&str, &str)] = &[
    // The one that sent the user round in circles: Tier 0 scored React at 0.28
    // and asked instead of acting.
    ("save this page to the react programming database", "SAVE Study/Programming/React"),
    ("add this to my react notes", "SAVE Study/Programming/React"),
    ("stick this in with the python stuff", "SAVE Study/Programming/Python"),
    ("file this under job applications", "SAVE Career/Job Applications"),
    ("keep this for the garden", "SAVE Life/Garden"),
    ("I want to remember this for my interviews", "SAVE Career/Interviews"),
    // Not a save at all — the router has to choose the intent too.
    ("what did I read about hooks last week", "SEARCH"),
    ("pull up that typescript thing", "SEARCH or OPEN"),
    ("jot down that the bins go out on tuesday", "NOTE"),
];

fn main() {
    let collections: Vec<String> = STARTER.iter().map(|s| s.to_string()).collect();
    let Some(model) = memos_llm::find_model(None, std::path::Path::new(".")) else {
        println!("No router model. Run: scripts/fetch-models.ps1 router");
        std::process::exit(1);
    };
    println!("model {}\n", model.display());

    let router = Router::new();
    let started = Instant::now();
    router.start(model, collections.clone());

    while router.state() == ModelState::Loading {
        std::thread::sleep(std::time::Duration::from_millis(100));
        if started.elapsed().as_secs() > 120 {
            println!("router never became ready");
            std::process::exit(1);
        }
    }
    if router.state() != ModelState::Ready {
        println!("router failed: {}", router.detail());
        std::process::exit(1);
    }
    println!("ready in {} ms — {}\n", started.elapsed().as_millis(), router.detail());

    let mut ok = 0;
    let mut total_ms = 0u128;
    for (transcript, expected) in CASES {
        let t = Instant::now();
        let answer = router.route(transcript);
        let took = t.elapsed().as_millis();
        total_ms += took;

        print!("  {transcript:<50}");
        match answer {
            None => println!("(no answer)                      want {expected}"),
            Some((json, decode)) => match memos_llm::parse(transcript, &json, &collections, decode, took as u32) {
                Ok(cmd) => {
                    ok += 1;
                    let slot = cmd
                        .slots
                        .collection
                        .clone()
                        .or(cmd.slots.query.clone())
                        .or(cmd.slots.title.clone())
                        .unwrap_or_else(|| "-".into());
                    println!(
                        "{:<7} {:<30} p{:.2} m{:.2} s{:.2} {took:>4} ms   want {expected}",
                        cmd.intent.as_str(),
                        slot,
                        cmd.confidence.logprob,
                        cmd.confidence.margin,
                        cmd.confidence.score(),
                    );
                }
                // The grammar is supposed to make this impossible. If it shows
                // up here, that claim is wrong and worth knowing about loudly.
                Err(e) => println!("REJECTED {e}   raw={json:?}"),
            },
        }
    }

    println!(
        "\n  {ok}/{} parsed, {} ms median-ish ({} ms total)",
        CASES.len(),
        total_ms / CASES.len() as u128,
        total_ms
    );
}
