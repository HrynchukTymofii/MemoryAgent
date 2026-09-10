import { useCallback, useEffect, useRef, useState } from "react";

import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import { Markdown } from "tiptap-markdown";

import { api, type NoteRow } from "../../lib/api";

/**
 * How long after the last keystroke the note is written.
 *
 * There is no save button, so this number is the whole contract: long enough
 * that a sentence is one write rather than forty, short enough that closing the
 * window mid-thought cannot lose it. The editor also flushes on the way out.
 */
const AUTOSAVE_MS = 700;

/**
 * The document a collection is (ADR-0010).
 *
 * Rich text on the way in, Markdown on the way out — the storage format has to
 * stay something a model can read, a diff can show and a sync can merge, and
 * the person typing should never have to know that. What Markdown cannot
 * express, the note does not have: no colours, no fonts, no page furniture.
 * That is a constraint on purpose, not a missing feature.
 *
 * The editor never reloads under a cursor. A capture arriving while you are
 * writing lands in the file, and the notice says so rather than replacing the
 * paragraph you are in the middle of.
 */
export function NoteEditor({ path, name }: { path: string; name: string }) {
  const [note, setNote] = useState<NoteRow | null | undefined>(undefined);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [stale, setStale] = useState(false);

  /** The timer, and the body it is going to write. */
  const timer = useRef<number | null>(null);
  const dirty = useRef<string | null>(null);
  const id = useRef<string | null>(null);

  const flush = useCallback(async () => {
    if (timer.current !== null) {
      window.clearTimeout(timer.current);
      timer.current = null;
    }
    const body = dirty.current;
    const noteId = id.current;
    dirty.current = null;
    if (body === null || !noteId) return;
    setSaving(true);
    try {
      await api.saveNote(noteId, body);
      setSavedAt(Date.now());
    } finally {
      setSaving(false);
    }
  }, []);

  const editor = useEditor({
    extensions: [
      StarterKit.configure({
        // The note's own title is the collection's name, drawn above the
        // editor. A second H1 inside the body would be the same words twice.
        heading: { levels: [2, 3] },
      }),
      Link.configure({ openOnClick: false, autolink: true }),
      Placeholder.configure({
        placeholder: "Write here, or say something and let it land in this note…",
      }),
      Markdown.configure({ html: false, linkify: true, transformPastedText: true }),
    ],
    editorProps: {
      attributes: { class: "doc-body", spellcheck: "true" },
      // A link in a document goes to the web, not to a tab inside the app.
      handleClickOn: (_view, _pos, _node, _nodePos, event) => {
        const a = (event.target as HTMLElement)?.closest("a");
        const href = a?.getAttribute("href");
        if (!href) return false;
        event.preventDefault();
        void api.openUrl(href).catch(() => undefined);
        return true;
      },
    },
    onUpdate: ({ editor }) => {
      dirty.current = editor.storage.markdown.getMarkdown();
      if (timer.current !== null) window.clearTimeout(timer.current);
      timer.current = window.setTimeout(() => void flush(), AUTOSAVE_MS);
    },
  });

  // Load on arrival, and whenever the page moves to another collection. Not on
  // the shell's poll: re-reading a document somebody is typing into is how an
  // editor eats a paragraph.
  useEffect(() => {
    let live = true;
    setNote(undefined);
    setStale(false);
    void api
      .note(path)
      .then((n) => {
        if (!live) return;
        setNote(n);
        id.current = n?.id ?? null;
        if (n && editor) {
          editor.commands.setContent(n.body || "");
          setSavedAt(null);
        }
      })
      .catch(() => live && setNote(null));
    return () => {
      live = false;
      void flush();
    };
  }, [path, editor, flush]);

  // A capture can land in this document while it is open. Say so; do not
  // reach into the editor and change what is under the cursor.
  useEffect(() => {
    if (!note) return;
    const t = window.setInterval(async () => {
      try {
        const fresh = await api.note(path);
        if (fresh && fresh.sources !== note.sources) setStale(true);
      } catch {
        // The shell already reports a backend that stopped answering.
      }
    }, 4000);
    return () => window.clearInterval(t);
  }, [path, note]);

  const start = async () => {
    const started = await api.startNote(path);
    setNote(started);
    id.current = started.id;
    editor?.commands.setContent(started.body || "");
    editor?.commands.focus();
  };

  const reload = async () => {
    await flush();
    const fresh = await api.note(path);
    if (!fresh) return;
    setNote(fresh);
    editor?.commands.setContent(fresh.body || "");
    setStale(false);
  };

  if (note === undefined) return null;

  if (note === null) {
    return (
      <div className="doc-start">
        <strong>No note yet for {name}</strong>
        <span>
          Say something into this collection and it will be written here, or start it
          yourself.
        </span>
        <button type="button" className="btn" onClick={() => void start()}>
          Start a note
        </button>
      </div>
    );
  }

  return (
    <div className="doc">
      <div className="doc-bar">
        <Marks editor={editor} />
        <span className="state">
          {saving
            ? "Saving…"
            : savedAt
              ? "Saved"
              : `${note.sources} capture${note.sources === 1 ? "" : "s"} folded in`}
        </span>
      </div>

      {stale && (
        <div className="doc-stale">
          Something new was filed here while you were writing.
          <button type="button" className="lnk" onClick={() => void reload()}>
            Load it
          </button>
        </div>
      )}

      <EditorContent editor={editor} />
    </div>
  );
}

/**
 * The formatting a Markdown document can actually hold, and nothing else.
 *
 * Typing `## ` still makes a heading — this row is for the people who do not
 * know that, which on the first day is everybody.
 */
function Marks({ editor }: { editor: Editor | null }) {
  if (!editor) return null;
  const mark = (label: string, title: string, on: boolean, run: () => void) => (
    <button
      type="button"
      className={on ? "mk on" : "mk"}
      title={title}
      // The editor loses focus to a click on a button; taking the mousedown
      // instead keeps the selection the formatting is meant to apply to.
      onMouseDown={(e) => {
        e.preventDefault();
        run();
      }}
    >
      {label}
    </button>
  );

  return (
    <span className="marks">
      {mark("H2", "Heading", editor.isActive("heading", { level: 2 }), () =>
        editor.chain().focus().toggleHeading({ level: 2 }).run(),
      )}
      {mark("H3", "Sub-heading", editor.isActive("heading", { level: 3 }), () =>
        editor.chain().focus().toggleHeading({ level: 3 }).run(),
      )}
      {mark("B", "Bold", editor.isActive("bold"), () => editor.chain().focus().toggleBold().run())}
      {mark("i", "Italic", editor.isActive("italic"), () =>
        editor.chain().focus().toggleItalic().run(),
      )}
      {mark("•", "List", editor.isActive("bulletList"), () =>
        editor.chain().focus().toggleBulletList().run(),
      )}
      {mark("1.", "Numbered list", editor.isActive("orderedList"), () =>
        editor.chain().focus().toggleOrderedList().run(),
      )}
      {mark("”", "Quote", editor.isActive("blockquote"), () =>
        editor.chain().focus().toggleBlockquote().run(),
      )}
      {mark("‹›", "Code", editor.isActive("codeBlock"), () =>
        editor.chain().focus().toggleCodeBlock().run(),
      )}
      {mark("↩", "Undo", false, () => editor.chain().focus().undo().run())}
    </span>
  );
}
