import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { EditorContent, useEditor, type Editor } from "@tiptap/react";
import StarterKit from "@tiptap/starter-kit";
import Link from "@tiptap/extension-link";
import Placeholder from "@tiptap/extension-placeholder";
import TaskList from "@tiptap/extension-task-list";
import TaskItem from "@tiptap/extension-task-item";
import { Markdown } from "tiptap-markdown";

import { api, type NoteRow } from "../../lib/api";
import { slashMenu, type Block, type SlashState } from "./slash";

/**
 * How long after the last keystroke the document is written.
 *
 * There is no save button, so this number is the whole contract: long enough
 * that a sentence is one write rather than forty, short enough that closing the
 * window mid-thought cannot lose it. The editor also flushes on the way out.
 */
const AUTOSAVE_MS = 700;

/** The block menu's height, for deciding which side of the caret it sits on. */
const MENU_H = 268;

/**
 * The one document (ADR-0010).
 *
 * Rich text on the way in, Markdown on the way out — the storage format has to
 * stay something a model can read, a diff can show and a sync can merge, and
 * the person typing should never have to know that. What Markdown cannot
 * express, the document does not have: no colours, no fonts, no page furniture.
 * That is a constraint on purpose, not a missing feature.
 *
 * Formatting is reached the way a person expects it: "/" for the block menu,
 * the Markdown shortcuts for anyone who knows them, and a toolbar for the
 * marks that are not blocks.
 *
 * The editor never reloads under a cursor. A capture arriving while you are
 * writing lands in the file, and the notice says so rather than replacing the
 * paragraph you are in the middle of.
 */
export function NoteEditor({
  /** A heading to scroll to on arrival — how a collection opens the file. */
  focusHeading,
}: {
  focusHeading?: string | null;
}) {
  const [note, setNote] = useState<NoteRow | null | undefined>(undefined);
  const [saving, setSaving] = useState(false);
  const [savedAt, setSavedAt] = useState<number | null>(null);
  const [stale, setStale] = useState(false);
  const [slash, setSlash] = useState<SlashState | null>(null);
  const [pick, setPick] = useState(0);

  /** The timer, and the body it is going to write. */
  const timer = useRef<number | null>(null);
  const dirty = useRef<string | null>(null);
  const id = useRef<string | null>(null);
  const shell = useRef<HTMLDivElement>(null);
  /** The menu's keyboard state, read from inside a ProseMirror handler. */
  const menu = useRef<{ items: Block[]; pick: number }>({ items: [], pick: 0 });
  /** Set below, so the key handler — made once — always has the live editor. */
  const choose = useRef<((b: Block) => void) | null>(null);

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

  // Built once: the extension carries a ProseMirror plugin, and rebuilding it
  // on every render would tear the plugin out from under the document.
  const slashExtension = useMemo(
    () =>
      slashMenu({
        onChange: (state) => {
          setSlash(state);
          setPick(0);
          menu.current = { items: state?.items ?? [], pick: 0 };
        },
        onKey: (event) => {
          const { items } = menu.current;
          if (!items.length) return false;
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            const next =
              event.key === "ArrowDown"
                ? (menu.current.pick + 1) % items.length
                : (menu.current.pick - 1 + items.length) % items.length;
            menu.current.pick = next;
            setPick(next);
            return true;
          }
          if (event.key === "Enter" || event.key === "Tab") {
            choose.current?.(items[menu.current.pick]);
            return true;
          }
          if (event.key === "Escape") {
            setSlash(null);
            return true;
          }
          return false;
        },
      }),
    [],
  );

  const editor = useEditor(
    {
      extensions: [
        StarterKit.configure({
          // Every level, because the document's own outline is written in them
          // — the collection tree lands as `#`, `##`, `###` and deeper.
          heading: { levels: [1, 2, 3, 4, 5, 6] },
        }),
        Link.configure({ openOnClick: false, autolink: true }),
        TaskList,
        TaskItem.configure({ nested: true }),
        Placeholder.configure({
          placeholder: 'Write here, or press "/" for a block. Captures land here too.',
        }),
        Markdown.configure({ html: false, linkify: true, transformPastedText: true }),
        slashExtension,
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
    },
    [slashExtension],
  );

  choose.current = (block: Block) => {
    if (!editor || !slash) return;
    block.run(editor, slash.range);
    setSlash(null);
  };

  // Load once. Not on the shell's poll: re-reading a document somebody is
  // typing into is how an editor eats a paragraph.
  useEffect(() => {
    let live = true;
    void api
      .book()
      .then((n) => {
        if (!live) return;
        setNote(n);
        id.current = n?.id ?? null;
        if (n && editor) editor.commands.setContent(n.body || "");
      })
      .catch(() => live && setNote(null));
    return () => {
      live = false;
      void flush();
    };
  }, [editor, flush]);

  // Arriving from a collection: put that section on screen. By its text, after
  // the content has rendered — the document has no ids, and inventing some
  // would be a second structure to keep in step with the words.
  useEffect(() => {
    if (!note || !focusHeading || !shell.current) return;
    const t = window.setTimeout(() => {
      const wanted = focusHeading.trim().toLowerCase();
      const heads = shell.current?.querySelectorAll("h1,h2,h3,h4,h5,h6") ?? [];
      for (const h of heads) {
        if ((h.textContent ?? "").trim().toLowerCase() === wanted) {
          h.scrollIntoView({ block: "start", behavior: "smooth" });
          h.classList.add("landed");
          window.setTimeout(() => h.classList.remove("landed"), 1600);
          break;
        }
      }
    }, 60);
    return () => window.clearTimeout(t);
  }, [note, focusHeading]);

  // A capture can land in the document while it is open. Say so; do not reach
  // into the editor and change what is under the cursor.
  useEffect(() => {
    if (!note) return;
    const t = window.setInterval(async () => {
      try {
        const fresh = await api.book();
        if (fresh && fresh.sources !== note.sources) setStale(true);
      } catch {
        // The shell already reports a backend that stopped answering.
      }
    }, 4000);
    return () => window.clearInterval(t);
  }, [note]);

  const start = async () => {
    const started = await api.startBook();
    setNote(started);
    id.current = started.id;
    editor?.commands.setContent(started.body || "");
    editor?.commands.focus();
  };

  const reload = async () => {
    await flush();
    const fresh = await api.book();
    if (!fresh) return;
    setNote(fresh);
    editor?.commands.setContent(fresh.body || "");
    setStale(false);
  };

  if (note === undefined) return null;

  if (note === null) {
    return (
      <div className="doc-start">
        <strong>Nothing written yet</strong>
        <span>
          Say something with the shortcut and it will be written here, under the collection
          you filed it in. Or start the page yourself.
        </span>
        <button type="button" className="btn" onClick={() => void start()}>
          Start writing
        </button>
      </div>
    );
  }

  return (
    <div className="doc" ref={shell}>
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

      {slash && slash.items.length > 0 && (
        <div
          className="slash"
          style={{
            // Fixed, from the caret's own rectangle: the document scrolls
            // inside a panel, and anything positioned against the page drifts.
            // Near the bottom of the window it flips above the caret rather
            // than sliding up the page, which would put it over the text you
            // were writing instead of under the slash you typed.
            ...(slash.rect.bottom + MENU_H < window.innerHeight
              ? { top: slash.rect.bottom + 6 }
              : { top: Math.max(8, slash.rect.top - MENU_H - 6) }),
            left: Math.min(slash.rect.left, window.innerWidth - 252),
          }}
        >
          {slash.items.map((b, i) => (
            <button
              key={b.title}
              type="button"
              className={i === pick ? "row on" : "row"}
              onMouseDown={(e) => {
                e.preventDefault();
                choose.current?.(b);
              }}
              onMouseEnter={() => {
                setPick(i);
                menu.current.pick = i;
              }}
            >
              <span className="t">{b.title}</span>
              {b.hint && <span className="k">{b.hint}</span>}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/**
 * The marks that are not blocks, which is all a toolbar is still for.
 *
 * The blocks moved to the slash menu, where a person now looks for them. Bold
 * and italic stay: opening a menu to make one word bold is worse than a button.
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
      {mark("B", "Bold", editor.isActive("bold"), () => editor.chain().focus().toggleBold().run())}
      {mark("i", "Italic", editor.isActive("italic"), () =>
        editor.chain().focus().toggleItalic().run(),
      )}
      {mark("S", "Strikethrough", editor.isActive("strike"), () =>
        editor.chain().focus().toggleStrike().run(),
      )}
      {mark("‹›", "Inline code", editor.isActive("code"), () =>
        editor.chain().focus().toggleCode().run(),
      )}
      <span className="bar" aria-hidden="true" />
      {mark("↩", "Undo", false, () => editor.chain().focus().undo().run())}
      {mark("↪", "Redo", false, () => editor.chain().focus().redo().run())}
      <span className="hint">Press / for headings, lists and quotes</span>
    </span>
  );
}
