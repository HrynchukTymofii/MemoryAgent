# ADR-0010: The note is the knowledge base; the capture is its provenance

**Status:** Accepted · **Date:** 2026-09-10 · **Supersedes:** the spec's
assumption that a collection is a list of items

## Context

Everything built so far treats one capture as one immutable row, and a
collection as the set of rows pointing at it. Open `Study/Programming/React`
after a month of use and you get forty fragments in reverse chronological
order: eleven of them about hooks, four of them the same fact said twice, none
of them next to the one that explains it.

That is a log, and a log is the correct shape for the *record* — it is what
makes undo honest, what the correction log audits, and what search indexes. It
is the wrong shape for the thing the product is actually for. What a person
wants back from "everything I know about React" is one document they can read
top to bottom, with the resources they gathered linked inside it, growing as
they learn more. Not forty receipts.

The two are not in conflict, because they are not the same object. The mistake
was assuming one of them could do both jobs.

## Decision

### A collection has at most one note, and the note is what you read

A **note** is a Markdown document that belongs to a collection. Standing in a
collection you read its note; the collections inside it are the sections of the
subject that got big enough to move out. This is deliberately Notion's shape —
a place is a page, and pages contain pages — because that shape is already what
the Collections screen navigates.

Notes are nullable-collection on purpose: deleting a collection must not
destroy the document that grew inside it. The note comes loose, exactly as its
memories do.

### A capture is never rewritten. It becomes the note's provenance

`knowledge_items` keeps its guarantees unchanged: append-only, undoable,
indexed by FTS5 and by the vector store, the audit trail behind every routing
decision. It gains one column — `note_id`, where the capture was integrated —
and nothing else moves.

So a fact exists twice, on purpose: once as the moment it was said, and once as
a paragraph in a document. That is not duplication to be normalised away. The
first is evidence; the second is knowledge. Losing the first would make undo a
lie, and losing the second is the state we are in today.

### Integration only ever appends, and never inside a line the user wrote

The merge rule is one function, `notes::integrate`, and it is pure:

1. The capture's title becomes a heading, matched against the note's existing
   `##` headings case- and punctuation-insensitively.
2. On a match, the new text is appended at the **end of that section** — after
   everything already under the heading, before the next heading.
3. On no match, a new section is appended at the end of the document.
4. The text is followed by a provenance line: the date it was captured, and a
   link to where it came from when there was one.

Nothing already in the file is edited, reordered, or deleted. This is what
makes the document safe to hand to a person and to a model at the same time: a
user's own paragraph cannot be silently rewritten by a capture, and when Tier 2
takes this over its freedom is bounded to *choosing the section* and *phrasing
the addition*. The blast radius of a bad model decision stays "a paragraph
landed under the wrong heading", which a person can fix by dragging it.

### The editor writes Markdown, and the round trip is lossy on purpose

The Hub edits the note as rich text and stores Markdown. Formatting that
Markdown cannot express is formatting the note does not have — no colours, no
tables of contents, no embedded databases. The constraint is the feature: the
document has to stay something a model can read, a diff can show, and a sync
can merge line by line.

## Consequences

**The Collections screen becomes the product.** It was a filing cabinet you
looked at; it is now where the writing is. The Library keeps its job as the
record — every capture, newest first, searchable — and that is now a
recognisably different job rather than a second view of the same list.

**Search still indexes captures, not notes.** A note is composed of its
captures, so indexing both would rank the same sentence twice. The cost is that
a paragraph a user *typed* into a note is not findable by search until notes
are indexed too; that is the first thing to fix after this lands.

**Tier 2 has a job now.** "Integrate this into the note properly" is the first
genuinely useful thing a large model can do here, and it can be added behind
the same function boundary without touching the capture path. Until then, the
deterministic rule above runs and is honest about what it did.

**Two open questions, deliberately unanswered here.** Sync: the outbox carries
items, and a note is a document that two machines can edit — that is a merge
problem, and it is not this ADR's. And a visible back-link from a paragraph to
the capture that produced it needs a `memos:` URL scheme the Hub can resolve;
until then the link exists in the database (`knowledge_items.note_id`) but not
in the text.
