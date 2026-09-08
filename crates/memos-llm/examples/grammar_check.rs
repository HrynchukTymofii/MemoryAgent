//! Print the grammar the router would decode under.
//!
//!   cargo run -p memos-llm --example grammar_check
//!
//! A grammar is the one part of this system that fails silently: a stale or
//! malformed one does not error, it just makes a valid destination unreachable
//! or a whole intent unroutable. Being able to read it is the difference
//! between diagnosing that in a minute and in an afternoon.

use memos_llm::grammar;

/// The collections a new install ships with.
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

fn main() {
    let collections: Vec<String> = STARTER.iter().map(|s| s.to_string()).collect();
    let grammar = grammar::build(&collections);

    println!("--- grammar for {} collections ---", collections.len());
    println!("{grammar}");
    println!(
        "{} bytes, {} rules\n",
        grammar.len(),
        grammar.lines().filter(|l| l.contains("::=")).count()
    );

    // The empty case is the one nobody tries by hand, and it is what every
    // brand-new install looks like for its first few minutes.
    let empty = grammar::build(&[]);
    println!("--- with no collections yet ---");
    println!("{empty}");
}
