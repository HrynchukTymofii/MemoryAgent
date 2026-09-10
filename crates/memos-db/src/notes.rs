//! Notes — the document a collection is (ADR-0010).
//!
//! A capture is a receipt: what was said, when, from where, and it never
//! changes. A note is what a person actually wants back — one Markdown file per
//! collection that grows as they learn, with the resources they gathered linked
//! inside it. Captures are integrated into it; they are not replaced by it.
//!
//! The merge rule lives in [`integrate`], which is pure and appends only. That
//! is the whole safety argument for letting a model take this over later: a
//! paragraph the user wrote cannot be rewritten by a capture, whoever decided
//! where the capture goes.

use memos_core::{Id, Timestamp};
use rusqlite::{params, Connection, OptionalExtension};
use serde::Serialize;

use crate::{Db, DbResult};

/// One collection's document.
#[derive(Debug, Clone, Serialize)]
pub struct Note {
    pub id: Id,
    /// The collection it belongs to. `None` once that collection is deleted —
    /// the note survives its shelf.
    pub collection_id: Option<Id>,
    pub title: String,
    /// Markdown. The editor round-trips through this, and so does sync.
    pub body: String,
    pub created_at: Timestamp,
    pub updated_at: Timestamp,
    /// When a person last edited it, as opposed to a capture growing it.
    pub edited_at: Option<Timestamp>,
}

/// Fold one capture into the document, at the end of the section its
/// collection names.
///
/// `trail` is the collection path — `["Study", "Programming", "React"]` — and
/// it becomes the heading trail: `# Study`, `## Programming`, `### React`. The
/// user's own filing *is* the outline, so the one document reads as the shape
/// they already think in, and a capture never needs a filename.
///
/// The rules, and nothing else:
///
/// 1. Each level is matched against the headings already inside its parent's
///    section, ignoring case and punctuation.
/// 2. A level that is not there is created at the end of its parent's section.
/// 3. The text is appended at the end of the deepest section, after everything
///    already under it and before the next heading.
/// 4. `provenance` follows the text as its own italic line.
///
/// Nothing already in the document is edited, moved or removed. Every path
/// through this function is an insertion — which is what makes it safe to let a
/// model choose the trail later instead of the router.
pub fn integrate(body: &str, trail: &[String], text: &str, provenance: Option<&str>) -> String {
    let text = text.trim();
    if text.is_empty() {
        return body.to_string();
    }
    // A capture with nowhere to go still has to land somewhere a person will
    // find it, and the bottom of the document is where they will look.
    let unfiled = [String::from("Unfiled")];
    let trail: &[String] = if trail.is_empty() { &unfiled } else { trail };

    let mut block: Vec<String> = vec![text.to_string()];
    if let Some(p) = provenance.map(str::trim).filter(|p| !p.is_empty()) {
        block.push(String::new());
        block.push(format!("*{p}*"));
    }

    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    while lines.last().is_some_and(|l| l.trim().is_empty()) {
        lines.pop();
    }

    // The span of the section we are currently inside, narrowing a level at a
    // time. It starts as the whole document.
    let (mut lo, mut hi) = (0usize, lines.len());

    for (depth, name) in trail.iter().enumerate() {
        // Markdown runs out of heading levels before a collection tree runs out
        // of depth. Past the sixth, everything shares the last one rather than
        // emitting `####### `, which is not a heading at all.
        let level = (depth + 1).min(6);
        match find_heading(&lines, lo, hi, level, name) {
            Some(at) => {
                hi = section_end(&lines, at, level, hi);
                lo = at + 1;
            }
            None => {
                let at = end_of_section(&lines, lo, hi);
                let mut insert = Vec::new();
                if at > 0 {
                    insert.push(String::new());
                }
                insert.push(format!("{} {name}", "#".repeat(level)));
                let added = insert.len();
                lines.splice(at..at, insert);
                lo = at + added;
                hi = lo;
            }
        }
    }

    let at = end_of_section(&lines, lo, hi);
    let mut insert = Vec::new();
    if at > 0 {
        insert.push(String::new());
    }
    insert.extend(block);
    lines.splice(at..at, insert);

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// `## Heading` -> `(2, "Heading")`. `None` for anything that is not one.
fn heading(line: &str) -> Option<(usize, &str)> {
    let hashes = line.chars().take_while(|c| *c == '#').count();
    if hashes == 0 || hashes > 6 {
        return None;
    }
    let rest = line[hashes..].strip_prefix(' ')?;
    Some((hashes, rest.trim()))
}

/// A heading of exactly `level` and this name, somewhere in `lo..hi`.
fn find_heading(lines: &[String], lo: usize, hi: usize, level: usize, name: &str) -> Option<usize> {
    let want = key(name);
    (lo..hi).find(|&i| heading(&lines[i]).is_some_and(|(l, h)| l == level && key(h) == want))
}

/// Where a section beginning at `start` ends: the next heading at the same
/// level or shallower, or the end of the span it lives in.
fn section_end(lines: &[String], start: usize, level: usize, hi: usize) -> usize {
    (start + 1..hi)
        .find(|&i| heading(&lines[i]).is_some_and(|(l, _)| l <= level))
        .unwrap_or(hi)
}

/// The line a new block belongs on: after the section's last non-empty line.
///
/// Trailing blank lines belong to the gap before the next heading, not to the
/// section — inserting after them would leave a hole in the middle of the page.
fn end_of_section(lines: &[String], lo: usize, hi: usize) -> usize {
    let mut end = hi;
    while end > lo && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    end
}

/// A heading reduced to what it means: lowercase, letters and digits only.
fn key(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_alphanumeric() || c.is_whitespace())
        .collect::<String>()
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect::<Vec<_>>()
        .join(" ")
}

fn row_to_note(r: &rusqlite::Row<'_>) -> rusqlite::Result<Note> {
    let ts = |s: String| {
        chrono::DateTime::parse_from_rfc3339(&s)
            .map(|d| d.with_timezone(&chrono::Utc))
            .unwrap_or_else(|_| memos_core::now())
    };
    Ok(Note {
        id: Id::parse(&r.get::<_, String>(0)?).unwrap_or_default(),
        collection_id: r
            .get::<_, Option<String>>(1)?
            .and_then(|s| Id::parse(&s).ok()),
        title: r.get(2)?,
        body: r.get(3)?,
        created_at: ts(r.get(4)?),
        updated_at: ts(r.get(5)?),
        edited_at: r.get::<_, Option<String>>(6)?.map(ts),
    })
}

const COLUMNS: &str = "id, collection_id, title, body, created_at, updated_at, edited_at";

/// What the one document is called until somebody renames it.
pub const BOOK_TITLE: &str = "Everything";

impl Db {
    /// The document, if anything has started it.
    ///
    /// One row, found by having no collection: a note that belongs to a
    /// collection is the old per-collection shape, and 006 folded those in.
    /// `Ok(None)` before the first capture — an empty file is not conjured
    /// because somebody opened a page.
    pub fn book(&self) -> DbResult<Option<Note>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!(
                    "SELECT {COLUMNS} FROM notes WHERE collection_id IS NULL
                      ORDER BY created_at LIMIT 1"
                ),
                [],
                row_to_note,
            )
            .optional()?)
        })
    }

    /// The document, started if it does not exist yet.
    pub fn ensure_book(&self) -> DbResult<Note> {
        if let Some(book) = self.book()? {
            return Ok(book);
        }
        self.transaction(|tx| {
            let id = Id::new();
            let now = memos_core::now().to_rfc3339();
            tx.execute(
                "INSERT INTO notes (id, collection_id, title, body, created_at, updated_at)
                 VALUES (?1, NULL, ?2, '', ?3, ?3)",
                params![id.to_string(), BOOK_TITLE, now],
            )?;
            Ok(tx.query_row(
                &format!("SELECT {COLUMNS} FROM notes WHERE id = ?1"),
                params![id.to_string()],
                row_to_note,
            )?)
        })
    }

    pub fn note(&self, id: Id) -> DbResult<Option<Note>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!("SELECT {COLUMNS} FROM notes WHERE id = ?1"),
                params![id.to_string()],
                row_to_note,
            )
            .optional()?)
        })
    }

    /// Replace the document's body with what a person wrote.
    ///
    /// Stamps `edited_at`, which is what separates a document somebody has
    /// worked on from one that has only ever accumulated.
    pub fn save_note(&self, id: Id, body: &str) -> DbResult<()> {
        self.with(|c| {
            let now = memos_core::now().to_rfc3339();
            c.execute(
                "UPDATE notes SET body = ?2, updated_at = ?3, edited_at = ?3 WHERE id = ?1",
                params![id.to_string(), body, now],
            )?;
            Ok(())
        })
    }

    /// Fold a capture into the document, under the section its collection path
    /// names, starting the document if this is the first thing ever captured.
    ///
    /// Returns the document's id, which the caller records on the item. Failing
    /// here must never fail a capture: the memory is already committed and the
    /// document can be rebuilt from the rows, never the other way round.
    pub fn integrate_capture(
        &self,
        item: Id,
        trail: &[String],
        text: &str,
        provenance: Option<&str>,
    ) -> DbResult<Id> {
        let book = self.ensure_book()?;
        let body = integrate(&book.body, trail, text, provenance);
        self.transaction(|tx| {
            tx.execute(
                "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1",
                params![
                    book.id.to_string(),
                    body,
                    memos_core::now().to_rfc3339()
                ],
            )?;
            tx.execute(
                "UPDATE knowledge_items SET note_id = ?2 WHERE id = ?1",
                params![item.to_string(), book.id.to_string()],
            )?;
            Ok(book.id)
        })
    }

    /// How many captures have been folded into a note.
    pub fn note_sources(&self, id: Id) -> DbResult<u32> {
        self.with(|c: &Connection| {
            Ok(c.query_row(
                "SELECT count(*) FROM knowledge_items WHERE note_id = ?1",
                params![id.to_string()],
                |r| r.get::<_, i64>(0),
            )? as u32)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use memos_core::KnowledgeItem;

    fn trail(path: &str) -> Vec<String> {
        path.split('/').map(str::to_string).collect()
    }

    #[test]
    fn the_first_capture_writes_the_outline_it_needs() {
        let out = integrate(
            "",
            &trail("Study/Programming/React"),
            "Setting state queues a re-render.",
            None,
        );
        assert_eq!(
            out,
            "# Study\n\n## Programming\n\n### React\n\n\
             Setting state queues a re-render.\n"
        );
    }

    #[test]
    fn provenance_follows_the_text() {
        let out = integrate(
            "",
            &trail("Household"),
            "Hold reset for ten seconds.",
            Some("10 Sep 2026 · [tp-link.com](https://tp-link.com/faq)"),
        );
        assert!(
            out.ends_with("*10 Sep 2026 · [tp-link.com](https://tp-link.com/faq)*\n"),
            "{out}"
        );
    }

    /// The whole point of one document: the second fact about a subject lands
    /// beside the first, not in another file.
    #[test]
    fn a_second_capture_on_the_same_subject_joins_that_section() {
        let body = integrate("", &trail("Study/React"), "One.", None);
        let out = integrate(&body, &trail("Study/React"), "Two.", None);
        assert_eq!(out, "# Study\n\n## React\n\nOne.\n\nTwo.\n");
    }

    #[test]
    fn a_sibling_subject_shares_the_parent_it_already_has() {
        let body = integrate("", &trail("Study/React"), "One.", None);
        let out = integrate(&body, &trail("Study/TypeScript"), "Two.", None);
        assert_eq!(
            out,
            "# Study\n\n## React\n\nOne.\n\n## TypeScript\n\nTwo.\n",
            "one Study, two subjects under it",
        );
    }

    /// The insertion goes at the end of *its* section, not the end of the file.
    #[test]
    fn a_capture_lands_inside_its_section_rather_than_at_the_bottom() {
        let body = "# Study\n\n## React\n\nOne.\n\n# Life\n\n## Household\n\nBins go out Thursday.\n";
        let out = integrate(body, &trail("Study/React"), "Two.", None);
        assert_eq!(
            out,
            "# Study\n\n## React\n\nOne.\n\nTwo.\n\n# Life\n\n## Household\n\n\
             Bins go out Thursday.\n"
        );
    }

    #[test]
    fn a_heading_matches_through_case_and_punctuation() {
        let body = "# Study\n\n## React!\n\nOne.\n";
        let out = integrate(body, &trail("study/react"), "Two.", None);
        assert_eq!(out.matches("## ").count(), 1, "no second section: {out}");
        assert_eq!(out.matches("# Study").count(), 1, "{out}");
    }

    #[test]
    fn a_capture_with_nowhere_to_go_still_has_somewhere_to_land() {
        let out = integrate("# Study\n\nOne.\n", &[], "Loose thought.", None);
        assert!(out.contains("# Unfiled"), "{out}");
        assert!(out.trim_end().ends_with("Loose thought."), "{out}");
    }

    /// Markdown runs out of levels before a collection tree runs out of depth.
    #[test]
    fn a_tree_deeper_than_markdown_stops_at_the_sixth_level() {
        let deep = trail("A/B/C/D/E/F/G");
        let out = integrate("", &deep, "Bottom.", None);
        assert!(out.contains("###### F"), "{out}");
        assert!(!out.contains("####### "), "there is no seventh level: {out}");
    }

    /// The whole safety argument in one assertion.
    #[test]
    fn nothing_a_person_wrote_is_ever_changed() {
        let mine = "# Study\n\nMy own paragraph, in my own words.\n\n## React\n\nAlso mine.\n";
        let out = integrate(mine, &trail("Study/React"), "Something the router heard.", None);
        for line in mine.lines().filter(|l| !l.trim().is_empty()) {
            assert!(out.contains(line), "lost: {line:?}");
        }
        assert!(out.find("Also mine.").unwrap() < out.find("Something the router heard.").unwrap());
    }

    #[test]
    fn an_empty_capture_leaves_the_document_alone() {
        let body = "# Study\n\nOne.\n";
        assert_eq!(integrate(body, &trail("Study"), "   ", None), body);
    }

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn integrating_starts_the_document_and_marks_the_capture_that_did_it() {
        let db = db();
        let item = KnowledgeItem::capture("Hooks", "useEffect runs after paint.");
        db.capture(&item, None).unwrap();

        let book = db
            .integrate_capture(
                item.id,
                &trail("Study/React"),
                "useEffect runs after paint.",
                Some("10 Sep 2026"),
            )
            .unwrap();

        let stored = db.book().unwrap().expect("a document exists");
        assert_eq!(stored.id, book);
        assert_eq!(stored.title, BOOK_TITLE);
        assert!(stored.body.contains("## React"), "{}", stored.body);
        assert!(stored.edited_at.is_none(), "growing is not editing");
        assert_eq!(db.note_sources(book).unwrap(), 1);
    }

    #[test]
    fn every_capture_grows_the_same_document() {
        let db = db();
        for (path, text) in [
            ("Study/React", "One."),
            ("Study/React", "Two."),
            ("Life/Household", "Three."),
        ] {
            let item = KnowledgeItem::capture("t", text);
            db.capture(&item, None).unwrap();
            db.integrate_capture(item.id, &trail(path), text, None).unwrap();
        }
        let all = db.book().unwrap().unwrap();
        assert_eq!(all.body.matches("\n# ").count() + 1, 2, "two roots: {}", all.body);
        assert_eq!(db.note_sources(all.id).unwrap(), 3);
    }

    #[test]
    fn editing_stamps_the_document_as_written_rather_than_grown() {
        let db = db();
        let book = db.ensure_book().unwrap();
        db.save_note(book.id, "# Study\n\nRewritten by hand.\n").unwrap();
        let after = db.note(book.id).unwrap().unwrap();
        assert_eq!(after.body, "# Study\n\nRewritten by hand.\n");
        assert!(after.edited_at.is_some());
    }

    /// ADR-0010: deleting the shelf must not burn the book — and now the book
    /// was never on a shelf to begin with.
    #[test]
    fn the_document_outlives_any_collection() {
        let db = db();
        let react = db.create_collection("React", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "One.");
        db.capture(&item, None).unwrap();
        let id = db.integrate_capture(item.id, &trail("React"), "One.", None).unwrap();

        db.delete_collection(react.id).unwrap();
        let book = db.note(id).unwrap().expect("the document survives");
        assert!(book.body.contains("One."));
    }
}
