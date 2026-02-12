use crate::cunning_core::ai_service::gemini::api_key::read_gemini_api_key_env;
use crate::tabs_registry::ai_assistant::prompt::ASSISTANT_SYSTEM_PROMPT;
use crate::tabs_registry::ai_workspace::tools::{
    build_tool_registry, ToolDefinition, ToolOutput, ToolProfile, ToolRegistry,
};
use crate::tabs_system::{EditorTab, EditorTabContext};
use bevy_egui::egui::{
    self, Align, Color32, Frame, Key, Layout, Margin, RichText, ScrollArea, TextEdit, Ui,
    WidgetText,
};
use crossbeam_channel::{unbounded, Receiver, TryRecvError};
use reqwest::blocking::Client;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_GEMINI_MODEL: &str = "gemini-3-flash-preview";
const MAX_AGENT_TURNS: usize = 6;
const MAX_LOG_TEXT_CHARS: usize = 240;
const MAX_VISIBLE_TOOL_LOGS: usize = 12;
const MAX_VISIBLE_TOOL_NAMES: usize = 16;
const MAX_MESSAGES: usize = 260;
const MAX_RENDER_CHARS_PER_MESSAGE: usize = 2200;
const MAX_PROMPT_CHAT_MESSAGES: usize = 40;

const QUICK_ACTIONS: [(&str, &str); 6] = [
    (
        "创建基础布尔组",
        "创建一个 cube，再创建一个 sphere，再创建一个 boolean，把 cube 接到输入1，sphere 接到输入2。",
    ),
    ("创建一个 Cube", "帮我创建一个 cube 节点。"),
    ("创建一个 Sphere", "帮我创建一个 sphere 节点。"),
    ("查看图状态", "帮我读取当前 node graph 状态，并用简洁中文总结。"),
    ("讲解 Boolean 节点", "请讲解 boolean 节点的作用、输入输出和常见用法。"),
    ("列出可用节点", "读取节点库并按类别简要列出可用节点。"),
];

#[derive(Clone, Copy, Debug)]
enum MessageRole {
    User,
    Assistant,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MessageKind {
    Chat,
    ToolLog,
    System,
}

#[derive(Clone, Debug)]
struct ChatMessage {
    role: MessageRole,
    kind: MessageKind,
    text: String,
}

impl ChatMessage {
    fn user(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::User,
            kind: MessageKind::Chat,
            text: text.into(),
        }
    }

    fn assistant(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            kind: MessageKind::Chat,
            text: text.into(),
        }
    }

    fn tool_log(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            kind: MessageKind::ToolLog,
            text: text.into(),
        }
    }

    fn system(text: impl Into<String>) -> Self {
        Self {
            role: MessageRole::Assistant,
            kind: MessageKind::System,
            text: text.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct GeminiSettings {
    api_key: String,
    model: String,
    source_path: Option<PathBuf>,
}

#[derive(Clone, Debug)]
struct FunctionCallRequest {
    name: String,
    args: Value,
}

#[derive(Clone, Debug)]
struct AssistantTurnResult {
    tool_logs: Vec<String>,
    assistant_text: String,
}

pub struct AiAssistantPane {
    tool_registry: Option<Arc<ToolRegistry>>,
    tool_names: Vec<String>,
    gemini_model_label: String,
    provider_source_label: String,
    messages: Vec<ChatMessage>,
    input: String,
    pending_registry_rx: Option<Receiver<Result<(Arc<ToolRegistry>, Vec<String>, GeminiSettings), String>>>,
    pending_response_rx: Option<Receiver<Result<AssistantTurnResult, String>>>,
    is_busy: bool,
    auto_scroll: bool,
    show_tool_catalog: bool,
    last_error: Option<String>,
}

impl Default for AiAssistantPane {
    fn default() -> Self {
        Self {
            tool_registry: None,
            tool_names: Vec::new(),
            gemini_model_label: DEFAULT_GEMINI_MODEL.to_string(),
            provider_source_label: "unknown".to_string(),
            messages: vec![
                ChatMessage::assistant(
                    "你好，我是 AI Assistant。你可以让我创建/连接节点，也可以让我读取知识库讲解节点。",
                ),
                ChatMessage::system("Tip: Use the Send button to submit."),
            ],
            input: String::new(),
            pending_registry_rx: None,
            pending_response_rx: None,
            is_busy: false,
            auto_scroll: true,
            show_tool_catalog: false,
            last_error: None,
        }
    }
}

impl EditorTab for AiAssistantPane {
    fn ui(&mut self, ui: &mut Ui, context: &mut EditorTabContext) {
        self.ensure_registry(context);
        self.poll_pending_response();

        self.render_header(ui);
        ui.add_space(6.0);
        self.render_quick_actions(ui);
        ui.add_space(8.0);

        let total_width = ui.available_width();
        let right_width = (total_width * 0.34).max(220.0).min(320.0);
        let left_width = (total_width - right_width - 10.0).max(240.0);
        let panel_height = (ui.available_height() - 132.0).max(220.0);

        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(left_width, panel_height),
                Layout::top_down(Align::Min),
                |ui| self.render_chat_panel(ui),
            );

            ui.add_space(10.0);

            ui.allocate_ui_with_layout(
                egui::vec2(right_width, panel_height),
                Layout::top_down(Align::Min),
                |ui| self.render_side_panel(ui),
            );
        });

        ui.add_space(8.0);
        self.render_composer(ui);
    }

    fn title(&self) -> WidgetText {
        "AI Assistant".into()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl AiAssistantPane {
    fn ensure_registry(&mut self, context: &EditorTabContext) {
        if self.tool_registry.is_some() {
            return;
        }
        if let Some(rx) = self.pending_registry_rx.as_ref() {
            match rx.try_recv() {
                Ok(Ok((registry, tool_names, settings))) => {
                    self.tool_registry = Some(registry);
                    self.tool_names = tool_names;
                    self.gemini_model_label = settings.model.clone();
                    self.provider_source_label = settings
                        .source_path
                        .as_ref()
                        .map(|path| path.display().to_string())
                        .unwrap_or_else(|| "env-or-default".to_string());
                    self.pending_registry_rx = None;
                }
                Ok(Err(e)) => {
                    self.last_error = Some(e.clone());
                    self.push_message(ChatMessage::system(format!("❌ Tool registry init failed: {e}")));
                    self.pending_registry_rx = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.last_error = Some("Tool registry init thread disconnected".to_string());
                    self.push_message(ChatMessage::system("❌ Tool registry init thread disconnected."));
                    self.pending_registry_rx = None;
                }
            }
            return;
        }

        // Build the tool registry off the egui/UI thread to avoid long stalls that can trigger epaint deadlock detection.
        let node_registry = context.node_registry.clone();
        let (tx, rx) =
            unbounded::<Result<(Arc<ToolRegistry>, Vec<String>, GeminiSettings), String>>();
        std::thread::spawn(move || {
            let registry = Arc::new(build_tool_registry(
                ToolProfile::NodeAssistant,
                Arc::new(node_registry),
            ));
            let tool_names = registry
                .list_definitions()
                .into_iter()
                .map(|definition| definition.name)
                .collect::<Vec<_>>();
            let settings = load_gemini_settings();
            let _ = tx.send(Ok((registry, tool_names, settings)));
        });
        self.pending_registry_rx = Some(rx);
    }

    fn poll_pending_response(&mut self) {
        let Some(receiver) = self.pending_response_rx.as_ref() else {
            return;
        };

        match receiver.try_recv() {
            Ok(result) => {
                self.pending_response_rx = None;
                self.is_busy = false;
                match result {
                    Ok(turn) => {
                        self.last_error = None;
                        for tool_log in turn.tool_logs {
                            self.push_message(ChatMessage::tool_log(format!("🔧 {tool_log}")));
                        }
                        if !turn.assistant_text.trim().is_empty() {
                            self.push_message(ChatMessage::assistant(turn.assistant_text));
                        }
                    }
                    Err(error) => {
                        self.last_error = Some(error.clone());
                        self.push_message(ChatMessage::system(format!("❌ {error}")));
                    }
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                self.pending_response_rx = None;
                self.is_busy = false;
                self.last_error = Some("后台线程已断开".to_string());
                self.push_message(ChatMessage::system("❌ 后台请求线程已断开。"));
            }
        }
    }

    fn render_header(&mut self, ui: &mut Ui) {
        let tool_count = self.tool_names.len();
        let chat_count = self
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::Chat)
            .count();

        Frame::group(ui.style())
            .fill(Color32::from_rgb(20, 28, 36))
            .stroke(egui::Stroke::new(1.0, Color32::from_rgb(50, 68, 84)))
            .inner_margin(Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(
                        RichText::new("AI Assistant")
                            .strong()
                            .color(Color32::from_rgb(135, 215, 255))
                            .size(16.0),
                    );

                    let status_text = if self.is_busy { "运行中" } else { "空闲" };
                    let status_color = if self.is_busy {
                        Color32::from_rgb(255, 204, 128)
                    } else {
                        Color32::from_rgb(143, 255, 171)
                    };
                    ui.label(
                        RichText::new(format!("● {status_text}"))
                            .color(status_color)
                            .strong(),
                    );
                    ui.separator();
                    ui.label(
                        RichText::new(format!("Model: {}", self.gemini_model_label))
                            .weak()
                            .small(),
                    );
                    ui.separator();
                    ui.label(RichText::new(format!("Tools: {tool_count}")).weak().small());
                    ui.separator();
                    ui.label(RichText::new(format!("Messages: {chat_count}")).weak().small());
                    ui.separator();
                    ui.label(
                        RichText::new(format!(
                            "Providers: {}",
                            truncate_chars(&self.provider_source_label, 44)
                        ))
                        .weak()
                        .small(),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .add_enabled(!self.is_busy, egui::Button::new("刷新配置"))
                            .clicked()
                        {
                            let settings = load_gemini_settings();
                            self.gemini_model_label = settings.model.clone();
                            self.provider_source_label = settings
                                .source_path
                                .as_ref()
                                .map(|path| path.display().to_string())
                                .unwrap_or_else(|| "env-or-default".to_string());
                        }
                    });
                });
            });
    }

    fn render_quick_actions(&mut self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("快捷动作").strong());
                    for (label, prompt) in QUICK_ACTIONS {
                        let clicked = ui
                            .add_enabled(!self.is_busy, egui::Button::new(label))
                            .clicked();
                        if clicked {
                            self.submit_user_message(prompt.to_string());
                        }
                    }
                });
            });
    }

    fn render_chat_panel(&mut self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("对话").strong());
                    ui.separator();
                    ui.checkbox(&mut self.auto_scroll, "自动滚动");
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("清空会话").clicked() && !self.is_busy {
                            self.reset_chat();
                        }
                        if ui.button("复制最后回复").clicked() {
                            if let Some(last_reply) = self.last_assistant_reply() {
                                ui.output_mut(|output| {
                                    output
                                        .commands
                                        .push(egui::OutputCommand::CopyText(last_reply.to_string()));
                                });
                            }
                        }
                    });
                });

                ui.separator();

                ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .stick_to_bottom(self.auto_scroll)
                    .show(ui, |ui| {
                        if self.messages.is_empty() {
                            ui.label(RichText::new("暂无消息").italics().weak());
                            return;
                        }

                        for message in &self.messages {
                            self.render_message_row(ui, message);
                            ui.add_space(6.0);
                        }
                    });
            });
    }

    fn render_message_row(&self, ui: &mut Ui, message: &ChatMessage) {
        let align_right = matches!(message.role, MessageRole::User);
        let layout = if align_right {
            Layout::right_to_left(Align::Min)
        } else {
            Layout::left_to_right(Align::Min)
        };

        ui.with_layout(layout, |ui| {
            let (title, fill, border) = match (message.role, message.kind) {
                (_, MessageKind::ToolLog) => (
                    "Tool Log",
                    Color32::from_rgb(30, 37, 48),
                    Color32::from_rgb(70, 84, 104),
                ),
                (_, MessageKind::System) => (
                    "System",
                    Color32::from_rgb(42, 30, 30),
                    Color32::from_rgb(120, 72, 72),
                ),
                (MessageRole::User, _) => (
                    "你",
                    Color32::from_rgb(43, 74, 111),
                    Color32::from_rgb(84, 130, 178),
                ),
                (MessageRole::Assistant, _) => (
                    "助手",
                    Color32::from_rgb(37, 67, 52),
                    Color32::from_rgb(76, 124, 96),
                ),
            };

            Frame::group(ui.style())
                .fill(fill)
                .stroke(egui::Stroke::new(1.0, border))
                .inner_margin(Margin::symmetric(10, 8))
                .show(ui, |ui| {
                    ui.set_max_width(560.0);
                    ui.label(RichText::new(title).strong().small().color(Color32::LIGHT_GRAY));
                    ui.add_space(2.0);
                    let render_text = truncate_chars(&message.text, MAX_RENDER_CHARS_PER_MESSAGE);
                    ui.label(RichText::new(render_text).color(Color32::from_rgb(235, 242, 250)));
                });
        });
    }

    fn render_side_panel(&mut self, ui: &mut Ui) {
        self.render_scope_card(ui);
        ui.add_space(8.0);
        self.render_tool_logs_card(ui);
        ui.add_space(8.0);
        self.render_tool_catalog_card(ui);
    }

    fn render_scope_card(&self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.label(RichText::new("能力范围").strong());
                ui.add_space(4.0);
                ui.label("• 创建节点 / 连接节点 / 设参数");
                ui.label("• 查询图状态与节点信息");
                ui.label("• 搜索并阅读知识库");
                ui.label("• 不做代码编辑、文件写入、插件生成");

                if let Some(error) = &self.last_error {
                    ui.add_space(6.0);
                    ui.separator();
                    ui.label(
                        RichText::new(format!("最近错误: {}", truncate_chars(error, 180)))
                            .color(Color32::LIGHT_RED)
                            .small(),
                    );
                }
            });
    }

    fn render_tool_logs_card(&self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.label(RichText::new("最近工具日志").strong());
                ui.add_space(4.0);

                let logs: Vec<&str> = self
                    .messages
                    .iter()
                    .rev()
                    .filter(|message| message.kind == MessageKind::ToolLog)
                    .map(|message| message.text.as_str())
                    .take(MAX_VISIBLE_TOOL_LOGS)
                    .collect();

                if logs.is_empty() {
                    ui.label(RichText::new("暂无工具调用").weak().italics());
                    return;
                }

                for log in logs.into_iter().rev() {
                    ui.label(RichText::new(truncate_chars(log, 160)).small().monospace());
                }
            });
    }

    fn render_tool_catalog_card(&mut self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("可用工具").strong());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let toggle_label = if self.show_tool_catalog { "收起" } else { "展开" };
                        if ui.button(toggle_label).clicked() {
                            self.show_tool_catalog = !self.show_tool_catalog;
                        }
                    });
                });

                if !self.show_tool_catalog {
                    return;
                }

                ui.add_space(4.0);
                if self.tool_names.is_empty() {
                    ui.label(RichText::new("工具尚未初始化").weak());
                    return;
                }

                for tool_name in self.tool_names.iter().take(MAX_VISIBLE_TOOL_NAMES) {
                    ui.label(RichText::new(format!("• {tool_name}")).small());
                }
            });
    }

    fn render_composer(&mut self, ui: &mut Ui) {
        Frame::group(ui.style())
            .inner_margin(Margin::symmetric(8, 8))
            .show(ui, |ui| {
                ui.label(RichText::new("输入区").strong());
                ui.add_space(4.0);

                let mut should_send = false;
                ui.add_enabled_ui(!self.is_busy, |ui| {
                    let ready = self.tool_registry.is_some();
                    let text_edit = TextEdit::multiline(&mut self.input)
                        .desired_rows(3)
                        .hint_text(
                            "例：创建一个 cube，再创建 sphere，再放一个 boolean 并把两个输入接好",
                        );
                    let response = if ready {
                        ui.add_sized([ui.available_width(), 78.0], text_edit)
                    } else {
                        ui.add_enabled_ui(false, |ui| ui.add_sized([ui.available_width(), 78.0], text_edit))
                            .inner
                    };

                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add_enabled(ready, egui::Button::new("发送")).clicked() {
                            should_send = true;
                        }
                        if ui.button("填入 /graph").clicked() {
                            self.input = "帮我读取当前 graph 状态".to_string();
                        }
                        if ui.button("清空输入").clicked() {
                            self.input.clear();
                        }
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(
                                RichText::new(format!("{} chars", self.input.chars().count()))
                                    .weak()
                                    .small(),
                            );
                        });
                    });
                });

                if self.is_busy {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("正在思考并调用节点工具…");
                    });
                } else if self.tool_registry.is_none() {
                    ui.add_space(6.0);
                    ui.label(RichText::new("正在加载工具…").weak());
                }

                if should_send && !self.input.trim().is_empty() {
                    let user_text = std::mem::take(&mut self.input);
                    self.submit_user_message(user_text);
                }
            });
    }

    fn submit_user_message(&mut self, user_text: String) {
        if self.is_busy {
            return;
        }

        let Some(tool_registry) = self.tool_registry.clone() else {
            self.push_message(ChatMessage::system("❌ 工具注册尚未完成。"));
            return;
        };

        self.push_message(ChatMessage::user(user_text));
        self.last_error = None;

        let prompt_contents = self.prompt_contents();
        let gemini_settings = load_gemini_settings();
        let (response_tx, response_rx) = unbounded::<Result<AssistantTurnResult, String>>();

        std::thread::spawn(move || {
            let result = run_prompt_driven_turn(prompt_contents, tool_registry, gemini_settings);
            if response_tx.send(result).is_err() {}
        });

        self.pending_response_rx = Some(response_rx);
        self.is_busy = true;
    }

    fn prompt_contents(&self) -> Vec<Value> {
        let chat_messages: Vec<&ChatMessage> = self
            .messages
            .iter()
            .filter(|message| message.kind == MessageKind::Chat)
            .collect();
        let keep_from = chat_messages.len().saturating_sub(MAX_PROMPT_CHAT_MESSAGES);

        chat_messages
            .into_iter()
            .skip(keep_from)
            .map(|message| {
                let role = match message.role {
                    MessageRole::User => "user",
                    MessageRole::Assistant => "model",
                };
                json!({
                    "role": role,
                    "parts": [{"text": message.text.clone()}]
                })
            })
            .collect()
    }

    fn last_assistant_reply(&self) -> Option<&str> {
        self.messages
            .iter()
            .rev()
            .find(|message| {
                matches!(message.role, MessageRole::Assistant) && message.kind == MessageKind::Chat
            })
            .map(|message| message.text.as_str())
    }

    fn reset_chat(&mut self) {
        self.messages.clear();
        self.push_message(ChatMessage::assistant(
            "会话已清空。告诉我你要做什么节点操作，我会直接调用工具执行。",
        ));
        self.push_message(ChatMessage::system("提示：可先用上方快捷动作验证流程。"));
        self.input.clear();
        self.last_error = None;
    }

    fn push_message(&mut self, message: ChatMessage) {
        self.messages.push(message);
        if self.messages.len() > MAX_MESSAGES {
            let extra = self.messages.len().saturating_sub(MAX_MESSAGES);
            if extra > 0 {
                self.messages.drain(0..extra);
            }
        }
    }
}

fn run_prompt_driven_turn(
    mut contents: Vec<Value>,
    tool_registry: Arc<ToolRegistry>,
    settings: GeminiSettings,
) -> Result<AssistantTurnResult, String> {
    if settings.api_key.trim().is_empty() {
        let settings_path = crate::runtime_paths::ai_providers_path();
        return Err(format!(
            "缺少 Gemini API Key。请在环境变量或 {} 中配置。",
            settings_path.display()
        ));
    }

    let client = Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(45))
        .build()
        .map_err(|error| format!("创建 Gemini 客户端失败: {error}"))?;

    let tool_definitions = tool_registry.list_definitions();
    let mut assistant_chunks: Vec<String> = Vec::new();
    let mut tool_logs: Vec<String> = Vec::new();

    for _ in 0..MAX_AGENT_TURNS {
        let response = request_gemini_generate(&client, &settings, &contents, &tool_definitions)?;
        let parts = extract_candidate_parts(&response)
            .ok_or_else(|| "Gemini 没有返回可解析内容。".to_string())?;

        if parts.is_empty() {
            break;
        }

        let assistant_text = collect_text_from_parts(&parts);
        if !assistant_text.trim().is_empty() {
            assistant_chunks.push(assistant_text.trim().to_string());
        }

        let function_calls = collect_function_calls(&parts);
        if function_calls.is_empty() {
            break;
        }

        contents.push(json!({
            "role": "model",
            "parts": parts
        }));

        let mut function_responses: Vec<Value> = Vec::new();
        for function_call in function_calls {
            let tool_name = function_call.name.clone();
            let tool_args = normalize_function_args(function_call.args);
            let args_preview = serde_json::to_string(&tool_args)
                .ok()
                .map(|text| truncate_chars(&text, MAX_LOG_TEXT_CHARS))
                .unwrap_or_else(|| "{}".to_string());

            tool_logs.push(format!("调用 `{tool_name}` 参数: {args_preview}"));

            match tool_registry.execute_sync(&tool_name, tool_args) {
                Ok(output) => {
                    tool_logs.push(format!(
                        "`{tool_name}` 完成: {}",
                        summarize_tool_output(&output)
                    ));
                    function_responses.push(json!({
                        "functionResponse": {
                            "name": tool_name,
                            "response": {
                                "ok": true,
                                "llm_text": output.llm_text,
                                "raw_text": output.raw_text
                            }
                        }
                    }));
                }
                Err(error) => {
                    let error_text = error.0;
                    tool_logs.push(format!("`{tool_name}` 失败: {error_text}"));
                    function_responses.push(json!({
                        "functionResponse": {
                            "name": tool_name,
                            "response": {
                                "ok": false,
                                "error": error_text
                            }
                        }
                    }));
                }
            }
        }

        contents.push(json!({
            "role": "user",
            "parts": function_responses
        }));
    }

    let assistant_text = assistant_chunks.join("\n").trim().to_string();
    let assistant_text = if assistant_text.is_empty() {
        if tool_logs.is_empty() {
            "这次没有拿到有效回复，你可以再试一次。".to_string()
        } else {
            "工具已执行完成，你可以继续给我下一步指令。".to_string()
        }
    } else {
        assistant_text
    };

    Ok(AssistantTurnResult {
        tool_logs,
        assistant_text,
    })
}

fn load_gemini_settings() -> GeminiSettings {
    let mut api_key = String::new();
    let mut model = DEFAULT_GEMINI_MODEL.to_string();
    let mut source_path: Option<PathBuf> = None;

    for settings_path in providers_path_candidates() {
        if let Ok(raw) = std::fs::read_to_string(&settings_path) {
            if let Ok(json_value) = serde_json::from_str::<Value>(&raw) {
                if let Some(gemini) = json_value.get("gemini").and_then(|value| value.as_object()) {
                    source_path = Some(settings_path.clone());

                    if let Some(key) = gemini.get("api_key").and_then(|value| value.as_str()) {
                        if !key.trim().is_empty() {
                            api_key = key.trim().to_string();
                        }
                    }
                    if let Some(saved_model) = gemini
                        .get("model_flash")
                        .and_then(|value| value.as_str())
                        .or_else(|| gemini.get("model_pro").and_then(|value| value.as_str()))
                    {
                        if !saved_model.trim().is_empty() {
                            model =
                                crate::cunning_core::ai_service::gemini::normalize_model_name(saved_model);
                        }
                    }
                }
            }
        }
        if !api_key.trim().is_empty() {
            break;
        }
    }

    if api_key.trim().is_empty() {
        api_key = read_gemini_api_key_env();
    }

    GeminiSettings {
        api_key,
        model,
        source_path,
    }
}

fn providers_path_candidates() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Ok(path_text) = std::env::var("CUNNING3D_AI_PROVIDERS_PATH") {
        let trimmed = path_text.trim();
        if !trimmed.is_empty() {
            candidates.push(PathBuf::from(trimmed));
        }
    }
    candidates.push(crate::runtime_paths::ai_providers_path());
    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("settings").join("ai").join("providers.json"));
        candidates.push(
            cwd.join("Cunning3D_1.0")
                .join("settings")
                .join("ai")
                .join("providers.json"),
        );
    }
    if let Ok(manifest_dir) = std::env::var("CARGO_MANIFEST_DIR") {
        candidates.push(
            PathBuf::from(manifest_dir)
                .join("settings")
                .join("ai")
                .join("providers.json"),
        );
    }

    let mut unique: Vec<PathBuf> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    for path in candidates {
        let key = path.to_string_lossy().to_string();
        if seen.insert(key) {
            unique.push(path);
        }
    }
    unique
}

fn request_gemini_generate(
    client: &Client,
    settings: &GeminiSettings,
    contents: &[Value],
    tools: &[ToolDefinition],
) -> Result<Value, String> {
    let model = crate::cunning_core::ai_service::gemini::normalize_model_name(&settings.model);
    let url = format!(
        "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
        model, settings.api_key
    );

    let function_declarations: Vec<Value> = tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.parameters
            })
        })
        .collect();

    let body = json!({
        "system_instruction": {
            "parts": [{"text": ASSISTANT_SYSTEM_PROMPT}]
        },
        "contents": contents,
        "tools": [{
            "functionDeclarations": function_declarations
        }],
        "generationConfig": {
            "temperature": 0.1
        }
    });

    let response = client
        .post(url)
        .json(&body)
        .send()
        .map_err(|error| format!("请求 Gemini 失败: {error}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body_text = response.text().unwrap_or_default();
        return Err(format!(
            "Gemini HTTP {}: {}",
            status,
            truncate_chars(&body_text, 600)
        ));
    }

    response
        .json::<Value>()
        .map_err(|error| format!("解析 Gemini 响应失败: {error}"))
}

fn extract_candidate_parts(response: &Value) -> Option<Vec<Value>> {
    response
        .get("candidates")
        .and_then(|value| value.as_array())
        .and_then(|candidates| candidates.first())
        .and_then(|candidate| candidate.get("content"))
        .and_then(|content| content.get("parts"))
        .and_then(|parts| parts.as_array())
        .cloned()
}

fn collect_text_from_parts(parts: &[Value]) -> String {
    parts
        .iter()
        .filter_map(|part| part.get("text").and_then(|value| value.as_str()))
        .collect::<Vec<_>>()
        .join("")
}

fn collect_function_calls(parts: &[Value]) -> Vec<FunctionCallRequest> {
    let mut calls = Vec::new();
    for part in parts {
        let Some(function_call) = part.get("functionCall").and_then(|value| value.as_object())
        else {
            continue;
        };
        let Some(name) = function_call.get("name").and_then(|value| value.as_str()) else {
            continue;
        };
        if name.trim().is_empty() {
            continue;
        }
        let args = function_call
            .get("args")
            .cloned()
            .unwrap_or_else(|| json!({}));
        calls.push(FunctionCallRequest {
            name: name.to_string(),
            args,
        });
    }
    calls
}

fn normalize_function_args(args: Value) -> Value {
    if args.is_null() {
        json!({})
    } else if args.is_object() {
        args
    } else {
        json!({ "value": args })
    }
}

fn summarize_tool_output(output: &ToolOutput) -> String {
    let raw = output.raw_text.trim();
    let llm = output.llm_text.trim();
    if !raw.is_empty() {
        truncate_chars(raw, 180)
    } else if !llm.is_empty() {
        truncate_chars(llm, 180)
    } else {
        "ok".to_string()
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out = String::new();
    for character in text.chars().take(max_chars) {
        out.push(character);
    }
    out.push_str("...");
    out
}
