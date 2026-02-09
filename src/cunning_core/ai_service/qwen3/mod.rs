//! Qwen3 model backend - 移植自 Oxide-Lab (https://github.com/FerrisMind/Oxide-Lab)
//! 支持 GGUF (quantized) 和 SafeTensors (full) 格式

mod gguf;
pub mod model;
mod safetensors;

pub use model::{Config, Qwen3Attention, Qwen3MLP, Qwen3RotaryEmbedding};

use candle_core::{Device, Tensor};
use candle_transformers::models::quantized_qwen3::ModelWeights as QuantizedQwen3;

use super::models_api::optimization::{OptimizationConfig, WeightFormat};
use super::models_api::ModelBackend;

use model::ModelForCausalLM;

/// Qwen3 后端 - 支持 GGUF 和 SafeTensors 两种格式
pub struct Qwen3Backend {
    inner: Qwen3Inner,
    device: Device,
    vocab_size: usize,
    max_seq_len: usize,
    optimization: OptimizationConfig,
}

enum Qwen3Inner {
    Quantized(QuantizedQwen3),
    Full(ModelForCausalLM),
}

impl Qwen3Backend {
    pub(crate) fn new_quantized(
        model: QuantizedQwen3,
        device: Device,
        vocab_size: usize,
        max_seq_len: usize,
    ) -> Self {
        Self {
            inner: Qwen3Inner::Quantized(model),
            device,
            vocab_size,
            max_seq_len,
            optimization: OptimizationConfig::for_gguf(),
        }
    }

    pub(crate) fn new_full(
        model: ModelForCausalLM,
        device: Device,
        vocab_size: usize,
        max_seq_len: usize,
        optimization: OptimizationConfig,
    ) -> Self {
        Self {
            inner: Qwen3Inner::Full(model),
            device,
            vocab_size,
            max_seq_len,
            optimization,
        }
    }

    pub fn device(&self) -> &Device {
        &self.device
    }
    pub fn is_quantized(&self) -> bool {
        matches!(self.inner, Qwen3Inner::Quantized(_))
    }
    pub fn optimization(&self) -> &OptimizationConfig {
        &self.optimization
    }
}

impl ModelBackend for Qwen3Backend {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        match &mut self.inner {
            Qwen3Inner::Quantized(model) => model.forward(input, pos),
            Qwen3Inner::Full(model) => {
                let logits = model.forward(input, pos)?;
                let seq_len = logits.dim(1)?;
                logits.narrow(1, seq_len - 1, 1)?.squeeze(1)
            }
        }
    }

    fn clear_kv_cache(&mut self) {
        match &mut self.inner {
            Qwen3Inner::Quantized(model) => model.clear_kv_cache(),
            Qwen3Inner::Full(model) => model.clear_kv_cache(),
        }
    }

    fn model_type(&self) -> &str {
        match self.optimization.weight_format() {
            WeightFormat::Gguf => "qwen3-gguf",
            WeightFormat::SafeTensors => {
                if self.optimization.uses_flash_attn() {
                    "qwen3-flash"
                } else {
                    "qwen3"
                }
            }
        }
    }

    fn vocab_size(&self) -> usize {
        self.vocab_size
    }
    fn max_seq_len(&self) -> usize {
        self.max_seq_len
    }
    fn supports_flash_attn(&self) -> bool {
        self.optimization.uses_flash_attn()
    }

    fn get_embeddings(&mut self, input: &Tensor) -> candle_core::Result<Tensor> {
        match &mut self.inner {
            Qwen3Inner::Full(model) => model.get_hidden_states(input, 0),
            Qwen3Inner::Quantized(_) => candle_core::bail!("GGUF 模型暂不支持 Embeddings"),
        }
    }
}
