# ADR-0005: Confidence is derived, never self-reported

**Status:** Accepted · **Date:** 2026-09-06 · **Refines:** spec section 17

## Context

Spec section 17 requires a confidence score on every action, with the goal:

> No unnecessary questions, no dangerous guesses.

The examples show the model emitting numbers (`intent confidence: 0.99`,
`collection confidence: 0.96`). Read literally, that means asking the LLM to
score itself.

## Problem

**Self-reported LLM confidence is badly calibrated.** Models emit round,
confident-sounding numbers largely independent of whether they are correct — and
a small quantized 0.6B router is the worst case for this. Gating destructive or
misfiling actions on a number the model invented gives the *appearance* of a
safety system with none of the substance.

There is also a subtler failure the spec's own example exposes:

```
"Put this with the house stuff."
  House/Internet     0.51
  House/Electricity  0.49
```

Both scores are moderate. What actually makes this ambiguous is not that the top
score is low — it is that **the top two are indistinguishable.** A pure
threshold on the top score would miss this, while flagging a confident lone
match at 0.55 that has no competitor.

## Decision

Confidence is computed by us, from measurable signals:

```
confidence = f(
    slot_token_logprobs,      // from the constrained decode, per slot
    retrieval_margin,         // score(top1) - score(top2)
    historical_accept_rate    // this intent, this user, from the correction log
)
```

- **Per-slot, not per-command.** Intent can be certain while the destination is
  not. Only the uncertain slot is queried, so the question stays narrow:
  "Internet or Electricity?" rather than "what did you mean?"
- **Margin is the primary destination signal.** Low margin means ask, regardless
  of absolute score.
- **Thresholds are calibrated from logged outcomes**, not hand-tuned constants,
  and are per-intent: `TAG` should act freely, `FILE_OPERATION` should not.

Two hard rules on top:

1. **Irreversible actions always confirm**, at any confidence. Deleting or
   overwriting files is never auto-executed (Principle 7).
2. **Everything else is undoable.** Every executed command writes an inverse
   operation, and the confirmation toast carries an undo affordance. Cheap undo
   is what lets thresholds be aggressive, which is what keeps the product fast.

## Consequences

**Good**

- The safety system is grounded in real measurements, so it can be evaluated:
  precision/recall of "should have asked" is computable from the correction log.
- Narrow, per-slot questions are far less annoying than restating the command.
- Undo-by-default means a wrong guess costs one keystroke, not lost data.

**Bad**

- Requires logprob access from the inference layer, so `LlmProvider` must expose
  them. Some cloud providers restrict this; those fall back to margin plus
  history only, and that limitation is recorded per provider.
- Every tool must implement an inverse operation. This is real work, and it is
  the price of being allowed to act without asking.

**Neutral**

- Cold-start uses conservative per-intent defaults and tightens as the
  correction log fills. Early users will be asked slightly more often.

## Status, 2026-09-08

All three signals are now measured rather than asserted:
`logprob` and `margin` come from the Tier 1 constrained decode, over the value
tokens only and among the tokens the grammar allowed; `prior` comes from the
correction log (ADR-0006) once an intent has five verdicts behind it.

**The thresholds are not switched on.** The first nine measurements — all of
them correct routings — put one answer below the 0.70 gate and a second at 0.72,
and showed that free-text slots have structurally narrower margins than a choice
among collections. Both findings argue against the constants and neither
supplies a replacement: correct answers say nothing about where wrong ones sit.
So `should_execute` stays unconsulted, the numbers keep accumulating, and the
gate is chosen when there is something to choose it from — which is what this
ADR asked for in the first place. The measurements are pinned in
`memos-core/src/intent.rs` so a change to the constants has to argue with them.
