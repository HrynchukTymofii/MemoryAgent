import { useEffect, useState } from "react";

import { api, type EmbedStatus, type Item, type LibrarySummary, type Stats } from "../../lib/api";
import { Empty, ItemsByDay } from "../../components/ItemList";
import type { Page } from "../../hub/App";

const RECENT = 20;

export function Home({
  summary,
  offline,
  revision,
  onGo,
}: {
  summary: LibrarySummary | null;
  offline: boolean;
  revision: number;
  onGo: (p: Page) => void;
}) {
  const [items, setItems] = useState<Item[] | null>(null);
  const [embed, setEmbed] = useState<EmbedStatus | null>(null);
  const [stats, setStats] = useState<Stats | null>(null);

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const [recent, status, mine] = await Promise.all([
          api.recent(RECENT),
          api.embed(),
          api.achievementStats(),
        ]);
        if (!live) return;
        setItems(recent);
        setEmbed(status);
        setStats(mine);
      } catch {
        if (live) setItems([]);
      }
    })();
    return () => {
      live = false;
    };
  }, [revision]);

  return (
    <div className="panel">
      <h1>Home</h1>
      <p className="sub">
        Hold your capture shortcut anywhere in Windows, say what you want, and let go. What you
        captured lands here.
      </p>

      {/* What you have built up, before what the library holds. Three numbers,
          and the same three every day, so a glance is enough to see whether
          any of them moved. */}
      {stats && stats.words > 0 && (
        <div className="tally">
          <div className="fig">
            <b>{stats.words.toLocaleString()}</b>
            <span>total words</span>
          </div>
          {/* Held back until there is a minute of speech behind it — before
              that the figure swings by forty between two sentences and reads
              as broken rather than as precise. */}
          {stats.wpm !== null && (
            <div className="fig">
              <b>{stats.wpm}</b>
              <span>words a minute</span>
            </div>
          )}
          <div className="fig">
            <b>{stats.streak}</b>
            <span>day streak</span>
          </div>
          <Fortnight days={stats.fortnight} />
        </div>
      )}

      <div className="grid">
        <div className="stat">
          <div className="k">Memories</div>
          <div className="v">{summary?.items ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="k">Collections</div>
          <div className="v">{summary?.collections ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="k">This week</div>
          <div className="v">{summary?.this_week ?? "—"}</div>
        </div>
        <div className="stat">
          <div className="k">Searchable</div>
          {/* Two numbers rather than a percentage: "1,200 of 1,204" says both
              that search works and that the backlog is trivial, which one
              rounded figure cannot. */}
          <div className="v">
            {embed ? embed.embedded : "—"}
            <small>of {summary?.items ?? "—"}</small>
          </div>
          <EmbedVerdict embed={embed} />
        </div>
      </div>

      {offline && (
        <p className="verdict bad" style={{ marginTop: 14 }}>
          Backend not responding — the running app is older than this page. Stop it (Ctrl+C) and
          run <kbd>npm start</kbd> again. Everything below is stale.
        </p>
      )}

      {items === null ? null : items.length === 0 ? (
        <Empty title="Nothing captured yet">
          Hold the shortcut and say something like “save this to reading”. Your captures will
          appear here, newest first.
        </Empty>
      ) : (
        <>
          {/* No "Recent" heading over this: the groups below already name the
              day, and two headings stacked read as two lists. */}
          <ItemsByDay items={items} />
          {summary && summary.items > items.length && (
            <p className="searchnote">
              Showing the last {items.length} of {summary.items}.{" "}
              <a
                href="#"
                onClick={(e) => {
                  e.preventDefault();
                  onGo("library");
                }}
              >
                Open the Library
              </a>{" "}
              to search all of them.
            </p>
          )}
        </>
      )}
    </div>
  );
}

/**
 * The last fortnight, as bars.
 *
 * Zero-filled by the query, which is the point: a chart that skips quiet days
 * draws a busy fortnight and a scattered one identically. Heights are relative
 * to the best day in the window, so the shape is about this fortnight rather
 * than about an outlier from March.
 */
function Fortnight({ days }: { days: number[] }) {
  const peak = Math.max(...days, 1);
  return (
    <div className="spark" aria-hidden="true">
      {days.map((w, i) => (
        <i key={i} style={{ height: `${Math.max(2, (w / peak) * 100)}%` }} data-quiet={w === 0} />
      ))}
    </div>
  );
}

/**
 * Whether semantic search is actually available.
 *
 * Stated rather than implied. Keyword search works with no model at all, so the
 * degraded state is easy to miss — everything appears to function, and only the
 * queries that needed meaning quietly stop working.
 */
function EmbedVerdict({ embed }: { embed: EmbedStatus | null }) {
  if (!embed) return null;
  if (embed.state === "ready") {
    return embed.pending > 0 ? (
      <div className="verdict">{embed.pending} still to index</div>
    ) : (
      <div className="verdict ok">semantic search ready</div>
    );
  }
  if (embed.state === "loading") return <div className="verdict">loading the model…</div>;
  return <div className="verdict bad">keyword search only</div>;
}
