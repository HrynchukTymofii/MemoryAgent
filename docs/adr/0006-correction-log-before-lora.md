# ADR-0006: The correction log is the asset; LoRA is deferred

**Status:** Accepted · **Date:** 2026-09-06 · **Refines:** spec section 21

## Context

Spec section 21 proposes a personal routing model trained on the user's own
command history:

```
personal command dataset -> fine-tuning / LoRA -> small local model
```

The direction is right. The sequencing is not.

## Problem

LoRA needs thousands of examples to beat a good prompt. A new user has zero. If
personalization only arrives after fine-tuning, then:

- the product feels generic for the entire period that matters most for
  retention — the first weeks;
- retraining is a batch job, so a correction made today does not change
  behaviour until the next training run. Users expect a correction to stick
  *immediately*, and a system that repeats a mistake it was just corrected on
  reads as broken;
- we would be shipping a training pipeline before knowing whether the routing
  errors are even model errors. Many will be grammar gaps or missing
  collections, which no amount of fine-tuning fixes.

## Decision

**The correction log is a day-one, first-class table.** Every command writes:

```
(transcript, context_snapshot, tier_used, prediction, confidence,
 user_correction, accepted, latency_ms, timestamp)
```

Personalization then proceeds in phases:

**Phase 1 — kNN few-shot (ships in V1).** Embed the incoming transcript, retrieve
the k most similar past commands and their *corrected* outcomes, inject them as
few-shot examples in the router prompt. Properties that matter:

- works from roughly the tenth example;
- a correction takes effect on the very next command, with no training;
- fully inspectable — "I did this because you did that last Tuesday" is
  explainable, which builds trust in a way weights never can.

**Phase 2 — LoRA (later).** Once the log holds thousands of corrections *and*
Phase 1 is demonstrably the bottleneck, train an adapter offline in
`experiments/lora/` and ship it as a downloadable per-user artefact.

The log is designed now precisely so Phase 2 needs no schema migration.

## Consequences

**Good**

- Personalization from week one instead of month six.
- Instant correction feedback, which is what users actually expect.
- The log doubles as the evaluation set for router changes, the calibration set
  for ADR-0005 thresholds, and the coverage metric for Tier 0.
- If Phase 1 turns out to be sufficient, we never pay for a training pipeline —
  and we will have learned that from data rather than assumption.

**Bad**

- Retrieval adds ~10-20 ms per Tier 1 command (embed + kNN over a small local
  index). Acceptable within the budget, and skipped entirely on Tier 0.
- The log is highly sensitive: it is a record of what the user said and did.
  It stays local by default, is excluded from sync unless explicitly enabled,
  and must be exportable and deletable from Settings.

**Neutral**

- LoRA remains a real project milestone for spec section 40, just gated on
  evidence rather than scheduled up front.
