import { useCallback, useEffect, useState } from "react";

import { api, type MeetingFile, type MeetingStatus } from "../../lib/api";
import { Empty } from "../../components/ItemList";

/** The recorder rewrites its document every half second or so; this keeps up. */
const POLL_MS = 1000;

/**
 * Meeting notes: the microphone as "Me", the PC's sound as "Them".
 *
 * The document on disk is the record — a Markdown file in Documents\Meetings
 * that outlives the app — so this page reads that file rather than keeping a
 * second copy. The same view serves the meeting being recorded and one from
 * last week.
 */
export function Meetings() {
  const [status, setStatus] = useState<MeetingStatus | null>(null);
  const [files, setFiles] = useState<MeetingFile[] | null>(null);
  /** The meeting on screen, by file name. `null` is the list. */
  const [open, setOpen] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const loadFiles = useCallback(async () => {
    try {
      setFiles(await api.meetings());
    } catch {
      setFiles([]);
    }
  }, []);

  useEffect(() => {
    let live = true;
    const tick = async () => {
      try {
        const s = await api.meetingStatus();
        if (live) setStatus(s);
      } catch {
        // The shell already says when the backend is gone.
      }
    };
    void tick();
    void loadFiles();
    const t = window.setInterval(tick, POLL_MS);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [loadFiles]);

  // Re-read the open document for as long as it is open: after Stop, the last
  // few seconds are still being transcribed into it.
  useEffect(() => {
    if (!open) {
      setText(null);
      return;
    }
    let live = true;
    const read = async () => {
      try {
        const t = await api.meetingText(open);
        if (live) setText(t);
      } catch (e) {
        if (live) setError(String(e));
      }
    };
    void read();
    const t = window.setInterval(read, POLL_MS);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, [open]);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const name = await api.meetingStart();
      setOpen(name);
      setStatus(await api.meetingStatus());
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const stop = async () => {
    setBusy(true);
    try {
      // Keep the one that was recording on screen, so its ending arrives in view.
      if (status?.name) setOpen(status.name);
      await api.meetingStop();
      setStatus(await api.meetingStatus());
      void loadFiles();
    } finally {
      setBusy(false);
    }
  };

  const recording = status?.recording ?? false;

  return (
    <div className="panel">
      <h1>Meeting notes</h1>
      <p className="sub">
        Transcribes a call as it happens: your microphone as “Me”, and whatever the PC plays — Zoom,
        Meet, Teams — as “Them”. Everything stays on this computer. Wear headphones, or the
        microphone hears the other side too.
      </p>

      <div className="meet-bar">
        {recording ? (
          <button type="button" className="btn primary" onClick={stop} disabled={busy}>
            Stop recording
          </button>
        ) : (
          <button type="button" className="btn primary" onClick={start} disabled={busy}>
            Start recording
          </button>
        )}
        {recording && status && (
          <span className="pill">
            <span className="led rec" />
            <b>Recording</b> {duration(status.elapsed_secs)}
          </span>
        )}
        {recording && status?.name && open !== status.name && (
          <button type="button" className="btn" onClick={() => setOpen(status.name)}>
            Show live transcript
          </button>
        )}
      </div>
      {error && <p className="searchnote err">{error}</p>}

      {open ? (
        <Transcript
          name={open}
          text={text}
          live={recording && status?.name === open}
          onBack={() => {
            setOpen(null);
            void loadFiles();
          }}
        />
      ) : files === null ? null : files.length === 0 ? (
        <Empty title="No meetings yet">
          Press Start recording when the call begins. The transcript is saved to
          Documents\Meetings as it goes.
        </Empty>
      ) : (
        <>
          <div className="day">Past meetings</div>
          <div className="items">
            {files.map((f) => (
              <button
                key={f.name}
                type="button"
                className="item"
                onClick={() => setOpen(f.name)}
              >
                <span className="at">{shortDate(f.modified)}</span>
                <span className="tx">{f.name.replace(/\.md$/, "")}</span>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}

function Transcript({
  name,
  text,
  live,
  onBack,
}: {
  name: string;
  text: string | null;
  live: boolean;
  onBack: () => void;
}) {
  const doc = text === null ? null : parse(text);
  return (
    <>
      <div className="meet-head">
        <button type="button" className="btn" onClick={onBack}>
          ← All meetings
        </button>
        <span className="meet-title">{doc?.title ?? name.replace(/\.md$/, "")}</span>
        <button type="button" className="btn" onClick={() => void api.openMeeting(name)}>
          Open file
        </button>
      </div>
      {doc?.notices.map((n) => (
        <div key={n} className="banner">
          {n}
        </div>
      ))}
      {doc === null ? null : doc.lines.length === 0 ? (
        <Empty title={live ? "Listening…" : "Nothing was transcribed"}>
          {live
            ? "Lines appear a few seconds after each pause."
            : "No speech was heard in this recording."}
        </Empty>
      ) : (
        <div className="items">
          {doc.lines.map((l, i) => (
            <div key={i} className="item inert">
              <span className="at">{l.at}</span>
              <span className="tx">
                <span className={l.who === "Me" ? "who me" : "who"}>{l.who}</span>
                {l.text}
              </span>
            </div>
          ))}
        </div>
      )}
    </>
  );
}

interface Line {
  at: string;
  who: string;
  text: string;
}

/**
 * Read the document the recorder writes. Its shape is fixed by `render` in
 * meeting.rs: a `#` title, `_notices_`, and `**mm:ss Who:** words` paragraphs.
 */
function parse(md: string): { title: string | null; notices: string[]; lines: Line[] } {
  let title: string | null = null;
  const notices: string[] = [];
  const lines: Line[] = [];
  for (const block of md.split(/\r?\n\r?\n/)) {
    const b = block.trim();
    if (!b) continue;
    const line = /^\*\*(\d+:\d\d) ([^:*]+):\*\*\s*([\s\S]*)$/.exec(b);
    if (line) lines.push({ at: line[1], who: line[2], text: line[3] });
    else if (b.startsWith("# ")) title = b.slice(2);
    else if (b.startsWith("_") && b.endsWith("_")) notices.push(b.slice(1, -1));
    // Anything else was typed into the file by hand; it is still in the file.
  }
  return { title, notices, lines };
}

function duration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = secs % 60;
  const mmss = `${String(m).padStart(2, "0")}:${String(s).padStart(2, "0")}`;
  return h > 0 ? `${h}:${mmss}` : mmss;
}

function shortDate(iso: string): string {
  return new Date(iso).toLocaleDateString(undefined, { day: "numeric", month: "short" });
}
