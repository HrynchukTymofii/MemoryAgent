import { useEffect, useRef, useState } from "react";

import { api, type Item } from "../../lib/api";
import { Empty, ItemRow } from "../../components/ItemList";

const PAGE = 60;
const RESULTS = 25;

/**
 * How long after the last keystroke a search runs.
 *
 * Search is local and takes under a millisecond, so this is not about load — it
 * is about not showing the user answers to half-typed questions. A list that
 * reshuffles on every keystroke is harder to read than one that waits.
 */
const DEBOUNCE_MS = 140;

export function Library({
  filter,
  onFilter,
  revision,
  onChanged,
}: {
  filter: string | null;
  onFilter: (path: string | null) => void;
  revision: number;
  onChanged: () => void;
}) {
  const [query, setQuery] = useState("");
  const [items, setItems] = useState<Item[] | null>(null);
  const [searching, setSearching] = useState(false);
  const [took, setTook] = useState<number | null>(null);
  const box = useRef<HTMLInputElement>(null);

  useEffect(() => {
    box.current?.focus();
  }, []);

  useEffect(() => {
    let live = true;
    const q = query.trim();

    const run = async () => {
      const started = performance.now();
      try {
        const rows = q ? await api.search(q, RESULTS) : await api.items(filter, PAGE, 0);
        if (!live) return;
        setItems(rows);
        setTook(performance.now() - started);
      } catch {
        if (live) setItems([]);
      } finally {
        if (live) setSearching(false);
      }
    };

    if (!q) {
      setSearching(false);
      void run();
      return () => {
        live = false;
      };
    }

    setSearching(true);
    const t = window.setTimeout(run, DEBOUNCE_MS);
    return () => {
      live = false;
      window.clearTimeout(t);
    };
  }, [query, filter, revision]);

  const q = query.trim();

  return (
    <div className="panel">
      <h1>Library</h1>
      <p className="sub">
        Everything you have captured. Search runs locally over both meaning and exact words — a
        memory you can only half remember is still findable.
      </p>

      <div className="search">
        <input
          ref={box}
          type="search"
          value={query}
          placeholder="Search your memories…"
          onChange={(e) => setQuery(e.target.value)}
          aria-label="Search your memories"
        />
        {filter && (
          <button type="button" className="btn" onClick={() => onFilter(null)}>
            Clear filter
          </button>
        )}
      </div>

      <p className="searchnote">
        {q ? (
          searching ? (
            "Searching…"
          ) : (
            <>
              {items?.length ?? 0} result{items?.length === 1 ? "" : "s"}
              {took !== null && ` in ${took.toFixed(0)} ms`}
            </>
          )
        ) : filter ? (
          <>
            Filtered to <span className="tag">{filter.replace(/\//g, " / ")}</span>
          </>
        ) : (
          "Newest first."
        )}
      </p>

      {items === null ? null : items.length === 0 ? (
        q ? (
          <Empty title="No matches">
            Nothing here matches “{q}”. Try fewer words, or the words you would have used when you
            saved it.
          </Empty>
        ) : filter ? (
          <Empty title="This collection is empty">
            Nothing has been filed here yet.
          </Empty>
        ) : (
          <Empty title="Nothing captured yet">
            Hold the shortcut and say something like “save this to reading”.
          </Empty>
        )
      ) : (
        <div className="items">
          {items.map((i) => (
            <ItemRow key={i.id} item={i} onOpened={onChanged} />
          ))}
        </div>
      )}
    </div>
  );
}
