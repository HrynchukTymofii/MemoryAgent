import { useEffect, useRef, useState } from "react";

import {
  api,
  type EmbedStatus,
  type HookStats,
  type LatencyReport,
  type MicStatus,
  type RouterStatus,
  type RoutingStats,
  type Account as AccountData,
  type Settings as SettingsData,
  type SttStatus,
} from "../../lib/api";
import { buildSpec, specLabel, vkName } from "../../lib/keys";

const HOLD_OPTIONS = [0, 120, 200, 350];

export function Settings({ hook, offline }: { hook: HookStats | null; offline: boolean }) {
  const [settings, setSettings] = useState<SettingsData | null>(null);

  // The backend returns the whole settings object, so the toggle reflects what
  // was actually persisted rather than what was clicked.
  const toggleIdlePill = async (enabled: boolean) => {
    try {
      setSettings(await api.setIdlePill(enabled));
    } catch {
      // Leave the toggle where it was: a switch that moves without the setting
      // changing is worse than one that does not move.
    }
  };
  const toggleAutoSummarize = async (enabled: boolean) => {
    try {
      setSettings(await api.setAutoSummarize(enabled));
    } catch {
      // As above: the switch stays where the setting is.
    }
  };
  const toggleStartAtLogin = async (enabled: boolean) => {
    try {
      setSettings(await api.setStartAtLogin(enabled));
    } catch {
      // As above. This one can genuinely fail — the login entry is a registry
      // write, and a locked-down machine may refuse it.
    }
  };
  const [mic, setMic] = useState<MicStatus | null>(null);
  const [stt, setStt] = useState<SttStatus | null>(null);
  const [embed, setEmbed] = useState<EmbedStatus | null>(null);
  const [router, setRouter] = useState<RouterStatus | null>(null);
  const [latency, setLatency] = useState<LatencyReport | null>(null);
  const [routing, setRouting] = useState<RoutingStats | null>(null);
  const [logNote, setLogNote] = useState<string | null>(null);
  const [account, setAccount] = useState<AccountData | null>(null);
  const [authBusy, setAuthBusy] = useState(false);
  const [authNote, setAuthNote] = useState<string | null>(null);

  useEffect(() => {
    void api.settings().then(setSettings).catch(() => {});
    void api.account().then(setAccount).catch(() => {});
    let live = true;
    const tick = async () => {
      try {
        const [m, s, e, l, r] = await Promise.all([
          api.mic(),
          api.stt(),
          api.embed(),
          api.latency(),
          api.router(),
        ]);
        if (!live) return;
        setMic(m);
        setStt(s);
        setEmbed(e);
        setLatency(l);
        setRouter(r);
      } catch {
        /* the shell already reports a dead backend */
      }
    };
    void tick();
    const t = window.setInterval(tick, 250);

    // Slower than the rest: this one is a group-by over the command log, and
    // the numbers it reports move once per spoken command, not four times a
    // second.
    const refreshRouting = () => {
      api.routingStats().then(setRouting).catch(() => {});
    };
    refreshRouting();
    const r = window.setInterval(refreshRouting, 2000);

    return () => {
      live = false;
      window.clearInterval(t);
      window.clearInterval(r);
    };
  }, []);

  const exportLog = async () => {
    try {
      setLogNote(`Written to ${await api.exportCommandLog()}`);
    } catch (e) {
      setLogNote(String(e));
    }
  };

  const forgetLog = async () => {
    try {
      const n = await api.forgetCommandLog();
      setRouting(await api.routingStats());
      setLogNote(n === 1 ? "Erased 1 command." : `Erased ${n} commands.`);
    } catch (e) {
      setLogNote(String(e));
    }
  };

  /**
   * Sign in, and wait — this resolves only once the browser round trip is over,
   * which can take minutes. Nothing else on this screen is blocked meanwhile.
   */
  const signIn = async () => {
    setAuthBusy(true);
    setAuthNote(null);
    try {
      setAccount(await api.signIn());
    } catch (e) {
      // Cancelling is the common case and is not a failure worth shouting
      // about: nothing changed, and the app works exactly as it did.
      setAuthNote(String(e).replace(/^Error:\s*/, ""));
    } finally {
      setAuthBusy(false);
    }
  };

  const signOut = async () => {
    try {
      setAccount(await api.signOut());
      setAuthNote(null);
    } catch (e) {
      setAuthNote(String(e));
    }
  };

  return (
    <div className="panel">
      <h1>Settings</h1>
      <p className="sub">
        The shortcut, the microphone and the models behind capture and search. Nothing here is
        required to use the app — including the account.
      </p>

      {account?.available && (
        <div className="card">
          <h2>Account</h2>
          <div className="row last">
            <span className="bd">
              <span className="k">
                {account.signed_in
                  ? (account.display_name ?? account.email ?? "Signed in")
                  : "Not signed in"}
              </span>
              <span className="v">
                {account.signed_in
                  ? account.email ?? "No email on this account."
                  : "Capture, search and everything you have already saved work without an " +
                    "account, and always will. Signing in is what will carry your memories " +
                    "between machines when sync arrives."}
              </span>
              {authNote && (
                <span className="v" style={{ marginTop: 4, display: "block", color: "var(--bad)" }}>
                  {authNote}
                </span>
              )}
            </span>
            {account.signed_in ? (
              <button type="button" className="btn" onClick={() => void signOut()}>
                Sign out
              </button>
            ) : (
              <button
                type="button"
                className="btn primary"
                disabled={authBusy}
                onClick={() => void signIn()}
              >
                {authBusy ? "Waiting for browser…" : "Sign in"}
              </button>
            )}
          </div>
        </div>
      )}

      {offline && (
        <p className="verdict bad">
          Backend not responding — the running app is older than this page. Stop it (Ctrl+C) and
          run <kbd>npm start</kbd> again. Readings below are stale.
        </p>
      )}

      <div className="card" id="capture">
        <h2>Capture</h2>
        <Shortcut
          name="Shortcut"
          blurb="Hold this anywhere in Windows to capture. Press Change, then hold the combination you want and let go."
          spec={settings?.hotkey ?? null}
          hook={hook}
          disabled={!settings}
          onSave={(spec) => api.setHotkey(spec, settings?.hold_threshold_ms ?? 120)}
          onSaved={setSettings}
        />

        <Shortcut
          name="Dictation"
          blurb="A second chord that types instead of capturing: hold it, speak, and the words go
                 straight into whatever you are working in. Nothing is routed and nothing is saved.
                 Speech is transcribed on this machine; a small model on our own server then
                 punctuates it and lays it out, so a spoken list arrives as a list — and if that
                 server is unreachable, the raw words are typed instead. Off until you bind it."
          spec={settings?.dictate_hotkey ?? null}
          hook={hook}
          disabled={!settings}
          onSave={(spec) => api.setDictateHotkey(spec)}
          onClear={() => api.setDictateHotkey(null)}
          onSaved={setSettings}
        />

        <div className="row">
          <span className="bd">
            <span className="k">Start with Windows</span>
            <span className="v">
              The app comes back when you sign in, straight to the tray with no window. An
              always-on shortcut you have to remember to launch is a shortcut you will not use.
            </span>
          </span>
          <span className="seg">
            <button
              type="button"
              className={settings?.start_at_login ? "on" : ""}
              onClick={() => void toggleStartAtLogin(true)}
            >
              On
            </button>
            <button
              type="button"
              className={settings && !settings.start_at_login ? "on" : ""}
              onClick={() => void toggleStartAtLogin(false)}
            >
              Off
            </button>
          </span>
        </div>

        <div className="row">
          <span className="bd">
            <span className="k">Resting pill</span>
            <span className="v">
              A small pill stays near the bottom of the screen when nothing is being captured.
              Click it for your last few memories. Turn it off if an always-on-top window gets in
              the way of full-screen work.
            </span>
          </span>
          <span className="seg">
            <button
              type="button"
              className={settings?.idle_pill ? "on" : ""}
              onClick={() => void toggleIdlePill(true)}
            >
              On
            </button>
            <button
              type="button"
              className={settings && !settings.idle_pill ? "on" : ""}
              onClick={() => void toggleIdlePill(false)}
            >
              Off
            </button>
          </span>
        </div>

        <div className="row">
          <span className="bd">
            <span className="k">Summarize meetings automatically</span>
            <span className="v">
              When a meeting recording stops, Claude writes its summary straight away. Off, the
              summary is written when you press Summarize on the meeting. Each summary sends the
              transcript to Anthropic and uses your API key.
            </span>
          </span>
          <span className="seg">
            <button
              type="button"
              className={settings?.auto_summarize_meetings ? "on" : ""}
              onClick={() => void toggleAutoSummarize(true)}
            >
              On
            </button>
            <button
              type="button"
              className={settings && !settings.auto_summarize_meetings ? "on" : ""}
              onClick={() => void toggleAutoSummarize(false)}
            >
              Off
            </button>
          </span>
        </div>

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

      <div className="card" id="models">
        <h2>Models</h2>
        <p>
          Speech and embeddings run on this machine; the router is a call to Claude. Only speech
          is required: without embeddings, search still works on exact words and only the queries
          that needed meaning stop working; without the router, familiar phrasings still work and
          anything else comes back as not understood.
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
          <span className="pill">
            <span
              className={`led ${
                router?.state === "ready" ? "ok" : router?.state === "loading" ? "warn" : ""
              }`}
            />
            <b>Router</b>
            {router ? router.detail || router.state : "…"}
          </span>
        </div>
        {router?.state === "missing" && (
          <div className="banner">
            <b>Only the built-in grammar is routing commands.</b> Familiar phrasings — “save this
            to react” — still work; anything else comes back as “not sure what to do with that”,
            and nothing can make a collection for you. Put your key in{" "}
            <kbd>anthropic_api_key</kbd> in <kbd>config.json</kbd>, or set{" "}
            <kbd>ANTHROPIC_API_KEY</kbd>, then restart.
          </div>
        )}
        {router?.state === "failed" && (
          <div className="banner">
            <b>The last command was not routed by the model.</b> {router.detail} The grammar
            answered it instead, which is why an unusual phrasing may have come back as not
            understood.
          </div>
        )}
        {embed && embed.state !== "ready" && embed.state !== "loading" && (
          <div className="banner">
            <b>Semantic search is off.</b> {embed.detail}
          </div>
        )}
      </div>

      <div className="card">
        <h2>Routing</h2>
        <p>
          Every command is recorded here — what was said, which tier routed it, and what you
          picked when it had to ask. It is how the grammar learns which phrasings it is missing,
          and the only evidence available for whether a question was worth asking. It stays on
          this machine.
        </p>
        <div className="grid">
          <div className="stat">
            <div className="k">Commands</div>
            <div className="v">{routing?.total ?? 0}</div>
          </div>
          <div className="stat">
            <div className="k">Grammar alone</div>
            <div className="v">
              {routing?.total ? Math.round((routing.tier0 / routing.total) * 100) : "—"}
              {routing?.total ? <small>%</small> : null}
            </div>
            {routing?.total ? (
              // ADR-0003 names ~40% as the line. Below it, either people are
              // phrasing things differently than the grammar assumes or it
              // needs extending — and this is the only number that says so.
              <div className={`verdict ${routing.tier0 / routing.total >= 0.4 ? "ok" : "bad"}`}>
                {routing.tier0 / routing.total >= 0.4
                  ? "no model needed"
                  : "below the 40% the grammar aims for"}
              </div>
            ) : null}
          </div>
          <div className="stat">
            <div className="k">Router rescued</div>
            <div className="v">{routing?.tier1 ?? 0}</div>
            <div className="verdict">{routing?.unrouted ?? 0} still not understood</div>
          </div>
          <div className="stat">
            <div className="k">Questions answered</div>
            <div className="v">{routing?.answered ?? 0}</div>
            {routing?.answered ? (
              // Picking the first option means the ranking was already right
              // and the question was the mistake. Worth showing plainly: it is
              // the cost side of asking.
              <div className="verdict">
                {Math.round((routing.accepted / routing.answered) * 100)}% picked the top suggestion
              </div>
            ) : null}
          </div>
        </div>
        <div className="row last">
          <span className="bd">
            <span className="k">Your command history</span>
            <span className="v">
              Export writes it as JSON beside the database. Erase removes every command and every
              answer with it — routing keeps working, it just starts learning again from nothing.
            </span>
          </span>
          <span className="seg">
            <button type="button" onClick={() => void exportLog()}>
              Export
            </button>
            <button type="button" onClick={() => void forgetLog()}>
              Erase
            </button>
          </span>
        </div>
        {logNote && <div className="banner">{logNote}</div>}
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
/// One bindable chord. Capture and dictation differ only in what they are for
/// and in whether they can be turned off, so they are the same control twice
/// rather than two that have to be kept in step.
function Shortcut({
  name,
  blurb,
  spec: bound,
  hook,
  disabled,
  onSave,
  onClear,
  onSaved,
}: {
  name: string;
  blurb: string;
  /// The bound spec, or null when nothing is.
  spec: string | null;
  hook: HookStats | null;
  disabled: boolean;
  onSave: (spec: string) => Promise<SettingsData>;
  /// Present only on a binding that is allowed to be unbound.
  onClear?: () => Promise<SettingsData>;
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
    if (!pending || disabled) return;
    try {
      onSaved(await onSave(pending));
      setPending(null);
      setHint({
        text: `${specLabel(pending)} is active now — no restart needed.`,
        kind: "ok",
      });
    } catch (e) {
      // Surface the backend's own words: it names the token that failed, or the
      // shortcut this one collides with, either of which is more use than a
      // generic "invalid shortcut".
      setHint({ text: `Not saved — ${String(e)}`, kind: "bad" });
    }
  };

  const clear = async () => {
    if (!onClear) return;
    try {
      onSaved(await onClear());
      setPending(null);
      setHint({ text: `${name} is off.`, kind: "" });
    } catch (e) {
      setHint({ text: `Not saved — ${String(e)}`, kind: "bad" });
    }
  };

  const label = pending
    ? specLabel(pending)
    : (preview ??
      (recording ? "Press keys…" : bound ? specLabel(bound) : onClear ? "Off" : "—"));

  return (
    <>
      <div className="row">
        <span className="bd">
          <span className="k">{name}</span>
          <span className="v">{blurb}</span>
        </span>
        <span className={`keycap${recording ? " listening" : ""}`}>{label}</span>
        {!pending && !recording && (
          <button className="btn" type="button" onClick={start}>
            {bound ? "Change" : "Bind"}
          </button>
        )}
        {!pending && !recording && bound && onClear && (
          <button className="btn" type="button" onClick={() => void clear()}>
            Turn off
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
