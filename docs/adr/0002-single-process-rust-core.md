# ADR-0002: Single-process Rust core, no Python sidecar on the hot path

**Status:** Accepted · **Date:** 2026-09-06 · **Qualifies:** spec section 15

## Context

Spec section 15 puts LangGraph in the main command flow:

```
speech -> intent router -> context resolver -> tool selection -> execution
```

LangGraph is Python. Using it on the client means bundling a Python runtime as a
sidecar process and talking to it over localhost.

## Problem

The capture path has a ~1 second total budget (see `docs/architecture.md`
section 2). A Python sidecar spends that budget on things the user gets nothing
for:

- **Cold start.** A PyInstaller bundle with torch is 400 MB - 1 GB and takes
  2-5 s to become ready. An always-on tool cannot afford that on login.
- **IPC.** Every command pays HTTP framing plus JSON serialisation in both
  directions, for a call that could be a direct function call.
- **Operational fragility.** Two processes means orphaned children, port
  conflicts, restart supervision, and antivirus heuristics firing on a bundled
  interpreter.

Meanwhile LangGraph's actual strengths — durable state, branching, retries,
human-in-the-loop interrupts — are not what a `SAVE` command needs. A save is:
classify, resolve slots, write a row.

## Decision

The hot path (capture, STT, routing, tools, storage) is a **plain Rust state
machine in the Tauri process**. No sidecar, no localhost HTTP, no serialisation.

LangGraph is used where it earns its cost: **Tier 2 work in `cloud/`** —
research agent, multi-step planning, quiz generation. Those are multi-second,
branching, retry-heavy workflows where durable graph state is genuinely the
right tool.

## Consequences

**Good**

- The whole capture path is one call stack. Easy to profile, easy to reason
  about, no cross-process tail latency.
- One binary to sign, ship and auto-update.
- LangGraph is still built and still on the CV — in the tier where it fits.

**Bad**

- The Rust router is hand-written rather than declared as a graph. Mitigated by
  keeping it a small explicit state machine with the intent enum in
  `memos-core`, and by keeping the tool interface identical to the cloud tier's
  so behaviour can be compared across both.
- Rust iteration is slower than Python for agent experimentation. Mitigated by
  `experiments/router/` staying Python for offline evaluation on logged
  commands; only the winning design is ported.

**Neutral**

- The `Tool` trait in `memos-agent` mirrors the tool list in spec section 16, so
  client and cloud expose the same controlled action space.
