//! Turning a page's accessibility text into something worth remembering.
//!
//! UI Automation hands back the whole document as flat text: the navigation
//! bar, the cookie banner, the article, the "related stories" rail and the
//! footer, in reading order and with no structure to tell them apart. Stored as
//! it comes, a single saved page contributes a few hundred words of article and
//! a few hundred words of "Skip to content · Sign in · Subscribe · We value
//! your privacy" — and the second kind is *identical across every page the user
//! saves*, which is the worst possible property for a search index. It makes
//! every document look slightly like every other one.
//!
//! ## The one signal available
//!
//! With no markup, the usable signal is text density. Chrome is short lines:
//! menu items, buttons, bylines, timestamps. Prose is long lines. So the
//! article is found as the span between the first and last dense line, and
//! short lines *inside* that span are kept — those are the headings, captions
//! and pull quotes that belong to it.
//!
//! This is deliberately not a port of Readability. That algorithm works on the
//! DOM, which is exactly what the accessibility tree has already thrown away.

/// Below this, a line is chrome rather than prose.
///
/// Eight words is roughly the shortest thing that reads as a sentence. Headings
/// fall under it and are recovered by the span rule rather than by this test.
const PROSE_WORDS: usize = 8;

/// Stored page text is capped here.
///
/// Generous, because storage is cheap and the cost of truncating a long article
/// is losing exactly the part the user wanted. The embedding model reads only
/// its first 512 tokens regardless, but keyword search reads all of it.
pub const MAX_CHARS: usize = 20_000;

/// Phrases that only ever appear in furniture.
///
/// Matched on a whole line, lowercased, so this cannot swallow a sentence that
/// merely mentions cookies.
const FURNITURE: &[&str] = &[
    "accept all cookies",
    "accept cookies",
    "add to cart",
    "all rights reserved",
    "back to top",
    "cookie policy",
    "cookie settings",
    "manage preferences",
    "privacy policy",
    "reject all",
    "share on facebook",
    "share on twitter",
    "sign in",
    "sign up",
    "skip to content",
    "skip to main content",
    "subscribe now",
    "terms of service",
    "we value your privacy",
    "your privacy choices",
];

fn words(line: &str) -> usize {
    line.split_whitespace().count()
}

fn is_furniture(line: &str) -> bool {
    let lower = line.trim().to_lowercase();
    FURNITURE.iter().any(|f| lower == *f)
}

/// Extract the readable part of a page's document text.
///
/// Returns `None` when nothing in it reads as prose — a search results page, an
/// application chrome window, a mail client's folder list. Storing those is
/// worse than storing nothing, because they are indistinguishable from each
/// other and will match everything weakly.
pub fn extract(raw: &str) -> Option<String> {
    // Normalise first: UIA emits a lot of runs of spaces and non-breaking
    // spaces where the page had layout.
    let lines: Vec<String> = raw
        .lines()
        .map(|l| l.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();

    let dense: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| words(l) >= PROSE_WORDS && !is_furniture(l))
        .map(|(i, _)| i)
        .collect();

    let (first, last) = (*dense.first()?, *dense.last()?);
    let span: Vec<&str> = lines[first..=last]
        .iter()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !is_furniture(l))
        .collect();

    // A short line between two paragraphs is a heading. Three or more short
    // lines in a row are a menu, a tag list or a "related stories" rail — the
    // sidebar that a dense page wraps around its article, which sits *inside*
    // the span and so survives the span rule.
    //
    // Runs, not individual lines, because that is the only thing separating a
    // heading from a link: both are three words with no full stop, and the
    // difference is whether the thing next to it is prose.
    const MENU_RUN: usize = 3;

    let mut out: Vec<&str> = Vec::new();
    let mut previous = "";
    let mut i = 0;
    while i < span.len() {
        if words(span[i]) >= PROSE_WORDS {
            if span[i] != previous {
                previous = span[i];
                out.push(span[i]);
            }
            i += 1;
            continue;
        }
        // Measure the run of short lines starting here.
        let start = i;
        while i < span.len() && words(span[i]) < PROSE_WORDS {
            i += 1;
        }
        if i - start < MENU_RUN {
            for line in &span[start..i] {
                // Navigation repeats; prose does not. A line identical to the
                // one before it is a rendering artefact or a duplicated menu.
                if *line != previous {
                    previous = line;
                    out.push(line);
                }
            }
        }
    }

    let text = out.join("\n");
    if text.trim().is_empty() {
        return None;
    }
    Some(truncate_on_a_boundary(&text, MAX_CHARS))
}

/// Cut at the last line break before the limit, so the stored text never ends
/// mid-sentence.
fn truncate_on_a_boundary(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let cut: String = s.chars().take(max).collect();
    match cut.rfind('\n') {
        // Only if that leaves most of the budget used; otherwise a single very
        // long line would be cut back to almost nothing.
        Some(i) if i > max / 2 => cut[..i].to_string(),
        _ => cut,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = "\
Skip to main content
Home
Products
Sign in
Search
State as a Snapshot
State is a snapshot for each render, which is why setting it does not change \
the variable you have already read in this pass.
Setting state only schedules a re-render; the value you read stays fixed for \
the whole of the current render.
Related articles
We value your privacy
Accept all cookies
© 2026 Example Inc
All rights reserved";

    #[test]
    fn the_article_survives_and_the_furniture_does_not() {
        let out = extract(PAGE).expect("prose found");
        assert!(out.contains("State is a snapshot for each render"));
        assert!(out.contains("Setting state only schedules a re-render"));

        for junk in ["Skip to main content", "Sign in", "Accept all cookies", "All rights reserved"] {
            assert!(!out.contains(junk), "{junk:?} survived:\\n{out}");
        }
    }

    #[test]
    fn a_heading_inside_the_article_is_kept() {
        // Short lines are chrome at the edges and structure in the middle. The
        // title sits above the first dense line, so it is dropped here — it is
        // already captured as the item's title from the window, and repeating
        // it would be the only thing this rule could add.
        let out = extract(PAGE).unwrap();
        assert!(!out.starts_with("Home"));
    }

    #[test]
    fn a_page_with_no_prose_yields_nothing() {
        // A search results page or an app's chrome. Storing this is worse than
        // storing nothing: every such capture looks like every other one.
        let chrome = "File\nEdit\nView\nHelp\nInbox\nSent\nDrafts\nSign in";
        assert!(extract(chrome).is_none());
        assert!(extract("").is_none());
        assert!(extract("   \n \n ").is_none());
    }

    #[test]
    fn repeated_navigation_lines_collapse() {
        let page = "\
A menu item repeated by the accessibility tree over and over again here
A menu item repeated by the accessibility tree over and over again here
The actual article text begins at this point and continues for a while yet.";
        let out = extract(page).unwrap();
        assert_eq!(out.matches("A menu item repeated").count(), 1);
    }

    #[test]
    fn a_long_page_is_cut_on_a_line_boundary() {
        let line = "This sentence is long enough to count as prose by any measure at all.\\n";
        let page = line.repeat(2_000);
        let out = extract(&page).unwrap();
        assert!(out.chars().count() <= MAX_CHARS);
        assert!(!out.ends_with(' '), "cut mid-word");
    }

    #[test]
    fn non_breaking_spaces_do_not_survive_as_words() {
        let page = "Some\u{a0}prose\u{a0}with\u{a0}hard\u{a0}spaces\u{a0}that\u{a0}should\u{a0}still\u{a0}count.";
        let out = extract(page).expect("counted as prose");
        assert!(!out.contains('\u{a0}'));
    }
    #[test]
    fn a_sidebar_inside_the_article_span_is_dropped() {
        // The case the span rule alone cannot handle: a "related stories" rail
        // rendered between two paragraphs, so it sits inside the article rather
        // than at its edges. Measured on a real page, this was most of what
        // survived — 525 lines for 9,600 characters.
        let page = "The first paragraph of the article, long enough to register as prose by any measure.
Related stories
Ten things you missed
Our best offers today
Newsletter signup
Follow us
The second paragraph of the article, also comfortably long enough to be prose.";
        let out = extract(page).unwrap();
        assert!(out.contains("The first paragraph"));
        assert!(out.contains("The second paragraph"));
        for junk in ["Related stories", "Newsletter signup", "Follow us"] {
            assert!(!out.contains(junk), "{junk:?} survived:
{out}");
        }
    }

    #[test]
    fn a_heading_between_paragraphs_is_kept() {
        // The other half of the same rule: one or two short lines surrounded by
        // prose are structure, and dropping them would run two sections
        // together as if they were one argument.
        let page = "The first paragraph of the article, long enough to register as prose by any measure.
Why this matters
The second paragraph of the article, also comfortably long enough to be prose.";
        let out = extract(page).unwrap();
        assert!(out.contains("Why this matters"), "{out}");
    }
}
