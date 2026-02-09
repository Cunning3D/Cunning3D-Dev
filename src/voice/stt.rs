//! Speech-to-Text using Whisper (whisper-rs binding)
use anyhow::Result;
use std::path::Path;

pub struct WhisperEngine {
    #[cfg(feature = "voice_whisper")]
    ctx: whisper_rs::WhisperContext,
    #[cfg(not(feature = "voice_whisper"))]
    _phantom: (),
}

impl WhisperEngine {
    pub fn new<P: AsRef<Path>>(model_path: P) -> Result<Self> {
        #[cfg(feature = "voice_whisper")]
        {
            use whisper_rs::{WhisperContext, WhisperContextParameters};
            let ctx = WhisperContext::new_with_params(
                model_path.as_ref().to_str().unwrap_or(""),
                WhisperContextParameters::default(),
            )?;
            Ok(Self { ctx })
        }
        #[cfg(not(feature = "voice_whisper"))]
        {
            let _ = model_path;
            Ok(Self { _phantom: () })
        }
    }

    /// Transcribe audio samples (mono, 16kHz expected)
    pub fn transcribe(&self, samples: &[f32]) -> Result<String> {
        #[cfg(feature = "voice_whisper")]
        {
            use whisper_rs::{FullParams, SamplingStrategy};
            let mut params = FullParams::new(SamplingStrategy::Greedy { best_of: 1 });
            params.set_language(None);
            params.set_print_special(false);
            params.set_print_progress(false);
            params.set_print_realtime(false);
            params.set_print_timestamps(false);
            let mut state = self.ctx.create_state()?;
            state.full(params, samples)?;
            let n = state.full_n_segments()?;
            let mut text = String::new();
            for i in 0..n {
                if let Ok(seg) = state.full_get_segment_text(i) {
                    text.push_str(&seg);
                }
            }
            Ok(text.trim().to_string())
        }
        #[cfg(not(feature = "voice_whisper"))]
        {
            let _ = samples;
            Ok("[whisper feature disabled]".to_string())
        }
    }
}

/// Simple VAD: returns true if audio energy exceeds threshold
pub fn simple_vad(samples: &[f32], threshold: f32) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy: f32 = samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32;
    energy.sqrt() > threshold
}

/// Resample audio from src_rate to 16000 Hz (linear interpolation)
pub fn resample_to_16k(samples: &[f32], src_rate: u32) -> Vec<f32> {
    if src_rate == 16000 {
        return samples.to_vec();
    }
    let ratio = 16000.0 / src_rate as f64;
    let out_len = (samples.len() as f64 * ratio) as usize;
    (0..out_len)
        .map(|i| {
            let src_idx = i as f64 / ratio;
            let idx0 = src_idx.floor() as usize;
            let idx1 = (idx0 + 1).min(samples.len() - 1);
            let frac = (src_idx - idx0 as f64) as f32;
            samples[idx0] * (1.0 - frac) + samples[idx1] * frac
        })
        .collect()
}
