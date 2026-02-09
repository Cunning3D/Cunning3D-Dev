//! AiWorkspaceHost: Actor driving Session/Tool/LLM, outputs HostToUi events.
use super::ide::{DocumentStore, Worktree};
use super::protocol::*;
use crate::tabs_registry::ai_workspace::client::gemini::GeminiClient;
use crate::tabs_registry::ai_workspace::client::openai_compat::{
    OpenAiCompatChatOutput, OpenAiCompatClient,
};
use crate::tabs_registry::ai_workspace::context::agent_index::AgentIndex;
use crate::tabs_registry::ai_workspace::session::event::SessionEvent;
use crate::tabs_registry::ai_workspace::session::message::{ImageAttachment, MessageState, ThinkingSection};
use crate::tabs_registry::ai_workspace::session::session::{BusyStage, Session, ToolRequestMeta};
use crate::tabs_registry::ai_workspace::session::thread_entry::{MentionUri, ThreadEntry, ToolCall, ToolCallStatus, ToolKind};
use crate::tabs_registry::ai_workspace::tools::{
    AsyncToolExecutor, CancellationToken, ToolLogLevel, ToolRegistry, ToolResult as ToolExecResult,
};
use crossbeam_channel::{unbounded, Receiver, Sender};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use uuid::Uuid;

fn tts_speakable(s: &str) -> Option<String> {
    let t = s.trim();
    if t.is_empty() {
        return None;
    }
    let t = t.lines().take_while(|l| !l.trim_start().starts_with("```")).collect::<Vec<_>>().join("\n");
    let t = t.trim();
    if t.is_empty() {
        return None;
    }
    let max = 260usize;
    Some(if t.chars().count() <= max { t.to_string() } else { t.chars().take(max).collect::<String>() })
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat Backend Selection
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ChatBackend {
    #[default]
    Gemini,
    OpenAiCompat,
}

// ─────────────────────────────────────────────────────────────────────────────
// Provider Profile (Zed-like multi-provider support)
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct GeminiProviderSettings {
    #[serde(default)]
    pub api_key: String,
    #[serde(default)]
    pub model_pro: String,
    #[serde(default)]
    pub model_flash: String,
    #[serde(default)]
    pub model_image: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OpenAiCompatProfile {
    pub name: String,
    pub base_url: String,
    pub api_key: String,
    pub models: Vec<String>,
    pub selected_model: String,
}

impl Default for OpenAiCompatProfile {
    fn default() -> Self {
        Self {
            name: "LM Studio".to_string(),
            base_url: "http://127.0.0.1:1234".to_string(),
            api_key: String::new(),
            models: vec!["qwen2.5-coder".to_string()],
            selected_model: "qwen2.5-coder".to_string(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
struct ProvidersSettingsFile {
    #[serde(default)]
    gemini: GeminiProviderSettings,
    #[serde(default)]
    openai_compat_profiles: Vec<OpenAiCompatProfile>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct ToolPermissionsFile {
    #[serde(default)]
    allow_always: HashSet<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct VoiceAssistantSettingsFile {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    wake_phrases: String,
    #[serde(default)]
    cmd_input_phrases: String,
    #[serde(default)]
    cmd_send_phrases: String,
    #[serde(default)]
    cmd_cancel_phrases: String,
    #[serde(default)]
    greet_text: String,
    #[serde(default)]
    sleep_text: String,
    #[serde(default = "default_idle_timeout")]
    idle_timeout_secs: i64,
    #[serde(default = "default_auto_send_pause")]
    auto_send_pause_secs: i64,
}

fn default_idle_timeout() -> i64 { 10 }
fn default_auto_send_pause() -> i64 { 3 }

impl Default for VoiceAssistantSettingsFile {
    fn default() -> Self {
        Self {
            enabled: true,
            wake_phrases: "hallo gemini|hello gemini|hi gemini|你好".into(),
            cmd_input_phrases: "start dictation|input|输入".into(),
            cmd_send_phrases: "send|发送".into(),
            cmd_cancel_phrases: "cancel|stop|取消|停止".into(),
            greet_text: "I'm here, what can I do for you?".into(),
            sleep_text: "I'll go to rest then.".into(),
            idle_timeout_secs: 10,
            auto_send_pause_secs: 3,
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// AiWorkspaceHost
// ─────────────────────────────────────────────────────────────────────────────

pub struct AiWorkspaceHost {
    // Sessions
    sessions: Vec<Session>,
    active_session_id: Option<Uuid>,

    // Tool system
    tool_registry: Arc<ToolRegistry>,
    tool_executor: AsyncToolExecutor,
    next_tool_request_id: u64,
    tool_allow_always: HashSet<String>,

    // Internal event channel (tool/LLM -> host)
    internal_tx: Sender<(Uuid, SessionEvent)>,
    internal_rx: Receiver<(Uuid, SessionEvent)>,

    // Output channel to UI
    ui_tx: Sender<HostToUi>,

    // Bridge to Bevy runtime (voice playback, settings writes, etc.)
    bevy_tx: Sender<HostToBevy>,

    // Provider configuration
    gemini_settings: GeminiProviderSettings,
    openai_profiles: Vec<OpenAiCompatProfile>,
    openai_profile_idx: usize,

    // Voice assistant state (mirrors bevy setting, updated via UI)
    voice_assistant_enabled: bool,

    // Context intelligence
    agent_index: Arc<RwLock<AgentIndex>>,

    // LLM clients
    chat_backend: ChatBackend,
    gemini_client: Arc<GeminiClient>,
    
    // Auto-feedback queue (tool result -> LLM continuation)
    pending_auto_feedback: Vec<(Uuid, String)>,

    // ─────────────────────────────────────────────────────────────────────────
    // IDE subsystem (Zed-isomorphic)
    // ─────────────────────────────────────────────────────────────────────────
    worktree: Worktree,
    documents: DocumentStore,
    project_panel_visible: bool,
    ide_event_rx: Receiver<IdeEvent>,

    ide_cursor: Option<(PathBuf, u32, u32)>,
    ide_selection: Option<(PathBuf, u32, u32)>,
}

impl AiWorkspaceHost {
    pub fn new(tool_registry: Arc<ToolRegistry>, ui_tx: Sender<HostToUi>, bevy_tx: Sender<HostToBevy>) -> Self {
        let (internal_tx, internal_rx) = unbounded();
        let sessions = Self::load_sessions().unwrap_or_else(|| vec![Session::new()]);
        let active_session_id = sessions.first().map(|s| s.id);
        let tool_allow_always = Self::load_tool_permissions().unwrap_or_default().allow_always;
        let providers = Self::load_providers_settings().unwrap_or_default();
        let gemini_settings = providers.gemini;
        let openai_profiles = if providers.openai_compat_profiles.is_empty() {
            Self::default_profiles()
        } else {
            providers.openai_compat_profiles
        };
        let gemini_client = Arc::new(GeminiClient::new(
            gemini_settings.api_key.clone(),
            gemini_settings.model_pro.clone(),
            gemini_settings.model_flash.clone(),
        ));
        let voice_assistant_enabled = Self::load_voice_assistant_enabled();
        let agent_index = Arc::new(RwLock::new(AgentIndex::new()));

        // Async build index
        if let Ok(cwd) = std::env::current_dir() {
            let idx = agent_index.clone();
            std::thread::spawn(move || {
                if let Ok(mut i) = idx.write() { i.build_index(&cwd); }
            });
        }

        // IDE subsystem: create event channel and components
        let (ide_event_tx, ide_event_rx) = unbounded();
        let mut worktree = Worktree::new(ide_event_tx.clone());
        let documents = DocumentStore::new(ide_event_tx);

        // Auto-set root to plugins directory
        if let Ok(cwd) = std::env::current_dir() {
            let plugins_dir = cwd.join("plugins");
            if plugins_dir.exists() { worktree.set_root(plugins_dir); }
            else { worktree.set_root(cwd); }
        }

        Self {
            sessions,
            active_session_id,
            tool_registry: tool_registry.clone(),
            tool_executor: AsyncToolExecutor::new(tool_registry),
            next_tool_request_id: 1,
            tool_allow_always,
            internal_tx,
            internal_rx,
            ui_tx,
            bevy_tx,
            gemini_settings,
            openai_profiles,
            openai_profile_idx: 0,
            voice_assistant_enabled,
            agent_index,
            chat_backend: ChatBackend::Gemini,
            gemini_client,
            pending_auto_feedback: Vec::new(),
            worktree,
            documents,
            project_panel_visible: true,
            ide_event_rx,

            ide_cursor: None,
            ide_selection: None,
        }
    }

    fn load_voice_assistant_enabled() -> bool {
        use crate::settings::{SettingValue, SettingsStores};
        let mut stores = SettingsStores::default();
        stores.load();
        match stores.user.get("voice.assistant.enabled") {
            Some(SettingValue::Bool(v)) => *v,
            _ => true,
        }
    }

    fn default_profiles() -> Vec<OpenAiCompatProfile> {
        vec![
            OpenAiCompatProfile::default(),
            OpenAiCompatProfile {
                name: "Kimi (Moonshot)".to_string(),
                base_url: "https://api.moonshot.cn".to_string(),
                api_key: String::new(),
                models: vec!["moonshot-v1-8k".into(), "moonshot-v1-32k".into(), "moonshot-v1-128k".into()],
                selected_model: "moonshot-v1-32k".to_string(),
            },
        ]
    }

    /// Internal event sender for tool/LLM async callbacks
    pub fn internal_tx(&self) -> Sender<(Uuid, SessionEvent)> {
        self.internal_tx.clone()
    }

    /// Poll internal events and process them, emitting HostToUi events
    pub fn poll(&mut self) {
        while let Ok((sid, event)) = self.internal_rx.try_recv() {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == sid) {
                session.pending_events.push_back(event);
            }
        }

        // Process pending events for all sessions (Zed-like parallel)
        let mut batch: Vec<(Uuid, Vec<SessionEvent>)> = Vec::new();
        for session in &mut self.sessions {
            let speed = if session.pending_events.len() > 50 { 10 } else if session.pending_events.len() > 20 { 5 } else { 2 };
            let mut events = Vec::new();
            for _ in 0..speed {
                let Some(event) = session.pending_events.pop_front() else { break; };
                events.push(event);
            }
            if !events.is_empty() {
                batch.push((session.id, events));
            }
        }
        for (session_id, events) in batch {
            for event in events {
                for ev in self.handle_session_event(session_id, event) {
                    let _ = self.ui_tx.send(ev);
                }
            }
        }

        // Process pending auto-feedback (tool result -> LLM continuation)
        if !self.pending_auto_feedback.is_empty() {
            let feedback_list = std::mem::take(&mut self.pending_auto_feedback);
            for (session_id, feedback_text) in feedback_list {
                // Send tool result as a system message to continue the conversation
                self.send_message(session_id, feedback_text, vec![], vec![]);
            }
        }

        // Poll IDE subsystem (worktree background scanner)
        self.worktree.poll();

        // Forward IDE events to UI
        while let Ok(event) = self.ide_event_rx.try_recv() {
            let _ = self.ui_tx.send(HostToUi::IdeEvent(event));
        }
    }

    /// Handle action from UI
    pub fn handle_action(&mut self, action: UiToHost) {
        match action {
            UiToHost::NewSession => {
                let s = Session::new();
                let id = s.id;
                self.sessions.push(s);
                self.active_session_id = Some(id);
                self.save_sessions();
                let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
            }
            UiToHost::SelectSession { session_id } => {
                if self.sessions.iter().any(|s| s.id == session_id) {
                    self.active_session_id = Some(session_id);
                    let _ = self.ui_tx.send(HostToUi::ActiveSessionChanged { session_id: Some(session_id) });
                }
            }
            UiToHost::RenameSession { session_id, title } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    s.title = title;
                    self.save_sessions();
                    let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
                }
            }
            UiToHost::CopySession { session_id } => {
                if let Some(s) = self.sessions.iter().find(|s| s.id == session_id).cloned() {
                    let mut copy = s;
                    copy.id = Uuid::new_v4();
                    copy.title = format!("{} (copy)", copy.title);
                    copy.created_at = std::time::SystemTime::now();
                    let id = copy.id;
                    self.sessions.push(copy);
                    self.active_session_id = Some(id);
                    self.save_sessions();
                    let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
                }
            }
            UiToHost::DeleteSession { session_id } => {
                self.sessions.retain(|s| s.id != session_id);
                if self.active_session_id == Some(session_id) {
                    self.active_session_id = self.sessions.first().map(|s| s.id);
                }
                if self.sessions.is_empty() {
                    self.sessions.push(Session::new());
                    self.active_session_id = self.sessions.first().map(|s| s.id);
                }
                self.save_sessions();
                let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
            }
            UiToHost::SendMessage { session_id, text, mentions, images } => {
                self.send_message(session_id, text, mentions, images);
            }
            UiToHost::SendMessageToActive { text, mentions, images } => {
                let sid = self
                    .active_session_id
                    .or_else(|| self.sessions.first().map(|s| s.id))
                    .unwrap_or_else(|| {
                        let s = Session::new();
                        let id = s.id;
                        self.sessions.push(s);
                        self.active_session_id = Some(id);
                        id
                    });
                self.active_session_id = Some(sid);
                self.send_message(sid, text, mentions, images);
            }
            UiToHost::AbortSession { session_id } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    if let Some(tx) = s.abort_sender.take() {
                        let _ = tx.send(());
                    }
                    s.clear_busy();
                    let _ = self.ui_tx.send(HostToUi::SessionEvent {
                        session_id,
                        event: SessionEventSnapshot::BusyChanged { is_busy: false, reason: None, stage: BusyStageSnapshot::Idle },
                    });
                }
            }
            UiToHost::LocalAssistantMessageToActive { text } => {
                let sid = self
                    .active_session_id
                    .or_else(|| self.sessions.first().map(|s| s.id))
                    .unwrap_or_else(|| {
                        let s = Session::new();
                        let id = s.id;
                        self.sessions.push(s);
                        self.active_session_id = Some(id);
                        id
                    });
                self.active_session_id = Some(sid);
                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs() as i64)
                    .unwrap_or(0);
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == sid) {
                    let entry = ThreadEntry::Assistant {
                        thinking: None,
                        content: text,
                        state: MessageState::Done,
                        timestamp: ts,
                    };
                    s.entries.push(entry.clone());
                    let idx = s.entries.len().saturating_sub(1);
                    let _ = self.ui_tx.send(HostToUi::SessionEvent {
                        session_id: sid,
                        event: SessionEventSnapshot::EntryAdded {
                            index: idx,
                            entry: Self::entry_snapshot(&entry),
                        },
                    });
                    self.save_sessions();
                }
            }
            UiToHost::CancelTool { session_id, request_id } => {
                if let Some(s) = self.sessions.iter_mut().find(|s| s.id == session_id) {
                    if let Some(token) = s.tool_cancel_tokens.remove(&request_id) {
                        token.cancel();
                    }
                    if let Some(ThreadEntry::ToolCall(c)) = s.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
                        c.mark_cancelled();
                    }
                    let _ = self.ui_tx.send(HostToUi::SessionEvent {
                        session_id,
                        event: SessionEventSnapshot::ToolExecutionCancelled { request_id },
                    });
                }
            }
            UiToHost::ApproveTool { session_id, request_id, remember } => {
                self.approve_tool(session_id, request_id, remember);
            }
            UiToHost::DenyTool { session_id, request_id } => {
                self.deny_tool(session_id, request_id);
            }
            UiToHost::SelectBackend { backend } => {
                use super::protocol::BackendType;
                self.chat_backend = match backend {
                    BackendType::Gemini => ChatBackend::Gemini,
                    BackendType::OpenAiCompat => ChatBackend::OpenAiCompat,
                };
                let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                    profiles: self.provider_snapshots(),
                    active_idx: self.active_provider_idx(),
                });
            }
            UiToHost::SelectProvider { profile_idx } => {
                // profile_idx 0 = Gemini, 1+ = OpenAI profiles
                if profile_idx == 0 {
                    self.chat_backend = ChatBackend::Gemini;
                } else if profile_idx - 1 < self.openai_profiles.len() {
                    self.chat_backend = ChatBackend::OpenAiCompat;
                    self.openai_profile_idx = profile_idx - 1;
                }
                let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                    profiles: self.provider_snapshots(),
                    active_idx: self.active_provider_idx(),
                });
            }
            UiToHost::SelectModel { profile_idx, model } => {
                // profile_idx is provider index: 0 = Gemini, 1+ = OpenAI-compat profiles
                if profile_idx == 0 {
                    self.gemini_settings.model_pro = model;
                    if self.gemini_settings.model_flash.trim().is_empty() {
                        self.gemini_settings.model_flash = self.gemini_settings.model_pro.clone();
                    }
                    Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                    let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                        profiles: self.provider_snapshots(),
                        active_idx: self.active_provider_idx(),
                    });
                } else {
                    let idx = profile_idx.saturating_sub(1);
                    if let Some(p) = self.openai_profiles.get_mut(idx) {
                        if p.models.contains(&model) {
                            p.selected_model = model;
                            Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                            let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                                profiles: self.provider_snapshots(),
                                active_idx: self.active_provider_idx(),
                            });
                        }
                    }
                }
            }
            UiToHost::UpdateGeminiSettings { api_key, model_pro, model_flash, model_image } => {
                if !api_key.trim().is_empty() {
                    self.gemini_settings.api_key = api_key;
                }
                self.gemini_settings.model_pro = model_pro;
                self.gemini_settings.model_flash = model_flash;
                self.gemini_settings.model_image = model_image;
                self.gemini_client = Arc::new(GeminiClient::new(
                    self.gemini_settings.api_key.clone(),
                    self.gemini_settings.model_pro.clone(),
                    self.gemini_settings.model_flash.clone(),
                ));
                Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                    profiles: self.provider_snapshots(),
                    active_idx: self.active_provider_idx(),
                });
            }
            UiToHost::UpdateOpenAiCompatProfile { profile_idx, name, base_url, api_key, models, selected_model } => {
                if let Some(p) = self.openai_profiles.get_mut(profile_idx) {
                    p.name = name;
                    p.base_url = base_url;
                    if !api_key.trim().is_empty() {
                        p.api_key = api_key;
                    }
                    p.models = models;
                    p.selected_model = selected_model;
                    Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                    let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                        profiles: self.provider_snapshots(),
                        active_idx: self.active_provider_idx(),
                    });
                }
            }
            UiToHost::AddOpenAiCompatProfile => {
                let mut p = OpenAiCompatProfile::default();
                p.name = format!("Provider {}", self.openai_profiles.len() + 1);
                self.openai_profiles.push(p);
                self.chat_backend = ChatBackend::OpenAiCompat;
                self.openai_profile_idx = self.openai_profiles.len().saturating_sub(1);
                Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                    profiles: self.provider_snapshots(),
                    active_idx: self.active_provider_idx(),
                });
            }
            UiToHost::DeleteOpenAiCompatProfile { profile_idx } => {
                if profile_idx < self.openai_profiles.len() {
                    self.openai_profiles.remove(profile_idx);
                    if self.openai_profiles.is_empty() {
                        self.chat_backend = ChatBackend::Gemini;
                        self.openai_profile_idx = 0;
                    } else {
                        self.chat_backend = ChatBackend::OpenAiCompat;
                        self.openai_profile_idx = self.openai_profile_idx.min(self.openai_profiles.len() - 1);
                    }
                    Self::save_providers_settings(&self.gemini_settings, &self.openai_profiles);
                    let _ = self.ui_tx.send(HostToUi::ProvidersUpdated {
                        profiles: self.provider_snapshots(),
                        active_idx: self.active_provider_idx(),
                    });
                }
            }
            UiToHost::SetVoiceAssistantEnabled { enabled } => {
                self.voice_assistant_enabled = enabled;
                let _ = self.bevy_tx.send(HostToBevy::VoiceSetAssistantEnabled { enabled });
                let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
            }
            UiToHost::UpdateVoiceAssistantSettings { enabled, wake_phrases, cmd_input_phrases, cmd_send_phrases, cmd_cancel_phrases, greet_text, sleep_text, idle_timeout_secs, auto_send_pause_secs } => {
                self.voice_assistant_enabled = enabled;
                let _ = self.bevy_tx.send(HostToBevy::VoiceSetAssistantEnabled { enabled });
                // Save to settings file
                Self::save_voice_assistant_settings(&VoiceAssistantSettingsFile {
                    enabled,
                    wake_phrases: wake_phrases.clone(),
                    cmd_input_phrases: cmd_input_phrases.clone(),
                    cmd_send_phrases: cmd_send_phrases.clone(),
                    cmd_cancel_phrases: cmd_cancel_phrases.clone(),
                    greet_text: greet_text.clone(),
                    sleep_text: sleep_text.clone(),
                    idle_timeout_secs,
                    auto_send_pause_secs,
                });
                let _ = self.ui_tx.send(HostToUi::VoiceAssistantSettingsUpdated(VoiceAssistantSettingsSnapshot {
                    enabled,
                    wake_phrases,
                    cmd_input_phrases,
                    cmd_send_phrases,
                    cmd_cancel_phrases,
                    greet_text,
                    sleep_text,
                    idle_timeout_secs,
                    auto_send_pause_secs,
                }));
            }
            UiToHost::RequestVoiceAssistantSettings => {
                let settings = Self::load_voice_assistant_settings();
                let _ = self.ui_tx.send(HostToUi::VoiceAssistantSettingsUpdated(VoiceAssistantSettingsSnapshot {
                    enabled: settings.enabled,
                    wake_phrases: settings.wake_phrases,
                    cmd_input_phrases: settings.cmd_input_phrases,
                    cmd_send_phrases: settings.cmd_send_phrases,
                    cmd_cancel_phrases: settings.cmd_cancel_phrases,
                    greet_text: settings.greet_text,
                    sleep_text: settings.sleep_text,
                    idle_timeout_secs: settings.idle_timeout_secs,
                    auto_send_pause_secs: settings.auto_send_pause_secs,
                }));
            }
            UiToHost::RequestSnapshot => {
                let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
            }
            UiToHost::Shutdown => {
                self.save_sessions();
                let _ = self.ui_tx.send(HostToUi::Shutdown);
            }

            // ─────────────────────────────────────────────────────────────────
            // IDE: File Tree
            // ─────────────────────────────────────────────────────────────────
            UiToHost::IdeSetRoot { path } => {
                self.worktree.set_root(path);
            }
            UiToHost::IdeRefreshTree => {
                if let Some(root) = self.worktree.root_path().map(|p| p.to_path_buf()) {
                    self.worktree.set_root(root);
                }
            }
            UiToHost::IdeExpandDir { entry_id } => {
                self.worktree.expand(entry_id);
            }
            UiToHost::IdeCollapseDir { entry_id } => {
                self.worktree.collapse(entry_id);
            }
            UiToHost::IdeToggleProjectPanel => {
                self.project_panel_visible = !self.project_panel_visible;
                let _ = self.ui_tx.send(HostToUi::Snapshot(self.build_snapshot()));
            }

            // ─────────────────────────────────────────────────────────────────
            // IDE: Documents
            // ─────────────────────────────────────────────────────────────────
            UiToHost::IdeOpenFile { path } => {
                if let Err(e) = self.documents.open(&path) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::FileError { path, error: e }));
                }
            }
            UiToHost::IdeCloseFile { path } => {
                self.documents.close(&path);
            }
            UiToHost::IdeCloseAllFiles => {
                self.documents.close_all();
            }
            UiToHost::IdeSaveFile { path } => {
                if let Err(e) = self.documents.save(&path) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::FileError { path, error: e }));
                }
            }
            UiToHost::IdeSaveAllFiles => {
                let errors = self.documents.save_all();
                for e in errors {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: e }));
                }
            }
            UiToHost::IdeSetActiveFile { path } => {
                self.documents.set_active(&path);
            }
            UiToHost::IdeEditFile { path, version, edits } => {
                if let Some(doc) = self.documents.get(&path) {
                    if doc.version != version {
                        let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: format!("Version mismatch for {}", path.display()) }));
                        return;
                    }
                }
                if let Err(e) = self.documents.edit(&path, edits) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::FileError { path, error: e }));
                }
            }

            UiToHost::IdeUndo { path } => {
                if let Err(e) = self.documents.undo(&path) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: e }));
                }
            }
            UiToHost::IdeRedo { path } => {
                if let Err(e) = self.documents.redo(&path) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: e }));
                }
            }

            UiToHost::IdeRevealInExplorer { path } => {
                let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
                #[cfg(target_os = "windows")]
                {
                    use std::process::Command;
                    if is_dir {
                        let _ = Command::new("explorer").arg(path).spawn();
                    } else {
                        let _ = Command::new("explorer").arg("/select,").arg(path).spawn();
                    }
                }
                #[cfg(target_os = "macos")]
                {
                    use std::process::Command;
                    if is_dir {
                        let _ = Command::new("open").arg(path).spawn();
                    } else {
                        let _ = Command::new("open").arg("-R").arg(path).spawn();
                    }
                }
                #[cfg(all(unix, not(target_os = "macos")))]
                {
                    use std::process::Command;
                    let open_path = if is_dir {
                        path
                    } else {
                        path.parent().map(|p| p.to_path_buf()).unwrap_or(path)
                    };
                    let _ = Command::new("xdg-open").arg(open_path).spawn();
                }
            }
            UiToHost::IdeRenamePath { from, to } => {
                if !from.exists() {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error {
                        message: format!("Path does not exist: {}", from.display()),
                    }));
                    return;
                }
                if to.exists() {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error {
                        message: format!("Target already exists: {}", to.display()),
                    }));
                    return;
                }
                if let Err(e) = std::fs::rename(&from, &to) {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: e.to_string() }));
                    return;
                }
                if let Some(root) = self.worktree.root_path().map(|p| p.to_path_buf()) {
                    self.worktree.set_root(root);
                }
            }
            UiToHost::IdeDeletePath { path } => {
                if !path.exists() {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error {
                        message: format!("Path does not exist: {}", path.display()),
                    }));
                    return;
                }
                let res = if std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false) {
                    std::fs::remove_dir_all(&path)
                } else {
                    std::fs::remove_file(&path)
                };
                if let Err(e) = res {
                    let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::Error { message: e.to_string() }));
                    return;
                }
                if let Some(root) = self.worktree.root_path().map(|p| p.to_path_buf()) {
                    self.worktree.set_root(root);
                }
            }

            UiToHost::IdeCursorChanged { path, line, col } => {
                self.ide_cursor = Some((path, line, col));
            }
            UiToHost::IdeSelectionChanged {
                path,
                start_line,
                end_line,
            } => {
                self.ide_selection = Some((path, start_line, end_line));
            }
            UiToHost::IdeSelectionCleared { path } => {
                if let Some((p, _, _)) = self.ide_selection.as_ref() {
                    if p == &path {
                        self.ide_selection = None;
                    }
                }
            }

            // ─────────────────────────────────────────────────────────────────
            // IDE: Search / Quick Open / Command Palette
            // ─────────────────────────────────────────────────────────────────
            UiToHost::IdeQuickOpenQuery { query } => {
                let items = self.build_quick_open_results(&query);
                let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::QuickOpenResults { items }));
            }
            UiToHost::IdeGlobalSearch { query: _query, case_sensitive: _case_sensitive, whole_word: _whole_word, regex: _regex } => {
                // TODO: implement ripgrep-like search
                let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::GlobalSearchResults { matches: vec![] }));
            }
            UiToHost::IdeCommandPalette { query } => {
                let commands = self.build_command_palette_results(&query);
                let _ = self.ui_tx.send(HostToUi::IdeEvent(IdeEvent::CommandPaletteResults { commands }));
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Message Sending (Multi-backend: Gemini / OpenAI-compat)
    // ─────────────────────────────────────────────────────────────────────────

    fn send_message(&mut self, session_id: Uuid, text: String, mentions: Vec<MentionSnapshot>, images: Vec<ImageSnapshot>) {
        use crate::tabs_registry::ai_workspace::client::parser::StreamParser;
        use crate::tabs_registry::ai_workspace::context::auto_pack::collect_auto_pack;
        use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

        let mention_uris: Vec<MentionUri> = mentions
            .iter()
            .cloned()
            .filter_map(|m| self.mention_from_snapshot(m))
            .collect();
        let image_attachments: Vec<ImageAttachment> = images.into_iter().map(|i| ImageAttachment {
            mime_type: i.mime_type,
            data_b64: i.data_b64,
            filename: i.filename,
        }).collect();

        // Build context from mentions + auto_pack
        let mut context_parts: Vec<String> = Vec::new();
        
        for m in &mention_uris {
            if let Some(content) = self.expand_mention(m) {
                context_parts.push(content);
            }
        }

        let has_explicit_file_context = mention_uris.iter().any(|m| {
            matches!(
                m,
                MentionUri::File { .. }
                    | MentionUri::Selection { .. }
                    | MentionUri::Symbol { .. }
                    | MentionUri::Directory { .. }
            )
        });

        if !has_explicit_file_context {
            if let Some(ide_ctx) = self.collect_ide_context_pack(180, 20_000) {
                context_parts.push(ide_ctx);
            }
        }
        
        // Auto-pack for plugin-related queries
        if let Ok(cwd) = std::env::current_dir() {
            let auto = collect_auto_pack(&text, &cwd);
            if !auto.is_empty() {
                context_parts.push(auto);
            }
        }
        
        let context = if context_parts.is_empty() { None } else { Some(context_parts.join("\n\n")) };

        // Setup abort channel
        let (abort_tx, abort_rx) = tokio::sync::oneshot::channel::<()>();

        let backend_name = match self.chat_backend {
            ChatBackend::Gemini => "Gemini",
            ChatBackend::OpenAiCompat => self.openai_profiles.get(self.openai_profile_idx).map(|p| p.name.as_str()).unwrap_or("OpenAI"),
        };

        let (entries_clone, images_clone) = {
            let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else { return; };
            let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
            session.entries.push(ThreadEntry::User { text: text.clone(), images: image_attachments.clone(), mentions: mention_uris, timestamp: ts });
            session.entries.push(ThreadEntry::Assistant { thinking: None, content: String::new(), state: MessageState::Pending, timestamp: ts });
            session.abort_sender = Some(abort_tx);
            session.set_busy(&format!("Connecting to {}...", backend_name));
            (session.entries.clone(), image_attachments)
        };

        let idx = {
            let session = self.sessions.iter().find(|s| s.id == session_id).unwrap();
            session.entries.len() - 1
        };

        // Notify UI of new entries
        let prev = entries_clone[idx - 1].clone();
        let last = entries_clone[idx].clone();
        let _ = self.ui_tx.send(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::EntryAdded { index: idx - 1, entry: Self::entry_snapshot(&prev) } });
        let _ = self.ui_tx.send(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::EntryAdded { index: idx, entry: Self::entry_snapshot(&last) } });
        let _ = self.ui_tx.send(HostToUi::SessionEvent {
            session_id,
            event: SessionEventSnapshot::BusyChanged { is_busy: true, reason: Some(format!("Connecting to {}...", backend_name)), stage: BusyStageSnapshot::WaitingModel },
        });

        // Build tool definitions
        let tool_defs: Vec<ToolDefinition> = self.tool_registry.list_definitions();

        let internal_tx = self.internal_tx.clone();
        let text_clone = text.clone();
        let context_clone = context.clone();

        match self.chat_backend {
            ChatBackend::Gemini => {
                // Gemini streaming
                let gemini_client = self.gemini_client.clone();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime");

                    rt.block_on(async move {
                        let mut parser = StreamParser::new();
                        let callback = |event: SessionEvent| {
                            let _ = internal_tx.send((session_id, event));
                        };

                        gemini_client.stream_chat_with_images(
                            &entries_clone[..entries_clone.len() - 1],
                            &text_clone,
                            context_clone.as_deref(),
                            Some(tool_defs),
                            &images_clone,
                            &mut parser,
                            callback,
                            abort_rx,
                        ).await;
                    });
                });
            }
            ChatBackend::OpenAiCompat => {
                // OpenAI-compatible (LM Studio, Kimi, etc.)
                let profile = self.openai_profiles.get(self.openai_profile_idx).cloned().unwrap_or_default();
                std::thread::spawn(move || {
                    let rt = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .expect("Failed to create tokio runtime");

                    rt.block_on(async move {
                        use crate::tabs_registry::ai_workspace::session::prompt::PromptBuilder;
                        let client = OpenAiCompatClient::new(
                            profile.base_url.clone(),
                            profile.selected_model.clone(),
                            profile.api_key.clone(),
                        );

                        let _ = internal_tx.send((session_id, SessionEvent::StartedThoughtProcess));

                        // Build OpenAI-style messages (system + history + tool calls/results), append context to final user message.
                        let hist = &entries_clone[..entries_clone.len() - 1];
                        let mut messages: Vec<serde_json::Value> = Vec::new();
                        messages.push(serde_json::json!({
                            "role": "system",
                            "content": PromptBuilder::build_system_prompt()
                        }));
                        for (i, entry) in hist.iter().enumerate() {
                            let is_last = i + 1 == hist.len();
                            match entry {
                                ThreadEntry::User { text, .. } => {
                                    let content = if is_last {
                                        match context_clone.as_deref() {
                                            Some(ctx) if !ctx.trim().is_empty() => format!("{text_clone}\n\n[Context]\n{ctx}"),
                                            _ => text_clone.clone(),
                                        }
                                    } else {
                                        text.clone()
                                    };
                                    messages.push(serde_json::json!({ "role": "user", "content": content }));
                                }
                                ThreadEntry::Assistant { content, .. } => {
                                    messages.push(serde_json::json!({ "role": "assistant", "content": content.clone() }));
                                }
                                ThreadEntry::ToolCall(c) => {
                                    use crate::tabs_registry::ai_workspace::session::thread_entry::ToolCallStatus as S;
                                    let terminal = matches!(c.status, S::Completed | S::Canceled | S::Failed(_));
                                    if !terminal {
                                        continue;
                                    }
                                    let id = c.tool_use_id();
                                    let raw_in = c.raw_input.as_deref().unwrap_or("{}");
                                    let args_val: serde_json::Value = serde_json::from_str(raw_in)
                                        .unwrap_or_else(|_| serde_json::json!({ "_raw": raw_in }));
                                    let args_str = serde_json::to_string(&args_val).unwrap_or_else(|_| raw_in.to_string());

                                    messages.push(serde_json::json!({
                                        "role": "assistant",
                                        "tool_calls": [{
                                            "id": id,
                                            "type": "function",
                                            "function": { "name": c.tool_name.clone(), "arguments": args_str }
                                        }]
                                    }));

                                    let tr = c.to_tool_result().unwrap_or_else(|| {
                                        crate::tabs_registry::ai_workspace::session::thread_entry::ToolResult {
                                            tool_use_id: c.tool_use_id(),
                                            tool_name: c.tool_name.clone(),
                                            is_error: true,
                                            content: "Tool result missing.".into(),
                                            debug_output: c.raw_output.clone(),
                                        }
                                    });
                                    messages.push(serde_json::json!({
                                        "role": "tool",
                                        "tool_call_id": tr.tool_use_id,
                                        "content": tr.content
                                    }));
                                }
                            }
                        }

                        match client.chat_once_with_tools(messages, tool_defs, 4096).await {
                            Ok(OpenAiCompatChatOutput::ToolCalls(calls)) => {
                                for (tool_name, args) in calls {
                                    let _ = internal_tx.send((
                                        session_id,
                                        SessionEvent::ToolCallRequest { tool_name, args },
                                    ));
                                }
                                let _ = internal_tx.send((session_id, SessionEvent::StreamedCompletion));
                            }
                            Ok(OpenAiCompatChatOutput::Text(response)) => {
                                if !response.is_empty() {
                                    let _ = internal_tx.send((session_id, SessionEvent::Text(response)));
                                }
                                let _ = internal_tx.send((session_id, SessionEvent::StreamedCompletion));
                            }
                            Err(e) => {
                                let _ = internal_tx.send((session_id, SessionEvent::Error(e)));
                            }
                        };
                    });
                });
            }
        }

        self.save_sessions();
    }

    fn collect_ide_context_pack(&self, max_lines: usize, max_chars: usize) -> Option<String> {
        fn push_bounded(dst: &mut String, s: &str, max_chars: usize) {
            if dst.len() >= max_chars {
                return;
            }
            let remaining = max_chars.saturating_sub(dst.len());
            if s.len() <= remaining {
                dst.push_str(s);
            } else {
                dst.push_str(&s[..remaining]);
            }
        }

        fn excerpt_lines(content: &str, start_line: usize, end_line_inclusive: usize) -> String {
            let lines: Vec<&str> = content.lines().collect();
            if lines.is_empty() {
                return String::new();
            }
            let start = start_line.min(lines.len().saturating_sub(1));
            let end_excl = end_line_inclusive.saturating_add(1).min(lines.len());
            if start >= end_excl {
                return String::new();
            }
            lines[start..end_excl].join("\n")
        }

        let active_path = self
            .documents
            .active_path()
            .map(|p| p.to_path_buf())
            .or_else(|| self.ide_cursor.as_ref().map(|(p, _, _)| p.clone()));

        let Some(active_path) = active_path else {
            return None;
        };

        let open_files = self.documents.open_files();
        let mut out = String::new();
        push_bounded(&mut out, "[IDE Context]\n", max_chars);

        if !open_files.is_empty() {
            push_bounded(&mut out, "// Open IDE files (unsaved changes may exist):\n", max_chars);
            for f in open_files.iter().take(20) {
                push_bounded(&mut out, &format!("// - {}{}\n", f.path.display(), if f.is_dirty { " (dirty)" } else { "" }), max_chars);
            }
        }

        let (cursor_line, cursor_col) = self
            .ide_cursor
            .as_ref()
            .and_then(|(p, l, c)| (p == &active_path).then_some((*l as usize, *c as usize)))
            .unwrap_or((0, 0));

        let content = if let Some(doc) = self.documents.get(&active_path) {
            doc.content.clone()
        } else {
            std::fs::read_to_string(&active_path).unwrap_or_default()
        };

        if !content.is_empty() {
            let lines: Vec<&str> = content.lines().collect();
            let total_lines = lines.len().max(1);
            let half = max_lines / 2;
            let start = cursor_line.saturating_sub(half);
            let end = (start + max_lines).min(total_lines).saturating_sub(1);
            let snippet = excerpt_lines(&content, start, end);
            push_bounded(
                &mut out,
                &format!(
                    "\n// @active_file: {}\n// @cursor: L{} C{}\n// @excerpt: L{}-L{}\n```\n{}\n```\n",
                    active_path.display(),
                    cursor_line.saturating_add(1),
                    cursor_col.saturating_add(1),
                    start.saturating_add(1),
                    end.saturating_add(1),
                    snippet
                ),
                max_chars,
            );
        }

        if let Some((p, s, e)) = self.ide_selection.as_ref() {
            if p == &active_path {
                let sel = excerpt_lines(&content, *s as usize, *e as usize);
                if !sel.is_empty() {
                    push_bounded(
                        &mut out,
                        &format!(
                            "\n// @selection: {}:L{}-L{}\n```\n{}\n```\n",
                            active_path.display(),
                            s.saturating_add(1),
                            e.saturating_add(1),
                            sel
                        ),
                        max_chars,
                    );
                }
            }
        }

        if out.trim().is_empty() {
            None
        } else {
            Some(out)
        }
    }

    fn mention_from_snapshot(&self, m: MentionSnapshot) -> Option<MentionUri> {
        match m {
            MentionSnapshot::File { path } => Some(MentionUri::File { path }),
            MentionSnapshot::Directory { path } => Some(MentionUri::Directory { path }),
            MentionSnapshot::Selection {
                path,
                start_line,
                end_line,
            } => {
                if let Some(path) = path {
                    Some(MentionUri::Selection {
                        path: Some(path),
                        start_line,
                        end_line,
                    })
                } else if let Some((p, s, e)) = self.ide_selection.as_ref() {
                    Some(MentionUri::Selection {
                        path: Some(p.to_string_lossy().to_string()),
                        start_line: *s,
                        end_line: *e,
                    })
                } else {
                    None
                }
            }
            MentionSnapshot::Symbol {
                path,
                name,
                start_line,
                end_line,
            } => Some(MentionUri::Symbol {
                path,
                name,
                start_line,
                end_line,
            }),
            MentionSnapshot::Diagnostics { errors, warnings } => {
                Some(MentionUri::Diagnostics { errors, warnings })
            }
            MentionSnapshot::PastedImage { id } => Some(MentionUri::PastedImage { id }),
            MentionSnapshot::Fetch { url } => Some(MentionUri::Fetch { url }),
        }
    }

    fn expand_mention(&self, m: &MentionUri) -> Option<String> {
        match m {
            MentionUri::File { path } => std::fs::read_to_string(path).ok().map(|content| {
                format!("// @file: {}\n```\n{}\n```", path, content)
            }),
            MentionUri::Directory { path } => {
                let mut files = Vec::new();
                if let Ok(entries) = std::fs::read_dir(path) {
                    for entry in entries.flatten().take(20) {
                        files.push(entry.file_name().to_string_lossy().to_string());
                    }
                }
                Some(format!("// @dir: {}\nFiles: {}", path, files.join(", ")))
            }
            MentionUri::Selection {
                path,
                start_line,
                end_line,
            } => {
                let Some(p) = path else { return None; };
                std::fs::read_to_string(p).ok().and_then(|content| {
                    let lines: Vec<&str> = content.lines().collect();
                    let start = *start_line as usize;
                    let end_exclusive = (*end_line as usize).saturating_add(1).min(lines.len());
                    if start < end_exclusive {
                        let snippet = lines[start..end_exclusive].join("\n");
                        Some(format!(
                            "// @file: {}:L{}-{}\n```\n{}\n```",
                            p,
                            start_line.saturating_add(1),
                            end_line.saturating_add(1),
                            snippet
                        ))
                    } else {
                        None
                    }
                })
            }
            MentionUri::Symbol {
                path,
                name,
                start_line,
                end_line,
            } => std::fs::read_to_string(path).ok().and_then(|content| {
                let lines: Vec<&str> = content.lines().collect();
                let start = *start_line as usize;
                let end_exclusive = (*end_line as usize).saturating_add(1).min(lines.len());
                if start < end_exclusive {
                    let snippet = lines[start..end_exclusive].join("\n");
                    Some(format!(
                        "// @symbol: {} ({}:L{}-{})\n```\n{}\n```",
                        name,
                        path,
                        start_line.saturating_add(1),
                        end_line.saturating_add(1),
                        snippet
                    ))
                } else {
                    None
                }
            }),
            MentionUri::Diagnostics { errors, warnings } => Some(format!(
                "[Diagnostics] errors={}, warnings={}",
                errors, warnings
            )),
            MentionUri::PastedImage { .. } | MentionUri::Fetch { .. } => None,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Tool Approval/Deny
    // ─────────────────────────────────────────────────────────────────────────

    fn approve_tool(&mut self, session_id: Uuid, request_id: u64, remember: bool) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else { return; };
        let meta = session.tool_request_meta.get(&request_id).cloned();
        let Some(meta) = meta else { return; };

        if remember {
            self.tool_allow_always.insert(meta.tool_name.clone());
            Self::save_tool_permissions_static(&self.tool_allow_always);
        }

        if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
            c.mark_running();
            c.push_log("Approved. Executing...", ToolLogLevel::Info);
        }

        session.set_busy(format!("Tool: {}", meta.tool_name));
        session.busy_stage = BusyStage::ToolRunning;

        let _ = self.ui_tx.send(HostToUi::SessionEvent {
            session_id,
            event: SessionEventSnapshot::ToolExecutionStarted { request_id },
        });

        // Execute tool
        self.execute_tool(session_id, request_id, meta.tool_name, meta.args);
    }

    fn deny_tool(&mut self, session_id: Uuid, request_id: u64) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else { return; };

        if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
            c.mark_rejected("Denied by user.".into());
            c.push_log("Denied by user.", ToolLogLevel::Info);
        }
        session.tool_request_meta.remove(&request_id);
        session.clear_busy();

        let _ = self.ui_tx.send(HostToUi::SessionEvent {
            session_id,
            event: SessionEventSnapshot::ToolRejected { request_id, reason: "Denied by user.".into() },
        });
        let _ = self.ui_tx.send(HostToUi::SessionEvent {
            session_id,
            event: SessionEventSnapshot::BusyChanged { is_busy: false, reason: None, stage: BusyStageSnapshot::Idle },
        });
        self.save_sessions();
    }

    fn execute_tool(&mut self, session_id: Uuid, request_id: u64, tool_name: String, args: serde_json::Value) {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else { return; };

        if self.tool_registry.is_long_running(&tool_name) {
            let cancel = CancellationToken::new();
            session.tool_cancel_tokens.insert(request_id, cancel.clone());
            let rx = self.tool_executor.submit(request_id, tool_name.clone(), args.clone(), cancel);
            let sid = session_id;
            let tx = self.internal_tx.clone();
            std::thread::spawn(move || {
                while let Ok((_id, r)) = rx.recv() {
                    match r {
                        ToolExecResult::Progress(log) => {
                            let _ = tx.send((sid, SessionEvent::ToolExecutionProgress { request_id, log }));
                        }
                        ToolExecResult::Success(output) => {
                            let _ = tx.send((sid, SessionEvent::ToolExecutionSuccess { request_id, output }));
                            break;
                        }
                        ToolExecResult::Error(e) => {
                            let _ = tx.send((sid, SessionEvent::ToolExecutionError { request_id, error: e.to_string() }));
                            break;
                        }
                        ToolExecResult::Cancelled => {
                            let _ = tx.send((sid, SessionEvent::ToolExecutionCancelled { request_id }));
                            break;
                        }
                    }
                }
            });
        } else {
            // Sync execution
            if let Some(tool) = self.tool_registry.get(&tool_name) {
                match tool.execute(args) {
                    Ok(output) => {
                        self.internal_tx.send((session_id, SessionEvent::ToolExecutionSuccess { request_id, output })).ok();
                    }
                    Err(e) => {
                        self.internal_tx.send((session_id, SessionEvent::ToolExecutionError { request_id, error: e.to_string() })).ok();
                    }
                }
            }
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Event Handling (converts SessionEvent to HostToUi events)
    // ─────────────────────────────────────────────────────────────────────────

    fn handle_session_event(&mut self, session_id: Uuid, event: SessionEvent) -> Vec<HostToUi> {
        let Some(session) = self.sessions.iter_mut().find(|s| s.id == session_id) else { return vec![]; };
        let mut out = Vec::new();

        match event {
            SessionEvent::TitleUpdated(title) => {
                session.title = title.clone();
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::TitleUpdated(title) });
            }
            SessionEvent::StartedThoughtProcess => {
                if let Some(ThreadEntry::Assistant { thinking, state, .. }) = session.entries.last_mut() {
                    *state = MessageState::Streaming;
                    *thinking = Some(ThinkingSection::default());
                }
                session.set_busy("Generating...");
                session.busy_stage = BusyStage::Generating;
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::StartedThoughtProcess });
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged {
                    is_busy: true, reason: Some("Generating...".into()), stage: BusyStageSnapshot::Generating
                }});
            }
            SessionEvent::Thinking(text) => {
                if let Some(ThreadEntry::Assistant { thinking, .. }) = session.entries.last_mut() {
                    if let Some(section) = thinking {
                        section.content.push_str(&text);
                    }
                }
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::Thinking(text) });
            }
            SessionEvent::EndedThoughtProcess => {
                if let Some(ThreadEntry::Assistant { thinking, .. }) = session.entries.last_mut() {
                    if let Some(section) = thinking {
                        section.done = true;
                        section.collapsed = true;
                    }
                }
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::EndedThoughtProcess });
            }
            SessionEvent::Text(text) => {
                if let Some(ThreadEntry::Assistant { content, state, .. }) = session.entries.last_mut() {
                    *state = MessageState::Streaming;
                    content.push_str(&text);
                }
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::Text(text) });
            }
            SessionEvent::StreamedCompletion => {
                let tools_running = session.entries.iter().any(|e| matches!(e, ThreadEntry::ToolCall(c) if matches!(c.status, ToolCallStatus::Pending | ToolCallStatus::AwaitingApproval | ToolCallStatus::InProgress)));
                if !tools_running {
                    let mut final_text: Option<String> = None;
                    if let Some(ThreadEntry::Assistant { content, state, .. }) = session.entries.last_mut() {
                        *state = MessageState::Done;
                        final_text = Some(content.clone());
                    }
                    session.clear_busy();
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::StreamedCompletion });
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged {
                        is_busy: false, reason: None, stage: BusyStageSnapshot::Idle
                    }});

                    if self.voice_assistant_enabled {
                        if let Some(text) = final_text.as_deref().and_then(tts_speakable) {
                            let _ = self.bevy_tx.try_send(HostToBevy::VoiceSpeak { text });
                        }
                    }
                    
                    // Auto-generate title for new sessions (first user message)
                    if session.title == "New Session" || session.title.is_empty() {
                        if let Some(ThreadEntry::User { text, .. }) = session.entries.iter().find(|e| matches!(e, ThreadEntry::User { .. })) {
                            let user_text = text.clone();
                            let gemini = self.gemini_client.clone();
                            let tx = self.internal_tx.clone();
                            let sid = session_id;
                            std::thread::spawn(move || {
                                let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
                                rt.block_on(async move {
                                    if let Ok(title) = gemini.generate_title(&user_text).await {
                                        let _ = tx.send((sid, SessionEvent::TitleUpdated(title)));
                                    }
                                });
                            });
                        }
                    }
                    
                    self.save_sessions();
                }
            }
            SessionEvent::ToolCallRequest { tool_name, args } => {
                let preview = serde_json::to_string(&args).unwrap_or_default();
                let request_id = self.next_tool_request_id;
                self.next_tool_request_id = self.next_tool_request_id.wrapping_add(1).max(1);

                if Self::tool_needs_approval_static(&self.tool_allow_always, &tool_name, &args) {
                    session.tool_request_meta.insert(request_id, ToolRequestMeta { tool_name: tool_name.clone(), args: args.clone() });
                    let mut c = ToolCall::new(request_id, tool_name.clone(), preview.clone());
                    c.raw_input = serde_json::to_string(&args).ok();
                    c.mark_awaiting_approval();
                    let insert_ix = session.entries.len().saturating_sub(1);
                    session.entries.insert(insert_ix, ThreadEntry::ToolCall(c));
                    session.set_busy(format!("Awaiting approval: {}", tool_name));
                    session.busy_stage = BusyStage::WaitingModel;
                    if let Some(e) = session.entries.get(insert_ix) {
                        out.push(HostToUi::SessionEvent {
                            session_id,
                            event: SessionEventSnapshot::EntryAdded {
                                index: insert_ix,
                                entry: Self::entry_snapshot(e),
                            },
                        });
                    }
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolCallRequest { tool_name, request_id, args_preview: preview } });
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolAwaitingApproval { request_id } });
                } else {
                    session.tool_request_meta.insert(request_id, ToolRequestMeta { tool_name: tool_name.clone(), args: args.clone() });
                    let mut c = ToolCall::new(request_id, tool_name.clone(), preview.clone());
                    c.raw_input = serde_json::to_string(&args).ok();
                    c.mark_running();
                    let insert_ix = session.entries.len().saturating_sub(1);
                    session.entries.insert(insert_ix, ThreadEntry::ToolCall(c));
                    session.set_busy(format!("Tool: {}", tool_name));
                    session.busy_stage = BusyStage::ToolRunning;
                    if let Some(e) = session.entries.get(insert_ix) {
                        out.push(HostToUi::SessionEvent {
                            session_id,
                            event: SessionEventSnapshot::EntryAdded {
                                index: insert_ix,
                                entry: Self::entry_snapshot(e),
                            },
                        });
                    }
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolCallRequest { tool_name: tool_name.clone(), request_id, args_preview: preview } });
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionStarted { request_id } });
                    self.execute_tool(session_id, request_id, tool_name, args);
                }
            }
            SessionEvent::ToolExecutionStarted { tool_name: _tool_name, request_id } => {
                if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
                    c.mark_running();
                }
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionStarted { request_id } });
            }
            SessionEvent::ToolExecutionProgress { request_id, log } => {
                if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
                    c.logs.push(log.clone());
                }
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionProgress {
                    request_id,
                    log: ToolLogSnapshot { message: log.message, level: Self::log_level_snapshot(log.level) },
                }});
            }
            SessionEvent::ToolExecutionSuccess { request_id, output } => {
                let tool_name = if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
                    c.llm_result = Some(output.llm_text.clone());
                    c.raw_output = Some(output.raw_text.clone());
                    c.diffs = output.ui_diffs.clone();
                    c.logs.extend(output.ui_logs.clone());
                    c.mark_ok();
                    Some(c.tool_name.clone())
                } else {
                    None
                };
                session.tool_request_meta.remove(&request_id);
                session.tool_cancel_tokens.remove(&request_id);
                let llm_text = output.llm_text.clone();
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionSuccess {
                    request_id, llm_result: output.llm_text, raw_output: Some(output.raw_text),
                }});
                
                // Auto-feedback: send tool result back to LLM to continue
                let tools_running = session.entries.iter().any(|e| matches!(e, ThreadEntry::ToolCall(c) if matches!(c.status, ToolCallStatus::Pending | ToolCallStatus::AwaitingApproval | ToolCallStatus::InProgress)));
                if !tools_running {
                    // All tools done, send feedback to LLM
                    let feedback = format!("[Tool Result: {}]\n{}", tool_name.unwrap_or_default(), llm_text);
                    session.set_busy("Processing tool result...");
                    session.busy_stage = BusyStage::ToolFeedback;
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged { 
                        is_busy: true, reason: Some("Processing tool result...".into()), stage: BusyStageSnapshot::ToolFeedback 
                    }});
                    // Queue auto-feedback message
                    self.pending_auto_feedback.push((session_id, feedback));
                }
                self.save_sessions();
            }
            SessionEvent::ToolExecutionError { request_id, error } => {
                let tool_name = if let Some(ThreadEntry::ToolCall(c)) = session
                    .entries
                    .iter_mut()
                    .find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id))
                {
                    c.mark_err(error.clone());
                    Some(c.tool_name.clone())
                } else {
                    None
                };
                session.tool_request_meta.remove(&request_id);
                session.tool_cancel_tokens.remove(&request_id);
                let err_for_feedback = error.clone();
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionError { request_id, error } });

                // Auto-feedback on error too: send tool error back to LLM to continue (explain + propose fix)
                let tools_running = session.entries.iter().any(|e| matches!(e, ThreadEntry::ToolCall(c) if matches!(c.status, ToolCallStatus::Pending | ToolCallStatus::AwaitingApproval | ToolCallStatus::InProgress)));
                if !tools_running {
                    let feedback = format!("[Tool Error: {}]\n{}", tool_name.unwrap_or_default(), err_for_feedback);
                    session.set_busy("Processing tool result...");
                    session.busy_stage = BusyStage::ToolFeedback;
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged {
                        is_busy: true, reason: Some("Processing tool result...".into()), stage: BusyStageSnapshot::ToolFeedback
                    }});
                    self.pending_auto_feedback.push((session_id, feedback));
                } else {
                    session.clear_busy();
                    out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged { is_busy: false, reason: None, stage: BusyStageSnapshot::Idle } });
                }
                self.save_sessions();
            }
            SessionEvent::ToolExecutionCancelled { request_id } => {
                if let Some(ThreadEntry::ToolCall(c)) = session.entries.iter_mut().find(|e| matches!(e, ThreadEntry::ToolCall(tc) if tc.id == request_id)) {
                    c.mark_cancelled();
                }
                session.tool_request_meta.remove(&request_id);
                session.tool_cancel_tokens.remove(&request_id);
                session.clear_busy();
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::ToolExecutionCancelled { request_id } });
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged { is_busy: false, reason: None, stage: BusyStageSnapshot::Idle } });
                self.save_sessions();
            }
            SessionEvent::NetworkRetry { attempt, max_seconds } => {
                session.is_busy = true;
                session.busy_stage = BusyStage::NetworkRetry(attempt);
                session.busy_reason = Some(format!("Network retry #{} (max {}s)", attempt, max_seconds));
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::NetworkRetry { attempt, max_seconds } });
            }
            SessionEvent::NetworkRetryPreparing { next_attempt } => {
                session.is_busy = true;
                session.busy_stage = BusyStage::NetworkRetry(next_attempt);
                session.busy_reason = Some(format!("Preparing retry #{}", next_attempt));
            }
            SessionEvent::Error(e) => {
                session.clear_busy();
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::Error(e) });
                out.push(HostToUi::SessionEvent { session_id, event: SessionEventSnapshot::BusyChanged { is_busy: false, reason: None, stage: BusyStageSnapshot::Idle } });
            }
            SessionEvent::ToolApproval { .. } => {
                // Handled via UiToHost::ApproveTool/DenyTool
            }
        }
        out
    }

    fn log_level_snapshot(level: ToolLogLevel) -> ToolLogLevelSnapshot {
        match level {
            ToolLogLevel::Info => ToolLogLevelSnapshot::Info,
            ToolLogLevel::Success => ToolLogLevelSnapshot::Progress,
            ToolLogLevel::Error => ToolLogLevelSnapshot::Error,
            ToolLogLevel::Warning => ToolLogLevelSnapshot::Warn,
        }
    }

    fn tool_needs_approval_static(allow_always: &HashSet<String>, tool_name: &str, args: &serde_json::Value) -> bool {
        if allow_always.contains(tool_name) { return false; }
        matches!(tool_name, "terminal" | "diagnostics" | "compile_rust_plugin")
            || (tool_name == "apply_rust_nodespec" && args.get("build").and_then(|v| v.as_bool()).unwrap_or(false))
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Snapshot Building
    // ─────────────────────────────────────────────────────────────────────────

    pub fn build_snapshot(&self) -> WorkspaceSnapshot {
        WorkspaceSnapshot {
            sessions: self.sessions.iter().map(|s| self.session_snapshot(s)).collect(),
            active_session_id: self.active_session_id,
            providers: self.provider_snapshots(),
            active_provider_idx: self.active_provider_idx(),
            tool_allow_always: self.tool_allow_always.iter().cloned().collect(),
            voice_assistant_enabled: self.voice_assistant_enabled,
            ide: self.build_ide_snapshot(),
        }
    }

    fn build_ide_snapshot(&self) -> IdeSnapshot {
        IdeSnapshot {
            root_path: self.worktree.root_path().map(|p| p.to_path_buf()),
            project_panel_visible: self.project_panel_visible,
            visible_entries: self.worktree.visible_entries(),
            open_files: self.documents.open_files(),
            active_file: self.documents.active_path().map(|p| p.to_path_buf()),
            recent_files: self.documents.recent_files().to_vec(),
        }
    }

    fn build_quick_open_results(&self, query: &str) -> Vec<QuickOpenItem> {
        let query_lower = query.to_lowercase();
        let mut results = Vec::new();
        for entry in self.worktree.visible_entries() {
            if matches!(entry.kind, EntryKind::File) {
                let name_lower = entry.name.to_lowercase();
                if name_lower.contains(&query_lower) || entry.path.to_string_lossy().to_lowercase().contains(&query_lower) {
                    let score = if name_lower.starts_with(&query_lower) { 1.0 } else if name_lower.contains(&query_lower) { 0.8 } else { 0.5 };
                    results.push(QuickOpenItem { path: entry.path.clone(), name: entry.name.clone(), icon: FileIcon::from_path(&entry.path), score });
                }
            }
        }
        results.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        results.truncate(50);
        results
    }

    fn build_command_palette_results(&self, query: &str) -> Vec<CommandItem> {
        let query_lower = query.to_lowercase();
        let commands = vec![
            CommandItem { id: "new_file".into(), label: "New File".into(), description: Some("Create a new file".into()), keybinding: Some("Ctrl+N".into()) },
            CommandItem { id: "open_file".into(), label: "Open File".into(), description: Some("Open a file".into()), keybinding: Some("Ctrl+O".into()) },
            CommandItem { id: "save_file".into(), label: "Save File".into(), description: Some("Save current file".into()), keybinding: Some("Ctrl+S".into()) },
            CommandItem { id: "save_all".into(), label: "Save All".into(), description: Some("Save all open files".into()), keybinding: Some("Ctrl+Shift+S".into()) },
            CommandItem { id: "close_file".into(), label: "Close File".into(), description: Some("Close current file".into()), keybinding: Some("Ctrl+W".into()) },
            CommandItem { id: "toggle_project_panel".into(), label: "Toggle Project Panel".into(), description: Some("Show/hide file tree".into()), keybinding: Some("Ctrl+B".into()) },
            CommandItem { id: "quick_open".into(), label: "Quick Open".into(), description: Some("Open file by name".into()), keybinding: Some("Ctrl+P".into()) },
            CommandItem { id: "find_in_file".into(), label: "Find in File".into(), description: Some("Search in current file".into()), keybinding: Some("Ctrl+F".into()) },
            CommandItem { id: "find_in_project".into(), label: "Find in Project".into(), description: Some("Search in all files".into()), keybinding: Some("Ctrl+Shift+F".into()) },
            CommandItem { id: "new_session".into(), label: "New AI Session".into(), description: Some("Start a new AI chat session".into()), keybinding: None },
        ];
        if query.is_empty() { return commands; }
        commands.into_iter().filter(|c| c.label.to_lowercase().contains(&query_lower) || c.id.contains(&query_lower)).collect()
    }

    fn session_snapshot(&self, s: &Session) -> SessionSnapshot {
        SessionSnapshot {
            id: s.id,
            title: s.title.clone(),
            is_busy: s.is_busy,
            busy_reason: s.busy_reason.clone(),
            busy_stage: Self::busy_stage_snapshot(s.busy_stage),
            entries: s.entries.iter().map(Self::entry_snapshot).collect(),
            token_usage: TokenUsageSnapshot {
                input_tokens: s.token_usage.input_tokens,
                output_tokens: s.token_usage.output_tokens,
                total_tokens: s.token_usage.total_tokens,
                max_tokens: s.token_usage.max_tokens,
            },
        }
    }

    fn busy_stage_snapshot(stage: BusyStage) -> BusyStageSnapshot {
        match stage {
            BusyStage::Idle => BusyStageSnapshot::Idle,
            BusyStage::ToolRunning => BusyStageSnapshot::ToolRunning,
            BusyStage::ToolFeedback => BusyStageSnapshot::ToolFeedback,
            BusyStage::WaitingModel => BusyStageSnapshot::WaitingModel,
            BusyStage::Generating => BusyStageSnapshot::Generating,
            BusyStage::AutoHeal(c, m) => BusyStageSnapshot::AutoHeal { current: c, max: m },
            BusyStage::NetworkRetry(a) => BusyStageSnapshot::NetworkRetry { attempt: a },
        }
    }

    fn entry_snapshot(e: &ThreadEntry) -> EntrySnapshot {
        match e {
            ThreadEntry::User { text, images, mentions, timestamp } => EntrySnapshot::User {
                text: text.clone(),
                images: images.iter().map(|i| ImageSnapshot { mime_type: i.mime_type.clone(), data_b64: i.data_b64.clone(), filename: i.filename.clone() }).collect(),
                mentions: mentions.iter().map(Self::mention_snapshot).collect(),
                timestamp: Some(*timestamp),
            },
            ThreadEntry::Assistant { thinking, content, state, timestamp } => EntrySnapshot::Assistant {
                thinking: thinking.as_ref().map(|t| ThinkingSnapshot { content: t.content.clone(), collapsed: t.collapsed, done: t.done }),
                content: content.clone(),
                state: Self::message_state_snapshot(state),
                timestamp: Some(*timestamp),
            },
            ThreadEntry::ToolCall(c) => EntrySnapshot::ToolCall(Self::tool_call_snapshot(c)),
        }
    }

    fn mention_snapshot(m: &MentionUri) -> MentionSnapshot {
        match m {
            MentionUri::File { path } => MentionSnapshot::File { path: path.clone() },
            MentionUri::Directory { path } => MentionSnapshot::Directory { path: path.clone() },
            MentionUri::Selection { path, start_line, end_line } => MentionSnapshot::Selection { path: path.clone(), start_line: *start_line, end_line: *end_line },
            MentionUri::Symbol { path, name, start_line, end_line } => MentionSnapshot::Symbol { path: path.clone(), name: name.clone(), start_line: *start_line, end_line: *end_line },
            MentionUri::Diagnostics { errors, warnings } => MentionSnapshot::Diagnostics { errors: *errors, warnings: *warnings },
            MentionUri::PastedImage { id } => MentionSnapshot::PastedImage { id: *id },
            MentionUri::Fetch { url } => MentionSnapshot::Fetch { url: url.clone() },
        }
    }

    fn message_state_snapshot(s: &MessageState) -> MessageStateSnapshot {
        match s {
            MessageState::Pending => MessageStateSnapshot::Pending,
            MessageState::Streaming => MessageStateSnapshot::Streaming,
            MessageState::Done => MessageStateSnapshot::Done,
            MessageState::Error(_) => MessageStateSnapshot::Error,
        }
    }

    fn tool_call_snapshot(c: &ToolCall) -> ToolCallSnapshot {
        ToolCallSnapshot {
            id: c.id,
            tool_name: c.tool_name.clone(),
            kind: Self::tool_kind_snapshot(c.kind),
            status: Self::tool_status_snapshot(&c.status),
            title: c.title.clone(),
            args_preview: c.args_preview.clone(),
            raw_input: c.raw_input.clone(),
            llm_result: c.llm_result.clone(),
            raw_output: c.raw_output.clone(),
            diffs: c.diffs.iter().map(Self::file_diff_snapshot).collect(),
            logs: c.logs.iter().map(|l| ToolLogSnapshot { message: l.message.clone(), level: Self::log_level_snapshot(l.level.clone()) }).collect(),
        }
    }

    fn file_diff_snapshot(d: &crate::tabs_registry::ai_workspace::tools::FileDiff) -> FileDiffSnapshot {
        FileDiffSnapshot {
            file_path: d.file_path.clone(),
            hunks: d.hunks.iter().map(|h| DiffHunkSnapshot {
                old_start: h.old_start,
                old_count: h.old_count,
                new_start: h.new_start,
                new_count: h.new_count,
                lines: h.lines.iter().map(|l| DiffLineSnapshot {
                    kind: match l.kind {
                        crate::tabs_registry::ai_workspace::tools::DiffLineKind::Context => DiffLineKindSnapshot::Context,
                        crate::tabs_registry::ai_workspace::tools::DiffLineKind::Added => DiffLineKindSnapshot::Added,
                        crate::tabs_registry::ai_workspace::tools::DiffLineKind::Removed => DiffLineKindSnapshot::Removed,
                    },
                    line_num_old: l.line_num_old,
                    line_num_new: l.line_num_new,
                    content: l.content.clone(),
                }).collect(),
            }).collect(),
        }
    }

    fn tool_kind_snapshot(k: ToolKind) -> ToolKindSnapshot {
        match k {
            ToolKind::Read => ToolKindSnapshot::Read,
            ToolKind::Search => ToolKindSnapshot::Search,
            ToolKind::Execute => ToolKindSnapshot::Execute,
            ToolKind::Edit => ToolKindSnapshot::Edit,
            ToolKind::Other => ToolKindSnapshot::Other,
        }
    }

    fn tool_status_snapshot(s: &ToolCallStatus) -> ToolCallStatusSnapshot {
        match s {
            ToolCallStatus::Pending => ToolCallStatusSnapshot::Pending,
            ToolCallStatus::AwaitingApproval => ToolCallStatusSnapshot::AwaitingApproval,
            ToolCallStatus::InProgress => ToolCallStatusSnapshot::InProgress,
            ToolCallStatus::Completed => ToolCallStatusSnapshot::Completed,
            ToolCallStatus::Rejected(r) => ToolCallStatusSnapshot::Rejected(r.clone()),
            ToolCallStatus::Failed(e) => ToolCallStatusSnapshot::Failed(e.clone()),
            ToolCallStatus::Canceled => ToolCallStatusSnapshot::Canceled,
        }
    }

    fn provider_snapshots(&self) -> Vec<ProviderSnapshot> {
        use super::protocol::BackendType;
        let env_gemini_key = std::env::var("GEMINI_API_KEY").unwrap_or_default();
        let has_gemini_key = !env_gemini_key.trim().is_empty() || !self.gemini_settings.api_key.trim().is_empty();
        let defaults = Self::default_gemini_settings();
        let pro = if self.gemini_settings.model_pro.trim().is_empty() {
            defaults.model_pro
        } else {
            self.gemini_settings.model_pro.trim().to_string()
        };
        let flash = if self.gemini_settings.model_flash.trim().is_empty() {
            defaults.model_flash
        } else {
            self.gemini_settings.model_flash.trim().to_string()
        };
        let image_model = if self.gemini_settings.model_image.trim().is_empty() {
            defaults.model_image
        } else {
            self.gemini_settings.model_image.trim().to_string()
        };
        let mut providers = vec![
            // Gemini is always index 0
            ProviderSnapshot {
                name: "Gemini".to_string(),
                base_url: "https://generativelanguage.googleapis.com".to_string(),
                models: vec![pro.clone(), flash],
                selected_model: pro,
                has_api_key: has_gemini_key,
                backend_type: BackendType::Gemini,
                image_model: Some(image_model),
            },
        ];
        // OpenAI-compat profiles are index 1+
        providers.extend(self.openai_profiles.iter().map(|p| ProviderSnapshot {
            name: p.name.clone(),
            base_url: p.base_url.clone(),
            models: p.models.clone(),
            selected_model: p.selected_model.clone(),
            has_api_key: !p.api_key.is_empty(),
            backend_type: BackendType::OpenAiCompat,
            image_model: None,
        }));
        providers
    }

    fn active_provider_idx(&self) -> usize {
        match self.chat_backend {
            ChatBackend::Gemini => 0,
            ChatBackend::OpenAiCompat => 1 + self.openai_profile_idx,
        }
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Persistence (shared with pane.rs logic)
    // ─────────────────────────────────────────────────────────────────────────

    fn sessions_path() -> PathBuf {
        std::env::current_dir().unwrap_or_default().join("settings/ai/sessions.json")
    }

    fn providers_path() -> PathBuf {
        std::env::current_dir().unwrap_or_default().join("settings/ai/providers.json")
    }

    fn permissions_path() -> PathBuf {
        std::env::current_dir().unwrap_or_default().join("settings/ai/tool_permissions.json")
    }

    fn load_sessions() -> Option<Vec<Session>> {
        let path = Self::sessions_path();
        std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    fn save_sessions(&self) {
        let path = Self::sessions_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&self.sessions) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn default_gemini_settings() -> GeminiProviderSettings {
        let pro = std::env::var("CUNNING_GEMINI_MODEL_PRO")
            .unwrap_or_else(|_| "gemini-3-pro-preview".to_string());
        let flash = std::env::var("CUNNING_GEMINI_MODEL_FLASH").unwrap_or_else(|_| pro.clone());
        let image = std::env::var("CUNNING_GEMINI_MODEL_IMAGE")
            .unwrap_or_else(|_| "gemini-3-pro-image-preview".to_string());
        GeminiProviderSettings {
            api_key: String::new(),
            model_pro: pro,
            model_flash: flash,
            model_image: image,
        }
    }

    fn load_providers_settings() -> Option<ProvidersSettingsFile> {
        let path = Self::providers_path();
        let raw = std::fs::read_to_string(&path).ok()?;
        let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
        match v {
            // Old format: array of OpenAI-compat profiles
            serde_json::Value::Array(_) => {
                let profiles: Vec<OpenAiCompatProfile> = serde_json::from_value(v).ok()?;
                Some(ProvidersSettingsFile {
                    gemini: Self::default_gemini_settings(),
                    openai_compat_profiles: profiles,
                })
            }
            // New format: object
            serde_json::Value::Object(_) => {
                let mut file: ProvidersSettingsFile = serde_json::from_value(v).ok()?;
                if file.gemini.model_pro.trim().is_empty() {
                    file.gemini.model_pro = Self::default_gemini_settings().model_pro;
                }
                if file.gemini.model_flash.trim().is_empty() {
                    file.gemini.model_flash = file.gemini.model_pro.clone();
                }
                if file.gemini.model_image.trim().is_empty() {
                    file.gemini.model_image = Self::default_gemini_settings().model_image;
                }
                Some(file)
            }
            _ => None,
        }
    }

    fn save_providers_settings(
        gemini: &GeminiProviderSettings,
        openai_profiles: &[OpenAiCompatProfile],
    ) {
        let path = Self::providers_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = ProvidersSettingsFile {
            gemini: gemini.clone(),
            openai_compat_profiles: openai_profiles.to_vec(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn load_tool_permissions() -> Option<ToolPermissionsFile> {
        let path = Self::permissions_path();
        std::fs::read_to_string(&path).ok().and_then(|s| serde_json::from_str(&s).ok())
    }

    fn save_tool_permissions_static(allow_always: &HashSet<String>) {
        let path = Self::permissions_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let file = ToolPermissionsFile { allow_always: allow_always.clone() };
        if let Ok(json) = serde_json::to_string_pretty(&file) {
            let _ = std::fs::write(&path, json);
        }
    }

    fn voice_assistant_settings_path() -> PathBuf {
        PathBuf::from("settings/ai/voice_assistant.json")
    }

    fn load_voice_assistant_settings() -> VoiceAssistantSettingsFile {
        let path = Self::voice_assistant_settings_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    fn save_voice_assistant_settings(settings: &VoiceAssistantSettingsFile) {
        let path = Self::voice_assistant_settings_path();
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(settings) {
            let _ = std::fs::write(&path, json);
        }
    }
}
