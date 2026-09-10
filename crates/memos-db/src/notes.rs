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

/// Fold one capture into a Markdown document, under a heading.
///
/// The four rules of ADR-0010, and nothing else:
///
/// 1. `heading` is matched against the document's `##` headings, ignoring case
///    and punctuation — "State as a snapshot" finds "State as a Snapshot".
/// 2. A match appends at the *end of that section*, after everything already
///    under it and before the next heading.
/// 3. No match appends a new section at the end of the document.
/// 4. `provenance` follows the text as its own italic line.
///
/// Nothing already in the document is edited, moved or removed. Every path
/// through this function is an insertion.
pub fn integrate(body: &str, heading: &str, text: &str, provenance: Option<&str>) -> String {
    let heading = heading.trim();
    let text = text.trim();
    if text.is_empty() {
        return body.to_string();
    }

    let mut block: Vec<String> = vec![text.to_string()];
    if let Some(p) = provenance.map(str::trim).filter(|p| !p.is_empty()) {
        block.push(String::new());
        block.push(format!("*{p}*"));
    }

    let mut lines: Vec<String> = body.lines().map(str::to_string).collect();
    // A document that has never had anything in it starts at its first
    // heading rather than with a blank line above it.
    let empty = lines.iter().all(|l| l.trim().is_empty());

    match section_of(&lines, heading) {
        Some(at) => {
            let mut insert = Vec::new();
            insert.push(String::new());
            insert.extend(block);
            lines.splice(at..at, insert);
        }
        None => {
            if !empty {
                lines.push(String::new());
            } else {
                lines.clear();
            }
            lines.push(format!("## {heading}"));
            lines.push(String::new());
            lines.extend(block);
        }
    }

    let mut out = lines.join("\n");
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

/// Where a new block belongs inside an existing section: the line after its
/// last non-empty one. `None` when the document has no such heading.
fn section_of(lines: &[String], heading: &str) -> Option<usize> {
    let want = key(heading);
    let start = lines
        .iter()
        .position(|l| l.strip_prefix("## ").is_some_and(|h| key(h) == want))?;

    // The section runs to the next heading of the same level or higher.
    let mut end = lines[start + 1..]
        .iter()
        .position(|l| l.starts_with("# ") || l.starts_with("## "))
        .map(|i| start + 1 + i)
        .unwrap_or(lines.len());
    // Trailing blank lines belong to the gap before the next heading, not to
    // the section — inserting after them would leave a hole in the middle.
    while end > start + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    Some(end)
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

impl Db {
    /// The note for a collection path, if one has been started.
    ///
    /// `Ok(None)` for a collection nobody has captured into yet — an empty
    /// document is not created just because somebody opened the page.
    pub fn note_for_path(&self, path: &str) -> DbResult<Option<Note>> {
        self.with(|c| {
            Ok(c.query_row(
                &format!(
                    "SELECT {} FROM notes n
                       JOIN collections c ON c.id = n.collection_id
                      WHERE c.path = ?1",
                    COLUMNS
                        .split(", ")
                        .map(|c| format!("n.{c}"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                params![path],
                row_to_note,
            )
            .optional()?)
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

    /// Replace a note's body with what a person wrote.
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

    pub fn rename_note(&self, id: Id, title: &str) -> DbResult<()> {
        self.with(|c| {
            c.execute(
                "UPDATE notes SET title = ?2, updated_at = ?3 WHERE id = ?1",
                params![
                    id.to_string(),
                    title.trim(),
                    memos_core::now().to_rfc3339()
                ],
            )?;
            Ok(())
        })
    }

    /// Fold a capture into its collection's note, starting the note if this is
    /// the first thing ever filed there.
    ///
    /// Returns the note's id, which the caller records on the item. Failing
    /// here must never fail a capture: the memory is already committed and the
    /// note can be rebuilt from the rows, never the other way round.
    pub fn integrate_capture(
        &self,
        collection: Id,
        collection_name: &str,
        item: Id,
        heading: &str,
        text: &str,
        provenance: Option<&str>,
    ) -> DbResult<Id> {
        self.transaction(|tx| {
            let existing: Option<(String, String)> = tx
                .query_row(
                    "SELECT id, body FROM notes WHERE collection_id = ?1",
                    params![collection.to_string()],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;

            let now = memos_core::now().to_rfc3339();
            let (id, body) = match existing {
                Some((id, body)) => {
                    let id = Id::parse(&id).unwrap_or_default();
                    (id, integrate(&body, heading, text, provenance))
                }
                None => {
                    let id = Id::new();
                    let body = integrate("", heading, text, provenance);
                    tx.execute(
                        "INSERT INTO notes (id, collection_id, title, body, created_at, updated_at)
                         VALUES (?1,?2,?3,'',?4,?4)",
                        params![
                            id.to_string(),
                            collection.to_string(),
                            collection_name,
                            now
                        ],
                    )?;
                    (id, body)
                }
            };

            tx.execute(
                "UPDATE notes SET body = ?2, updated_at = ?3 WHERE id = ?1",
                params![id.to_string(), body, now],
            )?;
            tx.execute(
                "UPDATE knowledge_items SET note_id = ?2 WHERE id = ?1",
                params![item.to_string(), id.to_string()],
            )?;
            Ok(id)
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

    #[test]
    fn the_first_capture_starts_the_document() {
        let out = integrate("", "State as a Snapshot", "Setting state queues a re-render.", None);
        assert_eq!(out, "## State as a Snapshot\n\nSetting state queues a re-render.\n");
    }

    #[test]
    fn provenance_follows_the_text() {
        let out = integrate(
            "",
            "Routers",
            "Hold reset for ten seconds.",
            Some("10 Sep 2026 · [tp-link.com](https://tp-link.com/faq)"),
        );
        assert!(out.ends_with("*10 Sep 2026 · [tp-link.com](https://tp-link.com/faq)*\n"), "{out}");
    }

    #[test]
    fn a_second_capture_on_the_same_subject_joins_the_section() {
        let body = "## Hooks\n\nuseEffect runs after paint.\n\n## Rendering\n\nReact batches.\n";
        let out = integrate(body, "hooks", "useMemo is a cache, not a promise.", None);
        assert_eq!(
            out,
            "## Hooks\n\nuseEffect runs after paint.\n\nuseMemo is a cache, not a promise.\n\n\
             ## Rendering\n\nReact batches.\n",
            "the addition lands under Hooks, not at the end of the file",
        );
    }

    #[test]
    fn a_heading_matches_through_case_and_punctuation() {
        let body = "## State as a Snapshot!\n\nOne.\n";
        let out = integrate(body, "state as a snapshot", "Two.", None);
        assert_eq!(out.matches("## ").count(), 1, "no second section: {out}");
    }

    #[test]
    fn an_unrelated_capture_starts_its_own_section() {
        let body = "## Hooks\n\nuseEffect runs after paint.\n";
        let out = integrate(body, "Suspense", "It is not a loading spinner.", None);
        assert!(out.starts_with("## Hooks\n\nuseEffect runs after paint.\n"));
        assert!(out.ends_with("## Suspense\n\nIt is not a loading spinner.\n"), "{out}");
    }

    /// The whole safety argument in one assertion.
    #[test]
    fn nothing_a_person_wrote_is_ever_changed() {
        let mine = "# React\n\nMy own paragraph, in my own words.\n\n## Hooks\n\nAlso mine.\n";
        let out = integrate(mine, "Hooks", "Something the router heard.", None);
        for line in mine.lines().filter(|l| !l.trim().is_empty()) {
            assert!(out.contains(line), "lost: {line:?}");
        }
        assert!(out.find("Also mine.").unwrap() < out.find("Something the router heard.").unwrap());
    }

    #[test]
    fn an_empty_capture_leaves_the_document_alone() {
        let body = "## Hooks\n\nOne.\n";
        assert_eq!(integrate(body, "Hooks", "   ", None), body);
    }

    fn db() -> Db {
        Db::open_in_memory().unwrap()
    }

    #[test]
    fn integrating_starts_a_note_and_marks_the_capture_that_did_it() {
        let db = db();
        let react = db.create_collection("React", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "useEffect runs after paint.");
        db.capture(&item, None).unwrap();

        let note = db
            .integrate_capture(
                react.id,
                "React",
                item.id,
                "Hooks",
                "useEffect runs after paint.",
                Some("10 Sep 2026"),
            )
            .unwrap();

        let stored = db.note_for_path("React").unwrap().expect("a note exists");
        assert_eq!(stored.id, note);
        assert!(stored.body.contains("## Hooks"), "{}", stored.body);
        assert!(stored.edited_at.is_none(), "growing is not editing");
        assert_eq!(db.note_sources(note).unwrap(), 1);
    }

    #[test]
    fn a_second_capture_grows_the_same_note() {
        let db = db();
        let react = db.create_collection("React", None).unwrap();
        for (title, text) in [("Hooks", "One."), ("Hooks", "Two."), ("Suspense", "Three.")] {
            let item = KnowledgeItem::capture(title, text);
            db.capture(&item, None).unwrap();
            db.integrate_capture(react.id, "React", item.id, title, text, None)
                .unwrap();
        }
        let note = db.note_for_path("React").unwrap().unwrap();
        assert_eq!(note.body.matches("## ").count(), 2, "{}", note.body);
        assert_eq!(db.note_sources(note.id).unwrap(), 3);
    }

    #[test]
    fn editing_stamps_the_note_as_written_rather_than_grown() {
        let db = db();
        let react = db.create_collection("React", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "One.");
        db.capture(&item, None).unwrap();
        let id = db
            .integrate_capture(react.id, "React", item.id, "Hooks", "One.", None)
            .unwrap();

        db.save_note(id, "# React\n\nRewritten by hand.\n").unwrap();
        let note = db.note(id).unwrap().unwrap();
        assert_eq!(note.body, "# React\n\nRewritten by hand.\n");
        assert!(note.edited_at.is_some());
    }

    /// ADR-0010: deleting the shelf must not burn the book.
    #[test]
    fn a_note_outlives_its_collection() {
        let db = db();
        let react = db.create_collection("React", None).unwrap();
        let item = KnowledgeItem::capture("Hooks", "One.");
        db.capture(&item, None).unwrap();
        let id = db
            .integrate_capture(react.id, "React", item.id, "Hooks", "One.", None)
            .unwrap();

        db.delete_collection(react.id).unwrap();
        let note = db.note(id).unwrap().expect("the document survives");
        assert!(note.collection_id.is_none(), "it is simply unshelved");
        assert!(note.body.contains("One."));
    }
}
