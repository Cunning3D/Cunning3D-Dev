use anyhow::{Error as E, Result};
use bevy::prelude::*;
use candle_core::quantized::gguf_file;
use candle_core::{DType, Device, Tensor};
use candle_transformers::generation::LogitsProcessor;
use candle_transformers::models::quantized_qwen3::ModelWeights;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use tokenizers::Tokenizer;

use super::knowledge_cache::GLOBAL_KNOWLEDGE_CACHE;
use crate::libs::ai_service::ai_defaults::{DEFAULT_TEMPERATURE, DEFAULT_TOP_P};

/// Tiny Model Host Resource
#[derive(Resource)]
pub struct TinyModelHost {
    tx: Sender<TinyRequest>,
    rx: Arc<Mutex<Receiver<TinyResponse>>>,
    cancel_flag: Arc<AtomicBool>,
}

pub struct TinyRequest {
    pub id: String,
    pub prompt: String,
    pub max_tokens: usize,
}

#[derive(Clone, Debug)]
pub struct TinyResponse {
    pub id: String,
    pub text: String,
    pub error: Option<String>,
}

impl TinyModelHost {
    pub fn new() -> Self {
        let (tx, rx_cmd) = channel::<TinyRequest>();
        let (tx_resp, rx) = channel::<TinyResponse>();
        let cancel_flag = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel_flag.clone();

        thread::spawn(move || {
            let Ok(mut runtime) = TinyModelRuntime::init() else {
                error!("Failed to init TinyModel");
                return;
            };
            while let Ok(req) = rx_cmd.recv() {
                cancel_clone.store(false, Ordering::Relaxed);
                let start = std::time::Instant::now();
                match runtime.predict(&req.prompt, req.max_tokens, &cancel_clone) {
                    Ok(text) => {
                        debug!(
                            "TinyModel predict [{}] took {:.2}ms",
                            req.id,
                            start.elapsed().as_millis()
                        );
                        let _ = tx_resp.send(TinyResponse {
                            id: req.id,
                            text,
                            error: None,
                        });
                    }
                    Err(e) if e.to_string().contains("cancelled") => {
                        info!("TinyModel cancelled [{}]", req.id);
                    }
                    Err(e) => {
                        error!("TinyModel predict error: {}", e);
                        let _ = tx_resp.send(TinyResponse {
                            id: req.id,
                            text: String::new(),
                            error: Some(e.to_string()),
                        });
                    }
                }
            }
        });
        Self {
            tx,
            rx: Arc::new(Mutex::new(rx)),
            cancel_flag,
        }
    }

    pub fn request(&self, id: &str, prompt: &str) {
        let _ = self.tx.send(TinyRequest {
            id: id.to_string(),
            prompt: prompt.to_string(),
            max_tokens: 64,
        });
    }
    pub fn cancel_current(&self) {
        self.cancel_flag.store(true, Ordering::Relaxed);
    }
    pub fn poll(&self) -> Vec<TinyResponse> {
        let mut res = Vec::new();
        if let Ok(rx) = self.rx.lock() {
            while let Ok(msg) = rx.try_recv() {
                res.push(msg);
            }
        }
        res
    }
}

struct TinyModelRuntime {
    model: ModelWeights,
    tokenizer: Tokenizer,
    logits_processor: LogitsProcessor,
    device: Device,
}

impl TinyModelRuntime {
    fn init() -> Result<Self> {
        // Hardcoded paths for now, should come from config
        let model_path = "Lmodels/Qwen3-1.7B-Q6_K.gguf";
        let tokenizer_path = "Lmodels/tokenizer.json";

        info!("Initializing TinyModel from {}", model_path);

        #[cfg(feature = "cuda")]
        let device = Device::new_cuda(0).unwrap_or(Device::Cpu);
        #[cfg(not(feature = "cuda"))]
        let device = Device::Cpu;

        let mut file = std::fs::File::open(model_path)?;
        let content = gguf_file::Content::read(&mut file).map_err(E::msg)?;

        let model = ModelWeights::from_gguf(content, &mut file, &device).map_err(E::msg)?;
        let tokenizer = Tokenizer::from_file(tokenizer_path).map_err(E::msg)?;
        let logits_processor =
            LogitsProcessor::new(299792458, Some(DEFAULT_TEMPERATURE), Some(DEFAULT_TOP_P));

        Ok(Self {
            model,
            tokenizer,
            logits_processor,
            device,
        })
    }

    fn predict(&mut self, prompt: &str, max_tokens: usize, cancel: &AtomicBool) -> Result<String> {
        self.model.clear_kv_cache();
        if cancel.load(Ordering::Relaxed) {
            return Err(E::msg("cancelled"));
        }
        let tokens = self.tokenizer.encode(prompt, true).map_err(E::msg)?;
        let prompt_ids = tokens.get_ids();
        let input = Tensor::new(prompt_ids, &self.device)?.unsqueeze(0)?;
        let mut output_tokens = Vec::new();
        // Prefill
        let logits = self.model.forward(&input, 0)?.squeeze(0)?;
        let mut next_token = self.logits_processor.sample(&logits)?;
        output_tokens.push(next_token);
        // Decode loop
        for i in 0..max_tokens {
            if cancel.load(Ordering::Relaxed) {
                return Err(E::msg("cancelled"));
            }
            let input = Tensor::new(&[next_token], &self.device)?.unsqueeze(0)?;
            let logits = self
                .model
                .forward(&input, prompt_ids.len() + i + 1)?
                .squeeze(0)?;
            next_token = self.logits_processor.sample(&logits)?;
            if next_token == self.tokenizer.token_to_id("<|endoftext|>").unwrap_or(0)
                || next_token == self.tokenizer.token_to_id("<|im_end|>").unwrap_or(0)
            {
                break;
            }
            output_tokens.push(next_token);
        }
        self.tokenizer.decode(&output_tokens, true).map_err(E::msg)
    }
}
