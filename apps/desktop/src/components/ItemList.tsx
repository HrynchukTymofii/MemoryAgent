import { useState } from "react";

import { api, type Item } from "../lib/api";

/**
 * The one way a memory is drawn.
 *
 * Home, Library and search results all render this. A memory that looks
 * different depending on how you arrived at it reads as a different object, and
 * the whole product is built on the idea that there is only one of each.
 *
 * The row is a time and what was said, and nothing else. Collection, source,
 * open count and retriever were all true and all on screen at once, which made
 * a list of memories read as a table of statistics about memories — you cannot
 * skim your own words past four pieces of metadata. What a row is for is
 * finding the thing you said; the rest belongs to the memory, not to the list.
 */
export function ItemRow({
  item,
  onOpened,
  withDate,
}: {
  item: Item;
  onOpened?: () => void;
  /** For lists with no day heading over them — search results, ranked. */
  withDate?: boolean;
}) {
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
      title={openable ? `Open ${item.source_url}` : undefined}
    >
      <span className="at">{withDate ? stamp(item.captured_at) : clock(item.captured_at)}</span>
      <span className="tx">
        {item.snippet?.trim() || item.title}
        {error && <span className="err">{error}</span>}
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

/**
 * The clock time a memory was captured at.
 *
 * A wall-clock time rather than an age, now that the day is stated by the
 * heading above the group: "11:40" and "Today" together say more than "3 h
 * ago", and they say it without changing while you read the list.
 */
function clock(iso: string): string {
  return new Date(iso).toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" });
}

/** The same column where no heading says which day it is. */
function stamp(iso: string): string {
  const d = new Date(iso);
  const today = new Date();
  const sameDay =
    d.getFullYear() === today.getFullYear() &&
    d.getMonth() === today.getMonth() &&
    d.getDate() === today.getDate();
  return sameDay ? clock(iso) : d.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/** Relative age, for the screens that list something without a day heading. */
export function when(iso: string): string {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 1) return "just now";
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours} h ago`;
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
