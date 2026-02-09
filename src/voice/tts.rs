//! Text-to-Speech using Edge TTS (Microsoft online, high quality Chinese female voice)
use anyhow::{anyhow, Result};
use std::path::Path;
use std::process::Command;
use super::audio::AudioPlayer;

pub struct VitsTtsEngine {
    sample_rate: u32,
}

impl VitsTtsEngine {
    pub fn new<P: AsRef<Path>>(_model_path: P) -> Result<Self> {
        Ok(Self { sample_rate: 24000 })
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Vec<i16>> {
        let temp_dir = std::env::temp_dir();
        let text_file = temp_dir.join("mari_tts_input.txt");
        let audio_file = temp_dir.join("mari_tts.mp3");

        // Keep only alphanumeric, whitespace, and basic punctuation
        let clean: String = text
            .chars()
            .filter(|&c| c.is_alphanumeric() || c.is_whitespace() || ",.!?:;()~".contains(c))
            .collect();
        let _ = std::fs::write(&text_file, &clean);

        let out = Command::new("edge-tts")
            .args([
                "--voice",
                "zh-CN-XiaoxiaoMultilingualNeural",
                "-f",
                &text_file.to_string_lossy(),
                "--write-media",
                &audio_file.to_string_lossy(),
            ])
            .output()
            .map_err(|e| anyhow!("failed to run edge-tts: {e}"))?;

        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
            let stderr_s = if stderr.is_empty() { String::new() } else { format!("\nstderr:\n{stderr}") };
            let stdout_s = if stdout.is_empty() { String::new() } else { format!("\nstdout:\n{stdout}") };
            return Err(anyhow!(
                "edge-tts failed (code={:?}){}{}",
                out.status.code(),
                stderr_s,
                stdout_s,
            ));
        }
        if !audio_file.exists() {
            return Err(anyhow!("edge-tts succeeded but output missing: {}", audio_file.display()));
        }

        // Play in-process (no external window/player).
        AudioPlayer::play_file(&audio_file.to_string_lossy());
        Ok(vec![])
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

pub const MARI_SYSTEM_PROMPT: &str =
    "You are Mari, EVA pilot. Cheerful, playful. Keep replies short.";
