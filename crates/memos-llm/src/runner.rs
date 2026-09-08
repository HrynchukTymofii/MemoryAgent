//! The local router, running.
//!
//! One thread owns the model and its context for the process lifetime and
//! answers routing requests over a channel. That is not an arbitrary choice:
//! `LlamaContext` borrows the model it came from, so a struct holding both is
//! self-referential and needs `unsafe` or a crate to express. A thread that owns
//! both and hands out results instead of references has neither problem, and it
//! matches how the speech and embedding workers already work.
//!
//! ## Why the cache is the whole design
//!
//! ADR-0003 budgets 150-400 ms for a route. The system prompt is prefilled
//! **once**, at startup, and every command decodes on top of that prefix — the
//! KV cache beyond it is dropped after each command rather than the whole
//! context being rebuilt. Re-prefilling per command would cost more than the
//! decode does.

use std::path::PathBuf;
use std::sync::mpsc::{channel, sync_channel, Sender, SyncSender};
use std::sync::Arc;
use std::time::Instant;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use llama_cpp_2::token::LlamaToken;
use parking_lot::RwLock;

use crate::{grammar, Decode, LlmError, ModelState, ValueScanner, ROUTE_TIMEOUT};

/// Longest JSON the router may produce, in tokens.
///
/// The grammar already bounds the text slots; this bounds everything else. A
/// routing decision that has not finished in this many tokens is not going to.
const MAX_TOKENS: i32 = 96;

/// Context window. The prefix is the system instruction plus every collection
/// path plus the examples, so it grows with the user's library — the headroom
/// is for someone with a hundred collections, and `prefill` refuses loudly
/// rather than letting an over-long prefix fail later on a command.
const CONTEXT_TOKENS: u32 = 2048;

enum Job {
    Route {
        transcript: String,
        reply: SyncSender<Result<(String, Decode), LlmError>>,
    },
    /// The collection set changed, so the grammar that constrains destinations
    /// is stale. ADR-0003 calls a stale grammar out specifically: it silently
    /// blocks a valid destination rather than failing.
    Regrammar(Vec<String>),
}

pub struct Router {
    tx: RwLock<Option<Sender<Job>>>,
    state: RwLock<ModelState>,
    detail: RwLock<String>,
}

impl Router {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tx: RwLock::new(None),
            state: RwLock::new(ModelState::Loading),
            detail: RwLock::new(String::new()),
        })
    }

    pub fn state(&self) -> ModelState {
        *self.state.read()
    }

    pub fn detail(&self) -> String {
        self.detail.read().clone()
    }

    /// Load the model off-thread and start answering.
    ///
    /// Returns immediately. Loading costs seconds, and the first capture must
    /// not be the thing that waits for it — Tier 0 answers most commands
    /// without this model, and the rest degrade to "not understood" until it is
    /// ready, which is what they did before it existed at all.
    pub fn start(self: &Arc<Self>, model: PathBuf, collections: Vec<String>) {
        let me = self.clone();
        let (tx, rx) = channel::<Job>();
        *self.tx.write() = Some(tx);

        std::thread::Builder::new()
            .name("router".into())
            .spawn(move || {
                let backend = match LlamaBackend::init() {
                    Ok(b) => b,
                    Err(e) => return me.failed(format!("llama backend: {e}")),
                };
                // CPU only. A router that competes with the user's own GPU work
                // for the sake of 50 ms is a bad trade on a machine that is
                // also running whatever they were doing when they spoke.
                let params = LlamaModelParams::default().with_n_gpu_layers(0);
                let started = Instant::now();
                let model = match LlamaModel::load_from_file(&backend, &model, &params) {
                    Ok(m) => m,
                    Err(e) => return me.failed(format!("{e}")),
                };

                // Four, measured rather than assumed: eight threads came out at
                // 614 ms a route against 596 ms for four, on a 16-core machine.
                // This decode is a dozen tokens through a 0.6B model — latency
                // bound, not throughput bound — so more threads buy nothing and
                // take cores from whatever the user was actually doing.
                let ctx_params = LlamaContextParams::default()
                    .with_n_ctx(std::num::NonZeroU32::new(CONTEXT_TOKENS))
                    .with_n_threads(4);
                let mut ctx = match model.new_context(&backend, ctx_params) {
                    Ok(c) => c,
                    Err(e) => return me.failed(format!("context: {e}")),
                };

                let mut batch = LlamaBatch::new(CONTEXT_TOKENS as usize, 1);
                let mut prefix_len = match prefill(&model, &mut ctx, &mut batch, &collections) {
                    Ok(n) => n,
                    Err(e) => return me.failed(e),
                };
                let mut grammar_text = grammar::build(&collections);
                *me.state.write() = ModelState::Ready;
                *me.detail.write() = format!(
                    "{} tokens prefilled, loaded in {} ms",
                    prefix_len,
                    started.elapsed().as_millis()
                );
                tracing::info!(
                    load_ms = started.elapsed().as_millis() as u64,
                    prefix_len,
                    collections = collections.len(),
                    "router ready"
                );

                for job in rx {
                    match job {
                        Job::Regrammar(paths) => {
                            grammar_text = grammar::build(&paths);
                            // The collection list is in the cached prefix too,
                            // so a new collection means re-prefilling — a
                            // grammar that allows a destination the prompt has
                            // never mentioned is half an update.
                            match prefill(&model, &mut ctx, &mut batch, &paths) {
                                Ok(n) => prefix_len = n,
                                Err(e) => tracing::error!(e, "could not re-prefill the router"),
                            }
                            tracing::debug!(collections = paths.len(), "router prefix rebuilt");
                        }
                        Job::Route { transcript, reply } => {
                            let result = route_once(
                                &model,
                                &mut ctx,
                                &mut batch,
                                &grammar_text,
                                prefix_len,
                                &transcript,
                            );
                            // A caller that has already given up is not an
                            // error: the overlay moved on, and so should this.
                            let _ = reply.send(result);
                        }
                    }
                }
            })
            .expect("spawn router thread");
    }

    fn failed(&self, why: String) {
        tracing::error!(why, "router unavailable");
        *self.state.write() = ModelState::Failed;
        *self.detail.write() = why;
    }

    /// Rebuild the grammar because the collection set changed.
    pub fn collections_changed(&self, collections: Vec<String>) {
        if let Some(tx) = self.tx.read().as_ref() {
            let _ = tx.send(Job::Regrammar(collections));
        }
    }

    /// Route one transcript, or give up.
    ///
    /// `None` means the router could not answer in time, was never loaded, or
    /// produced something the grammar should have prevented. Every one of those
    /// is the same thing to the caller: Tier 1 did not resolve this, so it
    /// stays unrecognised — which is exactly what happened before Tier 1
    /// existed, rather than a new failure mode.
    pub fn route(&self, transcript: &str) -> Option<(String, Decode)> {
        if self.state() != ModelState::Ready {
            return None;
        }
        let (reply, answers) = sync_channel(1);
        {
            let tx = self.tx.read();
            tx.as_ref()?
                .send(Job::Route {
                    transcript: transcript.to_string(),
                    reply,
                })
                .ok()?;
        }
        match answers.recv_timeout(ROUTE_TIMEOUT) {
            Ok(Ok(answer)) => Some(answer),
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "router declined");
                None
            }
            Err(_) => {
                tracing::warn!("router did not answer within {ROUTE_TIMEOUT:?}");
                None
            }
        }
    }
}

/// Prefill the part of the prompt that does not change per command.
///
/// The system instruction, the user's collection list and a handful of worked
/// examples — several hundred tokens that would otherwise be re-read on every
/// utterance. Returns how many tokens the prefix occupies, which is the
/// boundary every later command is trimmed back to.
fn prefill(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    batch: &mut LlamaBatch,
    collections: &[String],
) -> Result<i32, String> {
    let text = format!(
        "<|im_start|>system\n{}<|im_end|>\n",
        crate::prefix(collections)
    );
    let tokens = model
        .str_to_token(&text, AddBos::Always)
        .map_err(|e| format!("tokenise: {e}"))?;

    // Leave room for the command and its answer. A prefix that fills the
    // context does not fail here — it fails later, on a command, looking like
    // the model refusing to route.
    if tokens.len() + MAX_TOKENS as usize + 128 > CONTEXT_TOKENS as usize {
        return Err(format!(
            "prefix is {} tokens, too long for a {CONTEXT_TOKENS}-token context",
            tokens.len()
        ));
    }

    ctx.clear_kv_cache();
    batch.clear();
    for (i, token) in tokens.iter().enumerate() {
        let last = i == tokens.len() - 1;
        batch
            .add(*token, i as i32, &[0], last)
            .map_err(|e| format!("batch: {e}"))?;
    }
    ctx.decode(batch).map_err(|e| format!("prefill: {e}"))?;
    Ok(tokens.len() as i32)
}

/// One constrained decode, on top of the warm prefix.
fn route_once(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    batch: &mut LlamaBatch,
    grammar_text: &str,
    prefix_len: i32,
    transcript: &str,
) -> Result<(String, Decode), LlmError> {
    let started = Instant::now();

    // Drop whatever the previous command left behind, keeping the prefilled
    // system prompt. This is the difference between ~150 ms and ~400 ms.
    ctx.kv_cache_seq_rm(0, Some(prefix_len as u32), None)
        .map_err(|e| LlmError::Run(format!("could not reset the cache: {e}")))?;

    let turn = format!(
        "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n",
        crate::user_turn(transcript)
    );
    // No BOS: the prefix already carries it, and a second one mid-sequence is
    // a token the model has never seen there during training.
    let tokens = model
        .str_to_token(&turn, AddBos::Never)
        .map_err(|e| LlmError::Run(format!("tokenise: {e}")))?;

    batch.clear();
    let mut pos = prefix_len;
    for (i, token) in tokens.iter().enumerate() {
        let last = i == tokens.len() - 1;
        batch
            .add(*token, pos, &[0], last)
            .map_err(|e| LlmError::Run(format!("batch: {e}")))?;
        pos += 1;
    }
    ctx.decode(batch)
        .map_err(|e| LlmError::Run(format!("decode: {e}")))?;

    // The grammar does the work; greedy just takes what it leaves. Sampling
    // creatively among tokens the grammar permits would only ever make the
    // routing decision less predictable, never better.
    let mut sampler = LlamaSampler::chain_simple([
        LlamaSampler::grammar(model, grammar_text, "root")
            .map_err(|e| LlmError::Run(format!("grammar rejected: {e}")))?,
        LlamaSampler::greedy(),
    ]);

    // Bytes, not strings, until the end. One token can be half a multi-byte
    // character — a collection name in Cyrillic or an em dash in a title — and
    // decoding per token would turn that into a replacement character or an
    // error, in exactly the text the user cares most about.
    let mut bytes: Vec<u8> = Vec::new();
    // What the decode is measured on. Only the tokens inside a value are worth
    // measuring (ADR-0005, and `ValueScanner`), and the expensive path is taken
    // only for those — over a 151k vocabulary that is the difference between
    // measuring four tokens and measuring twenty.
    let mut scanner = ValueScanner::default();
    let mut probabilities: Vec<f32> = Vec::new();
    let mut narrowest = 1.0_f32;

    for _ in 0..MAX_TOKENS {
        let idx = batch.n_tokens() - 1;

        // Two ways to take one token, and they have to stay equivalent:
        // llama.cpp `sample` is apply-select-accept, so the manual path does
        // exactly that and no more. Accepting twice is what advanced the
        // grammar past the end of its own state machine during development and
        // aborted the process on an assertion.
        let (token, measured) = if scanner.in_value() {
            let mut candidates = LlamaTokenDataArray::from_iter(ctx.candidates_ith(idx), false);
            sampler.apply(&mut candidates);
            let token = candidates
                .selected_token()
                .ok_or_else(|| LlmError::Run("the sampler selected nothing".into()))?;
            let stats = measure_choice(&candidates, token);
            sampler.accept(token);
            (token, stats)
        } else {
            // `sample` accepts the token itself — its own documentation says
            // so, and accepting again here would be the double-accept above.
            (sampler.sample(ctx, idx), None)
        };

        if model.is_eog_token(token) {
            break;
        }
        let piece = model
            .token_to_piece_bytes(token, 16, false, None)
            .map_err(|e| LlmError::Run(format!("detokenise: {e}")))?;

        // Decided after the fact, not before: a token that *starts* inside a
        // value can still be the quote that closes it, and the grammar had no
        // choice about that one.
        if scanner.push(&piece) > 0 {
            if let Some((probability, gap)) = measured {
                probabilities.push(probability);
                narrowest = narrowest.min(gap);
            }
        }
        bytes.extend(piece);

        batch.clear();
        batch
            .add(token, pos, &[0], true)
            .map_err(|e| LlmError::Run(format!("batch: {e}")))?;
        pos += 1;
        ctx.decode(batch)
            .map_err(|e| LlmError::Run(format!("decode: {e}")))?;
    }

    let decode = if probabilities.is_empty() {
        Decode::UNMEASURED
    } else {
        Decode {
            logprob: probabilities.iter().sum::<f32>() / probabilities.len() as f32,
            margin: narrowest,
        }
    };

    let json = String::from_utf8(bytes)
        .map_err(|e| LlmError::Run(format!("router emitted invalid UTF-8: {e}")))?;
    tracing::debug!(
        took_ms = started.elapsed().as_millis() as u64,
        json = %json,
        logprob = decode.logprob,
        margin = decode.margin,
        "routed"
    );
    Ok((json, decode))
}

/// How sure the model was about one token, among the ones it was allowed.
///
/// Returns the probability of the chosen token and the gap to the runner-up,
/// both over the **grammar-filtered** distribution. That qualifier is the whole
/// point: a raw softmax over 151k tokens would mostly measure how badly the
/// model wanted to say something the grammar forbids, which says nothing about
/// whether it picked the right collection out of the ones that exist.
///
/// `None` when the grammar left one legal token, or none. Nothing was chosen,
/// so there is nothing to be confident about, and folding a structurally forced
/// 1.0 into the average would report certainty the model never expressed.
fn measure_choice(candidates: &LlamaTokenDataArray, chosen: LlamaToken) -> Option<(f32, f32)> {
    // Softmax over the legal tokens only. The grammar sets the rest to -inf,
    // so they drop out of the sum without having to be filtered first.
    let max = candidates
        .data
        .iter()
        .map(|d| d.logit())
        .fold(f32::NEG_INFINITY, f32::max);
    if !max.is_finite() {
        return None;
    }

    let mut total = 0.0_f32;
    let mut chosen_weight = 0.0_f32;
    let mut best = 0.0_f32;
    let mut second = 0.0_f32;
    let mut legal = 0_u32;

    for d in &candidates.data {
        let logit = d.logit();
        if !logit.is_finite() {
            continue;
        }
        legal += 1;
        let w = (logit - max).exp();
        total += w;
        if d.id() == chosen {
            chosen_weight = w;
        }
        if w > best {
            second = best;
            best = w;
        } else if w > second {
            second = w;
        }
    }

    if legal < 2 || total <= 0.0 {
        return None;
    }
    Some((chosen_weight / total, (best - second) / total))
}
