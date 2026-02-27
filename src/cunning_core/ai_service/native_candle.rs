//! Native AI inference backend - Learned from Oxide-Lab (https://github.com/FerrisMind/Oxide-Lab)
use anyhow::{Error as E, Result};
use bevy::prelude::*;
use candle_core::quantized::gguf_file::{Content, Value};
use candle_core::{DType, Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tokenizers::Tokenizer;

use crate::libs::ai_service::ai_defaults::{
    DEFAULT_QWEN_GGUF, DEFAULT_QWEN_TOKENIZER, DEFAULT_TEMPERATURE, DEFAULT_TOP_P,
    LOCAL_STREAM_LOG_VERBOSE,
};

/// Supported model architectures (Learned from Oxide-Lab/models/registry.rs)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchKind {
    Llama,
    Qwen2,
    Qwen3,
}

impl ArchKind {
    /// Automatically detect architecture from GGUF metadata
    pub fn detect(metadata: &std::collections::HashMap<String, Value>) -> Option<Self> {
        let arch = metadata
            .get("general.architecture")
            .and_then(|v| v.to_string().ok())?;
        let s = arch.to_lowercase();
        if s.contains("qwen3") {
            Some(Self::Qwen3)
        } else if s.contains("qwen2") || s.contains("qwen-2") {
            Some(Self::Qwen2)
        } else if s.contains("llama") || s.contains("mistral") {
            Some(Self::Llama)
        } else {
            None
        }
    }
}

/// Model backend trait (Learned from Oxide-Lab/models/api/model.rs)
trait ModelBackend: Send {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor>;
    fn clear_kv_cache(&mut self);
}

/// Qwen3 backend (using candle-transformers git version of quantized_qwen3)
struct Qwen3Backend(candle_transformers::models::quantized_qwen3::ModelWeights);
impl ModelBackend for Qwen3Backend {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        self.0.forward(input, pos)
    }
    fn clear_kv_cache(&mut self) {
        self.0.clear_kv_cache();
    }
}

/// Qwen2 backend
struct Qwen2Backend(candle_transformers::models::quantized_qwen2::ModelWeights);
impl ModelBackend for Qwen2Backend {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        self.0.forward(input, pos)
    }
    fn clear_kv_cache(&mut self) { /* qwen2 does not expose clear_kv_cache */
    }
}

/// Llama backend (also applies to Mistral etc.)
struct LlamaBackend(candle_transformers::models::quantized_llama::ModelWeights);
impl ModelBackend for LlamaBackend {
    fn forward(&mut self, input: &Tensor, pos: usize) -> candle_core::Result<Tensor> {
        self.0.forward(input, pos)
    }
    fn clear_kv_cache(&mut self) { /* quantized_llama does not expose clear_kv_cache */
    }
}

/// AI core running in background thread
struct NativeModel {
    backend: Box<dyn ModelBackend>,
    tokenizer: Tokenizer,
    logits_processor: LogitsProcessor,
    device: Device,
    model_path: String,
    tokenizer_path: String,
}

impl NativeModel {
    pub fn new<P: AsRef<Path>>(model_path: P, tokenizer_path: P) -> Result<Self> {
        // Automatically select device: CUDA -> CPU (Learned from Oxide-Lab)
        #[cfg(feature = "cuda")]
        let device = if candle_core::utils::cuda_is_available() {
            match Device::new_cuda(0) {
                Ok(d) => {
                    info!("Using CUDA inference (device=0)");
                    d
                }
                Err(e) => {
                    warn!("CUDA initialization failed: {e}, falling back to CPU");
                    Device::Cpu
                }
            }
        } else {
            info!("CUDA not available; using CPU");
            Device::Cpu
        };
        #[cfg(not(feature = "cuda"))]
        let device = {
            info!("CPU inference mode (enable GPU with `--features cuda`)");
            Device::Cpu
        };
        let model_path_s = model_path.as_ref().to_string_lossy().to_string();
        let tokenizer_path_s = tokenizer_path.as_ref().to_string_lossy().to_string();
        let mut file = std::fs::File::open(&model_path)?;
        let content = Content::read(&mut file).map_err(E::msg)?;

        // Automatically detect architecture and select correct loader (Learned from Oxide-Lab)
        let arch = ArchKind::detect(&content.metadata)
            .ok_or_else(|| E::msg("Unrecognized model architecture. Please use Qwen3/Qwen2/Llama."))?;

        info!("Detected model architecture: {:?}", arch);

        let backend: Box<dyn ModelBackend> = match arch {
            ArchKind::Qwen3 => {
                use candle_transformers::models::quantized_qwen3::ModelWeights;
                let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(E::msg)?;
                Box::new(Qwen3Backend(model))
            }
            ArchKind::Qwen2 => {
                use candle_transformers::models::quantized_qwen2::ModelWeights;
                let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(E::msg)?;
                Box::new(Qwen2Backend(model))
            }
            ArchKind::Llama => {
                use candle_transformers::models::quantized_llama::ModelWeights;
                let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(E::msg)?;
                Box::new(LlamaBackend(model))
            }
        };

        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(E::msg)?;
        let logits_processor =
            LogitsProcessor::new(299792458, Some(DEFAULT_TEMPERATURE), Some(DEFAULT_TOP_P));

        Ok(Self {
            backend,
            tokenizer,
            logits_processor,
            device,
            model_path: model_path_s,
            tokenizer_path: tokenizer_path_s,
        })
    }

    /// Streaming inference: call callback for each generated token, supports cancellation
    pub fn predict_streaming<F>(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        cancel: &AtomicBool,
        mut on_token: F,
    ) -> Result<String>
    where
        F: FnMut(&str),
    {
        info!(
            "[First Children] Starting streaming inference prompt_len={} max_tokens={} device={:?}",
            prompt.len(),
            max_tokens,
            self.device
        );
        self.backend.clear_kv_cache();
        let tokens = self.tokenizer.encode(prompt, true).map_err(E::msg)?;
        let prompt_ids = tokens.get_ids().to_vec();
        if prompt_ids.is_empty() {
            return Ok(String::new());
        }
        let base_len = prompt_ids.len();
        info!(
            "[First Children] Tokenized: {} tokens, starting prefill...",
            base_len
        );
        let eos = self
            .tokenizer
            .token_to_id("<|endoftext|>")
            .or_else(|| self.tokenizer.token_to_id("<|im_end|>"))
            .unwrap_or(0);

        // Check cancel before prefill
        if cancel.load(Ordering::Relaxed) {
            info!("[First Children] Inference cancelled (before prefill)");
            return Err(E::msg("cancelled"));
        }

        // Prefill
        let prefill_start = std::time::Instant::now();
        let input = Tensor::new(prompt_ids.as_slice(), &self.device)?.unsqueeze(0)?;
        let logits = self.backend.forward(&input, 0)?;
        let logits = logits.squeeze(0)?.to_dtype(DType::F32)?;
        let mut next = self.logits_processor.sample(&logits)?;
        info!(
            "[First Children] Prefill done ({:.2}s), first token={}",
            prefill_start.elapsed().as_secs_f32(),
            next
        );

        let mut out = Vec::new();
        let gen_start = std::time::Instant::now();
        for i in 0..max_tokens {
            if cancel.load(Ordering::Relaxed) {
                info!("[First Children] Inference cancelled (during generation)");
                return Err(E::msg("cancelled"));
            }
            if next == eos {
                break;
            }
            out.push(next);
            if let Ok(text) = self.tokenizer.decode(&[next], true) {
                if LOCAL_STREAM_LOG_VERBOSE {
                    info!("[First Children][tok {}] id={} text={:?}", i, next, text);
                }
                if !text.is_empty() {
                    on_token(&text);
                }
            }
            let input = Tensor::new(&[next], &self.device)?.unsqueeze(0)?;
            let logits = self
                .backend
                .forward(&input, base_len + i)?
                .squeeze(0)?
                .to_dtype(DType::F32)?;
            next = self.logits_processor.sample(&logits)?;
        }
        let total = self.tokenizer.decode(&out, true).map_err(E::msg)?;
        info!(
            "[First Children] Generation complete: {} tokens, {:.2}s ({:.1} tok/s)",
            out.len(),
            gen_start.elapsed().as_secs_f32(),
            out.len() as f32 / gen_start.elapsed().as_secs_f32()
        );
        Ok(total)
    }

    /// Non-streaming inference (compatible with old interface)
    pub fn predict(
        &mut self,
        prompt: &str,
        max_tokens: usize,
        cancel: &AtomicBool,
    ) -> Result<String> {
        self.predict_streaming(prompt, max_tokens, cancel, |_| {})
    }
}

pub enum AiCommand {
    LoadModel {
        model_path: String,
        tokenizer_path: String,
    },
    Predict {
        id: String,
        prompt: String,
        max_tokens: usize,
    },
}

pub enum AiResponse {
    ModelLoaded,
    /// Streaming output: send once per generated token
    StreamChunk {
        id: String,
        text: String,
        done: bool,
    },
    /// Compatible with old interface (full result)
    PredictionSuccess {
        id: String,
        result: String,
    },
    Error {
        id: String,
        message: String,
    },
}

/// Host service exposed externally
pub struct NativeAiHost {
    tx: Sender<AiCommand>,
    rx: Mutex<Receiver<AiResponse>>,
    cancel_flag: Arc<AtomicBool>,
}

impl NativeAiHost {
    pub fn new() -> Self {
        let (tx_cmd, rx_cmd) = channel::<AiCommand>();
        let (tx_resp, rx_resp) = channel::<AiResponse>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();

        thread::spawn(move || {
            let mut native_model: Option<NativeModel> = None;
            while let Ok(cmd) = rx_cmd.recv() {
                match cmd {
                    AiCommand::LoadModel {
                        model_path,
                        tokenizer_path,
                    } => {
                        if native_model.as_ref().is_some_and(|m| {
                            m.model_path == model_path && m.tokenizer_path == tokenizer_path
                        }) {
                            info!("[First Children] Model already loaded; skipping reload");
                            let _ = tx_resp.send(AiResponse::ModelLoaded);
                            continue;
                        }
                        info!("[First Children] Loading model: {}", model_path);
                        let load_start = std::time::Instant::now();
                        match NativeModel::new(&model_path, &tokenizer_path) {
                            Ok(model) => {
                                info!(
                                    "[First Children] Model loaded ({:.2}s)",
                                    load_start.elapsed().as_secs_f32()
                                );
                                native_model = Some(model);
                                let _ = tx_resp.send(AiResponse::ModelLoaded);
                            }
                            Err(e) => {
                                info!("[First Children] Model load failed: {}", e);
                                let _ = tx_resp.send(AiResponse::Error {
                                    id: "load".into(),
                                    message: format!("Model load failed: {}", e),
                                });
                            }
                        }
                    }
                    AiCommand::Predict {
                        id,
                        prompt,
                        max_tokens,
                    } => {
                        cancel_clone.store(false, Ordering::Relaxed); // Reset cancel flag
                        if let Some(model) = native_model.as_mut() {
                            let start = std::time::Instant::now();
                            let id_clone = id.clone();
                            let tx_stream = tx_resp.clone();
                            match model.predict_streaming(
                                &prompt,
                                max_tokens,
                                &cancel_clone,
                                |chunk| {
                                    let _ = tx_stream.send(AiResponse::StreamChunk {
                                        id: id_clone.clone(),
                                        text: chunk.to_string(),
                                        done: false,
                                    });
                                },
                            ) {
                                Ok(_result) => {
                                    info!(
                                        "Inference finished id={} elapsed={:.2}s",
                                        id,
                                        start.elapsed().as_secs_f32()
                                    );
                                    let _ = tx_resp.send(AiResponse::StreamChunk {
                                        id,
                                        text: String::new(),
                                        done: true,
                                    });
                                }
                                Err(e) if e.to_string().contains("cancelled") => {
                                    info!("Inference cancelled id={}", id);
                                }
                                Err(e) => {
                                    info!("Inference failed id={} err={}", id, e);
                                    let _ = tx_resp.send(AiResponse::Error {
                                        id,
                                        message: format!("Inference failed: {}", e),
                                    });
                                }
                            }
                        } else {
                            let _ = tx_resp.send(AiResponse::Error {
                                id,
                                message: "Model not loaded".into(),
                            });
                        }
                    }
                }
            }
        });

        Self {
            tx: tx_cmd,
            rx: Mutex::new(rx_resp),
            cancel_flag,
        }
    }

    pub fn load_model(&self, model_path: String, tokenizer_path: String) {
        let _ = self.tx.send(AiCommand::LoadModel {
            model_path,
            tokenizer_path,
        });
    }
    pub fn request_prediction(&self, id: String, prompt: String, max_tokens: usize) {
        let _ = self.tx.send(AiCommand::Predict {
            id,
            prompt,
            max_tokens,
        });
    }
    pub fn cancel_current(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
    pub fn poll_responses(&self) -> Vec<AiResponse> {
        let mut responses = Vec::new();
        if let Ok(rx) = self.rx.lock() {
            while let Ok(resp) = rx.try_recv() {
                responses.push(resp);
            }
        }
        responses
    }
}

impl Resource for NativeAiHost {}

#[derive(Resource, Default)]
pub struct NativeAiInbox(pub Vec<AiResultEvent>);

#[derive(Event, Clone)]
pub enum AiResultEvent {
    ModelLoaded,
    /// Streaming output: one event per token
    StreamChunk {
        id: String,
        text: String,
        done: bool,
    },
    /// Full result (compatible with old code)
    Success {
        id: String,
        result: String,
    },
    Error {
        id: String,
        message: String,
    },
}

pub struct NativeAiPlugin;

impl Plugin for NativeAiPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NativeAiHost::new())
            .init_resource::<NativeAiInbox>()
            .add_systems(Startup, autoload_native_ai_model)
            .add_systems(PreUpdate, poll_ai_system);
    }
}

fn autoload_native_ai_model(host: Res<NativeAiHost>) {
    info!(
        "[First Children] Auto-loading model on startup: {} / {}",
        DEFAULT_QWEN_GGUF, DEFAULT_QWEN_TOKENIZER
    );
    host.load_model(
        DEFAULT_QWEN_GGUF.to_string(),
        DEFAULT_QWEN_TOKENIZER.to_string(),
    );
}

fn poll_ai_system(host: Res<NativeAiHost>, mut inbox: ResMut<NativeAiInbox>) {
    inbox.0.clear();
    for resp in host.poll_responses() {
        inbox.0.push(match resp {
            AiResponse::ModelLoaded => AiResultEvent::ModelLoaded,
            AiResponse::StreamChunk { id, text, done } => {
                AiResultEvent::StreamChunk { id, text, done }
            }
            AiResponse::PredictionSuccess { id, result } => AiResultEvent::Success { id, result },
            AiResponse::Error { id, message } => AiResultEvent::Error { id, message },
        });
    }
}
