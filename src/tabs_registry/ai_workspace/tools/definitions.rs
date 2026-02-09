//! Tool definitions with async execution support and cancellation tokens.
use crate::libs::ai_service::context_config as cfg;
use crate::libs::ai_service::truncate_util;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use super::diff::FileDiff;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDefinition {
    pub name: String,
    pub description: String,
    pub parameters: Value, // JSON Schema
}

/// Tool output with separate LLM and UI representations (like Zed's pattern)
#[derive(Debug, Clone)]
pub struct ToolOutput {
    /// Compact text sent back to the LLM (should be <500 chars typically)
    pub llm_text: String,
    /// Verbose output for UI display and debugging
    pub raw_text: String,
    /// Structured logs for UI timeline
    pub ui_logs: Vec<ToolLog>,
    /// Optional structured diffs for UI rendering (clamped)
    pub ui_diffs: Vec<FileDiff>,
}

impl ToolOutput {
    /// Create output with same text for LLM and UI (backward compat)
    pub fn simple(text: impl Into<String>) -> Self {
        let t = text.into();
        Self {
            llm_text: t.clone(),
            raw_text: t,
            ui_logs: vec![],
            ui_diffs: vec![],
        }
    }

    /// Create output with separate LLM summary and raw content
    pub fn with_summary(llm_summary: impl Into<String>, raw: impl Into<String>) -> Self {
        Self {
            llm_text: llm_summary.into(),
            raw_text: raw.into(),
            ui_logs: vec![],
            ui_diffs: vec![],
        }
    }

    /// Backward-compat: create with text and ui_logs (auto-truncates for LLM)
    pub fn new(text: impl Into<String>, ui_logs: Vec<ToolLog>) -> Self {
        let raw = text.into();
        let llm = truncate_for_llm(&raw);
        Self {
            llm_text: llm,
            raw_text: raw,
            ui_logs,
            ui_diffs: vec![],
        }
    }

    /// Legacy getter for backward compat (returns llm_text)
    pub fn text(&self) -> &str {
        &self.llm_text
    }
}

/// Truncate tool output for LLM consumption - aggressive compression to prevent context blowup
pub fn truncate_for_llm(text: &str) -> String {
    let t = text.trim();
    if truncate_util::approx_token_count(t) > cfg::TOOL_OUTPUT_TOKEN_LIMIT {
        return truncate_util::truncate_mid_tokens(t, cfg::TOOL_OUTPUT_TOKEN_LIMIT);
    }
    if t.chars().count() > cfg::TOOL_LLM_MAX_CHARS {
        truncate_util::truncate_mid_chars(t, cfg::TOOL_LLM_MAX_CHARS, None)
    } else {
        t.to_string()
    }
}

/// Maximum chars for LLM feedback per tool call - aggressive limit to prevent context blowup
pub const MAX_LLM_OUTPUT_CHARS: usize = cfg::TOOL_LLM_MAX_CHARS;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolLog {
    pub message: String,
    pub level: ToolLogLevel,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ToolLogLevel {
    Info,
    Success,
    Error,
    Warning,
}

#[derive(Debug)]
pub struct ToolError(pub String);

impl std::fmt::Display for ToolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
impl std::error::Error for ToolError {}

/// Cancellation token for long-running tool operations
#[derive(Clone, Debug)]
pub struct CancellationToken(Arc<AtomicBool>);

impl CancellationToken {
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

/// Tool execution context with cancellation and progress reporting
pub struct ToolContext {
    pub cancel_token: CancellationToken,
    pub progress_callback: Option<Box<dyn Fn(ToolLog) + Send + Sync>>,
}

impl Default for ToolContext {
    fn default() -> Self {
        Self {
            cancel_token: CancellationToken::new(),
            progress_callback: None,
        }
    }
}

impl ToolContext {
    pub fn with_cancel(cancel_token: CancellationToken) -> Self {
        Self {
            cancel_token,
            progress_callback: None,
        }
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancel_token.is_cancelled()
    }
    pub fn report_progress(&self, log: ToolLog) {
        if let Some(cb) = &self.progress_callback {
            cb(log);
        }
    }
}

/// The trait all tools must implement (sync API for compatibility)
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn definition(&self) -> ToolDefinition;
    fn execute(&self, args: Value) -> Result<ToolOutput, ToolError>;
    /// Extended execute with context (cancellation + progress). Default delegates to execute().
    fn execute_with_context(
        &self,
        args: Value,
        ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        if ctx.is_cancelled() {
            return Err(ToolError("Cancelled".into()));
        }
        self.execute(args)
    }
    /// Whether this tool is long-running (should be executed off UI thread)
    fn is_long_running(&self) -> bool {
        false
    }
}

/// Canonical JSON for stable signatures (sorted keys, normalized numbers)
pub fn canonical_json(v: &Value) -> String {
    match v {
        Value::Object(map) => {
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            let pairs: Vec<String> = keys
                .iter()
                .map(|k| format!("\"{}\":{}", k, canonical_json(&map[*k])))
                .collect();
            format!("{{{}}}", pairs.join(","))
        }
        Value::Array(arr) => {
            let items: Vec<String> = arr.iter().map(canonical_json).collect();
            format!("[{}]", items.join(","))
        }
        Value::Number(n) => {
            if let Some(f) = n.as_f64() {
                format!("{:.6}", f)
            } else {
                n.to_string()
            }
        }
        Value::String(s) => format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\"")),
        Value::Bool(b) => b.to_string(),
        Value::Null => "null".to_string(),
    }
}

/// Generate stable tool call signature for deduplication
pub fn tool_call_signature(name: &str, args: &Value) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let canonical = canonical_json(args);
    let mut hasher = DefaultHasher::new();
    name.hash(&mut hasher);
    canonical.hash(&mut hasher);
    format!("{}::{:016x}", name, hasher.finish())
}
