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
        // The three that act on a memory that already exists. Each resolves
        // "this" the same way and each writes its own inverse, so any of them
        // can be taken back by the next word the user says.
        Intent::Move => move_to(db, cmd, started),
        Intent::Tag => tag(db, cmd, started),
        Intent::Task => task(db, cmd, started),
        Intent::Undo => undo(db, started),
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
        .referent()
        .map(str::to_string)
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
    db.capture(&item, Some(cmd.id))?;
    // And into the document, which is the half of this a person reads.
    integrate(db, &item, cmd.slots.collection.as_deref(), note_provenance(ctx, &item));

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
    let mut item = KnowledgeItem::capture(truncate(&text, 200), text.clone());
    // A spoken note is filed like anything else when the command said where.
    // Without this it is a thought with nowhere to live, which is the shape
    // the note model exists to get rid of.
    if let Some(path) = &cmd.slots.collection {
        item.collection_id = db.collection_id_by_path(path)?;
    }
    db.capture(&item, Some(cmd.id))?;
    integrate(db, &item, cmd.slots.collection.as_deref(), None);

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


/// Refile the memory the user just made.
///
/// "this" is the newest capture — see `Db::most_recent_capture`. That is the
/// whole referent story for `MOVE` today, and it is the case that actually
/// comes up: you say where something goes, see the receipt, and correct it.
/// Refiling an arbitrary memory is a job for the library, where you can see
/// what you are pointing at.
fn move_to(db: &Db, cmd: &RoutedCommand, started: std::time::Instant) -> DbResult<Outcome> {
    let Some(item) = db.most_recent_capture()? else {
        return Ok(Outcome::done(
            "nothing",
            "Nothing to move \u{2014} save something first".into(),
            started,
        ));
    };
    // The grammar only routes MOVE with a destination it resolved, so this is
    // belt and braces rather than a real branch.
    let Some(path) = cmd.slots.collection.clone() else {
        return Ok(Outcome::done(
            "nothing",
            "Move it where? Say a collection.".into(),
            started,
        ));
    };
    let Some(collection) = db.collection_id_by_path(&path)? else {
        return Ok(Outcome::done(
            "nothing",
            format!("No collection called \u{201c}{path}\u{201d}"),
            started,
        ));
    };

    db.move_item(item.id, Some(collection))?;
    Ok(Outcome {
        kind: "move",
        summary: format!("Moved to {}", path.replace('/', " / ")),
        provenance: Some(truncate(&item.title, 90)),
        item_id: Some(item.id.to_string()),
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// Attach a tag to the memory the user just made.
///
/// Tagging never moves anything: an item keeps its collection and gains a
/// second way of being found (§9). Re-tagging is reported as such rather than
/// as a fresh success, because a receipt that says "Tagged" twice for one tag
/// is a receipt that lies about what the second command did.
fn tag(db: &Db, cmd: &RoutedCommand, started: std::time::Instant) -> DbResult<Outcome> {
    let Some(name) = cmd.slots.tags.first().map(|t| t.trim().to_string()).filter(|t| !t.is_empty())
    else {
        return Ok(Outcome::done("nothing", "Tag it what?".into(), started));
    };
    let Some(item) = db.most_recent_capture()? else {
        return Ok(Outcome::done(
            "nothing",
            "Nothing to tag \u{2014} save something first".into(),
            started,
        ));
    };

    let added = db.tag_item(item.id, &name)?;
    Ok(Outcome {
        kind: if added { "tag" } else { "nothing" },
        summary: if added {
            format!("Tagged \u{201c}{name}\u{201d}")
        } else {
            format!("Already tagged \u{201c}{name}\u{201d}")
        },
        provenance: Some(truncate(&item.title, 90)),
        item_id: Some(item.id.to_string()),
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// Create a task.
///
/// A task is not a reminder, and the difference is why this routes at Tier 0
/// while `REMINDER` still does not: a reminder is meaningless without a time,
/// and "next Tuesday" needs interpretation the grammar cannot do safely. A task
/// is complete with nothing but its words.
///
/// Nothing is attached to it. This used to link the newest capture on the
/// theory that "create a task to finish this" follows a save — but it never
/// checked that "this" had been said, or that the save was seconds rather than
/// days ago, so every task came out stapled to whatever was saved last. A task
/// arrives when a thought does, which is usually nowhere near the last thing
/// the user filed. Task is task, memory is memory.
fn task(db: &Db, cmd: &RoutedCommand, started: std::time::Instant) -> DbResult<Outcome> {
    let Some(title) = cmd.slots.title.clone().filter(|t| !t.trim().is_empty()) else {
        return Ok(Outcome::done("nothing", "A task to do what?".into(), started));
    };

    let task = db.create_task(&truncate(&title, 200), None, None)?;
    Ok(Outcome {
        kind: "task",
        summary: format!("Task: {}", truncate(&task.title, 70)),
        provenance: None,
        item_id: None,
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// Take back the last thing that happened.
///
/// This is the other half of ADR-0005's bargain — the system acts without
/// asking because acting is cheap to reverse — and, since Tier 1 started
/// resolving the ambiguity that used to produce questions, it is also the main
/// way the user can tell the system it was wrong. So the reversal is recorded
/// as a verdict against the command that caused it (ADR-0006), not just
/// performed.
fn undo(db: &Db, started: std::time::Instant) -> DbResult<Outcome> {
    let Some(undone) = db.undo_last()? else {
        return Ok(Outcome::done(
            "nothing",
            "Nothing recent to undo".into(),
            started,
        ));
    };

    // Failing to record the verdict must not fail the undo. The user asked for
    // their memory back, not for bookkeeping.
    if let Some(command) = undone.command_id {
        if let Err(e) = db.record_correction(command, false, None, &memos_core::Slots::default()) {
            tracing::warn!(?e, "undone, but the verdict was not recorded");
        }
    }

    Ok(Outcome {
        kind: "undo",
        summary: format!("Undid {}", undone.what),
        provenance: None,
        item_id: None,
        results: Vec::new(),
        open: None,
        took_ms: started.elapsed().as_millis() as u32,
    })
}

/// Fold a capture into its collection's note (ADR-0010).
///
/// After the capture commits, never inside it. The memory is already durable
/// and the document can be rebuilt from the rows, so a failure here is a
/// warning and nothing more — the opposite ordering would let a bad heading
/// lose a thought.
fn integrate(db: &Db, item: &KnowledgeItem, path: Option<&str>, provenance: Option<String>) {
    let (Some(collection), Some(path)) = (item.collection_id, path) else {
        return;
    };
    let name = path.rsplit('/').next().unwrap_or(path);
    let text = if item.content.trim().is_empty() {
        &item.title
    } else {
        &item.content
    };
    if let Err(e) = db.integrate_capture(
        collection,
        name,
        item.id,
        &item.title,
        text,
        provenance.as_deref(),
    ) {
        tracing::warn!(?e, "captured, but not integrated into the note");
    }
}

/// The line under an integrated block: when it was captured, and what it came
/// from when that is somewhere you can go back to.
///
/// Markdown rather than prose — it lands inside a document the user edits, and
/// a link they can click is the whole reason the note is worth reading later.
fn note_provenance(ctx: &Context, item: &KnowledgeItem) -> Option<String> {
    let mut line = item.captured_at.format("%-d %b %Y").to_string();
    if let Some(url) = &ctx.current_url {
        line.push_str(&format!(" \u{00b7} [{}]({url})", domain_of(url)));
    }
    Some(line)
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
    // Say which kind of capture this was. "Page + voice" and "voice" look the
    // same in the library but mean very different things about what will be
    // findable later, and the moment to learn that is now rather than in three
    // months when a search comes up empty.
    parts.push(
        if ctx.selected_text.is_some() {
            "highlighted text + voice"
        } else if ctx.page_text.is_some() {
            "page + voice"
        } else if ctx.current_url.is_some() {
            "link only + voice"
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
            id: Id::new(),
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

    /// ADR-0010: the capture is the record, and the note is what you read.
    /// Both, from one command.
    #[test]
    fn saving_grows_the_collection_note_and_links_the_source() {
        let db = Db::open_in_memory().unwrap();
        db.create_collection("React", None).unwrap();
        let ctx = Context {
            selected_text: Some("Setting state queues a re-render".into()),
            current_url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            active_window_title: Some("State as a Snapshot - React".into()),
            ..Default::default()
        };
        let slots = || Slots {
            collection: Some("React".into()),
            ..Default::default()
        };

        execute(&db, &cmd(Intent::Save, slots(), "save this to react"), &ctx).unwrap();
        let note = db.note_for_path("React").unwrap().expect("a note was started");
        assert!(note.body.contains("## State as a Snapshot"), "{}", note.body);
        assert!(
            note.body.contains("[react.dev](https://react.dev/learn/state-as-a-snapshot)"),
            "the resource has to be reachable from the note: {}",
            note.body,
        );

        // The same subject a second time joins the section rather than opening
        // a second one — the whole point of a note over a list of rows.
        execute(&db, &cmd(Intent::Save, slots(), "save this to react"), &ctx).unwrap();
        let note = db.note_for_path("React").unwrap().unwrap();
        assert_eq!(note.body.matches("## ").count(), 1, "{}", note.body);
        assert_eq!(db.note_sources(note.id).unwrap(), 2, "both captures are recorded");
    }

    #[test]
    fn a_note_command_with_a_destination_lands_in_that_document() {
        let db = Db::open_in_memory().unwrap();
        db.create_collection("Household", None).unwrap();
        let out = execute(
            &db,
            &cmd(
                Intent::Note,
                Slots {
                    title: Some("Hold the reset pin for ten seconds".into()),
                    collection: Some("Household".into()),
                    ..Default::default()
                },
                "note that the router resets by holding the pin",
            ),
            &Context::default(),
        )
        .unwrap();
        assert_eq!(out.kind, "note");
        let note = db.note_for_path("Household").unwrap().expect("filed");
        assert!(note.body.contains("Hold the reset pin"), "{}", note.body);
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
        ), None)
        .unwrap();
        db.capture(&KnowledgeItem::capture("Router config", "on the fridge"), None)
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
        ), None)
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
        db.capture(&item, None).unwrap();
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
    #[test]
    fn the_article_is_saved_rather_than_a_link_to_it() {
        // The difference between a memory and a bookmark. Before page capture
        // the body of an unhighlighted save was the URL, so the item was
        // findable only by words in its title.
        let db = Db::open_in_memory().unwrap();
        let ctx = Context {
            active_window_title: Some("State as a Snapshot - React".into()),
            current_url: Some("https://react.dev/learn/state-as-a-snapshot".into()),
            page_text: Some("State is a snapshot for each render.".into()),
            ..Default::default()
        };
        let out = execute(&db, &cmd(Intent::Save, Slots::default(), "save this"), &ctx).unwrap();

        let id = Id::parse(out.item_id.as_ref().unwrap()).unwrap();
        let item = db.get_item(id).unwrap().unwrap();
        assert_eq!(item.content, "State is a snapshot for each render.");
        assert!(out.provenance.unwrap().contains("page + voice"));
    }

    // ------------------------------------------------ acting on what exists

    /// The whole referent story for MOVE and TAG: "this" is the memory you
    /// just made, not the one you last looked at.
    #[test]
    fn move_refiles_the_memory_that_was_just_captured() {
        let db = Db::open_in_memory().unwrap();
        let study = db.create_collection("Study", None).unwrap();
        execute(
            &db,
            &cmd(Intent::Save, Slots::default(), "save this"),
            &Context {
                selected_text: Some("state is a snapshot".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Move,
                Slots { collection: Some("Study".into()), ..Default::default() },
                "move this to study",
            ),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "move");
        assert_eq!(out.summary, "Moved to Study");
        let item = db.recent_items(1).unwrap().remove(0);
        assert_eq!(item.collection_id, Some(study.id));
    }

    /// A destination that does not exist must not be created by saying it.
    /// Collections are made deliberately; a typo or a mishearing that invented
    /// one would fill the sidebar with folders nobody chose.
    #[test]
    fn moving_to_a_collection_that_does_not_exist_says_so() {
        let db = Db::open_in_memory().unwrap();
        db.capture(&KnowledgeItem::capture("Hooks", "body"), None).unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Move,
                Slots { collection: Some("Nowhere".into()), ..Default::default() },
                "move this to nowhere",
            ),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "nothing");
        assert!(out.summary.contains("Nowhere"), "{}", out.summary);
        assert_eq!(db.collections_with_counts().unwrap().len(), 0);
    }

    #[test]
    fn a_tag_is_added_once_and_reported_honestly_the_second_time() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("Hooks", "body");
        db.capture(&item, None).unwrap();
        let c = cmd(
            Intent::Tag,
            Slots { tags: vec!["react".into()], ..Default::default() },
            "tag this react",
        );

        let first = execute(&db, &c, &Context::default()).unwrap();
        assert_eq!(first.kind, "tag");
        assert_eq!(first.summary, "Tagged \u{201c}react\u{201d}");

        // The receipt must not claim a second success for a tag that was
        // already there — that is a receipt that lies about what happened.
        let again = execute(&db, &c, &Context::default()).unwrap();
        assert_eq!(again.kind, "nothing");
        assert_eq!(again.summary, "Already tagged \u{201c}react\u{201d}");
        assert_eq!(db.tags_for_item(item.id).unwrap(), vec!["react"]);
    }

    /// The newest memory is not what the task is about. It is merely the last
    /// thing filed, which on any real timeline is days old and unrelated.
    #[test]
    fn a_task_is_not_linked_to_whatever_was_saved_last() {
        let db = Db::open_in_memory().unwrap();
        let item = KnowledgeItem::capture("Hooks", "body");
        db.capture(&item, None).unwrap();

        let out = execute(
            &db,
            &cmd(
                Intent::Task,
                Slots { title: Some("call the bank".into()), ..Default::default() },
                "add a task to call the bank",
            ),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "task");
        assert_eq!(out.summary, "Task: call the bank");
        assert_eq!(out.provenance, None);
        let tasks = db.tasks(10).unwrap();
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0].item_id, None);
    }

    /// A task stands on its own. This is what separates it from a reminder,
    /// and it must not require a memory to hang off.
    #[test]
    fn a_task_with_nothing_captured_yet_still_works() {
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(
                Intent::Task,
                Slots { title: Some("call the bank".into()), ..Default::default() },
                "add a task to call the bank",
            ),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "task");
        assert_eq!(out.provenance, None);
        assert_eq!(db.tasks(10).unwrap().len(), 1);
    }

    #[test]
    fn undo_reverses_the_save_and_names_what_it_removed() {
        let db = Db::open_in_memory().unwrap();
        execute(
            &db,
            &cmd(Intent::Save, Slots::default(), "save this"),
            &Context {
                selected_text: Some("state is a snapshot".into()),
                ..Default::default()
            },
        )
        .unwrap();

        let out = execute(
            &db,
            &cmd(Intent::Undo, Slots::default(), "undo"),
            &Context::default(),
        )
        .unwrap();

        assert_eq!(out.kind, "undo");
        assert!(out.summary.starts_with("Undid the memory"), "{}", out.summary);
        assert!(db.recent_items(10).unwrap().is_empty());
    }

    /// The reason undo is worth more than a repair: it is the verdict that
    /// derived confidence calibrates on (ADR-0006). Losing this link would
    /// leave the correction log reading as if every command was accepted.
    #[test]
    fn undo_records_a_verdict_against_the_command_it_reversed() {
        let db = Db::open_in_memory().unwrap();
        let save = cmd(Intent::Save, Slots::default(), "save this");
        let command_id = save.id;
        db.log_command(&save, &Context::default(), 1).unwrap();
        execute(
            &db,
            &save,
            &Context {
                selected_text: Some("state is a snapshot".into()),
                ..Default::default()
            },
        )
        .unwrap();

        execute(&db, &cmd(Intent::Undo, Slots::default(), "undo"), &Context::default()).unwrap();

        let logged = db.export_commands(10).unwrap();
        let row = logged
            .iter()
            .find(|c| c.id == command_id)
            .expect("the save is in the log");
        let verdict = row.correction.as_ref().expect("undo left a verdict");
        assert!(!verdict.accepted, "an undone save is a rejection");
    }

    #[test]
    fn undo_with_nothing_to_reverse_says_so_rather_than_failing() {
        let db = Db::open_in_memory().unwrap();
        let out = execute(
            &db,
            &cmd(Intent::Undo, Slots::default(), "undo"),
            &Context::default(),
        )
        .unwrap();
        assert_eq!(out.kind, "nothing");
        assert_eq!(out.summary, "Nothing recent to undo");
    }

    #[test]
    fn a_selection_still_beats_the_page_it_sits_in() {
        // Highlighting is an explicit choice about what matters; the page is
        // only what happened to be on screen.
        let db = Db::open_in_memory().unwrap();
        let ctx = Context {
            selected_text: Some("the one paragraph that mattered".into()),
            page_text: Some("the whole article, most of which did not".into()),
            current_url: Some("https://react.dev/learn".into()),
            ..Default::default()
        };
        let out = execute(&db, &cmd(Intent::Save, Slots::default(), "save this"), &ctx).unwrap();

        let id = Id::parse(out.item_id.as_ref().unwrap()).unwrap();
        assert_eq!(
            db.get_item(id).unwrap().unwrap().content,
            "the one paragraph that mattered"
        );
    }
}
