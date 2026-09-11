# ADR-0011: Cloud-first routing, and the grammar as the offline fallback

**Status:** Accepted · **Date:** 2026-09-11 · **Supersedes in part:** ADR-0003,
ADR-0008

## Context

ADR-0003 routed by how much thinking a command needs: a hand-written grammar
first, a 0.6B local model behind it, and a cloud tier for reasoning. Six real
commands later, the record says this did not work.

Two of six matched the grammar. The other four were logged `UNKNOWN`. Tier 2 was
never built, so `RESEARCH`, `EXPLAIN`, `PLAN`, `LEARN`, `REMINDER` and
`FILE_OPERATION` were enum variants that reached `"not implemented yet"`. And
Tier 1 had been down for three days behind a protocol handshake against a
sidecar binary seven minutes too old, with nothing on screen saying so.

The binary was a bug and is fixable. The other two findings are not bugs:

1. **The action space was the ceiling, not the model.** ADR-0003's structural
   guarantee — a GBNF grammar enumerating the user's collections as literal
   alternatives, so an unknown destination cannot be emitted — also made "file
   this under something new" unsayable. There was no `CREATE_COLLECTION` intent
   and no way to name a collection that did not exist. Replayed against the real
   transcript, the local router answered confidently and wrongly: it filed a
   page about public speaking into `Life/Finance`, because that was the closest
   thing the grammar let it say.
2. **One utterance was one action.** `Slots` is single-valued and every tier
   returned exactly one `RoutedCommand`. "Add this to a new collection for X" is
   two actions, and no model reading it could have expressed both.

## Decision

**Every command goes to a tool-calling model first. The grammar stays as the
offline fallback.**

- `memos-cloud` sends the transcript, the on-screen context and the collection
  list to Claude with one tool per executable intent, and executes the tool
  calls it gets back against the local store, feeding each receipt in until the
  model stops. A command is now a *plan*, and a plan can be more than one step.
- `CREATE_COLLECTION` joins the action space, so the destination can be made on
  the way to filing something into it.
- A collection argument is a plain string checked against the store, not a
  grammar alternative. An unknown one comes back as "No collection called X",
  which is information the model can act on rather than a sentence it cannot
  form.
- Tier 1 is gone: the sidecar, the GGUF, the protocol, and `router.rs`. A second
  routing model that is worse than the first one is not a fallback, it is a
  second thing to keep working.
- Tier 0 stays exactly as it is, and is consulted when the cloud tier says no —
  no key, no network, a refusal, a turn that ran long. It is the difference
  between an app that degrades on a plane and one that stops.

## Consequences

**Good**

- The action space is now the only ceiling, and raising it is adding a tool.
- One sentence can do two things.
- Nothing silently files a memory into the nearest wrong collection because the
  right one was unsayable.
- One routing path to keep working instead of three, two of which were unbuilt
  or unreachable.

**Bad**

- **Every non-formulaic command now pays a network round trip**, against a
  latency budget written for 5 ms and 600 ms. This is the deliberate trade and
  the one most likely to be revisited: the honest version of this decision is
  that correctness came first and the budget is now a cloud round trip plus
  whatever effort setting we are on.
- **It costs money per command**, on the user's own key, which the app now has
  to hold.
- **Context leaves the machine.** Bounded on purpose — the transcript, the
  window title, the URL and up to 400 characters of the selection. Not the page
  text, not the clipboard, and never the stored memories. That bound is a test
  (`the_page_and_the_clipboard_stay_here`), because it is a promise and not an
  implementation detail.
- **A local-first product now has a cloud dependency on its main path.** The
  grammar keeps the floor off the ground, but the floor is low: familiar
  phrasings only.

**Neutral**

- The correction log gains nothing and loses nothing: every step of a plan is
  logged as its own command at `tier = cloud`, so coverage is still measurable
  and undo still reverses one step at a time.
- `memos-llm` remains in the workspace, unreferenced by the app. Deleting it is
  a separate change, and keeping it costs a compile.
