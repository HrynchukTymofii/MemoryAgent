import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

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

/** Just enough of a memory to list it in the expanded pill. */
interface Recent {
  id: string;
  title: string;
  collection: string | null;
}

/** What the overlay does when nothing is being captured. */
interface RestState {
  idle_pill: boolean;
  /** Anchored by its top edge, so its panel opens downward. */
  top: boolean;
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
  // Five results overflow the resting window exactly as three options did.
  fitWindowToContent();
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
  for (const [i, option] of options.slice(0, 4).entries()) {
    const li = document.createElement("li");
    li.className = "option";

    const button = document.createElement("button");
    button.type = "button";
    // Numbered because a list of destinations is read, not scanned: the number
    // gives the eye somewhere to land, and it is what a keyboard answer will
    // bind to when that arrives.
    const badge = document.createElement("span");
    badge.className = "num";
    badge.textContent = String(i + 1);
    button.appendChild(badge);

    const title = document.createElement("span");
    title.className = "title";
    title.textContent = option.path.replace(/\//g, " / ");
    button.appendChild(title);

    // The backend answers by emitting a normal receipt, so this handler has
    // nothing to render — an answered command and one that never needed asking
    // end up looking identical, which is the point.
    button.addEventListener("click", () => {
      results.classList.add("answered");
      invoke("answer_question", { index: i }).catch(() => {});
    });

    li.appendChild(button);
    results.appendChild(li);
  }
  if (options.length) {
    results.classList.add("shown");
    fitWindowToContent();
  }
}

function clearResults() {
  results.replaceChildren();
  results.classList.remove("shown", "answered");
}

/**
 * Tell the backend how tall this window needs to be.
 *
 * The content is anchored to the bottom of the window, so a window shorter than
 * its content does not scroll — it overflows off the *top* and is clipped away,
 * leaving only the last row visible. Estimating the height in Rust got that
 * wrong, and it would keep getting it wrong: row heights depend on the font
 * Windows actually resolved, on display scaling, and on how many options there
 * are.
 *
 * So the page measures itself instead. `getBoundingClientRect` reports the full
 * laid-out height even for the part the viewport is currently clipping, which is
 * exactly the number needed to stop clipping it. Measured after a paint, or the
 * rows just added have no geometry yet.
 */
function fitWindowToContent() {
  requestAnimationFrame(() => {
    const top = results.getBoundingClientRect().top;
    const bottom = pill.getBoundingClientRect().bottom;
    // The margins live outside both rects: nothing above the list, 10px below
    // the pill, plus slack so a rounding difference cannot re-clip it.
    const css = Math.ceil(bottom - top) + 26;

    // Sent in *physical* pixels. These rects are CSS pixels, and a window is
    // sized in logical ones — on a scaled display those are not the same unit,
    // so passing the number straight through made the window short by exactly
    // the scale factor and clipped the top option away. `devicePixelRatio` is
    // the conversion, and it is right whichever unit the webview turns out to
    // be using.
    const dpr = window.devicePixelRatio || 1;
    invoke("size_overlay", { height: Math.ceil(css * dpr), css, dpr }).catch(() => {});
  });
}

// ------------------------------------------------------------------ idle

let idle = false;
let expanded = false;

/**
 * Shrink the window onto the pill, then let it be clicked.
 *
 * Strictly in that order. This is the only state where the overlay is clickable
 * without a question on screen, and a transparent window intercepts clicks
 * across its whole rectangle — so turning clicks on while the window is still
 * its resting 520px wide would swallow everything in a band across the display
 * where nothing is drawn.
 */
function fitIdleWindow() {
  requestAnimationFrame(() => {
    const box = pill.getBoundingClientRect();
    const panel = expanded ? results.getBoundingClientRect() : box;
    // Measured as an extent rather than pill-to-results, because which of the
    // two is on top depends on which edge the pill is anchored by.
    const css =
      Math.ceil(Math.max(box.bottom, panel.bottom) - Math.min(box.top, panel.top)) + 26;
    // The margins are outside the rect, and the shadow is drawn outside the
    // border box: too tight a width clips it into a visible hard edge.
    const width = Math.ceil(Math.max(box.width, panel.width) + 34);
    const dpr = window.devicePixelRatio || 1;
    invoke("size_overlay", {
      height: Math.ceil(css * dpr),
      width: Math.ceil(width * dpr),
      css,
      dpr,
    })
      .then(() => invoke("overlay_clickable", { clickable: true }))
      .catch(() => {});
  });
}

function enterIdle() {
  idle = false; // so collapse() does not try to re-fit mid-transition
  expanded = false;
  clearResults();
  animating = false;
  cancelAnimationFrame(raf);
  stopLevelPolling();
  setIdleBars();
  document.body.classList.add("idle");
  pill.className = "shown idle";
  dot.hidden = true;
  kbd.hidden = true;
  text.textContent = "Memory";
  idle = true;
  fitIdleWindow();
}

/** Leave idle for a capture: full width, click-through, normal pill. */
function leaveIdle() {
  idle = false;
  expanded = false;
  document.body.classList.remove("idle");
  invoke("overlay_clickable", { clickable: false }).catch(() => {});
}

async function expand() {
  expanded = true;
  pill.classList.add("expanded");
  text.textContent = "Recent";

  let items: Recent[] = [];
  let open = 0;
  try {
    [items, open] = await Promise.all([
      invoke<Recent[]>("recent", { limit: 3 }),
      invoke<number>("open_task_count"),
    ]);
  } catch {
    // An empty panel is still a correct answer here — the pill is expanded and
    // says so. Failing loudly over a decorative list would be worse.
  }

  results.replaceChildren();
  for (const item of items) {
    const li = document.createElement("li");
    li.className = "recent";
    const title = document.createElement("span");
    title.className = "title";
    title.textContent = item.title;
    li.appendChild(title);
    const meta = document.createElement("span");
    meta.className = "meta";
    meta.textContent = item.collection?.replace(/\//g, " / ") ?? "Unfiled";
    li.appendChild(meta);
    results.appendChild(li);
  }

  const foot = document.createElement("li");
  foot.className = "foot";
  const count = document.createElement("span");
  count.className = "count";
  count.textContent = open === 1 ? "1 task open" : `${open} tasks open`;
  foot.appendChild(count);
  const button = document.createElement("button");
  button.type = "button";
  button.textContent = "Open Hub";
  button.addEventListener("click", (e) => {
    e.stopPropagation();
    invoke("open_hub").catch(() => {});
    collapse();
  });
  foot.appendChild(button);
  results.appendChild(foot);

  results.classList.add("shown");
  fitIdleWindow();
}

function collapse() {
  if (!expanded) return;
  expanded = false;
  pill.classList.remove("expanded");
  text.textContent = "Memory";
  clearResults();
  fitIdleWindow();
}

/**
 * Drag to move, click to open — from one button, with no modifier.
 *
 * The window manager performs the drag, and `startDragging` hands control to it
 * immediately and irreversibly: called on mousedown it swallows the click, so
 * the pill could be moved or opened but never both. So the press is held until
 * the pointer actually travels, and only a real movement becomes a drag. Below
 * the threshold it stays an ordinary click.
 */
const DRAG_THRESHOLD = 3;
let press: { x: number; y: number } | null = null;
let dragged = false;

pill.addEventListener("mousedown", (e) => {
  if (!idle || expanded || e.button !== 0) return;
  press = { x: e.screenX, y: e.screenY };
  dragged = false;
});

window.addEventListener("mousemove", (e) => {
  if (!press) return;
  if (Math.abs(e.screenX - press.x) < DRAG_THRESHOLD &&
      Math.abs(e.screenY - press.y) < DRAG_THRESHOLD) {
    return;
  }
  press = null;
  dragged = true;
  void getCurrentWindow()
    .startDragging()
    // The drag ends when the button is released, and nothing tells the page
    // that happened — the pointer belongs to the window manager for the
    // duration. `startDragging` resolving is the signal, and the window's
    // final position is whatever it is by then.
    .then(() => savePillAnchor())
    .catch(() => {});
});

window.addEventListener("mouseup", () => {
  press = null;
});

pill.addEventListener("click", () => {
  if (!idle) return;
  // The mouseup that ends a drag also lands here as a click. Opening the panel
  // every time the pill is moved would make it impossible to just move it.
  if (dragged) {
    dragged = false;
    return;
  }
  if (expanded) collapse();
  else void expand();
});

/** Record where the pill ended up, and flip the panel if it changed sides. */
async function savePillAnchor() {
  try {
    applyAnchor(await invoke<RestState>("save_pill_anchor"));
  } catch {
    // The pill is where the user put it either way; only the memory of it is
    // lost, and it will be re-saved on the next drag.
  }
  fitIdleWindow();
}

function applyAnchor(rest: RestState) {
  document.body.classList.toggle("top", rest.top);
}

// The overlay never takes focus, so there is no blur to close on and no
// keyboard reaching this window. Escape is here for the case where the webview
// does happen to have focus — cheap, and the alternative is a panel with no
// keyboard way out at all.
window.addEventListener("keydown", (e) => {
  if (e.key === "Escape") collapse();
});

await listen("capture:idle", () => {
  enterIdle();
});

await listen("capture:begin", () => {
  // A capture starting is what dismisses an expanded pill. The window has
  // already been restored to its resting size and made click-through on the
  // Rust side; this is the page catching up with that.
  leaveIdle();
  clearResults();
  pill.className = "";
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
  // Deliberately not returning to idle here. This is the fade, and the Rust
  // side decides what follows it — hide, or `capture:idle` — because only it
  // knows whether the user wants a resting pill at all.
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
    // Heard, understood, and deliberately not acted on: the destination was
    // not decidable, and guessing would file the memory somewhere wrong
    // (ADR-0005).
    pill.classList.add("unsure");
    // "Which collection?" above an empty list is not a question, it is a
    // riddle. Nothing matching at all is a different answer from several
    // things matching equally, and the user can only act on the difference if
    // we say which one happened.
    text.textContent = r.ask.options.length
      ? `Which ${r.ask.slot}?`
      : `No ${r.ask.slot} matches that`;
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

// Last, after every listener above is registered: ask whether this window
// should be resting on screen rather than waiting to be summoned. Asked, not
// awaited from an event — see `idle_pill_enabled` for why the push version
// silently did nothing.
try {
  const rest = await invoke<RestState>("rest_state");
  applyAnchor(rest);
  if (rest.idle_pill) enterIdle();
} catch {
  // No answer means no resting pill, which is the pre-pill behaviour and a
  // perfectly good place to fail to.
}
