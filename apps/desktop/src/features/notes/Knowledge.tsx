import { NoteEditor } from "./NoteEditor";

/**
 * The file.
 *
 * One page, one document, nothing beside it — no cards, no counters, no list of
 * anything. Everything the system knows is written here, in the outline the
 * user's own collections make, and this screen's whole job is to get out of the
 * way of reading it.
 *
 * The heading and the blurb that every other panel carries are deliberately
 * absent: a document that opens under two paragraphs of chrome explaining what
 * a document is reads as a feature rather than as your own notebook.
 */
export function Knowledge({ heading }: { heading: string | null }) {
  return (
    <div className="panel wide">
      <NoteEditor focusHeading={heading} />
    </div>
  );
}
