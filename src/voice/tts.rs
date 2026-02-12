//! Text-to-Speech using Edge TTS (Microsoft online, high quality Chinese female voice)
use anyhow::{anyhow, Result};
use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};
use super::audio::AudioPlayer;

pub struct VitsTtsEngine {
    sample_rate: u32,
}

impl VitsTtsEngine {
    pub fn new<P: AsRef<Path>>(_model_path: P) -> Result<Self> {
        Ok(Self { sample_rate: 24000 })
    }

    fn sapi_speak(text: &str) -> Result<()> {
        if !cfg!(windows) { return Err(anyhow!("SAPI fallback not supported on this OS")); }
        let mut child = Command::new("powershell")
            .args(["-NoLogo","-NoProfile","-NonInteractive","-ExecutionPolicy","Bypass","-Command","Add-Type -AssemblyName System.Speech; $s=New-Object System.Speech.Synthesis.SpeechSynthesizer; $s.Speak([Console]::In.ReadToEnd())"])
            .stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::piped())
            .spawn()
            .map_err(|e| anyhow!("failed to spawn PowerShell SAPI: {e}"))?;
        if let Some(mut stdin) = child.stdin.take() { let _ = stdin.write_all(text.as_bytes()); }
        let out = child.wait_with_output().map_err(|e| anyhow!("SAPI wait failed: {e}"))?;
        if !out.status.success() {
            return Err(anyhow!("SAPI failed (code={:?})\nstderr:\n{}", out.status.code(), String::from_utf8_lossy(&out.stderr).trim()));
        }
        Ok(())
    }

    pub fn synthesize(&mut self, text: &str) -> Result<Vec<i16>> {
        let temp_dir = std::env::temp_dir();
        let text_file = temp_dir.join("mari_tts_input.txt");
        let audio_file = temp_dir.join("mari_tts.mp3");

        // Keep alphanumeric (including CJK), whitespace, and common punctuation
        let clean: String = text
            .chars()
            .filter(|&c| {
                c.is_alphanumeric() 
                    || c.is_whitespace() 
                    || ",.!?:;()~，。！？：；（）、".contains(c)
            })
            .collect();
        
        // Skip if nothing speakable
        let clean = clean.trim();
        if clean.is_empty() {
            bevy::log::warn!("[TTS] Skipping empty text");
            return Ok(vec![]);
        }
        
        let _ = std::fs::write(&text_file, clean);
        let edge = Command::new("edge-tts")
            .args(["--voice","zh-CN-XiaoxiaoMultilingualNeural","-f",&text_file.to_string_lossy(),"--write-media",&audio_file.to_string_lossy()])
            .output();

        match edge {
            Ok(out) if out.status.success() && audio_file.exists() => AudioPlayer::play_file(&audio_file.to_string_lossy()),
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
                let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
                bevy::log::warn!("[TTS] edge-tts failed (code={:?}): {}", out.status.code(), stderr);
                if Self::sapi_speak(clean).is_err() {
                    let stderr_s = if stderr.is_empty() { String::new() } else { format!("\nstderr:\n{stderr}") };
                    let stdout_s = if stdout.is_empty() { String::new() } else { format!("\nstdout:\n{stdout}") };
                    return Err(anyhow!("edge-tts failed (code={:?}){}{}", out.status.code(), stderr_s, stdout_s));
                }
            }
            Err(e) => {
                bevy::log::warn!("[TTS] edge-tts spawn failed: {e}");
                Self::sapi_speak(clean).map_err(|se| anyhow!("failed to run edge-tts ({e}); SAPI fallback failed ({se})"))?;
            }
        }
        Ok(vec![])
    }

    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }
}

pub const MARI_SYSTEM_PROMPT: &str =
    "You are Mari, EVA pilot. Cheerful, playful. Keep replies short.";
