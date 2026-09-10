import { useEffect, useMemo, useState } from "react";

import { api, type Notification } from "../../lib/api";
import type { Page } from "../../hub/App";

/** How many rows the panel will hold. Beyond this, the oldest simply age out. */
const LIMIT = 50;

type Filter = "all" | "milestone" | "alert";

/**
 * The notification centre.
 *
 * Two sources, one list. Milestones and nudges are rows in the database,
 * written when they were earned and marked when they were read; alerts — the
 * backend gone quiet, tasks still open — are built here on every render from
 * live state, because they are true or they are not and a stored copy would sit
 * there tomorrow claiming an outage that ended overnight.
 */
export function Notifications({
  openTasks,
  offline,
  revision,
  onGo,
  onClose,
  onRead,
}: {
  openTasks: number;
  offline: boolean;
  /** Bumped when something new lands, to re-fetch without waiting for a poll. */
  revision: number;
  onGo: (p: Page) => void;
  onClose: () => void;
  /** Told once the rows have been marked read, so the bell can drop its count. */
  onRead: () => void;
}) {
  const [stored, setStored] = useState<Notification[] | null>(null);
  const [filter, setFilter] = useState<Filter>("all");
  const [menu, setMenu] = useState(false);

  useEffect(() => {
    let live = true;
    void (async () => {
      try {
        const rows = await api.notifications(LIMIT);
        if (!live) return;
        setStored(rows);
        // Opening the panel is the act of reading it. Marking each row as it
        // scrolls past would leave the badge lit for the two below the fold
        // that the user never meant to leave unread.
        if (rows.some((r) => !r.read)) {
          await api.markNotificationsRead();
          onRead();
        }
      } catch {
        if (live) setStored([]);
      }
    })();
    return () => {
      live = false;
    };
  }, [revision, onRead]);

  const live = useMemo(() => {
    const rows: Notification[] = [];
    if (offline) {
      rows.push({
        id: "alert:offline",
        kind: "alert",
        code: null,
        title: "The backend stopped answering",
        body: "Nothing on screen is current until it comes back.",
        goto: null,
        created_at: new Date().toISOString(),
        read: true,
      });
    }
    if (openTasks > 0) {
      rows.push({
        id: "alert:tasks",
        kind: "alert",
        code: null,
        title: `${openTasks} task${openTasks === 1 ? "" : "s"} still open`,
        body: "Spoken, and waiting on you. Open the list.",
        goto: "tasks",
        created_at: new Date().toISOString(),
        read: true,
      });
    }
    return rows;
  }, [offline, openTasks]);

  const rows = useMemo(() => {
    const all = [...live, ...(stored ?? [])];
    if (filter === "all") return all;
    if (filter === "alert") return all.filter((r) => r.kind === "alert");
    return all.filter((r) => r.kind !== "alert");
  }, [live, stored, filter]);

  const { today, earlier } = useMemo(() => split(rows), [rows]);

  const dismiss = async (id: string) => {
    // Alerts are not stored, so there is nothing to dismiss — and dismissing
    // one would only hide a condition that is still true.
    if (id.startsWith("alert:")) return;
    setStored((rs) => (rs ?? []).filter((r) => r.id !== id));
    try {
      await api.dismissNotification(id);
    } catch {
      // It is gone from the list either way; a failed write means it comes
      // back on the next open, which is the honest outcome.
    }
  };

  const clearAll = async () => {
    setMenu(false);
    setStored([]);
    try {
      await api.dismissAllNotifications();
    } catch {
      /* as above */
    }
  };

  return (
    <>
      {/* Clicking anywhere else shuts it — including on the bell, which sits
          under this, so the bell's own toggle never sees that second click. */}
      <div className="scrim" onClick={onClose} />
      <div className="notes" role="dialog" aria-label="Notifications">
        <header className="notes-top">
          <h2>Notifications</h2>
          <div className="notes-acts">
            <FilterButton value={filter} onChange={setFilter} />
            <button
              type="button"
              className="notes-act"
              onClick={() => setMenu((m) => !m)}
              title="More"
              aria-label="More"
              aria-expanded={menu}
            >
              <MoreIcon />
            </button>
            {menu && (
              <div className="notes-menu" role="menu">
                <button type="button" onClick={() => void clearAll()} role="menuitem">
                  Clear all
                </button>
              </div>
            )}
          </div>
        </header>

        <div className="notes-list">
          {stored === null ? null : rows.length === 0 ? (
            <p className="notes-empty">
              {filter === "all"
                ? "Nothing new. Milestones land here as you reach them."
                : "Nothing under this filter."}
            </p>
          ) : (
            <>
              {today.length > 0 && <Group label="Today" rows={today} onGo={onGo} onDismiss={dismiss} />}
              {earlier.length > 0 && (
                <Group label="Earlier" rows={earlier} onGo={onGo} onDismiss={dismiss} />
              )}
            </>
          )}
        </div>
      </div>
    </>
  );
}

function Group({
  label,
  rows,
  onGo,
  onDismiss,
}: {
  label: string;
  rows: Notification[];
  onGo: (p: Page) => void;
  onDismiss: (id: string) => void;
}) {
  return (
    <>
      <div className="notes-h">{label}</div>
      {rows.map((n) => (
        <Row key={n.id} n={n} onGo={onGo} onDismiss={onDismiss} />
      ))}
    </>
  );
}

function Row({
  n,
  onGo,
  onDismiss,
}: {
  n: Notification;
  onGo: (p: Page) => void;
  onDismiss: (id: string) => void;
}) {
  // Only a row that leads somewhere is a button. A milestone is something to
  // read, and making it look pressable promises a page that does not exist.
  const goes = n.goto !== null && n.goto !== "referral";
  const cls = `note ${n.kind}${n.read ? "" : " unread"}${goes ? " goes" : ""}`;

  const body = (
    <>
      <span className="ic" aria-hidden="true">
        {n.kind === "milestone" ? <AwardIcon /> : n.kind === "nudge" ? <SparkIcon /> : <AlertIcon />}
      </span>
      <span className="bd">
        <span className="t">{n.title}</span>
        <span className="s">{n.body}</span>
      </span>
      {!n.read && <span className="dot" aria-label="Unread" />}
    </>
  );

  return (
    <div className="note-wrap">
      {goes ? (
        <button type="button" className={cls} onClick={() => onGo(n.goto as Page)}>
          {body}
        </button>
      ) : (
        <div className={cls}>{body}</div>
      )}
      {!n.id.startsWith("alert:") && (
        <button
          type="button"
          className="note-x"
          onClick={() => onDismiss(n.id)}
          title="Dismiss"
          aria-label={`Dismiss: ${n.title}`}
        >
          <CloseIcon />
        </button>
      )}
    </div>
  );
}

function FilterButton({
  value,
  onChange,
}: {
  value: Filter;
  onChange: (f: Filter) => void;
}) {
  const [open, setOpen] = useState(false);
  const label: Record<Filter, string> = {
    all: "Everything",
    milestone: "Milestones",
    alert: "Alerts",
  };
  return (
    <>
      <button
        type="button"
        className={value === "all" ? "notes-act" : "notes-act on"}
        onClick={() => setOpen((o) => !o)}
        title={`Showing: ${label[value]}`}
        aria-label="Filter"
        aria-expanded={open}
      >
        <FilterIcon />
      </button>
      {open && (
        <div className="notes-menu" role="menu">
          {(Object.keys(label) as Filter[]).map((f) => (
            <button
              key={f}
              type="button"
              role="menuitem"
              className={f === value ? "on" : ""}
              onClick={() => {
                onChange(f);
                setOpen(false);
              }}
            >
              {label[f]}
            </button>
          ))}
        </div>
      )}
    </>
  );
}

/** Today's rows and the rest. Two groups, not seven — a date on every row is
    noise when most of them arrived in the last week. */
function split(rows: Notification[]): { today: Notification[]; earlier: Notification[] } {
  const midnight = new Date();
  midnight.setHours(0, 0, 0, 0);
  const today: Notification[] = [];
  const earlier: Notification[] = [];
  for (const r of rows) {
    const at = new Date(r.created_at);
    (Number.isNaN(at.getTime()) || at >= midnight ? today : earlier).push(r);
  }
  return { today, earlier };
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

function SparkIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 2.6l1.9 4.5 4.5 1.9-4.5 1.9L10 15.4l-1.9-4.5-4.5-1.9 4.5-1.9z" />
      <path d="M15.8 14.2l.7 1.6 1.6.7-1.6.7-.7 1.6-.7-1.6-1.6-.7 1.6-.7z" />
    </svg>
  );
}

function AlertIcon() {
  return (
    <svg viewBox="0 0 20 20" aria-hidden="true">
      <path d="M10 2.9l8 14.2H2z" />
      <path d="M10 8.1v3.6M10 14.2v.1" />
    </svg>
  );
}

function FilterIcon() {
  return (
    <svg viewBox="0 0 16 16" className="ic" aria-hidden="true">
      <path d="M2.4 4h11.2M4.4 8h7.2M6.6 12h2.8" />
    </svg>
  );
}

function MoreIcon() {
  return (
    <svg viewBox="0 0 16 16" className="ic" aria-hidden="true">
      <circle cx="3.4" cy="8" r=".9" />
      <circle cx="8" cy="8" r=".9" />
      <circle cx="12.6" cy="8" r=".9" />
    </svg>
  );
}

function CloseIcon() {
  return (
    <svg viewBox="0 0 10 10" className="ic" aria-hidden="true">
      <path d="M1.2 1.2l7.6 7.6M8.8 1.2L1.2 8.8" />
    </svg>
  );
}
