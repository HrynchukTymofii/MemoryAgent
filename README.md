# Personal Memory OS

Local-first desktop assistant: press a shortcut, speak, and the system captures
context, performs the action, and disappears.

Architecture and reasoning live in [`docs/architecture-brief.html`](docs/architecture-brief.html);
decision records are in [`docs/adr/`](docs/adr/). Where those and the original
`personal_memory_os_spec.md` disagree, the brief wins and an ADR says why.

## Status — M2 complete

The loop closes. You can speak something into it and later ask for it back in
words you did not use when you saved it.

| | |
|---|---|
| ✅ **M0** skeleton | workspace, SQLite + FTS5, tray, global hotkey, pre-warmed overlay |
| ✅ **M1** capture | ring buffer, VAD, whisper.cpp, Windows context, Tier 0 grammar, `SAVE`/`NOTE` |
| ✅ **M2** retrieval | ONNX embeddings, background embed worker, FTS5 + vectors + RRF, `SEARCH`/`SHOW`/`OPEN`, page capture, the Hub |
| ⬜ **M3** intelligence | Tier 1 router, correction log, derived confidence, `MOVE`/`TAG`/`TASK` |

144 tests, clippy clean, `tsc --noEmit` clean.

## Prerequisites

```
rustup update stable          # 1.82+; older toolchains cannot build the deps
winget install Kitware.CMake  # whisper.cpp
cargo install tauri-cli --version "^2"
```

MSVC build tools, the Windows SDK and WebView2 are also required; recent
Windows 11 installs already have WebView2. whisper.cpp additionally needs
**libclang** — see [Toolchain](#toolchain).

Models are not committed. Fetch them once:

```
.\scripts\fetch-models.ps1 base.en     # speech, ~148 MB
.\scripts\fetch-models.ps1 embedding   # bge-small-en-v1.5, int8, ~33 MB
```

## Run

```
cd apps/desktop
npm install
npm start          # = tauri dev: starts Vite, builds Rust, launches the app
```

**Use `npm start`, not `cargo run`.** A debug Rust build puts Tauri in dev mode,
where both windows load `devUrl` (`http://localhost:1420`) rather than the
bundled files. Without the Vite dev server running, every window renders
Chromium's "site can't be reached" page — a grey rectangle with a scrollbar
where the overlay pill should be.

To run the binary standalone, build in release so it embeds the frontend:

```
cd apps/desktop && npm run build
cd ../.. && cargo run -p memos-desktop --release
```

Then **hold the capture chord** anywhere in Windows and speak.

```
save this to study programming react     -> files the page you are looking at
add this to react                        -> the same thing, in the words people use
note that the router is behind the books -> a standalone note
find the thing about state snapshots     -> results in the overlay
open the react article                   -> reopens the source in your browser
```

### What `SAVE` actually stores

In order of how directly you chose it:

| | |
|---|---|
| **A selection** | exactly what you highlighted. An explicit choice always wins |
| **The page** | its readable text, trimmed of navigation, banners and footer |
| **A URL alone** | only when the page has no prose in it — a bookmark, not a memory |

The receipt says which one happened — `highlighted text + voice`, `page + voice`
or `link only + voice` — because they look identical in the library and mean very
different things about what will be findable later.

The page is read through the same UI Automation interface as the selection: the
rendered text is already in the accessibility tree because screen readers need it
there, so there is no browser extension and no network fetch. Measured on a real
article: 9,603 characters in 177 ms, inside the 400 ms collection deadline.

**A save still needs something to save.** With no selection, no page and no URL,
it declines and points you at `note that…`: storing the command itself as a
memory titled "add this to react" is an echo, not a capture, and it would spend
one of the fifty weekly captures the free plan allows.

### Trimming is the hard half

UI Automation returns the article *and* the navigation bar, the cookie banner,
the "related stories" rail and the footer, in reading order with no structure to
tell them apart. That furniture is near-identical on every page anyone saves,
which is the worst possible property for a search index — it makes every
document look faintly like every other one.

With no markup the one usable signal is text density. Chrome is short lines:
menu items, buttons, bylines. Prose is long lines. So `readable::extract` takes
the span between the first and last dense line, then drops any run of three or
more short lines inside it — a menu or a sidebar. One or two short lines between
paragraphs are headings, and they stay.

A page with no prose at all returns nothing rather than something. A search
results page or a mail client's folder list would otherwise be stored, and every
such capture looks like every other one.

## The loop, end to end

```
routed   intent=SAVE  collection=Study/Programming/React  routing_ms=0  tier=Grammar
executed summary=Saved to Study / Programming / React     took_ms=0
transcribed  inference_ms=221  total_ms=234  rtf=18.0x
```

**234 ms from key release to done**, inside the 300 ms the budget allows. The
embedding is not in that number and never will be: it lands afterwards, on a
background thread, because a capture is durable the moment the transaction
commits and nothing waits for a vector.

```
embedded count=6 took_ms=25
```

Check any piece on its own:

```
cargo run -p memos-stt --example mic_check                              # microphone
cargo run -p memos-stt --features whisper --example transcribe_check    # record 5 s, transcribe
cargo run -p memos-context --example context_check                      # what we can read from a window
cargo run -p memos-embed --features onnx --example embed_check          # embeddings carry meaning
cargo run -p memos-embed --features onnx --example search_check         # capture -> embed -> retrieve
cargo run -p memos-agent --example route_check                          # what Tier 0 does with a sentence
```

## What the grammar understands

Tier 0 is a fixed prefix grammar (ADR-0003): it routes the formulaic majority
with no model at all, and refuses everything else rather than guessing. That
refusal is correct — and for a while it was also invisible, which is worse than
being wrong.

**Nothing that is heard is ever ignored in silence.** The overlay now
distinguishes three outcomes, because from the outside they are impossible to
tell apart otherwise:

| | |
|---|---|
| **done** (green) | something exists now that did not before, or something was found |
| **understood, did nothing** (amber) | nothing to save, nothing found, or a destination too ambiguous to pick — with the candidates listed |
| **not understood** (amber) | "Not sure what to do with that", above what was actually heard |

The second and third used to render identically to a successful capture: the
transcript, and no other signal. A user saying "add this to react" got a pill
that looked exactly like a save and a library that stayed empty.

`route_check` is the tool for this — it answers "I said something and nothing
happened" by naming which of the two happened:

```
  add this to react                                 SAVE      Study/Programming/React
  put this in react                                 SAVE      Study/Programming/React
  file this under react                             SAVE      Study/Programming/React
  add this to my react notes                        AMBIGUOUS collection: Study/Programming/React 0.28
  remind me next Tuesday                            UNRECOGNISED  -> Tier 1 (M3)

  17/18 routed by grammar alone
```

The verb list grew because "save" is one word for this out of many, and not the
one most people reach for first: `add`, `put`, `file`, `keep`, `store`,
`capture` and `bookmark` all route now. Each requires an object — "add this",
never a bare "add" — because the bare verbs belong to other intents ("add a
task"), and a first-match grammar would swallow them.

## What M2 proves

`search_check` captures six items, embeds them the way the worker does, then
asks five questions phrased the way somebody would actually ask months later:

```
  "why doesn't my component see the new value straight away"
    -> State as a Snapshot        (embed 3 ms, search 0.31 ms)
       1.3797  State as a Snapshot            vector
       1.1500  Customer Acquisition Cost      keyword
       keyword alone: Customer Acquisition Cost

  "where did I put the wifi details"
    -> Router config              (embed 2 ms, search 0.09 ms)
       1.4241  Router config                  vector
       keyword alone: nothing

hybrid  5/5
keyword 3/5
```

Both halves are load-bearing. Keyword search alone answers three of the five and
returns *nothing at all* for "where did I put the wifi details" — the item it
should find shares not one word with the question. The keyword half earns its
place at the other end: `pgbouncer` is one rare exact token, where BM25 is
decisive and an embedding is at the mercy of whether the model has a useful
representation of the word. Neither retriever knows which kind of question it
was handed, which is the whole reason this is a fusion rather than a choice.

The model is the int8-quantized ONNX export of bge-small-en-v1.5 — a third the
size of the fp32 one, which matters for something that runs on a background
thread on somebody's laptop while they keep working.

Two failures found on the way there, both of which made search quietly wrong
rather than obviously broken:

**A spoken query is mostly function words.** "Why doesn't my component see the
new value straight away" is two content words and nine that appear in every
document ever written. FTS5 ORs the terms, so BM25 ranked documents by *how
ordinary their vocabulary was* — and every item matched every query. Stop words
are now dropped before the query is built (`sanitise_fts`), unless dropping them
would leave nothing.

**Every corpus has a nearest neighbour.** An unfiltered kNN scan returns the
whole library ranked by how *least unrelated* each item is, and reciprocal rank
fusion reads that tail as corroborating evidence — so an item the model scored
as unrelated could out-rank a genuine paraphrase, purely by appearing in both
lists. `search_vector` now takes a similarity floor (0.55, measured: bge-small
scores a real paraphrase around 0.70–0.75 and two unrelated English sentences
still 0.43–0.48). Below it, the honest answer is that the vector side found
nothing.

Fusion works on ranks and throws magnitudes away, which is what makes it robust
across incomparable scoring scales — but it also means "barely above the floor"
and "near-identical" fuse the same. So cosine is carried through to re-ranking
as `w1·semantic_similarity`, the term §7 of the brief specifies.

### Measured

| | |
|---|---|
| Embedding model load | 261 ms, on a background thread at startup |
| Embed a document | 3.2 ms (batched) |
| Embed a query | 2–3 ms |
| Search 6 items, both retrievers, fused and re-ranked | 0.09 – 0.45 ms |
| Search from the Hub, including IPC | 7 ms |
| Backfill 6 items | 25 ms |

The vector scan is brute-force cosine over an in-memory index. At 384 dimensions
and 10k items that is 15 MB and a few milliseconds; sqlite-vec would do the same
brute-force scan, and only starts to win at the ANN threshold the brief puts
around 100k items.

## Why the embedding worker is never on the critical path

A capture commits and is acknowledged with no vector attached. The worker
notices afterwards and backfills, which has two consequences worth stating:

1. **A missing or broken embedding model degrades search; it does not break
   capture.** Keyword retrieval works on a machine that has never embedded
   anything — which is the state of every machine for the first minute after
   install. The Hub says so out loud rather than letting semantic search fail
   silently.
2. **The queue is derived, not authoritative.** It is "items with no vector", so
   a job row lost to a crash costs nothing and a half-finished backfill resumes
   by itself on the next launch. Captures nudge the worker awake; a 30-second
   idle poll catches anything that arrived another way.

### The ONNX Runtime has to be pointed at explicitly

`ort` is built with `load-dynamic`, which searches for `onnxruntime.dll` by bare
name — and **Windows ships its own copy, 1.17.1, in System32**. It loads first
and then fails against the 1.20 API this build requires, with an error about a
missing symbol rather than about the wrong file being found. `use_bundled_runtime`
resolves an absolute path (beside the executable when installed, `runtime/` in
the repo during development) and sets `ORT_DYLIB_PATH` before the first session.
An existing value always wins: somebody who set it meant it.

## The Hub

React arrives here at M2, and only here — the overlay stays framework-free
permanently, because its paint time *is* stage 1 of the latency budget. The
overlay bundle is 3.4 kB; the Hub's is 213 kB.

| Screen | |
|---|---|
| **Home** | totals, the weekly meter, how much of the library is searchable, recent captures by day |
| **Library** | everything, searchable. Typing here runs exactly the retrieval the voice command runs |
| **Collections** | the hierarchy with item counts; picking one filters the Library |
| **Settings** | shortcut recorder, hold timing, microphone, model status, latency, hook diagnostics |

Every result says which retriever found it — `keyword`, `vector`, or both. It is
a small thing, but a result the keyword side never saw is a different kind of
answer, and being able to see the difference is what makes a surprising result
legible rather than magic.

Clicking a result reopens its source and records the access. That access is the
cheapest real relevance label there is — the result the user opened is the
result that was right — and it feeds the ranking signal that section 11.6
describes.

## Choosing a shortcut

Change it in the app: **Hub → Settings → Change**, hold the combination, release,
Save. It applies immediately, with no restart, and is written back to
`config.json`.

`%APPDATA%\PersonalMemoryOS\config.json`, written with defaults on first run:

```json
{ "hotkey": "ctrl+win", "hold_threshold_ms": 120 }
```

Accepts modifier-only chords (`ctrl+win`, `rctrl`, `alt+shift`), chords with a
key (`ctrl+alt+space`), and **bare keys** (`f9`). A bare key is a first-class
option, not a fallback: it sidesteps conflicts with other always-on voice tools
bound to modifier chords. Prefix `l`/`r` to bind one side specifically.

**Check for a collision before choosing.** Low-level keyboard hooks are a chain:
Windows calls every installed hook for every keystroke, and no well-behaved tool
swallows modifiers (that would break Ctrl+C system-wide). So if another
always-on voice tool is bound to the same chord, **both fire** — two overlays,
two microphone streams, two transcripts. Nothing errors, it is just useless.
Wispr Flow ships on `ctrl+win`, which is why this machine is configured for
`f9`.

`hold_threshold_ms` debounces: the chord must be held that long before capture
starts, so a single-modifier binding does not fire during an ordinary Ctrl+C.
The delay is free — the microphone ring buffer is always running, so audio
spoken during it is already recorded.

## Latency

Real physical keypresses, debug build, Vite dev server in the loop:

| Stage | Measured | Budget |
|---|---|---|
| Position + `show()` (native) | 0.32 – 0.55 ms | |
| Hotkey → overlay painted, median | **9.9 ms** | 50 ms |
| Hotkey → overlay painted, p95 | **15.8 ms** | 50 ms |
| Key release → transcript → executed | **234 ms** | 300 ms |

Roughly 5x headroom on the overlay, and a release build removes the dev server
from the path. Synthetic injection over 10 cycles produced the same distribution,
so the figure is stable rather than a lucky run.

§4 of the brief claims hotkey → overlay painted stays under 50 ms. That holds
only because the overlay window is constructed and painted at startup and merely
*shown* on the shortcut — building it on demand costs 200–400 ms. Settings turns
its percentile tile red the moment that stops being true.

The measurement spans the hook callback through to the overlay's first
composited frame, so it includes the IPC round-trip and slightly overstates paint
time. Overstating is the right direction to be wrong.

## Three failure modes worth knowing about

All three cost hours, and all three are now guarded.

**A UTF-8 BOM in `config.json` silently reverted every setting.** `serde_json`
rejects a leading BOM, and the loader fell back to defaults without saying so —
so an edited shortcut simply never took effect. Notepad and PowerShell's
`Set-Content -Encoding utf8` both write a BOM, so this would have hit real users.
The loader now strips it, and an unparseable config is reported to `diag.log`.

**`tauri dev` hot-reloads the frontend but not the Rust binary.** A long-running
`npm start` serves a new Hub from an old backend, and because the Hub swallowed
IPC errors, every tile kept its initial markup and read as a confident `0 keys
seen` — indistinguishable from a dead hook. The Hub now says so out loud. After
changing Rust, stop the process and restart it.

**whisper.cpp pads every request to 30 seconds.** A 2-second clip costs almost as
much as a 10-second one, so inference is roughly constant at ~780 ms regardless
of utterance length — which is why the realtime factor looks worse on short
commands than on long ones. Batch-transcribing at release pays that full fixed
cost after the user stops speaking. Streaming fixes it by construction:
transcribe rolling windows *while* they talk, so only the last chunk is
outstanding at release. That is why §4 specifies streaming rather than treating
it as an optimisation.

## Diagnostics

`config.json` accepts `"debug_keys": true`, which appends every key transition
to `keylog.txt` — virtual-key code, edge, computed modifier bits, and whether
the chord matched. It is the fastest way to answer "why doesn't my shortcut
fire", and it is off by default because it records everything you type.

`diag.log` records startup, hook installation, model loading and health
unconditionally — stdout is block-buffered once redirected, so the startup
errors you most need are exactly the ones that vanish when the process is killed
rather than exiting cleanly. `latency.log` accumulates every measured capture.
All live beside `config.json` in `%APPDATA%\PersonalMemoryOS\`.

## Layout

```
crates/
  memos-core/       domain types, intent enum, confidence — no I/O
  memos-db/         SQLite, migrations, repositories, the vector index
  memos-stt/        ring buffer, VAD, whisper.cpp
  memos-context/    what the foreground window can tell us
  memos-agent/      Tier 0 grammar, collection resolution, execution
  memos-embed/      ONNX embeddings; knows nothing about SQLite
  memos-retrieval/  RRF fusion, signal re-ranking
apps/desktop/
  src-tauri/        hotkey hook, tray, latency, embed worker, window lifecycle
  src/hub/          React Hub
  src/features/     Home, Library, Collections, Settings
  src/overlay.ts    the capture pill — vanilla, deliberately
docs/               architecture brief + ADRs
runtime/            onnxruntime.dll, shipped rather than found
```

The dependency direction is worth preserving: `memos-embed` knows nothing about
storage, `memos-retrieval` knows nothing about models, and the application wires
them together. That is what lets search run with no embedder at all.

## Testing

```
cargo test --workspace                        # 144 tests
cargo clippy --workspace --all-targets        # clean
cd apps/desktop && npx tsc --noEmit           # clean
```

### Toolchain

whisper.cpp needs **cmake** and **libclang** (LLVM). `LIBCLANG_PATH` must point
at the LLVM `bin` directory; it is set persistently at the user level, so open a
new terminal after installing. `whisper-rs` is pinned to **0.16** — 0.14 emits
broken bindings against clang 22, where `whisper_full_params` comes out with
size 1 and the build fails on a layout assertion.

Both model features are on by default and both can be turned off:
`--no-default-features` still builds a working app that captures and stores,
losing speech and semantic search but not the product.

### Why audio owns a thread

`cpal::Stream` is `!Send` on Windows — a WASAPI stream belongs to the thread
that created it. So the stream is built on a dedicated thread that then parks
forever, holding it alive; everything else shares the `Arc<RingBuffer>`, which
is `Send + Sync`. Dropping the stream stops capture silently, so that parked
thread is load-bearing, not idle.
