# ADR-0010: One document is the knowledge base; captures are its history

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

### There is one document, and the collection tree is its outline

Not one note per collection — that is still a pile of files, and a pile of
files is what a person has to organise rather than read. There is **one
Markdown document**, and a capture's collection path becomes its heading trail:
`Study/Programming/React` lands under `# Study` › `## Programming` › `###
React`, creating whichever of those do not exist yet.

So filing and outlining are the same decision, made once. Collections keep
their real job — the destinations the router is allowed to route to — and stop
pretending to be folders full of documents. What the user opens is one file
they can read top to bottom, in the structure they already think in.

The document belongs to no collection, so deleting collections never touches
it. More than one is possible — the table has no such constraint, and a
document that outgrows one file will need it — but one is the default and the
only one anything creates today.

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

1. Each name in the trail is matched against the headings already inside its
   parent's section, ignoring case and punctuation.
2. A level that is not there is created at the end of its parent's section.
3. The text is appended at the **end of the deepest section** — after
   everything already under it, before the next heading.
4. The text is followed by a provenance line: the date it was captured, and a
   link to where it came from when there was one.

Markdown runs out of heading levels at six before a collection tree runs out of
depth; past that, everything shares the sixth. A capture with no destination
lands under `# Unfiled`, at the bottom, where somebody will find it.

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

**Three screens with three jobs, finally distinct.** The document is what you
read and write. The Library is the history — every capture, newest first,
searchable, in the order it happened. Collections are the outline and the
router's vocabulary. Before this, all three were views of one list.

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
