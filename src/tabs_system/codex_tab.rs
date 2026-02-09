//! Codex tab: bundled Codex CLI + Cunning3D MCP tools (streamable HTTP).

use bevy_egui::egui;
use crossbeam_channel::{unbounded, Receiver, Sender};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use toml::Value as TomlValue;

use crate::libs::ai_service::ai_defaults::{DEFAULT_QWEN_GGUF, DEFAULT_QWEN_TOKENIZER};
use crate::libs::codex_integration::DEFAULT_CODEX_MCP_URL;
use crate::tabs_registry::ai_workspace::session::message::{
    Message, MessageState, ThinkingSection,
};
use crate::tabs_registry::ai_workspace::session::session::{BusyStage, Session};
use crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry;
use crate::tabs_system::{EditorTab, EditorTabContext};

// Minimal ChatPanel stub (original deleted with egui UI migration)
#[derive(Default)]
pub struct ChatPanel {
    pub input: String,
}
impl ChatPanel {
    pub fn new() -> Self { Self::default() }
    pub fn ui(&mut self, ui: &mut egui::Ui, entries: &[ThreadEntry], is_busy: bool, busy_reason: Option<&str>, busy_stage: BusyStage, token_usage: &crate::tabs_registry::ai_workspace::session::session::TokenUsageInfo) -> Option<UserAction> {
        // Minimal render: input + send button
        ui.horizontal(|ui| {
            ui.text_edit_singleline(&mut self.input);
            if ui.button("Send").clicked() && !self.input.trim().is_empty() {
                let text = std::mem::take(&mut self.input);
                return Some(UserAction::SendMessage { text, images: vec![] });
            }
            if is_busy && ui.button("Stop").clicked() {
                return Some(UserAction::Abort);
            }
            None
        }).inner
    }
}
#[derive(Debug)]
pub enum UserAction {
    SendMessage { text: String, images: Vec<String> },
    Abort,
    SuggestNextNode,
    AutoConnect,
    ExplainNode,
    FixError,
    GenerateFromDesc,
    ClearChat,
}

// Provider presets
const GLM_PROVIDER_ID: &str = "glm";
const GLM_ENV_KEY: &str = "GLM_API_KEY";
const GLM_BASE_URL: &str = "https://open.bigmodel.cn/api/coding/paas/v4";
const GLM_DEFAULT_MODEL: &str = "GLM-4.7";

const GEMINI_PROVIDER_ID: &str = "gemini";
const GEMINI_ENV_KEY: &str = "GEMINI_API_KEY";
const GEMINI_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const GEMINI_DEFAULT_MODEL: &str = "gemini-3-pro-preview";

// MCP tool names (static list - matches what mcp_server.rs registers)
const MCP_TOOLS: &[&str] = &[
    "create_node_folder",
    "write_node_file",
    "patch_node_file",
    "explore_workspace",
    "search_workspace",
    "check_node_compile",
    "reload_plugin",
    "create_node",
    "delete_node",
    "connect_node",
    "set_node_flag",
    "set_parameter",
    "get_graph_state",
    "edit_node_graph",
    "get_geometry_insight",
    "run_graph_script",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CodexMode {
    Assist,
    Cli,
}

type ChildHandle = Arc<Mutex<Option<Child>>>;

pub struct CodexTab {
    mode: CodexMode,
    // Assist (Codex exec)
    chat: ChatPanel,
    session: Session,
    codex_bin: String,
    workspace_dir: String,
    oss: bool,
    local_provider: String,
    qwen_model_path: String,
    qwen_tokenizer_path: String,
    remote_provider_id: String,
    remote_base_url: String,
    remote_model: String,
    remote_api_key: String,
    remote_env_key: String,
    remote_wire_api: String,
    sandbox_mode: String,
    mcp_url: String,
    running: bool,
    rx: Option<Receiver<String>>,
    child_handle: ChildHandle,
    show_tools: bool,
    // CLI runner
    cmd: String,
    auto_scroll: bool,
    cli_running: bool,
    logs: Vec<String>,
    cli_rx: Option<Receiver<String>>,
}

impl CodexTab {
    fn detect_codex_bin() -> String {
        let exe_name = if cfg!(windows) { "codex.exe" } else { "codex" };
        // Try multiple locations
        let candidates: Vec<PathBuf> = vec![
            std::env::current_dir()
                .ok()
                .map(|p| p.join("Ltools").join(exe_name)),
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.join("Ltools").join(exe_name))),
            std::env::current_exe().ok().and_then(|p| {
                p.parent()
                    .and_then(|d| d.parent())
                    .map(|d| d.join("Ltools").join(exe_name))
            }),
            Some(PathBuf::from("Ltools").join(exe_name)),
        ]
        .into_iter()
        .flatten()
        .collect();
        for c in &candidates {
            if c.exists() {
                return c.display().to_string();
            }
        }
        option_env!("CUNNING_CODEX_BIN")
            .unwrap_or("codex")
            .to_string()
    }
}

impl Default for CodexTab {
    fn default() -> Self {
        Self {
            mode: CodexMode::Assist,
            chat: ChatPanel::new(),
            session: Session::new(),
            codex_bin: Self::detect_codex_bin(),
            workspace_dir: String::new(),
            oss: true,
            local_provider: "qwen_inprocess".to_string(),
            qwen_model_path: DEFAULT_QWEN_GGUF.to_string(),
            qwen_tokenizer_path: DEFAULT_QWEN_TOKENIZER.to_string(),
            remote_provider_id: GLM_PROVIDER_ID.to_string(),
            remote_base_url: GLM_BASE_URL.to_string(),
            remote_model: GLM_DEFAULT_MODEL.to_string(),
            remote_api_key: String::new(),
            remote_env_key: GLM_ENV_KEY.to_string(),
            remote_wire_api: "chat".to_string(),
            sandbox_mode: "read-only".to_string(),
            mcp_url: DEFAULT_CODEX_MCP_URL.to_string(),
            running: false,
            rx: None,
            child_handle: Arc::new(Mutex::new(None)),
            show_tools: false,
            cmd: String::new(),
            auto_scroll: false,
            cli_running: false,
            logs: Vec::new(),
            cli_rx: None,
        }
    }
}

impl CodexTab {
    fn push_log(logs: &mut Vec<String>, line: String) {
        logs.push(line);
        const MAX: usize = 2000;
        if logs.len() > MAX {
            logs.drain(0..(logs.len() - MAX));
        }
    }

    fn graph_context(context: &EditorTabContext) -> String {
        let graph = &context.node_graph_res.0;
        let mut out = String::new();
        out.push_str("\n[Graph Context]\n");
        out.push_str(&format!(
            "node_count: {}\nconnection_count: {}\n",
            graph.nodes.len(),
            graph.connections.len()
        ));
        if let Some(id) = graph.display_node {
            if let Some(n) = graph.nodes.get(&id) {
                out.push_str(&format!(
                    "display_node: {} {} [{:?}]\n",
                    id, n.name, n.node_type
                ));
            }
        }
        if context.ui_state.selected_nodes.is_empty() {
            out.push_str("selected_nodes: (none)\n");
        } else {
            out.push_str("selected_nodes:\n");
            for id in context.ui_state.selected_nodes.iter() {
                if let Some(n) = graph.nodes.get(id) {
                    out.push_str(&format!("- {} {} [{:?}]", id, n.name, n.node_type));
                    // Include node parameters for ExplainNode
                    if !n.parameters.is_empty() {
                        out.push_str(" params={");
                        for p in n.parameters.iter().take(5) {
                            out.push_str(&format!("{}:{:?},", p.name, p.value));
                        }
                        out.push('}');
                    }
                    out.push('\n');
                }
            }
        }
        out
    }

    fn node_registry_context(context: &EditorTabContext) -> String {
        let mut out = String::new();
        out.push_str("\n[Available Node Types]\n");
        let Ok(nodes) = context.node_registry.nodes.read() else {
            return out;
        };
        let types: Vec<_> = nodes.keys().take(80).cloned().collect();
        for t in types {
            out.push_str(&format!("- {t}\n"));
        }
        if nodes.len() > 80 {
            out.push_str(&format!("... and {} more\n", nodes.len() - 80));
        }
        out
    }

    fn list_lmodels(cwd: &PathBuf) -> (Vec<String>, Vec<String>) {
        let dir = cwd.join("Lmodels");
        let (mut models, mut tokenizers) = (Vec::new(), Vec::new());
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let Some(name) = e.file_name().to_str().map(str::to_string) else {
                    continue;
                };
                let lower = name.to_ascii_lowercase();
                if lower.ends_with(".gguf") {
                    models.push(format!("Lmodels/{name}"));
                }
                if lower.ends_with(".json") {
                    tokenizers.push(format!("Lmodels/{name}"));
                }
            }
        }
        models.sort();
        tokenizers.sort();
        (models, tokenizers)
    }

    fn resolve_path(cwd: &PathBuf, path: &str) -> String {
        let p = PathBuf::from(path.trim());
        if p.is_absolute() {
            p.display().to_string()
        } else {
            cwd.join(p).display().to_string()
        }
    }

    fn try_load_secret(cwd: &PathBuf, env_key: &str) -> Option<String> {
        if let Ok(v) = std::env::var(env_key) {
            if !v.trim().is_empty() {
                return Some(v);
            }
        }
        let p = cwd.join("settings").join("secrets.toml");
        let s = fs::read_to_string(p).ok()?;
        let toml = s.parse::<TomlValue>().ok()?;
        toml.get(env_key)
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .filter(|v| !v.trim().is_empty())
            .or_else(|| {
                toml.get("secrets")
                    .and_then(|t| t.get(env_key))
                    .and_then(|v| v.as_str())
                    .map(str::to_string)
                    .filter(|v| !v.trim().is_empty())
            })
    }

    fn stop_assist(&mut self) {
        if let Ok(mut guard) = self.child_handle.lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.kill();
            }
            *guard = None;
        }
        self.running = false;
        self.rx = None;
        if let Some(
            crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::Assistant {
                state,
                content,
                ..
            },
        ) = self.session.entries.last_mut()
        {
            content.push_str("\n[Stopped by user]\n");
            *state = MessageState::Done;
        }
        self.session.clear_busy();
    }

    fn clear_chat(&mut self) {
        self.session.entries.clear();
    }

    fn start_assist(&mut self, context: &EditorTabContext, user_text: String) {
        if self.running {
            return;
        }
        if self.workspace_dir.is_empty() {
            self.workspace_dir = std::env::current_dir()
                .unwrap_or_else(|_| PathBuf::from("."))
                .display()
                .to_string();
        }
        self.session
            .set_busy("Codex exec running (MCP tools enabled)");
        self.session.busy_stage = BusyStage::WaitingModel;
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.session.entries.push(
            crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::User {
                text: user_text.clone(),
                images: vec![],
                mentions: vec![],
                timestamp: ts,
            },
        );
        self.session.entries.push(
            crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::Assistant {
                thinking: None,
                content: String::new(),
                state: MessageState::Pending,
                timestamp: ts,
            },
        );
        self.running = true;

        let codex = self.codex_bin.clone();
        let cwd = PathBuf::from(self.workspace_dir.trim());
        let sandbox = self.sandbox_mode.clone();
        let oss = self.oss;
        let provider = self.local_provider.clone();
        let mcp_url = self.mcp_url.clone();
        let qwen_model_path = Self::resolve_path(&cwd, &self.qwen_model_path);
        let qwen_tokenizer_path = Self::resolve_path(&cwd, &self.qwen_tokenizer_path);
        let remote_provider_id = self.remote_provider_id.clone();
        let remote_base_url = self.remote_base_url.clone();
        let remote_model = self.remote_model.clone();
        let remote_env_key = self.remote_env_key.clone();
        let remote_wire_api = self.remote_wire_api.clone();
        if !oss && self.remote_api_key.trim().is_empty() {
            if let Some(v) = Self::try_load_secret(&cwd, &self.remote_env_key) {
                self.remote_api_key = v;
            }
        }
        let remote_api_key = self.remote_api_key.clone();
        let graph_ctx = Self::graph_context(context);
        let node_ctx = Self::node_registry_context(context);
        let prompt = format!("{graph_ctx}{node_ctx}\n\n{user_text}");

        let (tx, rx) = unbounded::<String>();
        self.rx = Some(rx);
        let child_handle = self.child_handle.clone();
        std::thread::spawn(move || {
            let out_path = std::env::temp_dir()
                .join(format!("cunning3d_codex_last_{}.txt", uuid::Uuid::new_v4()));
            let mcp_override = format!(
                "mcp_servers.cunning={{ url = '{}' }}",
                mcp_url.replace('\'', "")
            );
            let mut cmd = Command::new(&codex);
            cmd.arg("exec")
                .arg("--skip-git-repo-check")
                .arg("--cd")
                .arg(&cwd)
                .arg("--sandbox")
                .arg(&sandbox)
                .arg("--output-last-message")
                .arg(&out_path)
                .arg("-c")
                .arg(mcp_override)
                .arg(prompt)
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if oss {
                cmd.arg("--oss");
                if !provider.trim().is_empty() {
                    cmd.arg("--local-provider").arg(provider.trim());
                }
                if provider.trim() == "qwen_inprocess" {
                    if !qwen_model_path.trim().is_empty() {
                        cmd.arg("-c").arg(format!(
                            "model_providers.qwen_inprocess.local_model_path='{}'",
                            qwen_model_path.replace('\'', "")
                        ));
                    }
                    if !qwen_tokenizer_path.trim().is_empty() {
                        cmd.arg("-c").arg(format!(
                            "model_providers.qwen_inprocess.local_tokenizer_path='{}'",
                            qwen_tokenizer_path.replace('\'', "")
                        ));
                    }
                }
            } else {
                if !remote_provider_id.trim().is_empty() {
                    cmd.arg("-c").arg(format!(
                        "model_provider='{}'",
                        remote_provider_id.replace('\'', "")
                    ));
                }
                if !remote_model.trim().is_empty() {
                    cmd.arg("-c")
                        .arg(format!("model='{}'", remote_model.replace('\'', "")));
                }
                if !remote_provider_id.trim().is_empty() {
                    let pid = remote_provider_id.replace('\'', "");
                    if !remote_base_url.trim().is_empty() {
                        cmd.arg("-c").arg(format!(
                            "model_providers.{pid}.base_url='{}'",
                            remote_base_url.replace('\'', "")
                        ));
                    }
                    if !remote_wire_api.trim().is_empty() {
                        cmd.arg("-c").arg(format!(
                            "model_providers.{pid}.wire_api='{}'",
                            remote_wire_api.replace('\'', "")
                        ));
                    }
                    cmd.arg("-c")
                        .arg(format!("model_providers.{pid}.name='{}'", pid));
                    cmd.arg("-c")
                        .arg(format!("model_providers.{pid}.requires_openai_auth=false"));
                    if !remote_env_key.trim().is_empty() {
                        cmd.arg("-c").arg(format!(
                            "model_providers.{pid}.env_key='{}'",
                            remote_env_key.replace('\'', "")
                        ));
                    }
                    if !remote_env_key.trim().is_empty() && !remote_api_key.trim().is_empty() {
                        cmd.env(&remote_env_key, &remote_api_key);
                    }
                }
            }
            match cmd.spawn() {
                Ok(mut child) => {
                    let pid_info = format!("INFO| codex pid={}", child.id());
                    let _ = tx.send(pid_info);
                    if let Some(stdout) = child.stdout.take() {
                        let txo = tx.clone();
                        std::thread::spawn(move || {
                            for line in BufReader::new(stdout).lines().flatten() {
                                let _ = txo.send(format!("OUT| {line}"));
                            }
                        });
                    }
                    if let Some(stderr) = child.stderr.take() {
                        let txe = tx.clone();
                        std::thread::spawn(move || {
                            for line in BufReader::new(stderr).lines().flatten() {
                                let _ = txe.send(format!("ERR| {line}"));
                            }
                        });
                    }
                    if let Ok(mut guard) = child_handle.lock() {
                        *guard = Some(child);
                    }
                    // Wait for completion
                    let code = if let Ok(mut guard) = child_handle.lock() {
                        guard
                            .as_mut()
                            .and_then(|c| c.wait().ok())
                            .and_then(|s| s.code())
                            .unwrap_or(-1)
                    } else {
                        -1
                    };
                    if let Ok(mut guard) = child_handle.lock() {
                        *guard = None;
                    }
                    let last = std::fs::read_to_string(&out_path).unwrap_or_default();
                    if !last.trim().is_empty() {
                        let _ = tx.send(format!("FINAL| {last}"));
                    }
                    let _ = tx.send(format!("DONE| exit={code}"));
                }
                Err(e) => {
                    let _ = tx.send(format!("ERR| failed to run codex at '{}': {e}", codex));
                    let _ = tx.send("DONE| exit=-1".to_string());
                }
            }
        });
    }

    fn pump_assist(&mut self) {
        let Some(rx) = self.rx.as_ref() else {
            return;
        };
        let mut done = false;
        let mut final_text: Option<String> = None;
        for line in rx.try_iter() {
            if let Some(rest) = line.strip_prefix("FINAL| ") {
                final_text = Some(rest.to_string());
                continue;
            }
            if line.starts_with("DONE|") {
                done = true;
            }
            if let Some(
                crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::Assistant {
                    content,
                    state,
                    thinking,
                    ..
                },
            ) = self.session.entries.last_mut()
            {
                *state = MessageState::Streaming;
                if thinking.is_none() {
                    *thinking = Some(ThinkingSection {
                        content: "MCP tools enabled".to_string(),
                        collapsed: true,
                        done: true,
                    });
                }
                content.push_str(&line);
                content.push('\n');
            }
        }
        if let Some(t) = final_text {
            if let Some(
                crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::Assistant {
                    content,
                    ..
                },
            ) = self.session.entries.last_mut()
            {
                content.push_str("\n---\n");
                content.push_str(t.trim());
                content.push('\n');
            }
        }
        if done {
            self.running = false;
            self.rx = None;
            if let Some(
                crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry::Assistant {
                    state,
                    ..
                },
            ) = self.session.entries.last_mut()
            {
                *state = MessageState::Done;
            }
            self.session.clear_busy();
        }
    }

    fn start_cli(&mut self, cmd: String) {
        let (tx, rx) = unbounded::<String>();
        self.cli_running = true;
        self.cli_rx = Some(rx);
        std::thread::spawn(move || {
            let mut child = Command::new("powershell")
                .args(["-NoProfile", "-NonInteractive", "-Command", &cmd])
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let Ok(mut child) = child.as_mut() else {
                let _ = tx.send("ERR| failed to spawn process".into());
                let _ = tx.send("DONE| exit=-1".into());
                return;
            };
            if let Some(stdout) = child.stdout.take() {
                let txo = tx.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(stdout).lines().flatten() {
                        let _ = txo.send(format!("OUT| {line}"));
                    }
                });
            }
            if let Some(stderr) = child.stderr.take() {
                let txe = tx.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(stderr).lines().flatten() {
                        let _ = txe.send(format!("ERR| {line}"));
                    }
                });
            }
            let status = child.wait().ok().and_then(|s| s.code()).unwrap_or(-1);
            let _ = tx.send(format!("DONE| exit={status}"));
        });
    }

    fn ui_tools_preview(&mut self, ui: &mut egui::Ui) {
        egui::CollapsingHeader::new("MCP Tools")
            .default_open(false)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    for t in MCP_TOOLS {
                        ui.label(egui::RichText::new(*t).small().monospace());
                    }
                });
            });
    }
}

impl EditorTab for CodexTab {
    fn ui(&mut self, ui: &mut egui::Ui, context: &mut EditorTabContext) {
        ui.painter()
            .rect_filled(ui.clip_rect(), 0.0, ui.visuals().panel_fill);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.mode, CodexMode::Assist, "Assist (MCP)");
            ui.selectable_value(&mut self.mode, CodexMode::Cli, "CLI");
        });
        ui.separator();

        match self.mode {
            CodexMode::Assist => {
                self.pump_assist();
                ui.horizontal(|ui| {
                    ui.label("codex");
                    let exists = PathBuf::from(&self.codex_bin).exists();
                    let color = if exists {
                        egui::Color32::LIGHT_GREEN
                    } else {
                        egui::Color32::LIGHT_RED
                    };
                    ui.colored_label(color, if exists { "✓" } else { "✗" });
                    ui.text_edit_singleline(&mut self.codex_bin);
                    ui.label("workspace");
                    ui.text_edit_singleline(&mut self.workspace_dir);
                });
                ui.horizontal(|ui| {
                    ui.label("mcp url");
                    ui.text_edit_singleline(&mut self.mcp_url);
                    ui.label("sandbox");
                    ui.text_edit_singleline(&mut self.sandbox_mode);
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.oss, "oss/local");
                    if self.oss {
                        ui.label("local-provider");
                        ui.text_edit_singleline(&mut self.local_provider);
                    } else {
                        if ui.button("GLM").on_hover_text("Zhipu GLM-4.7").clicked() {
                            self.remote_provider_id = GLM_PROVIDER_ID.to_string();
                            self.remote_base_url = GLM_BASE_URL.to_string();
                            self.remote_model = GLM_DEFAULT_MODEL.to_string();
                            self.remote_env_key = GLM_ENV_KEY.to_string();
                            self.remote_wire_api = "chat".to_string();
                        }
                        if ui
                            .button("Gemini")
                            .on_hover_text("Google Gemini 3 Pro")
                            .clicked()
                        {
                            self.remote_provider_id = GEMINI_PROVIDER_ID.to_string();
                            self.remote_base_url = GEMINI_BASE_URL.to_string();
                            self.remote_model = GEMINI_DEFAULT_MODEL.to_string();
                            self.remote_env_key = GEMINI_ENV_KEY.to_string();
                            self.remote_wire_api = "chat".to_string();
                        }
                        ui.label("provider");
                        ui.text_edit_singleline(&mut self.remote_provider_id);
                    }
                });
                if self.oss && self.local_provider.trim() == "qwen_inprocess" {
                    if self.qwen_model_path.trim().is_empty() {
                        self.qwen_model_path = DEFAULT_QWEN_GGUF.to_string();
                    }
                    if self.qwen_tokenizer_path.trim().is_empty() {
                        self.qwen_tokenizer_path = DEFAULT_QWEN_TOKENIZER.to_string();
                    }
                    let cwd = if self.workspace_dir.trim().is_empty() {
                        std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                    } else {
                        PathBuf::from(self.workspace_dir.trim())
                    };
                    let (models, tokenizers) = Self::list_lmodels(&cwd);
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("model")
                            .selected_text(self.qwen_model_path.clone())
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(false, DEFAULT_QWEN_GGUF).clicked() {
                                    self.qwen_model_path = DEFAULT_QWEN_GGUF.to_string();
                                }
                                for m in &models {
                                    ui.selectable_value(&mut self.qwen_model_path, m.clone(), m);
                                }
                            });
                    });
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_label("tokenizer")
                            .selected_text(self.qwen_tokenizer_path.clone())
                            .show_ui(ui, |ui| {
                                if ui.selectable_label(false, DEFAULT_QWEN_TOKENIZER).clicked() {
                                    self.qwen_tokenizer_path = DEFAULT_QWEN_TOKENIZER.to_string();
                                }
                                for t in &tokenizers {
                                    ui.selectable_value(
                                        &mut self.qwen_tokenizer_path,
                                        t.clone(),
                                        t,
                                    );
                                }
                            });
                    });
                } else if !self.oss {
                    if self.remote_api_key.trim().is_empty() {
                        let cwd = if self.workspace_dir.trim().is_empty() {
                            std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
                        } else {
                            PathBuf::from(self.workspace_dir.trim())
                        };
                        if let Some(v) = Self::try_load_secret(&cwd, &self.remote_env_key) {
                            self.remote_api_key = v;
                        }
                    }
                    ui.horizontal(|ui| {
                        ui.label("base_url");
                        ui.text_edit_singleline(&mut self.remote_base_url);
                    });
                    ui.horizontal(|ui| {
                        ui.label("model");
                        ui.text_edit_singleline(&mut self.remote_model);
                    });
                    ui.horizontal(|ui| {
                        ui.label("wire_api");
                        ui.text_edit_singleline(&mut self.remote_wire_api);
                    });
                    ui.horizontal(|ui| {
                        ui.label("env_key");
                        ui.text_edit_singleline(&mut self.remote_env_key);
                    });
                    ui.horizontal(|ui| {
                        ui.label("api_key");
                        ui.add(egui::TextEdit::singleline(&mut self.remote_api_key).password(true));
                    });
                }
                self.ui_tools_preview(ui);
                ui.separator();

                // Quick Actions bar
                ui.horizontal(|ui| {
                    ui.label("Quick:");
                    if ui.small_button("Explain").on_hover_text("Explain selected node(s)").clicked() {
                        self.start_assist(context, "Explain what the selected node(s) do, including their inputs/outputs and purpose.".to_string());
                    }
                    if ui.small_button("Fix").on_hover_text("Diagnose and fix errors").clicked() {
                        self.start_assist(context, "Check the current graph for potential issues and suggest fixes. Use MCP tools if needed.".to_string());
                    }
                    if ui.small_button("Generate").on_hover_text("Generate node from description").clicked() {
                        self.chat.input = "Create a node that ".to_string();
                    }
                    if ui.small_button("Clear").on_hover_text("Clear chat history").clicked() { self.clear_chat(); }
                });
                ui.separator();

                if let Some(action) = self.chat.ui(
                    ui,
                    &self.session.entries,
                    self.session.is_busy,
                    self.session.busy_reason.as_deref(),
                    self.session.busy_stage,
                    &self.session.token_usage,
                ) {
                    match action {
                        UserAction::SendMessage { text, .. } => self.start_assist(context, text),
                        UserAction::Abort => self.stop_assist(),
                        UserAction::SuggestNextNode => self.start_assist(context, "Suggest the next node to add and APPLY it using MCP tools. If ambiguous, ask one question.".to_string()),
                        UserAction::AutoConnect => self.start_assist(context, "Auto-connect selected nodes if unambiguous; otherwise ask one question. Use MCP tools.".to_string()),
                        UserAction::ExplainNode => self.start_assist(context, "Explain what the selected node(s) do, including their inputs/outputs and purpose.".to_string()),
                        UserAction::FixError => self.start_assist(context, "Check the current graph for potential issues and suggest fixes. Use MCP tools if needed.".to_string()),
                        UserAction::GenerateFromDesc => { self.chat.input = "Create a node that ".to_string(); }
                        UserAction::ClearChat => self.clear_chat(),
                        _ => {}
                    }
                }
            }
            CodexMode::Cli => {
                if let Some(rx) = self.cli_rx.as_ref() {
                    for line in rx.try_iter() {
                        Self::push_log(&mut self.logs, line);
                    }
                }
                if self
                    .logs
                    .last()
                    .map(|l| l.starts_with("DONE|"))
                    .unwrap_or(false)
                {
                    self.cli_running = false;
                    self.cli_rx = None;
                }
                ui.horizontal(|ui| {
                    if self.cmd.is_empty() {
                        self.cmd = format!("& '{}' --help", self.codex_bin);
                    }
                    ui.label("Command:");
                    ui.add_sized(
                        [ui.available_width() - 220.0, 24.0],
                        egui::TextEdit::singleline(&mut self.cmd),
                    );
                    if ui
                        .add_enabled(!self.cli_running, egui::Button::new("Run"))
                        .clicked()
                    {
                        Self::push_log(&mut self.logs, format!("INFO| {}", self.cmd));
                        self.start_cli(self.cmd.clone());
                    }
                });
                ui.horizontal(|ui| {
                    ui.checkbox(&mut self.auto_scroll, "Auto-scroll");
                    if ui
                        .button("Reset Cmd")
                        .on_hover_text("Reset to codex --help with correct path")
                        .clicked()
                    {
                        self.cmd = format!("& '{}' --help", self.codex_bin);
                    }
                    if ui.button("Copy All").clicked() {
                        ui.output_mut(|o| {
                            o.commands
                                .push(egui::OutputCommand::CopyText(self.logs.join("\n")))
                        });
                    }
                    if ui.button("Clear").clicked() {
                        self.logs.clear();
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.auto_scroll)
                    .show(ui, |ui| {
                        if self.logs.is_empty() {
                            ui.colored_label(egui::Color32::GRAY, "No output yet.");
                            return;
                        }
                        for line in &self.logs {
                            let (c, t) = if let Some(rest) = line.strip_prefix("ERR| ") {
                                (egui::Color32::LIGHT_RED, rest)
                            } else if let Some(rest) = line.strip_prefix("OUT| ") {
                                (egui::Color32::GRAY, rest)
                            } else if let Some(rest) = line.strip_prefix("DONE| ") {
                                (egui::Color32::LIGHT_GREEN, rest)
                            } else if let Some(rest) = line.strip_prefix("INFO| ") {
                                (egui::Color32::LIGHT_BLUE, rest)
                            } else {
                                (egui::Color32::WHITE, line.as_str())
                            };
                            ui.colored_label(c, t);
                        }
                    });
            }
        }
    }

    fn title(&self) -> egui::WidgetText {
        "Codex".into()
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
