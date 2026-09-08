//! What does Tier 0 do with this sentence?
//!
//!   cargo run -p memos-agent --example route_check
//!   cargo run -p memos-agent --example route_check -- "add this to react"
//!
//! The answer to "I said something and nothing happened". Tier 0 is a fixed
//! grammar: it recognises the shapes it was taught and refuses everything else,
//! so the useful question is always *which* of those two happened.

use memos_agent::{parse, Tier0};

/// The collections a new install ships with.
const COLLECTIONS: &[&str] = &[
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

/// Phrasings a person actually uses for "put this in my memory".
const PHRASES: &[&str] = &[
    "save this",
    "save this to react",
    "save this to study programming react",
    "add this",
    "add this to react",
    "add this to my react notes",
    "put this in react",
    "file this under react",
    "keep this",
    "store this in react",
    "capture this",
    "bookmark this",
    "remember that the router is behind the books",
    "note that the bins go out on tuesday",
    "find the react article",
    "show me everything about react",
    "open the react article",
    "save this to programming",
];

fn main() {
    let collections: Vec<String> = COLLECTIONS.iter().map(|s| s.to_string()).collect();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let phrases: Vec<String> = if args.is_empty() {
        PHRASES.iter().map(|s| s.to_string()).collect()
    } else {
        vec![args.join(" ")]
    };

    let mut routed = 0;
    for phrase in &phrases {
        print!("  {phrase:<50}");
        match parse(phrase, &collections) {
            Tier0::Routed(cmd) => {
                routed += 1;
                let slot = cmd
                    .slots
                    .collection
                    .clone()
                    .or(cmd.slots.query.clone())
                    .or(cmd.slots.title.clone())
                    .unwrap_or_else(|| "-".into());
                println!("{:<9} {}", cmd.intent.as_str(), slot);
            }
            Tier0::Ambiguous { slot, resolution, .. } => {
                let top: Vec<String> = resolution
                    .candidates
                    .iter()
                    .take(3)
                    .map(|c| format!("{} {:.2}", c.path, c.score))
                    .collect();
                println!("AMBIGUOUS {slot}: {}", top.join(" | "));
            }
            // The case that reads, from outside, as the product being broken:
            // the transcript was perfect and nothing happened.
            Tier0::Unrecognised => println!("UNRECOGNISED  -> Tier 1 (M3)"),
        }
    }
    println!("\n  {routed}/{} routed by grammar alone", phrases.len());
}
