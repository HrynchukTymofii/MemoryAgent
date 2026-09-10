import { useEffect, useState } from "react";

import type { Notification } from "../../lib/api";

/** How long one stays up. Long enough to read twice, short enough to ignore. */
const DWELL_MS = 6_000;

/** At most this many on screen at once, newest at the top. */
const STACK = 3;

/**
 * Milestones, announced as they land.
 *
 * The panel is where notifications live; this is the moment one is earned, and
 * it exists because a badge appearing silently on a bell is not a
 * congratulation. It shows only what arrives while the window is open —
 * everything else is already in the panel, and a queue of stale toasts on
 * launch would be an app shouting about last Tuesday.
 *
 * Alerts never toast. Something being wrong is not a moment to celebrate, and
 * the panel says it plainly enough.
 */
export function Toasts({ earned, onGo }: { earned: Notification[]; onGo: (goto: string) => void }) {
  const [live, setLive] = useState<Notification[]>([]);

  useEffect(() => {
    if (earned.length === 0) return;
    const fresh = earned.filter((n) => n.kind !== "alert");
    if (fresh.length === 0) return;
    setLive((cur) => [...fresh, ...cur].slice(0, STACK));

    const ids = new Set(fresh.map((n) => n.id));
    const t = window.setTimeout(
      () => setLive((cur) => cur.filter((n) => !ids.has(n.id))),
      DWELL_MS,
    );
    return () => window.clearTimeout(t);
  }, [earned]);

  if (live.length === 0) return null;

  return (
    <div className="toasts" role="status" aria-live="polite">
      {live.map((n) => (
        <button
          key={n.id}
          type="button"
          className="toast"
          onClick={() => {
            setLive((cur) => cur.filter((x) => x.id !== n.id));
            if (n.goto) onGo(n.goto);
          }}
        >
          <span className="ic" aria-hidden="true">
            <AwardIcon />
          </span>
          <span className="bd">
            <span className="t">{n.title}</span>
            <span className="s">{n.body}</span>
          </span>
        </button>
      ))}
    </div>
  );
}

function AwardIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <circle cx="10" cy="8" r="4.6" />
      <path d="M10 5.9l.8 1.6 1.8.26-1.3 1.27.3 1.77L10 9.97l-1.6.83.3-1.77-1.3-1.27 1.8-.26z" />
      <path d="M7.2 12.4L6 18l4-2 4 2-1.2-5.6" />
    </svg>
  );
}
