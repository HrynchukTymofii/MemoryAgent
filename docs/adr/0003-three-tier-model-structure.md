# ADR-0003: Three-tier model structure with grammar-constrained routing

**Status:** Accepted · **Date:** 2026-09-06 · **Refines:** spec sections 20, 21

## Context

Spec section 20 defines an `LLMProvider` interface but does not say *which model
runs when*. Spec Principle 5 ("fast before clever") implies a policy but never
states one. Without that policy, the natural failure is calling one
general-purpose model for everything, which makes trivial commands slow and
hard commands unreliable.

## Decision

Route by **how much thinking the command actually needs**, not by capability.

### Tier 0 — deterministic, no model, ~5 ms

A hand-written grammar over the formulaic command shapes:

```
save this to <collection>       tag this <tag>
remind me <temporal>            open <ref>
find <query>                    move this to <collection>
```

Expected to absorb 40-60% of real traffic. This is the single largest latency
win available and it costs nothing per call. If Tier 0 matches with a clean
parse and the slot resolves unambiguously, no model is ever consulted.

### Tier 1 — local router, ~150-400 ms

Qwen3 0.6B/1.7B or Llama 3.2 1B via llama.cpp, producing
`{ intent, slots, ... }`.

Two properties are load-bearing:

1. **GBNF grammar-constrained decoding.** The grammar is regenerated whenever
   the collection set changes, and enumerates the valid intents and the user's
   actual collection paths as literal alternatives. The model therefore *cannot*
   emit invalid JSON, an unknown intent, or a nonexistent collection. This is
   what makes a sub-1B model safe to trust — the guarantee is structural, not
   prompt-based.
2. **Warm reused KV cache.** System prompt + collection list is ~800 tokens.
   Re-prefilling per command costs 100 ms+; a cached prefix costs ~0. The model
   is loaded at startup and stays resident for the process lifetime.

### Tier 2 — reasoning, seconds acceptable

Cloud (Claude/GPT) or a larger local model, for `RESEARCH`, `PLAN`, `EXPLAIN`,
`LEARN`, and disambiguation the router could not resolve. The user has switched
mental mode, so the latency expectation is completely different.

### Embeddings

`bge-small-en-v1.5` (384-dim) through **ONNX Runtime, not PyTorch**: ~30 MB
instead of ~800 MB, 5-15 ms per short text on CPU. Never on the critical path —
saves are acknowledged before embedding, and a background worker backfills.

## Consequences

**Good**

- Common commands never touch a model, so the product feels instant where it
  matters most.
- Grammar constraints eliminate the entire class of parse-failure bugs.
- Model choice is a config decision per hardware tier, not an architecture one.

**Bad**

- Three code paths for one logical operation, so all three must be tested
  against the same command corpus. Mitigated by a shared fixture set in
  `experiments/router/` replayed against every tier.
- The GBNF grammar must be regenerated on collection changes; a stale grammar
  silently blocks a valid destination. Mitigated by invalidating the grammar in
  the same transaction that mutates collections.

**Neutral**

- Tier 0 coverage is a measurable product metric. If it drops below ~40%, either
  users are phrasing differently than assumed or the grammar needs extending —
  in both cases the correction log (ADR-0006) reveals it.
