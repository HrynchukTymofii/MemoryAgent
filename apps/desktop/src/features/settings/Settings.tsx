import { useEffect, useRef, useState } from "react";

import {
  api,
  type EmbedStatus,
  type HookStats,
  type LatencyReport,
  type MicStatus,
  type Settings as SettingsData,
  type SttStatus,
} from "../../lib/api";
import { buildSpec, specLabel, vkName } from "../../lib/keys";

const HOLD_OPTIONS = [0, 120, 200, 350];

export function Settings({ hook, offline }: { hook: HookStats | null; offline: boolean }) {
  const [settings, setSettings] = useState<SettingsData | null>(null);
  const [mic, setMic] = useState<MicStatus | null>(null);
  const [stt, setStt] = useState<SttStatus | null>(null);
  const [embed, setEmbed] = useState<EmbedStatus | null>(null);
  const [latency, setLatency] = useState<LatencyReport | null>(null);

  useEffect(() => {
    void api.settings().then(setSettings).catch(() => {});
    let live = true;
    const tick = async () => {
      try {
        const [m, s, e, l] = await Promise.all([
          api.mic(),
          api.stt(),
          api.embed(),
          api.latency(),
        ]);
        if (!live) return;
        setMic(m);
        setStt(s);
        setEmbed(e);
        setLatency(l);
      } catch {
        /* the shell already reports a dead backend */
      }
    };
    void tick();
    const t = window.setInterval(tick, 250);
    return () => {
      live = false;
      window.clearInterval(t);
    };
  }, []);

  return (
    <div className="panel">
      <h1>Settings</h1>
      <p className="sub">
        The shortcut, the microphone and the models behind capture and search. Account and privacy
        arrive with the milestones that need them.
      </p>

      {offline && (
        <p className="verdict bad">
          Backend not responding — the running app is older than this page. Stop it (Ctrl+C) and
          run <kbd>npm start</kbd> again. Readings below are stale.
        </p>
      )}

      <div className="card">
        <h2>Capture</h2>
        <Shortcut settings={settings} hook={hook} onSaved={setSettings} />

        <div className="row last">
          <span className="bd">
            <span className="k">Microphone</span>
            <span className="v">
              {mic?.available
                ? `${mic.device} · ${mic.input_rate} Hz ${mic.channels}ch → 16 kHz mono · ` +
                  `${mic.buffered_secs.toFixed(0)} s buffered`
                : "No microphone available — capture will not work."}
            </span>
            <span className="v" style={{ marginTop: 4, display: "block" }}>
              The stream is open continuously and audio is discarded unless you press the shortcut.
              Opening a device costs 100–300&nbsp;ms, which would clip your first word.
            </span>
          </span>
          <span className="lvl" aria-hidden="true">
            {/* sqrt: speech sits around 0.02–0.3 RMS, so a linear bar barely moves. */}
            <i style={{ width: `${Math.min(100, Math.sqrt(mic?.level ?? 0) * 260)}%` }} />
          </span>
        </div>
      </div>

      <div className="card">
        <h2>Timing</h2>
        <div className="row last">
          <span className="bd">
            <span className="k">Hold before capture starts</span>
            <span className="v">
              Stops a single key firing on an accidental tap. Costs nothing once the microphone
              ring buffer is running — audio spoken during the delay is already recorded.
            </span>
          </span>
          <span className="seg">
            {HOLD_OPTIONS.map((ms) => (
              <button
                key={ms}
                type="button"
                className={settings?.hold_threshold_ms === ms ? "on" : ""}
                onClick={() => {
                  if (settings) {
                    void api
                      .setHotkey(settings.hotkey, ms)
                      .then(setSettings)
                      .catch(() => {});
                  }
                }}
              >
                {ms === 0 ? "Off" : `${ms} ms`}
              </button>
            ))}
          </span>
        </div>
      </div>

      <div className="card">
        <h2>Models</h2>
        <p>
          Both run locally. Speech is required for capture; embeddings are not — without them
          search still works on exact words, and only the queries that needed meaning stop working.
        </p>
        <div className="pillrow">
          <span className="pill">
            <span className={`led ${stt?.state === "ready" ? "ok" : stt?.state === "loading" ? "warn" : "bad"}`} />
            <b>Speech</b>
            {stt ? (stt.state === "ready" ? "whisper base.en" : stt.detail || stt.state) : "…"}
          </span>
          <span className="pill">
            <span
              className={`led ${embed?.state === "ready" ? "ok" : embed?.state === "loading" ? "warn" : "bad"}`}
            />
            <b>Embeddings</b>
            {embed
              ? embed.state === "ready"
                ? `${embed.model_id} · ${embed.dim}d`
                : embed.detail || embed.state
              : "…"}
          </span>
          {embed?.state === "ready" && (
            <span className="pill">
              <b>{embed.embedded}</b> vectors
              {embed.pending > 0 && ` · ${embed.pending} queued`}
            </span>
          )}
        </div>
        {embed && embed.state !== "ready" && embed.state !== "loading" && (
          <div className="banner">
            <b>Semantic search is off.</b> {embed.detail}
          </div>
        )}
      </div>

      <div className="card">
        <h2>Latency</h2>
        <p>
          Hotkey → overlay painted, against a <strong>50&nbsp;ms</strong> ceiling. It holds only
          because the overlay is built and painted at startup and merely <em>shown</em> on the
          shortcut — constructing it on demand costs 200–400&nbsp;ms.
        </p>
        <div className="grid">
          <div className="stat">
            <div className="k">Captures measured</div>
            <div className="v">{latency?.count ?? 0}</div>
          </div>
          <Ms label="Median" v={latency?.count ? latency.p50_ms : null} />
          <div className="stat">
            <div className="k">95th percentile</div>
            <div className="v">
              {latency?.count ? latency.p95_ms.toFixed(1) : "—"}
              <small>ms</small>
            </div>
            {latency?.count ? (
              <div className={`verdict ${latency.within_budget ? "ok" : "bad"}`}>
                {latency.within_budget ? "within 50 ms budget" : "over 50 ms budget"}
              </div>
            ) : null}
          </div>
          <Ms label="Worst" v={latency?.count ? latency.worst_ms : null} />
        </div>
      </div>

      <div className="card">
        <h2>Diagnostics</h2>
        <p>
          Live view of what the keyboard hook receives. If a shortcut does nothing, this says why:
          whether keys arrive at all, what code a key reports, and whether the chord matched.
        </p>
        <div className="grid">
          <div className="stat">
            <div className="k">Keys seen</div>
            <div className="v">{hook?.events ?? 0}</div>
          </div>
          <div className="stat">
            <div className="k">Modifier keys seen</div>
            <div className="v">{hook?.mod_events ?? 0}</div>
          </div>
          <div className="stat">
            <div className="k">Last key pressed</div>
            <div className="v" style={{ fontSize: 15 }}>
              {vkName(hook?.last_vk ?? 0)}
            </div>
          </div>
          <div className="stat">
            <div className="k">Chord</div>
            <div className="v" style={{ fontSize: 15, color: hook?.engaged ? "var(--ok)" : "" }}>
              {hook?.engaged ? "engaged" : "idle"}
            </div>
          </div>
        </div>
        <HookVerdict hook={hook} />
      </div>
    </div>
  );
}

function Ms({ label, v }: { label: string; v: number | null }) {
  return (
    <div className="stat">
      <div className="k">{label}</div>
      <div className="v">
        {v === null ? "—" : v.toFixed(1)}
        <small>ms</small>
      </div>
    </div>
  );
}

function HookVerdict({ hook }: { hook: HookStats | null }) {
  if (!hook) return null;
  if (hook.events === 0) {
    return <p className="verdict bad">No keys reaching the hook at all.</p>;
  }
  if (hook.mod_events === 0) {
    // Absent modifier events are weak evidence — a session of lowercase typing
    // produces none legitimately. State what is observed, not what it might
    // imply; reading absence as interception cost hours of misdiagnosis.
    return (
      <p className="verdict">
        Keys are reaching the hook ({hook.events} seen). No modifier keys pressed yet.
      </p>
    );
  }
  return (
    <p className="verdict ok">
      Hook is healthy — {hook.events} keys, {hook.mod_events} modifiers.
    </p>
  );
}

/**
 * The shortcut recorder.
 *
 * Reads the hook's held state from the shell's poll rather than from DOM key
 * events, because the point is to record chords the browser never sees —
 * modifier-only combinations, and keys that other applications swallow.
 */
function Shortcut({
  settings,
  hook,
  onSaved,
}: {
  settings: SettingsData | null;
  hook: HookStats | null;
  onSaved: (s: SettingsData) => void;
}) {
  const [recording, setRecording] = useState(false);
  const [pending, setPending] = useState<string | null>(null);
  const [preview, setPreview] = useState<string | null>(null);
  const [hint, setHint] = useState<{ text: string; kind: "" | "ok" | "bad" }>({
    text: "",
    kind: "",
  });

  // The *peak* of what was held, not the instantaneous state: a chord is
  // pressed key by key, so any single sample catches a half-formed
  // combination. This is what makes Ctrl+Win record as Ctrl+Win rather than
  // just Ctrl. Refs, not state — updating these must not re-render.
  const peak = useRef({ mods: 0, key: 0 });

  useEffect(() => {
    if (!recording || !hook) return;

    if (hook.held_mods !== 0 || hook.held_key !== 0) {
      peak.current.mods |= hook.held_mods;
      if (hook.held_key !== 0) peak.current.key = hook.held_key;
      const spec = buildSpec(peak.current.mods, peak.current.key);
      setPreview(spec ? specLabel(spec) : null);
      return;
    }

    // Everything released: that is the gesture finishing. Hold what you want,
    // let go, done.
    if (peak.current.mods || peak.current.key) {
      const spec = buildSpec(peak.current.mods, peak.current.key);
      setRecording(false);
      setPreview(null);
      if (spec) {
        setPending(spec);
        setHint({ text: `Captured ${specLabel(spec)} — press Save to apply.`, kind: "ok" });
      } else {
        setPending(null);
        setHint({ text: "That key cannot be used as a shortcut. Try another.", kind: "bad" });
      }
    }
  }, [hook, recording]);

  const start = () => {
    peak.current = { mods: 0, key: 0 };
    setPending(null);
    setPreview(null);
    setRecording(true);
    setHint({ text: "Hold the combination you want, then let go.", kind: "" });
  };

  const cancel = () => {
    setRecording(false);
    setPending(null);
    setPreview(null);
    setHint({ text: "", kind: "" });
  };

  const save = async () => {
    if (!pending || !settings) return;
    try {
      const s = await api.setHotkey(pending, settings.hold_threshold_ms);
      onSaved(s);
      setPending(null);
      setHint({
        text: `${specLabel(s.hotkey)} is active now — no restart needed.`,
        kind: "ok",
      });
    } catch (e) {
      // Surface the parser's own words: it names the token that failed, which
      // is more use than a generic "invalid shortcut".
      setHint({ text: `Not saved — ${String(e)}`, kind: "bad" });
    }
  };

  const label = pending
    ? specLabel(pending)
    : preview ?? (recording ? "Press keys…" : settings ? specLabel(settings.hotkey) : "—");

  return (
    <>
      <div className="row">
        <span className="bd">
          <span className="k">Shortcut</span>
          <span className="v">
            Hold this anywhere in Windows to capture. Press Change, then hold the combination you
            want and let go.
          </span>
        </span>
        <span className={`keycap${recording ? " listening" : ""}`}>{label}</span>
        {!pending && !recording && (
          <button className="btn" type="button" onClick={start}>
            Change
          </button>
        )}
      </div>

      <div className="row" style={{ paddingTop: 0 }}>
        <span className="bd">
          <span className={hint.kind ? `hint ${hint.kind}` : "hint"}>{hint.text}</span>
        </span>
        {pending && (
          <button className="btn primary" type="button" onClick={save}>
            Save
          </button>
        )}
        {(pending || recording) && (
          <button className="btn" type="button" onClick={cancel}>
            Cancel
          </button>
        )}
      </div>
    </>
  );
}
