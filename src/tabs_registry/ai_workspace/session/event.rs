//! Session events for async UI updates.
use crate::tabs_registry::ai_workspace::tools::{ToolError, ToolLog, ToolOutput};

#[derive(Debug, Clone)]
pub enum SessionEvent {
    StartedThoughtProcess,
    EndedThoughtProcess,
    Thinking(String),
    Text(String),
    StreamedCompletion,
    ToolCallRequest {
        tool_name: String,
        args: serde_json::Value,
    },
    ToolApproval {
        request_id: u64,
        approve: bool,
        remember: bool,
    },
    /// Tool execution started (async)
    ToolExecutionStarted {
        tool_name: String,
        request_id: u64,
    },
    /// Tool execution progress (streaming logs)
    ToolExecutionProgress {
        request_id: u64,
        log: ToolLog,
    },
    /// Tool execution completed successfully
    ToolExecutionSuccess {
        request_id: u64,
        output: ToolOutput,
    },
    /// Tool execution failed
    ToolExecutionError {
        request_id: u64,
        error: String,
    },
    /// Tool execution cancelled
    ToolExecutionCancelled {
        request_id: u64,
    },
    TitleUpdated(String),
    NetworkRetry {
        attempt: u32,
        max_seconds: u64,
    },
    NetworkRetryPreparing {
        next_attempt: u32,
    },
    Error(String),
}
