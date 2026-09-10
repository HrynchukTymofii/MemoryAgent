import { useCallback, useEffect, useRef, useState } from "react";

import { api, type TaskRow } from "../../lib/api";
import { Empty } from "../../components/ItemList";
import { ClockIcon, TrashIcon } from "../../components/icons";

/** How many the list shows. Beyond this it stops being a list you can read. */
const LIMIT = 200;

/**
 * The only screen where a spoken command has a home to come back to.
 *
 * Everything else the router does is either a capture, which lands in the
 * Library, or an answer, which the overlay shows and discards. A task is the
 * one outcome that has to persist somewhere visible — "add a task to call the
 * bank" is a promise the system made, and a promise you cannot find later was
 * not kept.
 *
 * A line, not a card. The row used to be a paragraph tall and carried a card of
 * its own, which made twelve things to do look like twelve documents; what a
 * list of tasks is for is counting what is left, and you cannot count what you
 * have to scroll. Everything that is not the words — when it is due, when it
 * was said, and the two ways to change it — sits on the right, quiet until the
 * cursor is on the row.
 */
export function Tasks({ revision, onChanged }: { revision: number; onChanged: () => void }) {
  const [rows, setRows] = useState<TaskRow[] | null>(null);
  // Ticks applied here but not yet confirmed by a re-fetch. Without this the
  // checkbox waits a full poll before it moves, which reads as a dropped click.
  const [pending, setPending] = useState<Record<string, boolean>>({});
  /** The row being reworded, and the row being asked about before deletion. */
  const [editing, setEditing] = useState<string | null>(null);
  const [confirming, setConfirming] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const t = await api.tasks(LIMIT);
      setRows(t);
      setPending({});
    } catch {
      setRows([]);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load, revision]);

  const toggle = async (task: TaskRow) => {
    const next = !(pending[task.id] ?? task.done);
    setPending((p) => ({ ...p, [task.id]: next }));
    try {
      await api.setTaskDone(task.id, next);
      // The nav badge counts open tasks, so the shell is now stale too.
      onChanged();
    } catch {
      // Put the box back where it was rather than leaving a tick that did not
      // reach the database.
      setPending((p) => {
        const { [task.id]: _dropped, ...rest } = p;
        return rest;
      });
    }
  };

  const rename = async (task: TaskRow, title: string) => {
    setEditing(null);
    const next = title.trim();
    if (!next || next === task.title) return;
    // Locally first: the list is re-fetched a moment later and would otherwise
    // flash the old words back while the round trip finishes.
    setRows((r) => r?.map((t) => (t.id === task.id ? { ...t, title: next } : t)) ?? r);
    try {
      await api.renameTask(task.id, next);
    } finally {
      void load();
    }
  };

  const setDue = async (task: TaskRow, due: string | null) => {
    try {
      await api.setTaskDue(task.id, due);
    } finally {
      void load();
    }
  };

  const remove = async (task: TaskRow) => {
    setConfirming(null);
    setRows((r) => r?.filter((t) => t.id !== task.id) ?? r);
    try {
      await api.deleteTask(task.id);
      onChanged();
    } finally {
      void load();
    }
  };

  const open = rows?.filter((t) => !(pending[t.id] ?? t.done)) ?? [];
  const done = rows?.filter((t) => pending[t.id] ?? t.done) ?? [];

  const line = (t: TaskRow, isDone: boolean) => (
    <TaskLine
      key={t.id}
      task={t}
      done={isDone}
      editing={editing === t.id}
      confirming={confirming === t.id}
      onToggle={() => toggle(t)}
      onEdit={() => setEditing(t.id)}
      onRename={(title) => rename(t, title)}
      onCancelEdit={() => setEditing(null)}
      onDue={(due) => setDue(t, due)}
      onAskDelete={() => setConfirming(t.id)}
      onCancelDelete={() => setConfirming(null)}
      onDelete={() => remove(t)}
    />
  );

  return (
    <div className="panel">
      <h1>Tasks</h1>
      <p className="sub">
        Spoken, not typed — “add a task to call the bank”. Click the words to reword one,
        the date to say when it is due.
      </p>

      {rows === null ? null : rows.length === 0 ? (
        <Empty title="Nothing on the list">
          Hold the shortcut and say what needs doing.
        </Empty>
      ) : (
        <div className="tasks">
          {open.map((t) => line(t, false))}
          {done.length > 0 && (
            <>
              {/* Ticked tasks stay on screen. A row that disappears the instant
                  you tick it gives no confirmation the tick landed, and leaves
                  nowhere to untick from if it was the wrong row. */}
              <div className="day">Done · {done.length}</div>
              {done.map((t) => line(t, true))}
            </>
          )}
        </div>
      )}
    </div>
  );
}

function TaskLine({
  task,
  done,
  editing,
  confirming,
  onToggle,
  onEdit,
  onRename,
  onCancelEdit,
  onDue,
  onAskDelete,
  onCancelDelete,
  onDelete,
}: {
  task: TaskRow;
  done: boolean;
  editing: boolean;
  confirming: boolean;
  onToggle: () => void;
  onEdit: () => void;
  onRename: (title: string) => void;
  onCancelEdit: () => void;
  onDue: (due: string | null) => void;
  onAskDelete: () => void;
  onCancelDelete: () => void;
  onDelete: () => void;
}) {
  const box = useRef<HTMLInputElement>(null);
  const [draft, setDraft] = useState(task.title);

  useEffect(() => {
    if (editing) {
      setDraft(task.title);
      box.current?.focus();
      box.current?.select();
    }
  }, [editing, task.title]);

  const due = task.due_at ? new Date(task.due_at) : null;
  const state = done ? "" : dueState(due);

  return (
    <div className={`task${done ? " done" : ""}${confirming ? " confirming" : ""}`}>
      <button
        type="button"
        className="box"
        onClick={onToggle}
        aria-pressed={done}
        title={done ? "Mark as still open" : "Mark as done"}
      >
        {done ? "✓" : ""}
      </button>

      {editing ? (
        <input
          ref={box}
          className="t edit"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onBlur={() => onRename(draft)}
          onKeyDown={(e) => {
            if (e.key === "Enter") onRename(draft);
            // Escape has to put the old words back itself: the blur that
            // follows would otherwise commit the abandoned draft.
            if (e.key === "Escape") {
              setDraft(task.title);
              onCancelEdit();
            }
          }}
          aria-label="Task"
        />
      ) : (
        <button type="button" className="t" onClick={onEdit} title="Reword this task">
          {task.title}
        </button>
      )}

      {confirming ? (
        <span className="ask">
          <span>Delete?</span>
          <button type="button" className="lnk bad" onClick={onDelete}>
            Delete
          </button>
          <button type="button" className="lnk" onClick={onCancelDelete}>
            Keep
          </button>
        </span>
      ) : (
        <span className="rt">
          {/* The deadline is a control, not a label: the chip is the date
              input, so seeing when it is due and changing it are the same
              gesture. Undated rows show it only under the cursor. */}
          <label className={`due${state ? ` ${state}` : ""}${due ? "" : " none"}`}>
            <span className="g" aria-hidden="true">
              <ClockIcon />
            </span>
            <span className="d">{due ? dueLabel(due) : "Due"}</span>
            <input
              type="date"
              value={due ? isoDay(due) : ""}
              onChange={(e) => onDue(e.target.value ? new Date(`${e.target.value}T09:00`).toISOString() : null)}
              aria-label="Due date"
            />
          </label>

          {/* When it was said. Last, smallest, and never in the way of the
              words: it is the one thing here nobody looks for on purpose. */}
          <span className="made" title={new Date(task.created_at).toLocaleString()}>
            {age(task.created_at)}
          </span>

          <button type="button" className="act" onClick={onAskDelete} title="Delete this task">
            <TrashIcon />
          </button>
        </span>
      )}
    </div>
  );
}

/** `overdue` and `today` are worth a colour. Everything else is just a date. */
function dueState(due: Date | null): "" | "overdue" | "today" {
  if (!due) return "";
  const end = new Date(due);
  end.setHours(23, 59, 59, 999);
  if (end.getTime() < Date.now()) return "overdue";
  return isSameDay(due, new Date()) ? "today" : "";
}

function isSameDay(a: Date, b: Date): boolean {
  return (
    a.getFullYear() === b.getFullYear() &&
    a.getMonth() === b.getMonth() &&
    a.getDate() === b.getDate()
  );
}

/** What the chip says: a word where there is one, a date where there is not. */
function dueLabel(due: Date): string {
  const today = new Date();
  const midnight = (x: Date) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
  const days = Math.round((midnight(due) - midnight(today)) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Tomorrow";
  if (days === -1) return "Yesterday";
  if (days > 1 && days < 7) return due.toLocaleDateString(undefined, { weekday: "long" });
  return due.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

/** The value a native date input wants: the local day, not a UTC instant. */
function isoDay(d: Date): string {
  const pad = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`;
}

/** How long ago it was spoken, in as few characters as will carry it. */
function age(iso: string): string {
  const mins = Math.round((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 1) return "now";
  if (mins < 60) return `${mins}m`;
  const hours = Math.round(mins / 60);
  if (hours < 24) return `${hours}h`;
  const days = Math.round(hours / 24);
  if (days < 7) return `${days}d`;
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
