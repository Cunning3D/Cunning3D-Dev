//! Trait ModelBackend - 移植自 Oxide-Lab

use candle_core::Tensor;

/// 所有模型后端必须实现的统一 trait
pub trait ModelBackend: Send {
    /// Forward pass - 返回 logits [batch_size, seq_len, vocab_size]
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor>;

    /// 清除 KV cache
    fn clear_kv_cache(&mut self);

    /// 模型类型 (e.g. "qwen3", "llama")
    fn model_type(&self) -> &str;

    /// 词表大小
    fn vocab_size(&self) -> usize;

    /// 最大序列长度
    fn max_seq_len(&self) -> usize {
        4096
    }

    /// 是否支持 Flash Attention
    fn supports_flash_attn(&self) -> bool {
        false
    }

    /// 获取 embeddings (可选实现)
    fn get_embeddings(&mut self, _input: &Tensor) -> candle_core::Result<Tensor> {
        candle_core::bail!("Embeddings not supported")
    }
}

/// 加载后的模型信息
#[derive(Debug, Clone)]
pub struct ModelInfo {
    pub model_type: String,
    pub vocab_size: usize,
    pub max_seq_len: usize,
    pub is_quantized: bool,
    pub dtype: String,
    pub device: String,
}
