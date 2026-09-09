import { useState } from "react";

import { api, type Item } from "../lib/api";

/**
 * The one way a memory is drawn.
 *
 * Home, Library and search results all render this. A memory that looks
 * different depending on how you arrived at it reads as a different object, and
 * the whole product is built on the idea that there is only one of each.
 */
export function ItemRow({ item, onOpened }: { item: Item; onOpened?: () => void }) {
  const [error, setError] = useState<string | null>(null);
  const openable = Boolean(item.source_url);

  const open = async () => {
    if (!openable) return;
    try {
      await api.openItem(item.id);
      setError(null);
      // The access counter just changed, and it is a ranking signal — so the
      // list this row came from is now slightly stale.
      onOpened?.();
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <button
      type="button"
      className={openable ? "item" : "item inert"}
      onClick={open}
      // A row with nowhere to go is not a control. Leaving it focusable but
      // inert would promise a click that does nothing.
      tabIndex={openable ? 0 : -1}
      title={openable ? `Open ${item.source_url}` : "Captured by voice — no source to open"}
    >
      <span className="bd">
        <span className="t">{item.title}</span>
        <span className="s">{item.snippet}</span>
        <span className="m">
          {item.collection ? (
            <span className="tag">{item.collection.replace(/\//g, " / ")}</span>
          ) : (
            <span className="tag">Unfiled</span>
          )}
          <span>{when(item.captured_at)}</span>
          {item.source_url && (
            <>
              <span className="dot">·</span>
              <span>{domain(item.source_url)}</span>
            </>
          )}
          {item.access_count > 0 && (
            <>
              <span className="dot">·</span>
              <span>opened {item.access_count}×</span>
            </>
          )}
          {item.why && (
            <>
              <span className="dot">·</span>
              <span className="why">{item.why}</span>
            </>
          )}
          {error && (
            <>
              <span className="dot">·</span>
              <span style={{ color: "var(--bad)" }}>{error}</span>
            </>
          )}
        </span>
      </span>
    </button>
  );
}

/** Recent captures, under the day they happened on. */
export function ItemsByDay({ items, onOpened }: { items: Item[]; onOpened?: () => void }) {
  const groups = new Map<string, Item[]>();
  for (const item of items) {
    const key = dayLabel(item.captured_at);
    const list = groups.get(key);
    if (list) list.push(item);
    else groups.set(key, [item]);
  }
  return (
    <>
      {[...groups].map(([day, group]) => (
        <div key={day}>
          <div className="day">{day}</div>
          <div className="items">
            {group.map((i) => (
              <ItemRow key={i.id} item={i} onOpened={onOpened} />
            ))}
          </div>
        </div>
      ))}
    </>
  );
}

export function Empty({ title, children }: { title: string; children?: React.ReactNode }) {
  return (
    <div className="empty">
      <strong>{title}</strong>
      {children}
    </div>
  );
}

function dayLabel(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const midnight = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((midnight(today) - midnight(d)) / 86_400_000);
  if (days <= 0) return "Today";
  if (days === 1) return "Yesterday";
  if (days < 7) return d.toLocaleDateString(undefined, { weekday: "long" });
  return d.toLocaleDateString(undefined, { day: "numeric", month: "long" });
}

/** Relative age, shared with any screen that lists something time-stamped. */
export function when(iso: string): string {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

function domain(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    // A file path, or something the capture recorded verbatim.
    return url.split(/[\\/]/).pop() ?? url;
  }
}
