//! Settings provider for global voice assistant and voice models.
use crate::settings::{SettingMeta, SettingScope, SettingValue, SettingsRegistry};

pub fn register_voice_assistant_settings(reg: &mut SettingsRegistry) {
    reg.upsert(SettingMeta {
        id: "voice.assistant.enabled".into(),
        path: "Voice/Assistant".into(),
        label: "Enabled".into(),
        help: "Enable global wake word + dictation assistant".into(),
        scope: SettingScope::User,
        default: SettingValue::Bool(true),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "wake".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.wake_phrases".into(),
        path: "Voice/Assistant".into(),
        label: "Wake phrases".into(),
        help: "Separated by '|' or newline (e.g. hallo gemini|hello gemini)".into(),
        scope: SettingScope::User,
        default: SettingValue::String("hallo gemini|hello gemini|halo gemini".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "wake".into(), "keyword".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.cmd_input_phrases".into(),
        path: "Voice/Assistant".into(),
        label: "Cmd: input".into(),
        help: "Start dictation / wake (separated by '|' or newline)".into(),
        scope: SettingScope::User,
        default: SettingValue::String("start dictation|input".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "command".into(), "input".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.cmd_send_phrases".into(),
        path: "Voice/Assistant".into(),
        label: "Cmd: send".into(),
        help: "Send current dictation buffer (separated by '|' or newline)".into(),
        scope: SettingScope::User,
        default: SettingValue::String("send".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "command".into(), "send".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.cmd_cancel_phrases".into(),
        path: "Voice/Assistant".into(),
        label: "Cmd: cancel".into(),
        help: "Cancel dictation and go back to sleep (separated by '|' or newline)".into(),
        scope: SettingScope::User,
        default: SettingValue::String("cancel|stop".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "command".into(), "cancel".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.greet_text".into(),
        path: "Voice/Assistant".into(),
        label: "Greet text".into(),
        help: "Text shown/spoken after wake word".into(),
        scope: SettingScope::User,
        default: SettingValue::String("I'm here, what can I do for you?".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "tts".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.sleep_text".into(),
        path: "Voice/Assistant".into(),
        label: "Sleep text".into(),
        help: "Text shown/spoken on idle sleep".into(),
        scope: SettingScope::User,
        default: SettingValue::String("I'll go to rest then.".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "assistant".into(), "sleep".into(), "tts".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.idle_timeout_secs".into(),
        path: "Voice/Assistant".into(),
        label: "Idle timeout (sec)".into(),
        help: "Go back to sleep after this many seconds without speech".into(),
        scope: SettingScope::User,
        default: SettingValue::I64(10),
        min: Some(1.0),
        max: Some(600.0),
        step: Some(1.0),
        keywords: vec!["voice".into(), "assistant".into(), "timeout".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.assistant.auto_send_pause_secs".into(),
        path: "Voice/Assistant".into(),
        label: "Auto send pause (sec)".into(),
        help: "Auto-send dictation after this many seconds of silence".into(),
        scope: SettingScope::User,
        default: SettingValue::I64(3),
        min: Some(1.0),
        max: Some(30.0),
        step: Some(1.0),
        keywords: vec!["voice".into(), "assistant".into(), "send".into(), "silence".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.stt_model_path".into(),
        path: "Voice/Models".into(),
        label: "STT model path".into(),
        help: "Whisper ggml model path".into(),
        scope: SettingScope::User,
        default: SettingValue::String("Lmodels/ggml-base.bin".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "stt".into(), "whisper".into(), "model".into()],
    });
    reg.upsert(SettingMeta {
        id: "voice.tts_model_path".into(),
        path: "Voice/Models".into(),
        label: "TTS model path".into(),
        help: "Reserved for local TTS engine model path".into(),
        scope: SettingScope::User,
        default: SettingValue::String("Lmodels/vits_mari.onnx".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["voice".into(), "tts".into(), "model".into()],
    });
}

crate::register_settings_provider!("voice_assistant", register_voice_assistant_settings);

