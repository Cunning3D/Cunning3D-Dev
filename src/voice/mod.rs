//! Voice module: STT (Whisper ONNX) + TTS (VITS ONNX) + Audio I/O for AI assistant "Mari" - fully local
pub mod audio;
pub mod service;
pub mod stt;
pub mod tts;

pub use service::{VoiceCommand, VoiceEvent, VoiceMode, VoiceService, VoiceServicePlugin};
