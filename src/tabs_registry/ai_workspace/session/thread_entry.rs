use crate::tabs_registry::ai_workspace::session::message::{
    ImageAttachment, MessageState, ThinkingSection,
};
use crate::tabs_registry::ai_workspace::tools::{FileDiff, ToolLog, ToolLogLevel};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolKind {
    Read,
    Search,
    Execute,
    Edit,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ToolCallStatus {
    Pending,
    AwaitingApproval,
    InProgress,
    Completed,
    Rejected(String),
    Failed(String),
    Canceled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolUse {
    pub id: String,
    pub name: String,
    pub raw_input: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub tool_name: String,
    pub is_error: bool,
    pub content: String,
    pub debug_output: Option<String>,
}

/// Zed-style structured @ mention reference
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum MentionUri {
    File {
        path: String,
    },
    Directory {
        path: String,
    },
    Selection {
        path: Option<String>,
        start_line: u32,
        end_line: u32,
    },
    Symbol {
        path: String,
        name: String,
        start_line: u32,
        end_line: u32,
    },
    Diagnostics {
        errors: bool,
        warnings: bool,
    },
    PastedImage {
        id: u64,
    },
    Fetch {
        url: String,
    },
}

impl MentionUri {
    pub fn file(p: impl Into<String>) -> Self {
        Self::File { path: p.into() }
    }
    pub fn dir(p: impl Into<String>) -> Self {
        Self::Directory { path: p.into() }
    }
    pub fn selection(path: Option<String>, start: u32, end: u32) -> Self {
        Self::Selection {
            path,
            start_line: start,
            end_line: end,
        }
    }
    pub fn symbol(path: impl Into<String>, name: impl Into<String>, start: u32, end: u32) -> Self {
        Self::Symbol {
            path: path.into(),
            name: name.into(),
            start_line: start,
            end_line: end,
        }
    }
    pub fn diagnostics(errors: bool, warnings: bool) -> Self {
        Self::Diagnostics { errors, warnings }
    }
    pub fn fetch(url: impl Into<String>) -> Self {
        Self::Fetch { url: url.into() }
    }

    pub fn display(&self) -> String {
        match self {
            Self::File { path } | Self::Directory { path } => path.clone(),
            Self::Selection {
                path,
                start_line,
                end_line,
            } => format!(
                "{}:L{}-{}",
                path.as_deref().unwrap_or("selection"),
                start_line + 1,
                end_line + 1
            ),
            Self::Symbol {
                name,
                start_line,
                end_line,
                ..
            } => format!("{}:L{}-{}", name, start_line + 1, end_line + 1),
            Self::Diagnostics { errors, warnings } => {
                format!("diagnostics(err={},warn={})", errors, warnings)
            }
            Self::PastedImage { id } => format!("image#{}", id),
            Self::Fetch { url } => url.clone(),
        }
    }
    pub fn to_path(&self, base_dir: &Path) -> std::path::PathBuf {
        match self {
            Self::File { path } | Self::Directory { path } | Self::Symbol { path, .. } => {
                base_dir.join(path)
            }
            Self::Selection { path: Some(p), .. } => base_dir.join(p),
            _ => base_dir.to_path_buf(),
        }
    }
    pub fn icon_char(&self) -> char {
        match self {
            Self::File { .. } => '📄',
            Self::Directory { .. } => '📁',
            Self::Selection { .. } => '✂',
            Self::Symbol { .. } => '⚙',
            Self::Diagnostics { .. } => '⚠',
            Self::PastedImage { .. } => '🖼',
            Self::Fetch { .. } => '🌐',
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolCall {
    pub id: u64,
    pub tool_name: String,
    pub kind: ToolKind,
    pub status: ToolCallStatus,
    pub title: String,
    pub args_preview: String,
    pub raw_input: Option<String>,
    pub llm_result: Option<String>,
    pub raw_output: Option<String>,
    #[serde(default)]
    pub diffs: Vec<FileDiff>,
    pub logs: Vec<ToolLog>,
}

impl ToolCall {
    pub fn new(id: u64, tool_name: String, args_preview: String) -> Self {
        let kind = match tool_name.as_str() {
            "read_file" => ToolKind::Read,
            "search_workspace" | "explore_workspace" => ToolKind::Search,
            "terminal" | "diagnostics" => ToolKind::Execute,
            "apply_rust_nodespec" => ToolKind::Edit,
            _ => ToolKind::Other,
        };
        Self {
            id,
            title: tool_name.clone(),
            tool_name,
            kind,
            status: ToolCallStatus::Pending,
            args_preview,
            raw_input: None,
            llm_result: None,
            raw_output: None,
            diffs: vec![],
            logs: vec![],
        }
    }
    pub fn mark_running(&mut self) {
        self.status = ToolCallStatus::InProgress;
    }
    pub fn mark_awaiting_approval(&mut self) {
        self.status = ToolCallStatus::AwaitingApproval;
    }
    pub fn mark_ok(&mut self) {
        self.status = ToolCallStatus::Completed;
    }
    pub fn mark_rejected(&mut self, reason: String) {
        self.status = ToolCallStatus::Rejected(reason);
    }
    pub fn mark_err(&mut self, e: String) {
        self.status = ToolCallStatus::Failed(e);
    }
    pub fn mark_cancelled(&mut self) {
        self.status = ToolCallStatus::Canceled;
    }
    pub fn push_log(&mut self, message: impl Into<String>, level: ToolLogLevel) {
        self.logs.push(ToolLog {
            message: message.into(),
            level,
        });
    }
    pub fn tool_use_id(&self) -> String {
        format!("tc_{}", self.id)
    }
    pub fn to_tool_use(&self) -> ToolUse {
        ToolUse {
            id: self.tool_use_id(),
            name: self.tool_name.clone(),
            raw_input: self.raw_input.clone().unwrap_or_else(|| "{}".into()),
        }
    }
    pub fn to_tool_result(&self) -> Option<ToolResult> {
        let (is_error, content) = match &self.status {
            ToolCallStatus::Rejected(e) => (true, e.clone()),
            ToolCallStatus::Failed(e) => (true, e.clone()),
            ToolCallStatus::Canceled => (true, "Tool canceled by user.".into()),
            ToolCallStatus::Completed => (
                false,
                self.llm_result.clone().unwrap_or_else(|| "OK".into()),
            ),
            _ => return None,
        };
        Some(ToolResult {
            tool_use_id: self.tool_use_id(),
            tool_name: self.tool_name.clone(),
            is_error,
            content,
            debug_output: self.raw_output.clone(),
        })
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ThreadEntry {
    User {
        text: String,
        images: Vec<ImageAttachment>,
        mentions: Vec<MentionUri>,
        timestamp: i64,
    },
    Assistant {
        thinking: Option<ThinkingSection>,
        content: String,
        state: MessageState,
        timestamp: i64,
    },
    ToolCall(ToolCall),
}

impl ThreadEntry {
    pub fn text_content(&self) -> String {
        match self {
            Self::User { text, .. } => text.clone(),
            Self::Assistant { content, .. } => content.clone(),
            Self::ToolCall(c) => format!(
                "ToolCall {} {}",
                c.tool_name,
                c.llm_result.as_deref().unwrap_or("")
            ),
        }
    }
}
