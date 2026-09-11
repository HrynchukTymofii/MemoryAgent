# Personal Memory OS — Architecture

> Derived from `personal_memory_os_spec.md`. Where this document and the spec
> disagree, this document wins and an ADR in `docs/adr/` explains why.

## 1. The one requirement everything else serves

```
hotkey -> speak -> done
```

Everything below is downstream of a single number: **the user must see the
result within ~1 second of finishing speaking.** Any design that endangers that
budget is wrong, regardless of how good it looks on paper.

## 2. Latency budget

This is a contract, not an aspiration. Each stage has an owner and a ceiling.

| # | Stage | Ceiling | Owned by | How it is met |
|---|-------|---------|----------|---------------|
| 1 | hotkey -> overlay painted | 50 ms | `src-tauri` | Overlay window created hidden at boot; show = 1 call |
| 2 | hotkey -> first audio sample | ~0 ms | `memos-stt` | Audio stream **always open** into a ring buffer |
| 3 | speech -> partial transcript | streaming | `memos-stt` | whisper.cpp on rolling windows while user still talks |
| 4 | key release -> final transcript | 300 ms | `memos-stt` | Only the ~300 ms tail remains unprocessed |
| 5 | transcript -> intent JSON | 5-400 ms | `memos-agent` | Tier 0 fast path, else Tier 1 with warm KV cache |
| 6 | intent -> executed + acked | 20 ms | `memos-db` | Single SQLite tx; embedding deferred |
| 7 | embedding + index | async | `memos-embed` | Off the critical path entirely |

Stage 2 is the one people get wrong. Opening a WASAPI capture device costs
100-300 ms. If that happens on hotkey press, the first word is lost and the
product feels broken. **The microphone stream is opened at application start and
never closed**; the hotkey only marks a read offset in the ring buffer.

Stage 7 matters just as much: the user is told "Saved" **before** the embedding
exists. Embeddings are computed by a background worker and backfilled. A save is
durable the moment the SQLite transaction commits.

## 3. Process and storage topology

```
+--------------------------- Tauri app (single process) ----------------------+
|                                                                             |
|  WebView2 (React/TS)          Rust core                                     |
|  +-------------------+        +------------------------------------------+  |
|  | overlay (pre-warm)|<--IPC--| memos-context   Win32 UIA / foreground   |  |
|  | main window       |        | memos-stt       ring buffer + VAD + STT  |  |
|  | toast / undo      |        | memos-agent     router - tools - confid. |  |
|  +-------------------+        | memos-llm       llama.cpp | cloud        |  |
|                               | memos-embed     ONNX Runtime             |  |
|                               | memos-retrieval hybrid FTS5 + vec + RRF  |  |
|                               | memos-db        SQLite (source of truth) |  |
|                               | memos-license   entitlements             |  |
|                               +------------------------------------------+  |
+-----------------------------------------------------------------------------+
                                        | optional, paid tier only
                                        v
        +------------------------------------------------------+
        | Cloud (FastAPI + LangGraph + Postgres/pgvector + Redis)|
        | sync - Tier-2 reasoning - research agent - billing     |
        +------------------------------------------------------+
```

**One process on the hot path.** No localhost HTTP, no IPC serialisation between
the hotkey and the database. A capture is a direct function call from the hotkey
handler to SQLite.

The one exception is Tier 1, which runs as a `memos-router` child process spoken
to over a pipe — because llama.cpp `abort()`s on a failed assertion, and a
routing failure must not be able to take the tray, the hotkey and the capture
loop with it. Tier 1 is never on the hot path: it is only reached once Tier 0
has already failed. See ADR-0008.

**SQLite is the source of truth on the client.** Postgres + pgvector exists, but
server-side, for the sync tier — see ADR-0001.

## 4. The model structure

Three tiers, chosen by *how much thinking the command actually needs*.

### Tier 0 — no model (target: 40-60% of commands, ~5 ms)

Real voice commands are overwhelmingly formulaic. A grammar over the common
shapes handles them with zero inference:

```
save this to <collection>
remind me <temporal>
find <query>
open <ref>
tag this <tag>
```

Spending 400 ms of GPU on `save this to React` is waste. Tier 0 is the single
largest latency win available, and it is deterministic, testable and free.

### Tier 1 — local router (~150-400 ms)

Qwen3 0.6B / 1.7B (or Llama 3.2 1B) via llama.cpp, emitting:

```json
{ "intent": "SAVE", "slots": { "collection": "Study/Programming/React" } }
```

Two non-negotiables:

1. **GBNF grammar-constrained decoding.** The grammar is generated from the live
   intent enum and the user's actual collection list. A 0.6B model *structurally
   cannot* emit malformed JSON, a non-existent intent, or an unknown collection.
   This is what makes a tiny model trustworthy — not prompt engineering.
2. **Warm, reused KV cache.** The system prompt plus collection list is ~800
   tokens. Re-prefilling it per command costs 100 ms+; caching the prefix costs
   ~0. The model loads once at startup and stays resident.

### Tier 2 — reasoning (seconds are fine)

Cloud (Claude / GPT) or a larger local model. Handles `RESEARCH`, `PLAN`,
`EXPLAIN`, `LEARN`, and low-confidence disambiguation. The user has switched
mental mode by this point, so latency expectations are completely different.

### Embeddings

`bge-small-en-v1.5`, 384-dim, via **ONNX Runtime — not PyTorch**. ~30 MB instead
of ~800 MB, 5-15 ms per short text on CPU. Always off the critical path.

## 5. Retrieval (spec section 11)

Hybrid, fused in **one SQL query, in-process**:

```
FTS5 (BM25)  --+
               +-- Reciprocal Rank Fusion --> signal re-rank --> results
sqlite-vec   --+
```

Re-rank signals, per spec 11.6: recency, collection match, source match,
relationship distance, and access frequency. Vector search is *one* input, never
the whole system (spec Principle 6).

## 6. Confidence (spec section 17)

**Never ask the model for its own confidence** — self-reported scores are badly
calibrated. Confidence is derived:

```
confidence = f(
    slot_token_logprobs,      // from the constrained decode
    retrieval_margin,         // score(top1) - score(top2)
    historical_accept_rate    // for this intent, this user
)
```

`retrieval_margin` is the important one. `House/Internet` at 0.51 vs
`House/Electricity` at 0.49 is not a low-score problem, it is a *low-margin*
problem — and margin is precisely what should trigger a question.

Thresholds: execute silently above, ask below, and **every action is undoable**
regardless (spec Principle 7).

## 7. Personalization (spec section 21)

The asset is the **correction log**, not the adapter. From day one, every command
writes:

```
(transcript, context, prediction, confidence, user_correction, accepted)
```

- **Phase 1 — kNN few-shot.** Retrieve the k most similar past commands and
  inject them as examples. Works from the 10th example, updates instantly, no
  training. This beats fine-tuning at low data volume.
- **Phase 2 — LoRA.** Only once there are thousands of logged corrections.

The log is designed now; the training is deferred.

## 8. Hardware tiers

The dev machine is not the customer's machine. Benchmarked once at first run and
stored in a profile.

| Tier | Detection | STT | Router |
|------|-----------|-----|--------|
| A | CUDA/Vulkan GPU >= 6 GB | small.en or large-v3-turbo, GPU | Qwen3 1.7B Q4, GPU |
| B | >= 8 cores, AVX2, no dGPU | base.en, CPU | Qwen3 0.6B Q4, CPU + heavy Tier 0 |
| C | weak / old | tiny.en | Tier 0 only; cloud offered |

**Tier B is the development default**, so local testing reflects the median
customer rather than the developer's hardware.

## 9. Crate layout and dependency direction

Acyclic, `memos-core` at the root, no crate depends on the Tauri app.

```
memos-core        domain types, intents, errors - no I/O
  |-- memos-db          SQLite, migrations, repositories
  |-- memos-embed       EmbeddingProvider trait + ONNX impl
  |-- memos-stt         ring buffer, VAD, whisper
  |-- memos-llm         LlmProvider trait, llama.cpp + cloud, GBNF
  |-- memos-context     OS context capture (trait + per-OS impl)
  |-- memos-license     entitlements, Ed25519 offline tokens
  \-- memos-retrieval   hybrid search (needs db + embed)
        \-- memos-agent   router, tools, confidence, execution
              \-- apps/desktop/src-tauri
```

`memos-context` is the **only** crate that needs a macOS-specific
implementation — behind a trait it already defines. That is what makes the
Windows -> macOS move cheap.

## 9b. What a collection holds (ADR-0010)

```
collection with children   the way to a subject. Shows what is inside it.
collection without         a subject. Keeps one Markdown page.
  |-- note                 what a person reads and edits
  \-- knowledge_items      every capture, append-only. History and provenance.
```

A capture filed into a subject is integrated into that subject's page: its
title becomes a heading, matched against the headings already there, and the
text is appended at the end of that section — never rewriting a line already
in the file. The capture keeps its own row as the record. Search and undo run
on the rows; the person reads the page. `notes::integrate` is the whole merge
rule and is pure, which is what lets Tier 2 replace it later without touching
the capture path.

## 10. Deliberate deviations from the spec

| Spec | Decision | ADR |
|------|----------|-----|
| 13: Postgres + pgvector on client | SQLite + FTS5 + sqlite-vec on client; Postgres server-side | 0001 |
| 14: Redis on client | SQLite `jobs` table + Tokio + Tauri events | 0001 |
| 15: LangGraph in the hot path | Rust state machine on hot path; LangGraph for Tier 2 in cloud | 0002 |
| 19: Fish Speech local TTS | Dropped; OS built-in TTS behind a trait | 0008 |
| 17: model-reported confidence | Derived from logprobs + retrieval margin | 0005 |
| 21: LoRA as the personalization plan | Correction log + kNN first, LoRA later | 0006 |
| 9: a collection is a list of items | A subject keeps one page; items are its history | 0010 |
