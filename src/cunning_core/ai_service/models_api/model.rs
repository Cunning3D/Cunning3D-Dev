//! Trait ModelBackend - ported from Oxide-Lab

use candle_core::Tensor;

/// A unified trait that all model backends must implement
pub trait ModelBackend: Send {
    /// Forward pass - returns logits [batch_size, seq_len, vocab_size]
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor>;

    /// Clear the KV cache
    fn clear_kv_cache(&mut self);

    /// Model type (e.g. "qwen3", "llama")
    fn model_type(&self) -> &str;

    /// Vocabulary size
    fn vocab_size(&self) -> usize;

    /// Maximum sequence length
    fn max_seq_len(&self) -> usize {
        4096
    }

    /// Whether Flash Attention is supported
    fn supports_flash_attn(&self) -> bool {
        false
    }

    /// Get embeddings (optional)
    fn get_embeddings(&mut self, _input: &Tensor) -> candle_core::Result<Tensor> {
        candle_core::bail!("Embeddings not supported")
    }
}

/// Loaded model information
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model_type: String,
    pub vocab_size: usize,
    pub max_seq_len: usize,
    pub is_quantized: bool,
    pub dtype: String,
    pub device: String,
}
