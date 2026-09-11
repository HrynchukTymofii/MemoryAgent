import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { api, type CollectionRow, type Item } from "../../lib/api";
import { Empty, ItemsByDay } from "../../components/ItemList";
import { NoteEditor } from "../notes/NoteEditor";
import { FolderIcon, PencilIcon, PlusIcon, TrashIcon } from "../../components/icons";

/** Memories shown under the grid. A folder you have to scroll is a Library. */
const PAGE = 40;

/**
 * Collections, walked one level at a time.
 *
 * The whole tree used to be on screen at once, indented by depth. That reads
 * fine at nine collections and stops reading at forty: indentation is not a
 * structure you can act on, it is a structure you have to hold in your head,
 * and by the third level nobody is holding it.
 *
 * So the page is a *place* instead of a picture. You are always standing in one
 * collection; you see the ones directly inside it as cards, and the memories
 * filed in it underneath. The crumb trail says where you are and takes you
 * back. Depth is unbounded and costs nothing to add, because the screen never
 * shows more than one level of it — four deep looks exactly like one deep, and
 * the way out is always the same two words at the top.
 */
export function Collections({
  revision,
  onOpen,
}: {
  revision: number;
  /** Hand this collection to the Library, for searching within it. */
  onOpen: (path: string | null) => void;
}) {
  const [rows, setRows] = useState<CollectionRow[] | null>(null);
  /** Where we are standing. `null` is the top, which is not a collection. */
  const [here, setHere] = useState<string | null>(null);
  const [items, setItems] = useState<Item[] | null>(null);
  /** The card being renamed, the card being asked about, and the new-tile. */
  const [renaming, setRenaming] = useState<string | null>(null);
  const [asking, setAsking] = useState<{ id: string; items: number } | null>(null);
  const [adding, setAdding] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      setRows(await api.collections());
    } catch {
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, revision]);

  // The memories filed *here*, not anywhere below. A folder's own contents are
  // what "inside" means to a person opening it; the count on a child card is
  // how they find out there is more further down.
  useEffect(() => {
    let live = true;
    if (here === null) {
      setItems(null);
      return;
    }
    void api
      .items(here, PAGE, 0)
      .then((r) => live && setItems(r))
      .catch(() => live && setItems([]));
    return () => {
      live = false;
    };
  }, [here, revision, rows]);

  const children = useMemo(() => {
    const all = rows ?? [];
    const parent = (path: string) => {
      const cut = path.lastIndexOf("/");
      return cut < 0 ? null : path.slice(0, cut);
    };
    return all
      .filter((c) => parent(c.path) === here)
      .map((c) => ({
        ...c,
        // What the card has to promise: there is more inside than the count of
        // memories says, and this is how much more.
        inside: all.filter((o) => o.path.startsWith(`${c.path}/`)).length,
      }));
  }, [rows, here]);

  const crumbs = useMemo(() => {
    if (here === null) return [];
    const parts = here.split("/");
    return parts.map((name, i) => ({ name, path: parts.slice(0, i + 1).join("/") }));
  }, [here]);

  const act = async (fn: () => Promise<unknown>) => {
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(String(e));
    } finally {
      await load();
    }
  };

  const create = (name: string) => {
    setAdding(false);
    if (!name.trim()) return;
    void act(() => api.createCollection(name, here));
  };

  const rename = (c: CollectionRow, name: string) => {
    setRenaming(null);
    if (!name.trim() || name === c.name) return;
    void act(() => api.renameCollection(c.id, name));
  };

  const ask = async (c: CollectionRow) => {
    setAsking({ id: c.id, items: await api.collectionSize(c.path).catch(() => 0) });
  };

  const remove = (c: CollectionRow) => {
    setAsking(null);
    void act(() => api.deleteCollection(c.id));
  };

  return (
    <div className="panel">
      <h1>Collections</h1>
      <p className="sub">
        Your subjects, and the destinations the router understands — “save this to reading”
        works because a collection called Reading exists. Walk in far enough and you reach
        the page that subject keeps: one document, growing as you capture into it.
      </p>

      <div className="crumbs">
        <button
          type="button"
          className={here === null ? "cr on" : "cr"}
          onClick={() => setHere(null)}
        >
          All
        </button>
        {crumbs.map((c, i) => (
          <span key={c.path} className="crw">
            <span className="sep" aria-hidden="true">
              ›
            </span>
            <button
              type="button"
              className={i === crumbs.length - 1 ? "cr on" : "cr"}
              onClick={() => setHere(c.path)}
            >
              {c.name}
            </button>
          </span>
        ))}
        {here !== null && (
          <span className="crumbs-acts">
            <button
              type="button"
              className="btn ghost"
              onClick={() => onOpen(here)}
              title="Search inside this collection"
            >
              Search here
            </button>
          </span>
        )}
      </div>

      {error && <p className="searchnote err">{error}</p>}

      <div className="folders">
        {children.map((c) =>
          asking?.id === c.id ? (
            <div key={c.id} className="fold asking">
              <div className="q">
                Delete <b>{c.name}</b>?
                <span>
                  {c.inside > 0 && `${c.inside} collection${c.inside === 1 ? "" : "s"} inside go too. `}
                  {asking.items > 0
                    ? `${asking.items} ${asking.items === 1 ? "memory" : "memories"} become unfiled — nothing is erased.`
                    : "Nothing is filed in it."}
                </span>
              </div>
              <div className="acts">
                <button type="button" className="lnk bad" onClick={() => remove(c)}>
                  Delete
                </button>
                <button type="button" className="lnk" onClick={() => setAsking(null)}>
                  Keep
                </button>
              </div>
            </div>
          ) : (
            <Folder
              key={c.id}
              row={c}
              inside={c.inside}
              renaming={renaming === c.id}
              onOpen={() => setHere(c.path)}
              onRename={(name) => rename(c, name)}
              onStartRename={() => setRenaming(c.id)}
              onCancelRename={() => setRenaming(null)}
              onAskDelete={() => void ask(c)}
            />
          ),
        )}

        {/* The way to make one. Last in the grid rather than a button in the
            header: a new collection belongs where the collections are, and at
            the top level that tile is the whole empty state. */}
        {adding ? (
          <NewFolder onDone={create} onCancel={() => setAdding(false)} />
        ) : (
          <button type="button" className="fold new" onClick={() => setAdding(true)}>
            <span className="g" aria-hidden="true">
              <PlusIcon />
            </span>
            <span className="n">New collection</span>
            <span className="m">{here === null ? "At the top" : `Inside ${lastName(here)}`}</span>
          </button>
        )}
      </div>

      {/* The page a subject has. Offered where the subject actually is: a
          collection with collections inside it is the way to a subject, not
          one itself, and a blank page there would sit above the pages people
          do write in. One it already has still shows — putting a collection
          inside a subject must not hide what was written about it. */}
      {here !== null && (
        <NoteEditor
          key={here}
          path={here}
          name={lastName(here)}
          offerStart={children.length === 0}
        />
      )}

      {here !== null && (
        <>
          <div className="day">
            Captured into {lastName(here)}
            {items && items.length > 0 ? ` · ${items.length}` : ""}
          </div>
          {items === null ? null : items.length === 0 ? (
            <Empty title="Nothing captured into this collection yet">
              {children.length > 0
                ? "Its collections may still have something in them."
                : "Say “save this to " + lastName(here).toLowerCase() + "” while you have something on screen."}
            </Empty>
          ) : (
            <ItemsByDay items={items} />
          )}
        </>
      )}

      {here === null && children.length === 0 && rows !== null && (
        <Empty title="No collections yet">
          Captures land unfiled until there is somewhere to put them.
        </Empty>
      )}
    </div>
  );
}

function Folder({
  row,
  inside,
  renaming,
  onOpen,
  onRename,
  onStartRename,
  onCancelRename,
  onAskDelete,
}: {
  row: CollectionRow;
  inside: number;
  renaming: boolean;
  onOpen: () => void;
  onRename: (name: string) => void;
  onStartRename: () => void;
  onCancelRename: () => void;
  onAskDelete: () => void;
}) {
  const box = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState(row.name);

  useEffect(() => {
    if (renaming) {
      setDraft(row.name);
      box.current?.focus();
      box.current?.select();
    }
  }, [renaming, row.name]);

  return (
    <div className="fold">
      {/* The card is the way in; the two controls sit on top of it and stop
          the click before it opens anything. */}
      <button
        type="button"
        className="hit"
        onClick={onOpen}
        title={`Open ${row.path.replace(/\//g, " / ")}`}
        aria-label={`Open ${row.name}`}
        disabled={renaming}
      />
      <span className="g" aria-hidden="true">
        <FolderIcon />
      </span>

      {renaming ? (
        <input
          ref={box}
          className="n edit"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => onRename(draft)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onRename(draft);
            if (e.key === "Escape") {
              setDraft(row.name);
              onCancelRename();
            }
          }}
          aria-label="Collection name"
        />
      ) : (
        <span className="n">{row.name}</span>
      )}

      <span className="m">
        {row.items} {row.items === 1 ? "memory" : "memories"}
        {inside > 0 && ` · ${inside} inside`}
      </span>

      <span className="acts">
        <button type="button" className="act" onClick={onStartRename} title="Rename">
          <PencilIcon />
        </button>
        <button type="button" className="act" onClick={onAskDelete} title="Delete">
          <TrashIcon />
        </button>
      </span>
    </div>
  );
}

/** The new-collection tile, once it has been clicked. */
function NewFolder({
  onDone,
  onCancel,
}: {
  onDone: (name: string) => void;
  onCancel: () => void;
}) {
  const box = useRef<HTMLInputElement>(null);
  const [name, setName] = useState("");

  useEffect(() => {
    box.current?.focus();
  }, []);

  return (
    <div className="fold">
      <span className="g" aria-hidden="true">
        <FolderIcon />
      </span>
      <input
        ref={box}
        className="n edit"
        value={name}
        placeholder="Name it"
        onChange={(e) => setName(e.target.value)}
        onBlur={() => (name.trim() ? onDone(name) : onCancel())}
        onKeyDown={(e) => {
          if (e.key === "Enter") onDone(name);
          if (e.key === "Escape") onCancel();
        }}
        aria-label="New collection name"
      />
      <span className="m">Enter to create</span>
    </div>
  );
}

function lastName(path: string): string {
  return path.slice(path.lastIndexOf("/") + 1);
}
