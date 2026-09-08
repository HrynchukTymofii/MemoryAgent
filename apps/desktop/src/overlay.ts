import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

interface Hit {
  id: string;
  title: string;
  snippet: string;
  collection: string | null;
  source_url: string | null;
  score: number;
  /// Which retrievers found it: "keyword", "vector", or both.
  why: string;
}

interface Outcome {
  kind: string;
  summary: string;
  provenance: string | null;
  item_id: string | null;
  results?: Hit[];
  open?: { item_id: string; title: string; url: string | null } | null;
  took_ms: number;
}

interface Candidate {
  path: string;
  score: number;
}

/** A narrow question: one slot, a few indistinguishable candidates. */
interface Ambiguity {
  slot: string;
  options: Candidate[];
}

interface CaptureResult {
  text: string;
  audio_secs: number;
  inference_ms: number;
  total_ms: number;
  empty: boolean;
  outcome: Outcome | null;
  ask: Ambiguity | null;
}

// Everything is resolved once, at load, while the window is still hidden.
// Nothing on the shortcut path may touch the DOM tree or allocate.
const pill = document.getElementById("pill") as HTMLDivElement;
const wave = document.getElementById("wave") as HTMLSpanElement;
const text = document.getElementById("text") as HTMLSpanElement;
const kbd = document.getElementById("kbd") as HTMLSpanElement;
const dot = document.getElementById("dot") as HTMLSpanElement;
const results = document.getElementById("results") as HTMLUListElement;

const BARS = 14;
const bars: HTMLElement[] = [];
for (let i = 0; i < BARS; i++) {
  const b = document.createElement("i");
  wave.appendChild(b);
  bars.push(b);
}

let animating = false;
let raf = 0;
let levelTimer = 0;
let level = 0;

/**
 * Waveform driven by the real microphone level.
 *
 * The level is polled rather than pushed: an event stream would be ~20 IPC
 * messages a second for a decorative signal, and the overlay only needs it while
 * visible. The bars interpolate between polls so the motion stays smooth at a
 * poll rate far below the frame rate.
 */
function animate(t: number) {
  if (!animating) return;
  // sqrt: speech sits around 0.02-0.3 RMS, so a linear mapping barely moves.
  const amp = Math.min(1, Math.sqrt(level) * 2.6);
  for (let i = 0; i < BARS; i++) {
    const phase = t / 190 + i * 0.62;
    const wobble = 0.55 + 0.45 * Math.sin(phase);
    const h = 4 + amp * wobble * 17;
    bars[i].style.height = `${h.toFixed(1)}px`;
  }
  raf = requestAnimationFrame(animate);
}

function startLevelPolling() {
  stopLevelPolling();
  levelTimer = window.setInterval(async () => {
    try {
      level = await invoke<number>("capture_level");
    } catch {
      level = 0;
    }
  }, 60);
}

function stopLevelPolling() {
  if (levelTimer) window.clearInterval(levelTimer);
  levelTimer = 0;
}

function setIdleBars() {
  for (const b of bars) b.style.height = "4px";
}

/**
 * Render search results above the pill.
 *
 * Built with `createElement` and `textContent` throughout. Every string here
 * originates in a speech model transcribing whatever was audible, or in a web
 * page the user once saved — and it is about to be rendered in a window that
 * floats above everything they are doing, in a process that can open URLs.
 * `innerHTML` anywhere on this path would be a genuine hole.
 */
function showResults(hits: Hit[]) {
  results.replaceChildren();
  if (!hits.length) {
    results.classList.remove("shown");
    return;
  }
  for (const [i, hit] of hits.entries()) {
    const li = document.createElement("li");
    if (i === 0) li.classList.add("best");

    const title = document.createElement("span");
    title.className = "title";
    title.textContent = hit.title;
    li.appendChild(title);

    const meta = document.createElement("span");
    meta.className = "meta";
    meta.textContent = hit.collection ?? hit.snippet;
    const why = document.createElement("span");
    why.className = "why";
    why.textContent = `  ${hit.why}`;
    meta.appendChild(why);
    li.appendChild(meta);

    results.appendChild(li);
  }
  results.classList.add("shown");
}

/**
 * The candidates behind a question the router could not answer itself.
 *
 * Read-only, like the results: the overlay is click-through by design, so the
 * way to answer is to say it again more specifically. Showing the options is
 * what makes that possible — "which one?" with no list is not a question, it is
 * a shrug.
 */
function showCandidates(options: Candidate[]) {
  results.replaceChildren();
  for (const option of options.slice(0, 4)) {
    const li = document.createElement("li");
    const title = document.createElement("span");
    title.className = "title";
    title.textContent = option.path.replace(/\//g, " / ");
    li.appendChild(title);
    results.appendChild(li);
  }
  if (options.length) results.classList.add("shown");
}

function clearResults() {
  results.replaceChildren();
  results.classList.remove("shown");
}

await listen("capture:begin", () => {
  clearResults();
  pill.classList.remove("done", "empty", "unsure");
  pill.classList.add("shown", "live");
  dot.hidden = false;
  kbd.hidden = false;
  text.innerHTML = '<span class="hint">Listening…</span>';

  // Report the first frame that actually composites after the window was
  // shown, closing the stage-1 measurement in latency.rs. Two nested rAFs on
  // purpose: the first fires before this frame's paint, the second after it,
  // so we time a real pixel rather than a scheduling callback.
  //
  // The timeout is not belt-and-braces, it is required. A hidden WebView2 is
  // throttled and rAF does not run at all while the window is not visible; if
  // show() and the event race such that the callback lands before the webview
  // has resumed, the rAF chain can stall indefinitely and the measurement — and
  // any real work chained behind it — would simply never happen.
  let reported = false;
  const report = () => {
    if (reported) return;
    reported = true;
    invoke("overlay_painted").catch(() => {});
  };
  requestAnimationFrame(() => requestAnimationFrame(report));
  setTimeout(report, 250);

  animating = true;
  startLevelPolling();
  raf = requestAnimationFrame(animate);
});

// Key released: speech is over, transcription is running. The overlay stays up.
await listen("capture:end", () => {
  animating = false;
  cancelAnimationFrame(raf);
  stopLevelPolling();
  setIdleBars();
  dot.hidden = true;
  kbd.hidden = true;
  // Keep `shown`: the pill must stay visible to display the transcript.
  pill.classList.remove("live");
  text.innerHTML = '<span class="hint">Transcribing…</span>';
});

await listen("capture:hide", () => {
  pill.classList.remove("shown", "live", "done", "empty", "unsure");
  clearResults();
});

await listen<CaptureResult>("capture:result", (e) => {
  const r = e.payload;
  if (r.empty || !r.text.trim()) {
    pill.classList.add("empty");
    text.innerHTML = '<span class="hint">Didn\'t catch that</span>';
    return;
  }
  // textContent, never innerHTML, everywhere below: these strings came from a
  // speech model acting on whatever was audible, and they are rendered in a
  // window floating above everything the user is doing.
  const sub = document.createElement("span");
  sub.className = "sub";

  if (r.outcome) {
    // The receipt: what happened, with where it came from beneath it.
    //
    // Green means something exists now that did not before, or something was
    // found. A command that was understood and correctly did nothing — nothing
    // to save, nothing found, an intent that is not built yet — gets the amber
    // treatment, because "understood, did nothing" and "done" must not look the
    // same at a glance.
    const acted =
      r.outcome.item_id !== null || (r.outcome.results?.length ?? 0) > 0;
    pill.classList.add(acted ? "done" : "unsure");
    text.textContent = r.outcome.summary;
    sub.textContent = r.outcome.provenance ?? r.text;
    showResults(r.outcome.results ?? []);
  } else if (r.ask) {
    // Heard, understood, and deliberately not acted on: two destinations were
    // indistinguishable, and guessing would file the memory somewhere wrong
    // (ADR-0005).
    pill.classList.add("unsure");
    text.textContent = `Which ${r.ask.slot}?`;
    sub.textContent = r.text;
    showCandidates(r.ask.options);
  } else {
    // Heard perfectly, understood not at all. This is the case that has to be
    // said out loud: silence here is indistinguishable from "it saved and you
    // cannot see it", and the user has no way to tell which.
    pill.classList.add("unsure");
    text.textContent = "Not sure what to do with that";
    sub.textContent = r.text;
    clearResults();
  }

  if (sub.textContent) text.appendChild(sub);

  kbd.hidden = false;
  kbd.textContent = `${r.total_ms} ms`;
});
