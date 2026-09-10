//! ONNX Runtime backend.

use std::path::Path;

use ort::session::Session;
use ort::value::Tensor;
use parking_lot::Mutex;
use tokenizers::Tokenizer;

use crate::model::{normalise, ModelSpec, Pooling, Role, BGE_SMALL_EN_V15};
use crate::{EmbedError, Embedder};

pub struct OnnxEmbedder {
    // ONNX sessions are not `Sync` for inference, and the background worker is
    // the only caller anyway. A mutex is the honest representation of that.
    session: Mutex<Session>,
    tokenizer: Tokenizer,
    spec: ModelSpec,
    /// Which of the model's three possible inputs it actually declares. Some
    /// exports omit `token_type_ids`, and feeding an input the graph does not
    /// have is a hard error at run time.
    wants_token_type_ids: bool,
}

impl OnnxEmbedder {
    /// Load `model.onnx` and `tokenizer.json` from a directory.
    pub fn load(dir: &Path) -> Result<Self, EmbedError> {
        let model = dir.join("model.onnx");
        let tok = dir.join("tokenizer.json");
        if !model.exists() || !tok.exists() {
            return Err(EmbedError::ModelMissing(dir.to_path_buf()));
        }

        // Before the first session: `ort` resolves the DLL lazily, so this has
        // to happen ahead of the builder, not at start-up somewhere else.
        //
        // `dir` is the *model* directory, so the data-directory candidate inside
        // this call cannot match; what it resolves here are the exe-relative and
        // development layouts. The application sets ORT_DYLIB_PATH from its real
        // data directory at start-up, and that takes precedence.
        crate::use_bundled_runtime(dir, None);

        // Refuse rather than let `ort` panic.
        //
        // With no library located, `ort` falls back to loading by bare name and
        // panics when that fails — and under the hardened runtime it always
        // fails, because a relative path is not allowed. The release profile
        // sets `panic = "abort"`, so that panic takes the whole application
        // down, on a background thread, for a feature this module's own
        // documentation promises is optional. An error keeps that promise:
        // search falls back to keywords and capture is untouched.
        match std::env::var("ORT_DYLIB_PATH") {
            Ok(p) if Path::new(&p).exists() => {}
            _ => {
                return Err(EmbedError::Load(format!(
                    "no ONNX Runtime library found (looked beside the executable \
                     and under the data directory). Fetch it with \
                     `scripts/fetch-models.sh onnxruntime`, then `install`. \
                     Model directory: {}",
                    dir.display()
                )))
            }
        }

        let started = std::time::Instant::now();
        // No explicit graph optimisation level: ONNX Runtime 1.20 rejects the
        // value ort passes for Level3, and the runtime's default is already
        // "all basic + extended" for a model this size. Not worth a version
        // constraint for a difference we cannot measure here.
        let session = Session::builder()
            .map_err(|e| EmbedError::Load(e.to_string()))?
            // Two threads, not all of them. This runs in the background while
            // the user keeps working; saturating the CPU to backfill vectors
            // would make the whole desktop stutter for no visible gain.
            .with_intra_threads(2)
            .map_err(|e| EmbedError::Load(e.to_string()))?
            .commit_from_file(&model)
            .map_err(|e| EmbedError::Load(e.to_string()))?;

        let wants_token_type_ids = session
            .inputs()
            .iter()
            .any(|i| i.name() == "token_type_ids");

        let mut tokenizer =
            Tokenizer::from_file(&tok).map_err(|e| EmbedError::Load(e.to_string()))?;
        // Pad to the longest item in the batch, not to the model maximum:
        // padding every short capture out to 512 tokens would multiply the work
        // by an order of magnitude for identical results.
        tokenizer.with_padding(Some(tokenizers::PaddingParams {
            strategy: tokenizers::PaddingStrategy::BatchLongest,
            ..Default::default()
        }));
        tokenizer
            .with_truncation(Some(tokenizers::TruncationParams {
                max_length: BGE_SMALL_EN_V15.max_tokens,
                ..Default::default()
            }))
            .map_err(|e| EmbedError::Load(e.to_string()))?;

        tracing::info!(
            model = %model.display(),
            load_ms = started.elapsed().as_millis() as u64,
            token_type_ids = wants_token_type_ids,
            "embedding model loaded"
        );

        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
            spec: BGE_SMALL_EN_V15,
            wants_token_type_ids,
        })
    }

    pub fn spec(&self) -> ModelSpec {
        self.spec
    }

    /// Embed with an explicit role, so a query gets the model's prefix and a
    /// stored document does not.
    pub fn embed_as(&self, texts: &[&str], role: Role) -> Result<Vec<Vec<f32>>, EmbedError> {
        if texts.is_empty() {
            return Ok(Vec::new());
        }
        let prepared: Vec<String> = texts
            .iter()
            .map(|t| self.spec.prepare(t, role).into_owned())
            .collect();

        let encodings = self
            .tokenizer
            .encode_batch(prepared, true)
            .map_err(|e| EmbedError::Run(e.to_string()))?;

        let batch = encodings.len();
        let seq = encodings.first().map(|e| e.len()).unwrap_or(0);
        if seq == 0 {
            return Ok(vec![vec![0.0; self.spec.dim]; batch]);
        }

        let mut ids = Vec::with_capacity(batch * seq);
        let mut mask = Vec::with_capacity(batch * seq);
        let mut types = Vec::with_capacity(batch * seq);
        for e in &encodings {
            ids.extend(e.get_ids().iter().map(|&x| x as i64));
            mask.extend(e.get_attention_mask().iter().map(|&x| x as i64));
            types.extend(e.get_type_ids().iter().map(|&x| x as i64));
        }

        // Raw shape-and-data tensors rather than ndarray: `ort` binds its own
        // ndarray version, and matching it would couple this crate to whichever
        // release ort happens to depend on. The tuple form has no such coupling.
        let shape = [batch, seq];
        let mk = |v: Vec<i64>| -> Result<Tensor<i64>, EmbedError> {
            Tensor::from_array((shape, v.into_boxed_slice()))
                .map_err(|e| EmbedError::Run(e.to_string()))
        };

        let mut inputs = vec![
            ("input_ids", mk(ids)?),
            ("attention_mask", mk(mask.clone())?),
        ];
        if self.wants_token_type_ids {
            inputs.push(("token_type_ids", mk(types)?));
        }

        let mut session = self.session.lock();
        let outputs = session
            .run(inputs)
            .map_err(|e| EmbedError::Run(e.to_string()))?;

        let (shape, data) = outputs[0]
            .try_extract_tensor::<f32>()
            .map_err(|e| EmbedError::Run(e.to_string()))?;
        // last_hidden_state: [batch, seq, hidden]
        if shape.len() != 3 {
            return Err(EmbedError::Run(format!(
                "expected a 3-D hidden state, got shape {shape:?}"
            )));
        }
        let hidden = shape[2] as usize;
        let seq_out = shape[1] as usize;

        let mut out = Vec::with_capacity(batch);
        for b in 0..batch {
            let mut v = vec![0.0f32; hidden];
            match self.spec.pooling {
                Pooling::Cls => {
                    let base = b * seq_out * hidden;
                    v.copy_from_slice(&data[base..base + hidden]);
                }
                Pooling::Mean => {
                    let mut n = 0.0f32;
                    for s in 0..seq_out {
                        // Skip padding, or short texts get their vector pulled
                        // toward whatever the pad token encodes.
                        if mask[b * seq + s] == 0 {
                            continue;
                        }
                        let base = (b * seq_out + s) * hidden;
                        for h in 0..hidden {
                            v[h] += data[base + h];
                        }
                        n += 1.0;
                    }
                    if n > 0.0 {
                        for x in v.iter_mut() {
                            *x /= n;
                        }
                    }
                }
            }
            normalise(&mut v);
            out.push(v);
        }
        Ok(out)
    }
}

impl Embedder for OnnxEmbedder {
    fn embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.embed_as(texts, Role::Document)
    }

    fn dim(&self) -> usize {
        self.spec.dim
    }

    fn model_id(&self) -> &str {
        self.spec.id
    }
}
