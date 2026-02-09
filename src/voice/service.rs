//! Voice service: orchestrates STT + TTS + Audio I/O as Bevy Resource
use super::{
    audio::{AudioCapture, AudioChunk, AudioPlayer},
    stt::{resample_to_16k, simple_vad, WhisperEngine},
    tts::VitsTtsEngine,
};
use bevy::prelude::*;
use crossbeam_channel::{bounded, Receiver, Sender};
use std::time::{Duration, Instant};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc, Mutex,
};
use crate::settings::{SettingValue, SettingsMerge, SettingsRegistry, SettingsStores};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VoiceMode {
    Off,
    PushToTalk,
    WakeWordSleep,
    WakeWordAwake,
}

#[derive(Debug, Clone)]
pub enum VoiceCommand {
    StartListening,
    StopListening,
    Speak(String),
    SetMode(VoiceMode),
    SetWakePhrases(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum VoiceEvent {
    TranscriptionReady(String),
    WakeWordDetected(String),
    UtteranceFinalized(String),
    SpeechStarted,
    SpeechEnded,
    Error(String),
}

#[derive(Resource)]
pub struct VoiceService {
    cmd_tx: Sender<VoiceCommand>,
    event_rx: Receiver<VoiceEvent>,
    event_subs: Arc<Mutex<Vec<Sender<VoiceEvent>>>>,
    is_listening: Arc<AtomicBool>,
    is_speaking: Arc<AtomicBool>,
}

impl VoiceService {
    pub fn new(whisper_model: &str, tts_model: &str) -> Self {
        let (cmd_tx, cmd_rx) = bounded::<VoiceCommand>(16);
        let (event_tx, event_rx) = bounded::<VoiceEvent>(16);
        let event_subs = Arc::new(Mutex::new(vec![event_tx]));
        let is_listening = Arc::new(AtomicBool::new(false));
        let is_speaking = Arc::new(AtomicBool::new(false));
        let listen_flag = is_listening.clone();
        let speak_flag = is_speaking.clone();
        let subs = event_subs.clone();
        let stt_path = whisper_model.to_string();
        let tts_path = tts_model.to_string();
        std::thread::spawn(move || {
            Self::worker_loop(
                cmd_rx,
                subs,
                listen_flag,
                speak_flag,
                stt_path,
                tts_path,
            )
        });
        Self {
            cmd_tx,
            event_rx,
            event_subs,
            is_listening,
            is_speaking,
        }
    }

    pub fn send(&self, cmd: VoiceCommand) {
        let _ = self.cmd_tx.try_send(cmd);
    }
    pub fn poll_events(&self) -> Vec<VoiceEvent> {
        self.event_rx.try_iter().collect()
    }
    pub fn subscribe(&self) -> Receiver<VoiceEvent> {
        let (tx, rx) = bounded::<VoiceEvent>(64);
        if let Ok(mut subs) = self.event_subs.lock() {
            subs.push(tx);
        }
        rx
    }
    pub fn is_listening(&self) -> bool {
        self.is_listening.load(Ordering::SeqCst)
    }
    pub fn is_speaking(&self) -> bool {
        self.is_speaking.load(Ordering::SeqCst)
    }

    fn worker_loop(
        cmd_rx: Receiver<VoiceCommand>,
        event_subs: Arc<Mutex<Vec<Sender<VoiceEvent>>>>,
        listen_flag: Arc<AtomicBool>,
        speak_flag: Arc<AtomicBool>,
        stt_path: String,
        tts_path: String,
    ) {
        fn emit(subs: &Arc<Mutex<Vec<Sender<VoiceEvent>>>>, ev: VoiceEvent) {
            if let Ok(mut list) = subs.lock() {
                list.retain(|tx| tx.try_send(ev.clone()).is_ok());
            }
        }
        let whisper = match WhisperEngine::new(&stt_path) {
            Ok(w) => Some(w),
            Err(e) => {
                emit(&event_subs, VoiceEvent::Error(format!("Whisper model load failed: {stt_path} ({e})")));
                None
            }
        };
        let mut tts = VitsTtsEngine::new(&tts_path).ok();
        let mut capture = AudioCapture::new();
        let samples_rx = capture.take_receiver();
        let mut mode = VoiceMode::Off;
        let mut wake_phrases: Vec<String> = Vec::new();
        let mut ptt_buf: Vec<f32> = Vec::new();
        let mut ptt_sr: u32 = 48_000;
        let mut utt_buf: Vec<f32> = Vec::new();
        let mut utt_sr: u32 = 48_000;
        let mut speech_active = false;
        let mut speech_started_at = Instant::now();
        let mut last_voice_at = Instant::now();
        let vad_th = 0.01;
        let end_silence = Duration::from_millis(850);
        let min_utt = Duration::from_millis(250);

        fn norm(s: &str) -> String {
            s.chars()
                .filter(|c| c.is_alphanumeric())
                .flat_map(|c| c.to_lowercase())
                .collect()
        }

        fn wake_hit(text: &str, phrases: &[String]) -> bool {
            let t = norm(text);
            !t.is_empty() && phrases.iter().any(|p| t.contains(&norm(p)))
        }

        loop {
            while let Ok(cmd) = cmd_rx.try_recv() {
                match cmd {
                    VoiceCommand::StartListening => {
                        mode = VoiceMode::PushToTalk;
                        ptt_buf.clear();
                        capture.start();
                        listen_flag.store(true, Ordering::SeqCst);
                    }
                    VoiceCommand::StopListening => {
                        if mode == VoiceMode::PushToTalk {
                            mode = VoiceMode::Off;
                            capture.stop();
                            listen_flag.store(false, Ordering::SeqCst);
                        }
                        if let Some(ref w) = whisper {
                            if !ptt_buf.is_empty() {
                                let samples_16k = resample_to_16k(&ptt_buf, ptt_sr);
                                match w.transcribe(&samples_16k) {
                                    Ok(text) if !text.is_empty() => {
                                        emit(&event_subs, VoiceEvent::TranscriptionReady(text));
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        emit(&event_subs, VoiceEvent::Error(e.to_string()));
                                    }
                                }
                            }
                        }
                        ptt_buf.clear();
                    }
                    VoiceCommand::Speak(text) => {
                        speak_flag.store(true, Ordering::SeqCst);
                        emit(&event_subs, VoiceEvent::SpeechStarted);
                        if let Some(ref mut engine) = tts {
                            match engine.synthesize(&text) {
                                Ok(samples) if !samples.is_empty() => {
                                    let bytes: Vec<u8> =
                                        samples.iter().flat_map(|&s| s.to_le_bytes()).collect();
                                    AudioPlayer::play_bytes(&bytes, engine.sample_rate());
                                }
                                Ok(_) => {}
                                Err(e) => {
                                    emit(&event_subs, VoiceEvent::Error(e.to_string()));
                                }
                            }
                        }
                        speak_flag.store(false, Ordering::SeqCst);
                        emit(&event_subs, VoiceEvent::SpeechEnded);
                    }
                    VoiceCommand::SetMode(m) => {
                        mode = m;
                        match mode {
                            VoiceMode::Off => {
                                capture.stop();
                                listen_flag.store(false, Ordering::SeqCst);
                            }
                            _ => {
                                capture.start();
                                listen_flag.store(true, Ordering::SeqCst);
                            }
                        }
                        if mode != VoiceMode::PushToTalk {
                            ptt_buf.clear();
                        }
                        if !matches!(mode, VoiceMode::WakeWordSleep | VoiceMode::WakeWordAwake) {
                            utt_buf.clear();
                            speech_active = false;
                        }
                    }
                    VoiceCommand::SetWakePhrases(p) => wake_phrases = p,
                }
            }
            if let Some(ref rx) = samples_rx {
                while let Ok(AudioChunk { samples, sample_rate }) = rx.try_recv() {
                    match mode {
                        VoiceMode::Off => {}
                        VoiceMode::PushToTalk => {
                            ptt_sr = sample_rate;
                            if simple_vad(&samples, vad_th) {
                                ptt_buf.extend(samples);
                            }
                        }
                        VoiceMode::WakeWordSleep | VoiceMode::WakeWordAwake => {
                            utt_sr = sample_rate;
                            let voiced = simple_vad(&samples, vad_th);
                            if voiced {
                                if !speech_active {
                                    speech_active = true;
                                    speech_started_at = Instant::now();
                                    utt_buf.clear();
                                }
                                utt_buf.extend(samples);
                                last_voice_at = Instant::now();
                                continue;
                            }
                            if speech_active
                                && last_voice_at.elapsed() >= end_silence
                                && speech_started_at.elapsed() >= min_utt
                                && !utt_buf.is_empty()
                            {
                                speech_active = false;
                                if let Some(ref w) = whisper {
                                    let samples_16k = resample_to_16k(&utt_buf, utt_sr);
                                    match w.transcribe(&samples_16k) {
                                        Ok(text) if !text.trim().is_empty() => {
                                            if mode == VoiceMode::WakeWordSleep && wake_hit(&text, &wake_phrases) {
                                                mode = VoiceMode::WakeWordAwake;
                                                emit(&event_subs, VoiceEvent::WakeWordDetected(text));
                                            } else if mode == VoiceMode::WakeWordAwake {
                                                emit(&event_subs, VoiceEvent::UtteranceFinalized(text));
                                            }
                                        }
                                        Ok(_) => {}
                                        Err(e) => {
                                            emit(&event_subs, VoiceEvent::Error(e.to_string()));
                                        }
                                    }
                                }
                                utt_buf.clear();
                            }
                        }
                    }
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
    }
}

pub struct VoiceServicePlugin;

impl Plugin for VoiceServicePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Startup,
            init_voice_service_from_settings.after(crate::app::startup::setup_registries),
        );
    }
}

fn init_voice_service_from_settings(
    existing: Option<Res<VoiceService>>,
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut commands: Commands,
) {
    if existing.is_some() {
        return;
    }
    let def_stt = "Lmodels/ggml-base.bin".to_string();
    let def_tts = "Lmodels/vits_mari.onnx".to_string();
    let get = |id: &str| {
        reg.get(id).map(|m| SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id)).1)
    };
    let stt = match get("voice.stt_model_path") {
        Some(SettingValue::String(v)) if !v.trim().is_empty() => v,
        _ => def_stt,
    };
    let tts = match get("voice.tts_model_path") {
        Some(SettingValue::String(v)) if !v.trim().is_empty() => v,
        _ => def_tts,
    };
    commands.insert_resource(VoiceService::new(&stt, &tts));
}
