//! Voice module: STT (Whisper ONNX) + TTS (VITS ONNX) + Gemini Live API for AI assistant
pub mod audio;
pub mod gemini_live;
pub mod service;
pub mod stt;
pub mod tts;

pub use gemini_live::{GeminiLiveClient, GeminiLiveCommand, GeminiLiveEvent};
pub use service::{VoiceCommand, VoiceEvent, VoiceMode, VoiceService, VoiceServicePlugin};
