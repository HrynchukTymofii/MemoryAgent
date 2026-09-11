# ADR-0008: The Tier 1 router runs in a sidecar process

**Status:** Superseded by ADR-0011 · **Date:** 2026-09-08 · **Qualifies:** ADR-0002 · **Refines:** ADR-0003

## Context

ADR-0002 put the whole hot path in one process and rejected a sidecar. ADR-0003
put a local llama.cpp router at Tier 1. Building Tier 1 made those two decisions
collide.

## Problem

Two things, discovered in that order. Only the second one changed the decision.

**It does not link.** whisper.cpp and llama.cpp each vendor their own copy of
ggml, so an executable containing both defines every ggml symbol twice:

```
libllama_cpp_sys_2(ggml.c.obj): ggml_abs already defined in
libwhisper_rs_sys(ggml.c.obj)
```

`llama-cpp-sys-2` offers `dynamic-link` and `system-ggml` to get around this,
but both mean shipping a prebuilt llama.cpp matched to the exact revision these
bindings were generated from — a version-pinning problem in the build, traded
for a version-pinning problem at runtime.

**It aborts.** llama.cpp calls `abort()` on a failed assertion. One was hit
during development: the sampler was fed each token twice, which advanced the
grammar twice per token and walked it into a state where no token was legal.
llama.cpp did not return an error — it killed the process.

That is the part that matters. In one process, a router assertion takes the
tray, the global hotkey, the capture loop and the database with it. The user
loses the entire product because an unusual phrasing walked a 0.6B model into a
grammar corner. And the tier this happens in is the *optional* one: everything
Tier 0 handles was working fine a microsecond earlier.

A workaround for the linker would have left that intact.

## Decision

Tier 1 runs as **`memos-router`, a separate binary**, spawned by the app and
spoken to over newline-delimited JSON on stdin/stdout.

- `crates/memos-llm` keeps both halves. The library — grammar, prompt, parsing,
  protocol types — has no native dependency and is what the app links. The
  `local` feature adds llama.cpp and builds the `memos-router` binary. The app
  therefore needs no native toolchain at all, and the escalation path compiles
  unconditionally rather than behind a feature flag.
- **The app supervises.** A router that exits is restarted, up to three times
  per healthy run; the budget resets when one comes up `ready`, so three crashes
  spread over a week do not retire the tier, while a model that cannot load
  stops being retried. In-flight requests are failed immediately rather than
  waiting out their deadline.
- **The pipe is the lifetime.** The sidecar exits when stdin closes. That is the
  operating system closing a handle, not code remembering to clean up, so a
  crashed or force-killed app cannot leave a 600 MB model resident.
- **Stdout is protocol, stderr is logs.** Every request carries an id and every
  reply carries it back, so a reply arriving after its caller timed out is
  discarded rather than attributed to the next command.

## Consequences

**Good**

- A router crash costs the router. `route` returns `None`, the command comes
  back "not understood", a fresh process starts — which is exactly the
  degradation the app already handles for a machine with no model downloaded.
- The app links no ML native code beyond whisper and ONNX, and the router's
  610 MB model is not in the app's address space.
- The router can be driven by hand — `echo` a line into it — which is how it was
  verified before the app ever called it.

**Bad**

- Two artefacts to build, sign and ship instead of one. Mitigated by
  `scripts\build-router.ps1` and by the app finding the binary beside its own,
  which is where both `cargo build` and the installer put it.
- IPC cost per route. Measured at noise: a route is ~450-760 ms of decode, and
  the framing either side of it is microseconds. This is the trade ADR-0002
  refused for the *hot* path, and the reason it is acceptable here is that Tier 1
  is not the hot path — it is only reached when Tier 0 has already failed.
- Startup is asynchronous in a second way: the app now waits for a `ready`
  message rather than reading a shared state. In practice the same ~1.3 s.

**Neutral**

- ADR-0002 still holds where it was aimed. Capture, STT, routing at Tier 0,
  tools and storage remain one call stack in one process. What moved out is a
  model that ADR-0003 always described as optional, into the failure mode that
  description implies.
