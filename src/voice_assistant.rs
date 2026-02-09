//! Global voice assistant: wake word → open AI Workspace → dictation → auto-send.
use crate::ai_workspace_gpui::protocol::{ImageSnapshot, MentionSnapshot, UiToHost};
use crate::app::windowing::GpuiAiWorkspaceState;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::settings::{SettingValue, SettingsMerge, SettingsRegistry, SettingsStores};
use crate::voice::{VoiceCommand, VoiceEvent, VoiceMode, VoiceService};
use bevy::prelude::*;
use crossbeam_channel::Receiver;
use std::time::{Duration, Instant};

#[derive(Resource, Clone)]
pub struct AiVoiceAssistantConfig {
    pub enabled: bool,
    pub wake_phrases: Vec<String>,
    pub cmd_input_phrases: Vec<String>,
    pub cmd_send_phrases: Vec<String>,
    pub cmd_cancel_phrases: Vec<String>,
    pub greet_text: String,
    pub sleep_text: String,
    pub auto_send_pause: Duration,
    pub idle_timeout: Duration,
}

impl Default for AiVoiceAssistantConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            wake_phrases: vec!["hallo gemini".into(), "hello gemini".into(), "hi gemini".into(), "halo gemini".into()],
            cmd_input_phrases: vec!["输入".into(), "start dictation".into(), "input".into()],
            cmd_send_phrases: vec!["发送".into(), "send".into()],
            cmd_cancel_phrases: vec!["取消".into(), "停止".into(), "cancel".into(), "stop".into()],
            greet_text: "在呢，请问有什么需求。".into(),
            sleep_text: "那我先去休息了。".into(),
            auto_send_pause: Duration::from_secs(3),
            idle_timeout: Duration::from_secs(10),
        }
    }
}

#[derive(Resource)]
struct AiVoiceAssistantState {
    rx: Option<Receiver<VoiceEvent>>,
    awake: bool,
    last_activity: Instant,
    configured: bool,
    draft: String,
    last_draft_at: Instant,
    last_error: Option<String>,
    last_error_at: Instant,
}

impl Default for AiVoiceAssistantState {
    fn default() -> Self {
        Self {
            rx: None,
            awake: false,
            last_activity: Instant::now(),
            configured: false,
            draft: String::new(),
            last_draft_at: Instant::now(),
            last_error: None,
            last_error_at: Instant::now(),
        }
    }
}

pub struct AiVoiceAssistantPlugin;

impl Plugin for AiVoiceAssistantPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AiVoiceAssistantConfig>()
            .init_resource::<AiVoiceAssistantState>()
            .add_systems(
                Startup,
                load_voice_assistant_config.after(crate::app::startup::setup_registries),
            )
            .add_systems(Update, ai_voice_assistant_system);
    }
}

fn load_voice_assistant_config(
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut commands: Commands,
) {
    let d = AiVoiceAssistantConfig::default();
    let get = |id: &str| {
        reg.get(id).map(|m| SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id)).1)
    };
    let split_phrases = |v: &str| {
        v.split(|c| c == '\n' || c == '|' || c == ',' || c == ';')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect::<Vec<_>>()
    };
    let enabled = matches!(get("voice.assistant.enabled"), Some(SettingValue::Bool(true)) | None);
    let wake_phrases = match get("voice.assistant.wake_phrases") {
        Some(SettingValue::String(v)) => split_phrases(&v),
        _ => d.wake_phrases.clone(),
    };
    let cmd_input_phrases = match get("voice.assistant.cmd_input_phrases") {
        Some(SettingValue::String(v)) => split_phrases(&v),
        _ => d.cmd_input_phrases.clone(),
    };
    let cmd_send_phrases = match get("voice.assistant.cmd_send_phrases") {
        Some(SettingValue::String(v)) => split_phrases(&v),
        _ => d.cmd_send_phrases.clone(),
    };
    let cmd_cancel_phrases = match get("voice.assistant.cmd_cancel_phrases") {
        Some(SettingValue::String(v)) => split_phrases(&v),
        _ => d.cmd_cancel_phrases.clone(),
    };
    let greet_text = match get("voice.assistant.greet_text") {
        Some(SettingValue::String(v)) if !v.trim().is_empty() => v,
        _ => d.greet_text.clone(),
    };
    let sleep_text = match get("voice.assistant.sleep_text") {
        Some(SettingValue::String(v)) if !v.trim().is_empty() => v,
        _ => d.sleep_text.clone(),
    };
    let idle_timeout = match get("voice.assistant.idle_timeout_secs") {
        Some(SettingValue::I64(v)) if v > 0 => Duration::from_secs(v as u64),
        _ => d.idle_timeout,
    };
    let auto_send_pause = match get("voice.assistant.auto_send_pause_secs") {
        Some(SettingValue::I64(v)) if v > 0 => Duration::from_secs(v as u64),
        _ => d.auto_send_pause,
    };
    commands.insert_resource(AiVoiceAssistantConfig {
        enabled,
        wake_phrases,
        cmd_input_phrases,
        cmd_send_phrases,
        cmd_cancel_phrases,
        greet_text,
        sleep_text,
        auto_send_pause,
        idle_timeout,
    });
}

fn ai_voice_assistant_system(
    voice: Option<Res<VoiceService>>,
    cfg: Res<AiVoiceAssistantConfig>,
    mut st: ResMut<AiVoiceAssistantState>,
    mut gpui: ResMut<GpuiAiWorkspaceState>,
    node_registry: Res<NodeRegistry>,
) {
    let Some(voice) = voice else { return; };
    if !cfg.enabled {
        if st.configured || st.awake || voice.is_listening() {
            voice.send(VoiceCommand::SetMode(VoiceMode::Off));
        }
        st.awake = false;
        st.configured = false;
        return;
    }

    // Gate all microphone listening behind the AI Workspace window lifecycle to avoid
    // background capture when the user is not using the assistant UI.
    let window_running = gpui.is_running();
    if !window_running {
        if st.configured || st.awake || voice.is_listening() {
            voice.send(VoiceCommand::SetMode(VoiceMode::Off));
        }
        st.awake = false;
        st.configured = false;
        st.draft.clear();
        return;
    }

    if st.rx.is_none() {
        st.rx = Some(voice.subscribe());
    }
    if !st.configured {
        let mut wake = cfg.wake_phrases.clone();
        wake.extend(cfg.cmd_input_phrases.clone());
        voice.send(VoiceCommand::SetWakePhrases(wake));
        voice.send(VoiceCommand::SetMode(VoiceMode::WakeWordSleep));
        st.configured = true;
    }

    fn norm(s: &str) -> String {
        s.chars().filter(|c| c.is_alphanumeric()).flat_map(|c| c.to_lowercase()).collect()
    }
    fn hit(text: &str, phrases: &[String]) -> bool {
        let t = norm(text);
        !t.is_empty() && phrases.iter().any(|p| t.contains(&norm(p)))
    }

    let mut on_wake = false;
    let mut utterances: Vec<String> = Vec::new();
    let mut errors: Vec<String> = Vec::new();
    if let Some(ref rx) = st.rx {
        while let Ok(ev) = rx.try_recv() {
            match ev {
                VoiceEvent::WakeWordDetected(_) => on_wake = true,
                VoiceEvent::UtteranceFinalized(t) => utterances.push(t),
                VoiceEvent::Error(e) => errors.push(e),
                _ => {}
            }
        }
    }

    if !errors.is_empty() {
        let msg = errors.last().cloned().unwrap_or_default();
        let should_notify = st
            .last_error
            .as_deref()
            .map(|prev| prev != msg)
            .unwrap_or(true)
            || st.last_error_at.elapsed() >= Duration::from_secs(5);
        if should_notify && !msg.trim().is_empty() {
            st.last_error = Some(msg.clone());
            st.last_error_at = Instant::now();
            gpui.launch_if_not_running(node_registry.clone());
            let ui = format!("[Voice] {}", msg);
            let _ = gpui.try_send(UiToHost::LocalAssistantMessageToActive { text: ui });
        }
    }

    if on_wake {
        st.awake = true;
        st.last_activity = Instant::now();
        st.draft.clear();
        st.last_draft_at = Instant::now();
        gpui.launch_if_not_running(node_registry.clone());
        let _ = gpui.try_send(UiToHost::LocalAssistantMessageToActive {
            text: cfg.greet_text.clone(),
        });
        voice.send(VoiceCommand::Speak(cfg.greet_text.clone()));
    }

    if st.awake {
        let mut any_draft_update = false;
        for text in utterances {
            st.last_activity = Instant::now();
            if hit(&text, &cfg.cmd_cancel_phrases) {
                st.awake = false;
                st.draft.clear();
                st.last_draft_at = Instant::now();
                voice.send(VoiceCommand::SetMode(VoiceMode::WakeWordSleep));
                break;
            }
            if hit(&text, &cfg.cmd_input_phrases) {
                st.draft.clear();
                st.last_draft_at = Instant::now();
                continue;
            }
            if hit(&text, &cfg.cmd_send_phrases) {
                let msg = st.draft.trim().to_string();
                st.draft.clear();
                st.last_draft_at = Instant::now();
                if !msg.is_empty() {
                    gpui.launch_if_not_running(node_registry.clone());
                    let _ = gpui.try_send(UiToHost::SendMessageToActive {
                        text: msg,
                        mentions: Vec::<MentionSnapshot>::new(),
                        images: Vec::<ImageSnapshot>::new(),
                    });
                }
                continue;
            }
            if !text.trim().is_empty() {
                if !st.draft.is_empty() { st.draft.push(' '); }
                st.draft.push_str(text.trim());
                st.last_draft_at = Instant::now();
                any_draft_update = true;
            }
        }

        if st.awake && !st.draft.trim().is_empty() && (any_draft_update || st.last_draft_at.elapsed() >= cfg.auto_send_pause) && st.last_draft_at.elapsed() >= cfg.auto_send_pause {
            let msg = st.draft.trim().to_string();
            st.draft.clear();
            st.last_activity = Instant::now();
            gpui.launch_if_not_running(node_registry.clone());
            let _ = gpui.try_send(UiToHost::SendMessageToActive {
                text: msg,
                mentions: Vec::<MentionSnapshot>::new(),
                images: Vec::<ImageSnapshot>::new(),
            });
        }

        if st.last_activity.elapsed() >= cfg.idle_timeout {
            st.awake = false;
            st.draft.clear();
            st.last_draft_at = Instant::now();
            gpui.launch_if_not_running(node_registry.clone());
            let _ = gpui.try_send(UiToHost::LocalAssistantMessageToActive {
                text: cfg.sleep_text.clone(),
            });
            voice.send(VoiceCommand::Speak(cfg.sleep_text.clone()));
            voice.send(VoiceCommand::SetMode(VoiceMode::WakeWordSleep));
        }
    }
}

