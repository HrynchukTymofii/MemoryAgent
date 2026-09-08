import { useEffect, useState } from "react";

import { api, type EmbedStatus, type Item, type LibrarySummary } from "../../lib/api";
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

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const [recent, status] = await Promise.all([api.recent(RECENT), api.embed()]);
        if (!live) return;
        setItems(recent);
        setEmbed(status);
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
          <div className="day" style={{ marginTop: 22 }}>
            Recent
          </div>
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
