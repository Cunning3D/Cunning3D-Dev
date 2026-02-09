use super::message::Message;
use super::message::MessageState;
use super::thread_entry::ThreadEntry;
use crate::tabs_registry::ai_workspace::tools::CancellationToken;

/// Minimal text buffer for working copy (replaces deleted editor::buffer::TextBuffer)
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct TextBuffer {
    pub content: String,
}

impl TextBuffer {
    pub fn from_original(content: String) -> Self {
        Self { content }
    }
    
    pub fn text(&self) -> &str {
        &self.content
    }
}
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone)]
pub struct ToolRequestMeta {
    pub tool_name: String,
    pub args: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BusyStage {
    Idle,
    ToolRunning,
    ToolFeedback,
    WaitingModel,
    Generating,
    AutoHeal(u8, u8),
    NetworkRetry(u32),
}

impl Default for BusyStage {
    fn default() -> Self {
        Self::Idle
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Session {
    pub id: Uuid,
    pub title: String,
    pub created_at: std::time::SystemTime,

    pub entries: Vec<ThreadEntry>,

    #[serde(default, rename = "messages", skip_serializing)]
    pub legacy_messages: Vec<Message>,

    /// 多文件工作副本：记录每个文件在当前会话中的编辑状态（Diff）
    /// Key: 文件的绝对路径
    pub working_copy: HashMap<PathBuf, TextBuffer>,

    /// 当前会话关注的主文件（用于 Context 构建和初始视图）
    pub active_file: Option<PathBuf>,

    /// 原始文件快照缓存（用于 Diff 基准）
    /// Key: 文件的绝对路径, Value: 文件内容
    pub original_snapshots: HashMap<PathBuf, String>,

    /// 待处理的流式事件队列（为了 UI 打字机效果）
    /// 以前在 Pane 里，现在移到 Session 级别，这样后台可以继续累积
    #[serde(skip)]
    pub pending_events: std::collections::VecDeque<super::event::SessionEvent>,

    /// 当前正在运行的任务的取消信号发送端
    /// 如果为 Some，说明有任务在跑；调用 send() 可停止任务
    #[serde(skip)]
    pub abort_sender: Option<tokio::sync::oneshot::Sender<()>>,

    /// Cancellation tokens for running tools (key: request_id)
    #[serde(skip)]
    pub tool_cancel_tokens: HashMap<u64, CancellationToken>,

    /// Async tool request metadata (key: request_id) for post-processing (auto-insert/focus)
    #[serde(skip)]
    pub tool_request_meta: HashMap<u64, ToolRequestMeta>,

    /// 当前一次 LLM 流式对话中已经执行过的工具调用签名，用于去重
    /// Key 形如 "tool_name::{json_args}"，仅存在于内存中，不参与持久化
    #[serde(skip)]
    pub executed_tool_signatures: std::collections::HashSet<String>,

    /// 编译重试计数器，Key 为 node_name，Value 为当前连续重试次数
    #[serde(skip)]
    pub compile_retry_count: HashMap<String, u32>,

    /// 当前会话是否正在进行一次 AI 交互（网络流/工具链/自愈链路进行中）
    #[serde(skip)]
    pub is_busy: bool,

    /// Busy 原因（用于 UI 展示），例如：模型生成中 / 工具执行中：xxx / 自动修复中：n/3
    #[serde(skip)]
    pub busy_reason: Option<String>,

    /// Busy 阶段（用于 UI 稳定显示“工具→回喂→等待→生成”）
    #[serde(skip)]
    pub busy_stage: BusyStage,

    /// Skill-first policy markers (in-memory only)
    #[serde(skip)]
    pub policy_seen: std::collections::HashSet<String>,

    /// Estimated token usage tracking (in-memory only)
    #[serde(skip)]
    pub token_usage: TokenUsageInfo,

    /// Compaction state for LLM-driven summarization
    #[serde(skip)]
    pub compaction_state: CompactionState,

    /// Stored compaction summary from previous LLM call
    pub compaction_summary: Option<String>,

    /// Last injected environment working directory (for env diff).
    pub last_env_cwd: Option<String>,
}

/// Token usage tracking similar to Zed/Codex
#[derive(Debug, Clone, Default)]
pub struct TokenUsageInfo {
    pub input_tokens: u64,  // Last request input tokens
    pub output_tokens: u64, // Last request output tokens
    pub total_tokens: u64,  // Cumulative estimate for history
    pub max_tokens: u64,    // Model context window (Gemini: 200k safe limit)
}

impl TokenUsageInfo {
    /// Estimate tokens from text (rough: 1 token ≈ 3.5 chars for mixed zh/en)
    pub fn estimate(text: &str) -> u64 {
        (text.chars().count() as f64 / 3.5).ceil() as u64
    }
    /// Usage ratio (0.0 - 1.0+)
    pub fn ratio(&self) -> f32 {
        if self.max_tokens == 0 {
            0.0
        } else {
            self.total_tokens as f32 / self.max_tokens as f32
        }
    }
    /// Is context nearing limit (>75% - Codex uses this threshold)
    pub fn is_warning(&self) -> bool {
        self.ratio() >= 0.75
    }
    /// Is context exceeded (>90%)
    pub fn is_exceeded(&self) -> bool {
        self.ratio() >= 0.90
    }
}

/// Compaction state for LLM-driven summarization (like Codex)
#[derive(Debug, Clone, Default)]
pub enum CompactionState {
    #[default]
    None,
    Pending,     // Compaction triggered, waiting for LLM summary
    Summarizing, // LLM is generating summary
}

/// Prompt for context compaction (from Codex)
pub const COMPACTION_PROMPT: &str = r#"You are performing a CONTEXT CHECKPOINT COMPACTION. Create a handoff summary for continuing the task.
Include:
- Current progress and key decisions made
- Important context, constraints, or user preferences
- What remains to be done (clear next steps)
- Any critical data, examples, or references needed to continue
Be concise (<500 words), structured, and focused on helping seamlessly continue the work."#;

/// Prefix for compacted summaries (from Codex)
pub const SUMMARY_PREFIX: &str = "[Previous context summary] ";

impl Clone for Session {
    fn clone(&self) -> Self {
        Self {
            id: self.id,
            title: self.title.clone(),
            created_at: self.created_at,
            entries: self.entries.clone(),
            legacy_messages: self.legacy_messages.clone(),
            working_copy: self.working_copy.clone(),
            active_file: self.active_file.clone(),
            original_snapshots: self.original_snapshots.clone(),
            pending_events: self.pending_events.clone(),
            abort_sender: None,
            tool_cancel_tokens: HashMap::new(),
            tool_request_meta: HashMap::new(),
            executed_tool_signatures: std::collections::HashSet::new(),
            compile_retry_count: HashMap::new(),
            is_busy: false,
            busy_reason: None,
            busy_stage: BusyStage::Idle,
            policy_seen: std::collections::HashSet::new(),
            token_usage: self.token_usage.clone(),
            compaction_state: CompactionState::None,
            compaction_summary: self.compaction_summary.clone(),
            last_env_cwd: self.last_env_cwd.clone(),
        }
    }
}

impl Session {
    pub fn new() -> Self {
        Self {
            id: Uuid::new_v4(),
            title: "New Chat".to_string(),
            created_at: std::time::SystemTime::now(),
            entries: Vec::new(),
            legacy_messages: Vec::new(),
            working_copy: HashMap::new(),
            active_file: None,
            original_snapshots: HashMap::new(),
            pending_events: std::collections::VecDeque::new(),
            abort_sender: None,
            tool_cancel_tokens: HashMap::new(),
            tool_request_meta: HashMap::new(),
            executed_tool_signatures: std::collections::HashSet::new(),
            compile_retry_count: HashMap::new(),
            is_busy: false,
            busy_reason: None,
            busy_stage: BusyStage::Idle,
            policy_seen: std::collections::HashSet::new(),
            token_usage: TokenUsageInfo {
                max_tokens: 200_000,
                ..Default::default()
            },
            compaction_state: CompactionState::None,
            compaction_summary: None,
            last_env_cwd: None,
        }
    }

    /// Update token usage after a request (called from pane after LLM response)
    pub fn update_token_usage(&mut self, input: u64, output: u64) {
        self.token_usage.input_tokens = input;
        self.token_usage.output_tokens = output;
        self.token_usage.total_tokens =
            self.token_usage.total_tokens.saturating_add(input + output);
    }

    /// Estimate and update token usage from history (recalculate)
    pub fn recalc_token_usage(&mut self) {
        self.token_usage.total_tokens = self
            .entries
            .iter()
            .map(|e| TokenUsageInfo::estimate(&e.text_content()))
            .sum();
    }

    /// Check if compaction is needed (like Codex's auto-compact trigger)
    pub fn needs_compaction(&self) -> bool {
        self.token_usage.is_warning()
            && self.entries.len() > 6
            && matches!(self.compaction_state, CompactionState::None)
    }

    /// Trigger LLM-driven compaction - returns prompt if compaction needed
    pub fn trigger_compaction(&mut self) -> Option<String> {
        if !self.needs_compaction() {
            return None;
        }
        self.compaction_state = CompactionState::Pending;
        // Build conversation summary for LLM to process
        let conversation = self.build_conversation_for_compaction();
        Some(format!(
            "{}\n\n---\nConversation to summarize:\n{}",
            COMPACTION_PROMPT, conversation
        ))
    }

    /// Build conversation text for compaction (exclude system prompts, focus on user/AI exchanges)
    fn build_conversation_for_compaction(&self) -> String {
        let mut out = String::new();
        let start = 2.min(self.entries.len());
        let end = self.entries.len().saturating_sub(4);
        for e in &self.entries[start..end] {
            match e {
                ThreadEntry::User { text, .. } => {
                    out.push_str("User: ");
                    out.push_str(&text.chars().take(500).collect::<String>());
                    out.push_str("\n\n");
                }
                ThreadEntry::Assistant { content, .. } => {
                    if !content.starts_with("[[_tool_result_internal_]]") && content.len() > 20 {
                        out.push_str("Assistant: ");
                        out.push_str(&content.chars().take(800).collect::<String>());
                        out.push_str("\n\n");
                    }
                }
                ThreadEntry::ToolCall(_) => {}
            }
        }
        out
    }

    /// Apply compaction summary from LLM (replaces middle messages with summary)
    pub fn apply_compaction(&mut self, summary: String) {
        if self.entries.len() <= 6 {
            return;
        }
        let keep_start = 2.min(self.entries.len());
        let keep_end = 4.min(self.entries.len());
        let first: Vec<_> = self.entries[..keep_start].to_vec();
        let last: Vec<_> = self.entries[self.entries.len() - keep_end..].to_vec();
        self.entries = first;
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.entries.push(ThreadEntry::Assistant {
            thinking: None,
            content: format!("{}{}", SUMMARY_PREFIX, summary),
            state: MessageState::Done,
            timestamp: ts,
        });
        self.entries.extend(last);

        self.compaction_state = CompactionState::None;
        self.compaction_summary = Some(summary);
        self.recalc_token_usage();
    }

    /// Fallback: Simple truncation when LLM compaction fails (keeps first + recent)
    pub fn simple_compact(&mut self) {
        if self.entries.len() <= 6 {
            return;
        }
        let keep_start = 2.min(self.entries.len());
        let keep_end = 4.min(self.entries.len());
        let removed = self.entries.len() - keep_start - keep_end;
        let first: Vec<_> = self.entries[..keep_start].to_vec();
        let last: Vec<_> = self.entries[self.entries.len() - keep_end..].to_vec();
        self.entries = first;
        let summary = format!("Compacted {removed} entries to fit context window.");
        let ts = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
        self.entries.push(ThreadEntry::Assistant {
            thinking: None,
            content: format!("{}{}", SUMMARY_PREFIX, summary),
            state: MessageState::Done,
            timestamp: ts,
        });
        self.entries.extend(last);

        self.compaction_state = CompactionState::None;
        self.compaction_summary = Some(summary);
        self.recalc_token_usage();
    }

    pub fn with_file(path: PathBuf, original_code: String) -> Self {
        let mut session = Self::new();
        session.active_file = Some(path.clone());
        session
            .original_snapshots
            .insert(path.clone(), original_code); // 存一份快照
                                                  // 自动设置标题
        if let Some(name) = session.active_file.as_ref().and_then(|p| p.file_name()) {
            session.title = format!("Chat about {}", name.to_string_lossy());
        }
        session
    }

    pub fn current_status(&self) -> MessageState {
        self.entries
            .iter()
            .rev()
            .find_map(|e| match e {
                ThreadEntry::Assistant { state, .. } => Some(state.clone()),
                _ => None,
            })
            .unwrap_or(MessageState::Done)
    }

    pub fn llm_history(&self) -> Vec<Message> {
        fn trunc(s: &str, max: usize) -> String {
            let t = s.trim();
            if t.chars().count() <= max {
                return t.to_string();
            }
            let head = max.saturating_sub(60).max(120);
            let tail = 40usize.min(max.saturating_sub(head + 10));
            let h: String = t.chars().take(head).collect();
            let t2: String = t
                .chars()
                .rev()
                .take(tail)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect();
            format!("{}\n…<truncated>…\n{}", h, t2)
        }
        self.entries
            .iter()
            .flat_map(|e| match e {
                ThreadEntry::User { text, images, .. } => vec![Message::User { text: text.clone(), images: images.clone() }],
                ThreadEntry::Assistant { thinking, content, state, .. } => vec![Message::Ai { thinking: thinking.clone(), content: content.clone(), state: state.clone() }],
                ThreadEntry::ToolCall(c) => {
                    use super::thread_entry::ToolCallStatus as S;
                    // Zed parity: if we include ToolUse, we MUST include ToolResult (no dangling tool_use).
                    // So only include terminal states in request history.
                    let (status, terminal) = match &c.status {
                        S::Pending => ("pending", false),
                        S::AwaitingApproval => ("awaiting_approval", false),
                        S::InProgress => ("in_progress", false),
                        S::Completed => ("completed", true),
                        S::Canceled => ("canceled", true),
                        S::Failed(_) => ("failed", true),
                        S::Rejected(_) => ("rejected", true),
                    };
                    if !terminal { return vec![]; }
                    let tool_use_id = format!("tc_{}", c.id);
                    let raw_in = c.raw_input.as_deref().map(|s| trunc(s, 520)).unwrap_or_else(|| "{}".into());
                    let tool_use = format!("[ToolUse] name={} id={}\n```json\n{}\n```", c.tool_name, tool_use_id, raw_in);
                    let (is_error, result_text) = match &c.status {
                        S::Failed(e) => (true, trunc(e, 520)),
                        S::Canceled => (true, "Tool canceled by user.".to_string()),
                        S::Rejected(e) => (true, trunc(e, 520)),
                        _ => (false, c.llm_result.as_deref().map(|s| trunc(s, 620)).unwrap_or_else(|| "OK".into())),
                    };
                    let dbg = c.raw_output.as_deref().map(|s| trunc(s, 800)).unwrap_or_default();
                    let tool_result = if dbg.is_empty() {
                        format!("[ToolResult] name={} id={} status={} is_error={}\n{}", c.tool_name, tool_use_id, status, is_error, result_text)
                    } else {
                        format!("[ToolResult] name={} id={} status={} is_error={}\n{}\n\n[DebugOutput]\n{}", c.tool_name, tool_use_id, status, is_error, result_text, dbg)
                    };
                    vec![
                        Message::Ai { thinking: None, content: tool_use, state: MessageState::Done },
                        Message::User { text: tool_result, images: vec![] },
                    ]
                }
            })
            .collect()
    }

    #[inline]
    pub fn set_busy(&mut self, reason: impl Into<String>) {
        self.is_busy = true;
        self.busy_reason = Some(reason.into());
    }

    #[inline]
    pub fn clear_busy(&mut self) {
        self.is_busy = false;
        self.busy_reason = None;
        self.abort_sender = None;
        self.busy_stage = BusyStage::Idle;
    }

    /// 获取当前活跃文件的 Diff Buffer，如果不存在则基于 Snapshot 创建
    pub fn get_or_create_buffer(&mut self, path: &PathBuf) -> Option<&mut TextBuffer> {
        if !self.working_copy.contains_key(path) {
            if let Some(original) = self.original_snapshots.get(path) {
                self.working_copy
                    .insert(path.clone(), TextBuffer::from_original(original.clone()));
            } else {
                return None;
            }
        }
        self.working_copy.get_mut(path)
    }

    /// 强制设置某个文件的 Snapshot（当用户拖入新文件或切换关注点时）
    pub fn set_file_context(&mut self, path: PathBuf, code: String) {
        self.active_file = Some(path.clone());
        if !self.original_snapshots.contains_key(&path) {
            self.original_snapshots.insert(path, code);
        }
    }
}

#[cfg(all(test, feature = "ai_ws_toolturn_parity_tests"))]
mod toolturn_parity_tests {
    use super::*;
    use crate::tabs_registry::ai_workspace::session::thread_entry::{ToolCall, ToolCallStatus};

    #[test]
    fn llm_history_emits_tooluse_then_toolresult_for_terminal_toolcall() {
        let mut s = Session::new();
        s.entries.push(ThreadEntry::User {
            text: "hi".into(),
            images: vec![],
            mentions: vec![],
            timestamp: 0,
        });
        s.entries.push(ThreadEntry::Assistant {
            thinking: None,
            content: "ok".into(),
            state: MessageState::Done,
            timestamp: 0,
        });
        let mut tc = ToolCall::new(7, "read_file".into(), "{\"file\":\"a.rs\"}".into());
        tc.raw_input = Some("{\"file\":\"a.rs\"}".into());
        tc.llm_result = Some("file content...".into());
        tc.raw_output = Some("RAW...".into());
        tc.status = ToolCallStatus::Completed;
        s.entries.push(ThreadEntry::ToolCall(tc));
        let h = s.llm_history();
        let joined = h
            .iter()
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n---\n");
        let i_use = joined.find("[ToolUse]").unwrap();
        let i_res = joined.find("[ToolResult]").unwrap();
        assert!(i_use < i_res);
    }

    #[test]
    fn cancelled_toolcall_emits_error_toolresult() {
        let mut s = Session::new();
        let mut tc = ToolCall::new(9, "explore_workspace".into(), "{}".into());
        tc.status = ToolCallStatus::Canceled;
        s.entries.push(ThreadEntry::ToolCall(tc));
        let joined = s
            .llm_history()
            .iter()
            .map(|m| m.text_content())
            .collect::<Vec<_>>()
            .join("\n");
        assert!(joined.contains("is_error=true") || joined.contains("canceled"));
    }
}
