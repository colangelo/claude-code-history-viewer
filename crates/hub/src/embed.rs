//! In-process sentence embedder for semantic journal search.
//!
//! One embedder produces BOTH the stored entry vectors and the query-time
//! vectors — a cosine space is only meaningful within a single model, so the
//! model identifier travels with every stored embedding and mismatched rows
//! are ignored by queries and re-embedded by the sweep.
//!
//! The production backend is candle (pure Rust, CPU) running bge-small-en-v1.5
//! from a directory staged at deploy time (`embed_model_dir` /
//! `HUB_EMBED_MODEL_DIR`; weights are never in git or the binary). Spike
//! numbers (task 1.1, M-series CPU): model load ~190 ms, ~26 ms per embed,
//! +~6 MB binary. Load happens lazily on first use; ANY init failure parks the
//! embedder in a disabled state that only fails `embed()` calls — the hub
//! itself must keep serving (degradation is handled at the query layer).

use anyhow::{Context, Result};
use candle_core::{DType, Device, IndexOp, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use std::path::PathBuf;
use std::sync::OnceLock;

/// bge models are asymmetric: retrieval QUERIES carry this instruction prefix,
/// stored passages do not. Applied by [`query_text`]; entry embedding uses the
/// raw text.
pub const QUERY_PREFIX: &str = "Represent this sentence for searching relevant passages: ";

/// The text actually embedded for a search query.
pub fn query_text(q: &str) -> String {
    format!("{QUERY_PREFIX}{q}")
}

/// A sentence embedder. Implementations MUST be deterministic for a given
/// (model, text) and return L2-normalized vectors of exactly `dim()` floats,
/// so cosine similarity is a plain dot product.
pub trait Embedder: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    /// Identifier recorded on stored embeddings; vectors from different ids
    /// are never compared.
    fn model_id(&self) -> &str;
    fn dim(&self) -> usize;
}

/// candle-backed embedder loading a BERT-family model (bge-small-en-v1.5
/// layout: `config.json` + `tokenizer.json` + `model.safetensors`) from a
/// directory. CLS pooling + L2 normalization — bge's trained pooling mode.
pub struct CandleEmbedder {
    dir: PathBuf,
    model_id: String,
    dim: usize,
    loaded: OnceLock<Option<Loaded>>,
}

struct Loaded {
    model: BertModel,
    tokenizer: tokenizers::Tokenizer,
}

impl CandleEmbedder {
    /// `dir` is checked lazily on first `embed()`, not here: a missing or
    /// corrupt model directory must never take the hub down.
    pub fn new(dir: PathBuf) -> Self {
        Self {
            dir,
            model_id: "bge-small-en-v1.5".into(),
            dim: 384,
            loaded: OnceLock::new(),
        }
    }

    fn load(&self) -> Result<Loaded> {
        let dir = &self.dir;
        let device = Device::Cpu;
        let config: Config = serde_json::from_str(
            &std::fs::read_to_string(dir.join("config.json"))
                .with_context(|| format!("reading {}/config.json", dir.display()))?,
        )
        .context("parsing embed model config.json")?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| anyhow::anyhow!("loading {}/tokenizer.json: {e}", dir.display()))?;
        // Buffered (not mmaped) load: the workspace denies `unsafe`, and a
        // one-time ~130 MB read at first use is acceptable for a daemon.
        let weights = std::fs::read(dir.join("model.safetensors"))
            .with_context(|| format!("reading {}/model.safetensors", dir.display()))?;
        let vb = VarBuilder::from_buffered_safetensors(weights, DType::F32, &device)?;
        let model = BertModel::load(vb, &config).context("loading embed model weights")?;
        Ok(Loaded { model, tokenizer })
    }

    fn loaded(&self) -> Option<&Loaded> {
        self.loaded
            .get_or_init(|| match self.load() {
                Ok(l) => {
                    tracing::info!(dir = %self.dir.display(), model = %self.model_id,
                        "embed model loaded");
                    Some(l)
                }
                Err(e) => {
                    tracing::warn!(dir = %self.dir.display(), error = %format!("{e:#}"),
                        "embed model unavailable — semantic search degrades to keyword");
                    None
                }
            })
            .as_ref()
    }
}

impl Embedder for CandleEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let Some(loaded) = self.loaded() else {
            anyhow::bail!("embedder disabled (model failed to load)");
        };
        let enc = loaded
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow::anyhow!("tokenize: {e}"))?;
        let device = &loaded.model.device;
        let ids = Tensor::new(enc.get_ids(), device)?.unsqueeze(0)?;
        let type_ids = Tensor::new(enc.get_type_ids(), device)?.unsqueeze(0)?;
        let hidden = loaded.model.forward(&ids, &type_ids, None)?; // [1, seq, dim]
        let pooled = hidden.i((.., 0))?; // CLS token, [1, dim]
        let norm = pooled.sqr()?.sum_keepdim(1)?.sqrt()?;
        let normalized = pooled.broadcast_div(&norm)?;
        Ok(normalized.squeeze(0)?.to_vec1::<f32>()?)
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// Deterministic test embedder. Exact texts registered via [`StubEmbedder::with`]
/// get hand-picked vectors (letting tests contrive similarity orderings);
/// everything else hashes into a stable pseudo-vector. All outputs are
/// L2-normalized like the real backend.
pub struct StubEmbedder {
    model_id: String,
    dim: usize,
    fixed: std::collections::HashMap<String, Vec<f32>>,
}

impl StubEmbedder {
    pub fn new(model_id: &str, dim: usize) -> Self {
        Self {
            model_id: model_id.into(),
            dim,
            fixed: std::collections::HashMap::new(),
        }
    }

    /// Register an exact text → vector mapping (normalized on use).
    pub fn with(mut self, text: &str, vector: Vec<f32>) -> Self {
        self.fixed.insert(text.into(), vector);
        self
    }
}

fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for x in &mut v {
            *x /= norm;
        }
    }
    v
}

impl Embedder for StubEmbedder {
    fn embed(&self, text: &str) -> Result<Vec<f32>> {
        if let Some(v) = self.fixed.get(text) {
            anyhow::ensure!(v.len() == self.dim, "fixed vector has wrong dim");
            return Ok(normalize(v.clone()));
        }
        // Stable pseudo-embedding: bucket token hashes into the vector.
        use sha2::{Digest, Sha256};
        let mut v = vec![0.0f32; self.dim];
        for token in text.split_whitespace() {
            let h = Sha256::digest(token.as_bytes());
            let idx = usize::from(h[0]) % self.dim;
            v[idx] += 1.0;
        }
        Ok(normalize(v))
    }

    fn model_id(&self) -> &str {
        &self.model_id
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

/// Cosine similarity of two already-normalized vectors (dot product). Length
/// mismatch → `None` (callers treat it as a skipped candidate, not an error).
pub fn cosine(a: &[f32], b: &[f32]) -> Option<f32> {
    if a.len() != b.len() {
        return None;
    }
    Some(a.iter().zip(b).map(|(x, y)| x * y).sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stub_is_deterministic_and_normalized() {
        let s = StubEmbedder::new("stub", 8);
        let a = s.embed("hello world").unwrap();
        let b = s.embed("hello world").unwrap();
        assert_eq!(a, b);
        let norm: f32 = a.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-5);
    }

    #[test]
    fn stub_fixed_vectors_control_similarity() {
        let s = StubEmbedder::new("stub", 4)
            .with("query", vec![1.0, 0.0, 0.0, 0.0])
            .with("near", vec![0.9, 0.1, 0.0, 0.0])
            .with("far", vec![0.0, 0.0, 1.0, 0.0]);
        let q = s.embed("query").unwrap();
        let near = s.embed("near").unwrap();
        let far = s.embed("far").unwrap();
        assert!(cosine(&q, &near).unwrap() > cosine(&q, &far).unwrap());
    }

    #[test]
    fn cosine_rejects_dim_mismatch() {
        assert!(cosine(&[1.0, 0.0], &[1.0, 0.0, 0.0]).is_none());
    }

    #[test]
    fn candle_embedder_with_missing_dir_is_disabled_not_fatal() {
        let e = CandleEmbedder::new(PathBuf::from("/nonexistent/model/dir"));
        assert!(e.embed("anything").is_err());
        // Still answers metadata queries.
        assert_eq!(e.model_id(), "bge-small-en-v1.5");
        assert_eq!(e.dim(), 384);
    }
}
