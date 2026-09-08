//! Executing a routed command.
//!
//! This is stage 6 of the latency budget: one SQLite transaction, then the
//! acknowledgement. Nothing here waits on a model, a network, or an embedding —
//! a capture is durable the moment the transaction commits, and everything else
//! is backfilled (ADR-0001).

use memos_context::Context;
use memos_core::{Id, Intent, KnowledgeItem, RoutedCommand, Source, SourceKind};
use memos_db::{Db, DbResult};
use serde::Serialize;

/// What happened, in the words the receipt will use.
#[derive(Debug, Clone, Serialize)]
pub struct Outcome {
    pub kind: &'static str,
    /// One line, already phrased for the user: "Saved to Study / Programming / React".
    pub summary: String,
    /// Where it came from, shown beneath the summary.
    pub provenance: Option<String>,
    /// The item this created, so undo has something to reverse.
    pub item_id: Option<String>,
    /// What a retrieval intent found. Empty for everything else.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub results: Vec<Hit>,
    /// Where `OPEN` decided to go. The agent chooses the target; the shell call
    /// itself belongs to the platform layer, which is why this is returned
    /// rather than performed here.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub open: Option<OpenTarget>,
    pub took_ms: u32,
}

/// One search result, flattened for the interface.
#[derive(Debug, Clone, Serialize)]
pub struct Hit {
    pub id: String,
    pub title: String,
    /// Enough of the content to recognise the item by.
    pub snippet: String,
    pub collection: Option<String>,
    pub source_url: Option<String>,
    pub score: f32,
    /// Which retrievers found it: `keyword`, `vector`, or both. Shown because a
    /// result only the vector side found is a different kind of answer from an
    /// exact term match, and the user can tell the difference.
    pub why: String,
}

/// Something to reopen.
#[derive(Debug, Clone, Serialize)]
pub struct OpenTarget {
    pub item_id: String,
    pub title: String,
    /// A URL or a file path. Both `None` when the item was only ever a spoken
    /// note — there is nothing to reopen, and the receipt says so.
    pub url: Option<String>,
    pub file_path: Option<String>,
}

impl Outcome {
    fn done(kind: &'static str, summary: String, started: std::time::Instant) -> Self {
        Self {
            kind,
            summary,
            provenance: None,
            item_id: None,
            results: Vec::new(),
            open: None,
            took_ms: started.elapsed().as_millis() as u32,
        }
    }
}

/// Turn a routed command into a durable change.
///
/// Keyword-only retrieval. The embedder is optional by construction: a machine
/// that has never embedded anything still searches, it just searches worse.
pub fn execute(db: &Db, cmd: &RoutedCommand, ctx: &Context) -> DbResult<Outcome> {
    execute_with(db, cmd, ctx, None)
}

/// As `execute`, with the query already embedded.
///
/// The vector arrives from the caller rather than being computed here: the
/// embedder is loaded once, lives in the application, and must not become a
/// dependency of the routing layer.
pub fn execute_with(
    db: &Db,
    cmd: &RoutedCommand,
    ctx: &Context,
    query_vector: Option<&[f32]>,
) -> DbResult<Outcome> {
    let started = std::time::Instant::now();

    match cmd.intent {
        Intent::Save => save(db, cmd, ctx, started),
        Intent::Note => note(db, cmd, started),
        // Retrieval mutates nothing but the access counter, so it runs eagerly
        // and the answer is in the overlay before it fades.
        Intent::Search | Intent::Show => search(db, cmd, query_vector, started),
        Intent::Open => open(db, cmd, query_vector, started),
        other => Ok(Outcome::done(
            "unsupported",
            format!("{} is not implemented yet", other.as_str()),
            started,
        )),
    }
}

/// How many results the overlay can usefully show.
///
/// Small on purpose: this is a voice interface, and a list you have to read
/// through has already failed. The Hub carries the long tail.
const OVERLAY_RESULTS: usize = 5;

fn search(
    db: &Db,
    cmd: &RoutedCommand,
    query_vector: Option<&[f32]>,
    started: std::time::Instant,
) -> DbResult<Outcome> {
    let query = query_of(cmd);
    let hits = to_hits(
        db,
        memos_retrieval::search_hybrid(db, &query, query_vector, OVERLAY_RESULTS)?,
    )?;

    let summary = match hits.len() {
        0 => format!("Nothing found for \u{201c}{query}\u{201d}"),
        1 => hits[0].title.clone(),
        n => format!("{n} results for \u{201c}{query}\u{201d}"),
    };
    Ok(Outcome {
        kind: "search",
        summary,
        provenance: hits.first().and_then(|h| h.collection.clone()),
        item_id: None,
        results: hits,
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// Reopen the source behind the best match.
///
/// OPEN is a search whose answer is acted on rather than listed. It commits to
/// the top hit: asking "which one?" about a command the user phrased as a
/// single destination is worse than being occasionally wrong, and the
/// runners-up come back too, so a wrong guess is one click from corrected.
fn open(
    db: &Db,
    cmd: &RoutedCommand,
    query_vector: Option<&[f32]>,
    started: std::time::Instant,
) -> DbResult<Outcome> {
    let query = query_of(cmd);
    let hits = to_hits(
        db,
        memos_retrieval::search_hybrid(db, &query, query_vector, OVERLAY_RESULTS)?,
    )?;

    let Some(best) = hits.first().cloned() else {
        return Ok(Outcome::done(
            "open",
            format!("Nothing found for \u{201c}{query}\u{201d}"),
            started,
        ));
    };

    let id = Id::parse(&best.id).unwrap_or_default();
    let source = db.source_for_item(id)?;
    // Opening is an access, and an access is the cheapest real relevance label
    // there is: the result the user opened is the result that was right.
    db.record_access(id)?;

    let target = OpenTarget {
        item_id: best.id.clone(),
        title: best.title.clone(),
        url: source.as_ref().and_then(|s| s.url.clone()),
        file_path: source.as_ref().and_then(|s| s.file_path.clone()),
    };
    let summary = if target.url.is_some() || target.file_path.is_some() {
        format!("Opening {}", best.title)
    } else {
        // Nothing to reopen is not a failure — the memory itself is the answer,
        // and saying so beats a silent no-op.
        format!("{} \u{2014} a note, nothing to open", best.title)
    };

    Ok(Outcome {
        kind: "open",
        summary,
        provenance: best.collection.clone(),
        item_id: Some(best.id.clone()),
        results: hits,
        open: Some(target),
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// The words to search for.
///
/// Falls back to the whole transcript: a shape that routed to a retrieval
/// intent without filling the slot is better served by searching for everything
/// the user said than by refusing.
fn query_of(cmd: &RoutedCommand) -> String {
    cmd.slots
        .query
        .clone()
        .unwrap_or_else(|| cmd.transcript.clone())
}

fn to_hits(db: &Db, results: Vec<memos_retrieval::SearchResult>) -> DbResult<Vec<Hit>> {
    let paths = collection_paths_by_id(db)?;
    results
        .into_iter()
        .map(|r| {
            // Only ask for a source when the item claims one: the common case
            // is a spoken note, and a join per result to learn nothing is the
            // classic way a result list gets slow.
            let url = match r.item.source_id {
                Some(_) => db.source_for_item(r.item.id)?.and_then(|s| s.url),
                None => None,
            };
            Ok(Hit {
                id: r.item.id.to_string(),
                title: r.item.title.clone(),
                snippet: truncate(r.item.content.trim(), 160),
                collection: r
                    .item
                    .collection_id
                    .and_then(|c| paths.get(&c).cloned())
                    .map(|p| p.replace('/', " / ")),
                source_url: url,
                score: r.score,
                why: r
                    .sources
                    .iter()
                    .map(|(name, _)| *name)
                    .collect::<Vec<_>>()
                    .join(" + "),
            })
        })
        .collect()
}

fn collection_paths_by_id(db: &Db) -> DbResult<std::collections::HashMap<Id, String>> {
    Ok(db
        .collections_with_counts()?
        .into_iter()
        .map(|(c, _)| (c.id, c.path))
        .collect())
}

fn save(
    db: &Db,
    cmd: &RoutedCommand,
    ctx: &Context,
    started: std::time::Instant,
) -> DbResult<Outcome> {
    // "Save this" needs a *this*. With no selection, no URL and no window worth
    // naming, the only text available is the command itself — and storing "add
    // this to react" as a memory titled "add this to react" is not a capture,
    // it is an echo. It pollutes search, and it spends one of the fifty weekly
    // captures the free plan allows.
    //
    // Declining is not losing a thought: there was no thought in it. The
    // receipt says which word to use instead, because the user asking for this
    // almost always wanted NOTE.
    if !ctx.has_referent() && cmd.slots.title.is_none() && ctx.suggested_title().is_none() {
        return Ok(Outcome::done(
            "nothing",
            "Nothing to save here \u{2014} try \u{201c}note that\u{2026}\u{201d}".into(),
            started,
        ));
    }

    // Prefer what the user selected: it is the most direct evidence of what
    // "this" meant. Then the page it came from, then — only for a NOTE-shaped
    // command that carried its own words — the title slot.
    //
    // Never the raw transcript. "add this to react" is the instruction, not the
    // memory, and storing it as the body puts the command vocabulary into the
    // search index, where it matches every future query containing "this".
    let content = ctx
        .selected_text
        .clone()
        .or_else(|| ctx.current_url.clone())
        .or_else(|| cmd.slots.title.clone())
        .or_else(|| ctx.suggested_title())
        .unwrap_or_default();

    let title = cmd
        .slots
        .title
        .clone()
        .or_else(|| ctx.suggested_title())
        .unwrap_or_else(|| first_words(&cmd.transcript, 8));

    let mut item = KnowledgeItem::capture(truncate(&title, 200), content);

    // Record where this came from, when there is a "where". This is what makes
    // the memory reopenable rather than merely findable, and it has to happen
    // now: the browser tab the user is looking at is evidence that expires the
    // moment they move on.
    if let Some(source) = source_from(ctx) {
        db.put_source(&source)?;
        item.source_id = Some(source.id);
    }

    if let Some(path) = &cmd.slots.collection {
        item.collection_id = db.collection_id_by_path(path)?;
    }
    db.capture(&item)?;

    let where_to = cmd
        .slots
        .collection
        .clone()
        .map(|p| p.replace('/', " / "))
        .unwrap_or_else(|| "Unfiled".into());

    Ok(Outcome {
        kind: "save",
        summary: format!("Saved to {where_to}"),
        provenance: provenance_line(ctx),
        item_id: Some(item.id.to_string()),
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

fn note(db: &Db, cmd: &RoutedCommand, started: std::time::Instant) -> DbResult<Outcome> {
    let text = cmd
        .slots
        .title
        .clone()
        .unwrap_or_else(|| cmd.transcript.clone());
    let item = KnowledgeItem::capture(truncate(&text, 200), text.clone());
    db.capture(&item)?;

    Ok(Outcome {
        kind: "note",
        summary: "Noted".into(),
        provenance: Some(truncate(&text, 90)),
        item_id: Some(item.id.to_string()),
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// What the capture came from, if anything durable enough to reopen.
///
/// A window title alone does not qualify: "Inbox (12)" is not somewhere you can
/// be sent back to. A URL is, and so is a file the user had open.
fn source_from(ctx: &Context) -> Option<Source> {
    let url = ctx.current_url.clone();
    let file = url.is_none().then(|| local_file(ctx)).flatten();
    if url.is_none() && file.is_none() {
        return None;
    }
    Some(Source {
        id: memos_core::Id::new(),
        kind: if url.is_some() {
            // A highlight is still a webpage as far as reopening goes; the
            // distinction that matters here is where it lives, not how it was
            // taken.
            SourceKind::Webpage
        } else {
            SourceKind::File
        },
        domain: url.as_deref().map(domain_of),
        url,
        file_path: file,
        title: ctx.suggested_title(),
        retrieved_at: memos_core::now(),
    })
}

/// A file path inferred from the window title.
///
/// Editors put the path in the title bar; nothing else reliably does. Anything
/// that is not an absolute path with an extension is left alone rather than
/// guessed at — a source that does not open is worse than no source at all.
fn local_file(ctx: &Context) -> Option<String> {
    let title = ctx.active_window_title.as_deref()?;
    title
        .split(['\u{2014}', '|'])
        .map(str::trim)
        .find(|part| {
            let looks_absolute = part.len() > 3
                && part.as_bytes()[1] == b':'
                && (part.contains('\\') || part.contains('/'));
            looks_absolute && std::path::Path::new(part).extension().is_some()
        })
        .map(str::to_string)
}

/// The source line under a receipt: where this came from and how.
fn provenance_line(ctx: &Context) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(url) = &ctx.current_url {
        parts.push(domain_of(url));
    } else if let Some(app) = &ctx.active_application {
        parts.push(app.trim_end_matches(".exe").to_string());
    }
    parts.push(
        if ctx.selected_text.is_some() {
            "highlighted text + voice"
        } else {
            "voice"
        }
        .to_string(),
    );
    if parts.is_empty() {
        None
    } else {
        Some(parts.join(" · "))
    }
}

fn domain_of(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .trim_start_matches("www.")
        .to_string()
}

fn first_words(s: &str, n: usize) -> String {
    let w: Vec<&str> = s.split_whitespace().take(n).collect();
    if w.is_empty() {
        "Untitled".into()
    } else {
        w.join(" ")
    }
}

fn truncate(s: &str, max: usize) -> String {
    // By characters, not bytes: slicing a multi-byte character in half would
    // panic, and titles are routinely not ASCII.
    if s.chars().count() <= max {
        s.to_string()
    } else {
        s.chars().take(max.saturating_sub(1)).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::{Confidence, Slots, Tier};

    fn cmd(intent: Intent, slots: Slots, transcript: &str) -> RoutedCommand {
        RoutedCommand {
            transcript: transcript.into(),
            intent,
            slots,
            confidence: Confidence::CERTAIN,
            tier: Tier::Grammar,
            routing_ms: 1,
        }
    }

    #[test]
    fn save_writes_an_item_into_the_named_collection() {
        let db = Db::open_in_memory().unwrap();
        let study = db.create_collection("Study", None).unwrap();
        let prog = db.create_collection("Programming", Some(study.id)).unwrap();
        db.create_collection("React", Some(prog.id)).unwrap();

        let ctx = Context {
            selected_text: Some("State is a snapshot for each render".into()),
            current_url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            active_window_title: Some("State as a Snapshot - React".into()),
            ..Default::default()
        };
        let c = cmd(
            Intent::Save,
            Slots {
                collection: Some("Study/Programming/React".into()),
                ..Default::default()
            },
            "save this to programming react",
        );

        let out = execute(&db, &c, &ctx).unwrap();
        assert_eq!(out.kind, "save");
        assert_eq!(out.summary, "Saved to Study / Programming / React");
        assert_eq!(
            out.provenance.as_deref(),
            Some("react.dev · highlighted text + voice")
        );

        let hits = db.search_keyword("snapshot", 5).unwrap();
        assert_eq!(hits.len(), 1, "the item must be findable immediately");
        assert!(hits[0].collection_id.is_some(), "and filed, not orphaned");
    }

    #[test]
    fn save_without_a_collection_is_still_captured() {
        let db = Db::open_in_memory().unwrap();
        let ctx = Context {
            selected_text: Some("something worth keeping".into()),
            ..Default::default()
        };
        let out = execute(&db, &cmd(Intent::Save, Slots::default(), "save this"), &ctx).unwrap();
        assert_eq!(out.summary, "Saved to Unfiled");
        assert_eq!(db.search_keyword("keeping", 5).unwrap().len(), 1);
    }

    #[test]
    fn a_note_stores_the_spoken_text() {
        let db = Db::open_in_memory().unwrap();
        let c = cmd(
            Intent::Note,
            Slots {
                title: Some("the router config is on the fridge".into()),
                ..Default::default()
            },
            "remember that the router config is on the fridge",
        );
        let out = execute(&db, &c, &Context::default()).unwrap();
        assert_eq!(out.kind, "note");
        assert_eq!(db.search_keyword("fridge", 5).unwrap().len(), 1);
    }

    #[test]
    fn falls_back_to_the_transcript_when_there_is_no_context() {
        // A window title is a weak referent, but it is a real one: the memory
        // is about something, so it is captured rather than refused.
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(Intent::Save, Slots::default(), "save this to react"),
            &Context {
                active_window_title: Some("State as a Snapshot - React".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(out.item_id.is_some(), "a capture must never be lost");
    }

    #[test]
    fn saving_with_nothing_to_save_says_so_instead_of_echoing_the_command() {
        // The failure this prevents: "add this to react" spoken at a blank
        // desktop, stored as a memory titled "add this to react". Nothing is
        // lost by declining — there was no content — and a library full of
        // command echoes is worse than an empty one.
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(Intent::Save, Slots::default(), "add this to react"),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "nothing");
        assert!(out.item_id.is_none());
        assert_eq!(db.item_count().unwrap(), 0, "no row may be written");
        assert!(out.summary.contains("note that"), "{}", out.summary);
    }

    #[test]
    fn truncation_never_splits_a_character() {
        let s = "Спецоперації Левиця ".repeat(30);
        let t = truncate(&s, 50);
        assert!(t.chars().count() <= 50);
    }

    #[test]
    fn domain_extraction() {
        assert_eq!(domain_of("https://www.react.dev/learn"), "react.dev");
        assert_eq!(domain_of("http://localhost:1420/x"), "localhost:1420");
    }
    #[test]
    fn saving_from_a_browser_records_a_reopenable_source() {
        let db = Db::open_in_memory().unwrap();
        let ctx = Context {
            active_application: Some("chrome.exe".into()),
            active_window_title: Some("State as a Snapshot - React".into()),
            current_url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            ..Default::default()
        };
        let out = execute(&db, &cmd(Intent::Save, Slots::default(), "save this"), &ctx).unwrap();

        let id = Id::parse(out.item_id.as_ref().unwrap()).unwrap();
        let source = db.source_for_item(id).unwrap().expect("a source row");
        assert_eq!(source.domain.as_deref(), Some("react.dev"));
        assert_eq!(source.kind, SourceKind::Webpage);
    }

    #[test]
    fn a_spoken_note_records_no_source() {
        // Nothing to reopen is a legitimate state, not a missing row.
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(
                Intent::Note,
                Slots {
                    title: Some("the router is behind the books".into()),
                    ..Default::default()
                },
                "note that the router is behind the books",
            ),
            &Context::default(),
        )
        .unwrap();
        let id = Id::parse(out.item_id.as_ref().unwrap()).unwrap();
        assert!(db.source_for_item(id).unwrap().is_none());
    }

    #[test]
    fn search_returns_the_matching_items() {
        let db = Db::open_in_memory().unwrap();
        db.capture(&KnowledgeItem::capture(
            "State as a Snapshot",
            "State is a snapshot for each render",
        ))
        .unwrap();
        db.capture(&KnowledgeItem::capture("Router config", "on the fridge"))
            .unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Search,
                Slots {
                    query: Some("snapshot".into()),
                    ..Default::default()
                },
                "find the snapshot one",
            ),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "search");
        assert_eq!(out.results.len(), 1);
        assert_eq!(out.results[0].title, "State as a Snapshot");
        assert_eq!(out.results[0].why, "keyword");
    }

    #[test]
    fn a_search_with_no_matches_says_so_rather_than_failing() {
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(
                Intent::Search,
                Slots {
                    query: Some("nothing here".into()),
                    ..Default::default()
                },
                "find nothing here",
            ),
            &Context::default(),
        )
        .unwrap();
        assert!(out.results.is_empty());
        assert!(out.summary.starts_with("Nothing found"));
    }

    #[test]
    fn open_targets_the_best_match_and_counts_the_access() {
        let db = Db::open_in_memory().unwrap();
        let ctx = Context {
            current_url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            active_window_title: Some("State as a Snapshot - React".into()),
            ..Default::default()
        };
        execute(&db, &cmd(Intent::Save, Slots::default(), "save this"), &ctx).unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Open,
                Slots {
                    query: Some("snapshot".into()),
                    ..Default::default()
                },
                "open the snapshot one",
            ),
            &Context::default(),
        )
        .unwrap();

        let target = out.open.expect("an open target");
        assert_eq!(
            target.url.as_deref(),
            Some("https://react.dev/learn/state-as-a-snapshot")
        );
        let id = Id::parse(&target.item_id).unwrap();
        assert_eq!(db.get_item(id).unwrap().unwrap().access_count, 1);
    }

    #[test]
    fn opening_a_note_reports_that_there_is_nothing_to_open() {
        let db = Db::open_in_memory().unwrap();
        db.capture(&KnowledgeItem::capture(
            "the router is behind the books",
            "the router is behind the books",
        ))
        .unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Open,
                Slots {
                    query: Some("router".into()),
                    ..Default::default()
                },
                "open the router one",
            ),
            &Context::default(),
        )
        .unwrap();

        let target = out.open.expect("still a target: the item itself");
        assert!(target.url.is_none() && target.file_path.is_none());
        assert!(out.summary.contains("nothing to open"));
    }

    #[test]
    fn a_vector_only_match_is_found_and_labelled_as_such() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("Snapshot", "State is a snapshot per render");
        db.capture(&item).unwrap();
        let mut v = vec![0.0f32; 384];
        v[3] = 1.0;
        db.put_embedding(item.id, "test", &v).unwrap();

        let out = execute_with(
            &db,
            &cmd(
                Intent::Search,
                Slots {
                    query: Some("zzzz".into()),
                    ..Default::default()
                },
                "find zzzz",
            ),
            &Context::default(),
            Some(&v),
        )
        .unwrap();

        assert_eq!(out.results.len(), 1);
        assert_eq!(out.results[0].why, "vector");
    }
    #[test]
    fn the_spoken_command_is_never_stored_as_the_memory() {
        // Saving from a window with a title but nothing selected: the title is
        // a real referent, so the capture happens — but the body must be the
        // page, not the instruction. "save this to react" in the content field
        // puts command vocabulary into the search index, where it goes on to
        // match every later query containing those words.
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(Intent::Save, Slots::default(), "save this to react"),
            &Context {
                active_window_title: Some("State as a Snapshot - React".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let id = Id::parse(out.item_id.as_ref().unwrap()).unwrap();
        let item = db.get_item(id).unwrap().unwrap();
        assert_eq!(item.title, "State as a Snapshot");
        assert!(
            !item.content.contains("save this"),
            "content was {:?}",
            item.content
        );
    }
}
