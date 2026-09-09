import { useCallback, useEffect, useState } from "react";

import { api, type TaskRow } from "../../lib/api";
import { Empty, when } from "../../components/ItemList";

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
 */
export function Tasks({ revision, onChanged }: { revision: number; onChanged: () => void }) {
  const [rows, setRows] = useState<TaskRow[] | null>(null);
  // Ticks applied here but not yet confirmed by a re-fetch. Without this the
  // checkbox waits a full poll before it moves, which reads as a dropped click.
  const [pending, setPending] = useState<Record<string, boolean>>({});

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

  const open = rows?.filter((t) => !(pending[t.id] ?? t.done)) ?? [];
  const done = rows?.filter((t) => pending[t.id] ?? t.done) ?? [];

  return (
    <div className="panel">
      <h1>Tasks</h1>
      <p className="sub">
        Spoken, not typed — “add a task to call the bank”. A task needs nothing but its
        words, which is why it is the one thing the grammar can act on without asking
        when; a reminder still needs a time, and that is a question for later.
      </p>

      {rows === null ? null : rows.length === 0 ? (
        <Empty title="Nothing on the list">
          Hold the shortcut and say what needs doing.
        </Empty>
      ) : (
        <div className="tasks">
          {open.map((t) => (
            <TaskLine key={t.id} task={t} done={false} onToggle={() => toggle(t)} />
          ))}
          {done.length > 0 && (
            <>
              {/* Ticked tasks stay on screen. A row that disappears the instant
                  you tick it gives no confirmation the tick landed, and leaves
                  nowhere to untick from if it was the wrong row. */}
              <div className="day">Done</div>
              {done.map((t) => (
                <TaskLine key={t.id} task={t} done onToggle={() => toggle(t)} />
              ))}
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
  onToggle,
}: {
  task: TaskRow;
  done: boolean;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      className={done ? "task done" : "task"}
      onClick={onToggle}
      aria-pressed={done}
      title={done ? "Mark as still open" : "Mark as done"}
    >
      <span className="box" aria-hidden="true">
        {done ? "✓" : ""}
      </span>
      <span className="bd">
        <span className="t">{task.title}</span>
        <span className="m">
          {task.about && <span className="tag">about {task.about}</span>}
          <span>{when(task.created_at)}</span>
        </span>
      </span>
    </button>
  );
}
