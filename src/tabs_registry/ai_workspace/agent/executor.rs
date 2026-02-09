//! Agent executor: true auto-retry loop with LLM fix, hash validation, failure budget, auto-rollback.
use crate::cunning_core::plugin_system::rust_build::{check_fast, CompilerError};
use crate::tabs_registry::ai_workspace::session::message::Message;
use crate::tabs_registry::ai_workspace::tools::{ToolError, ToolOutput, ToolRegistry};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

#[derive(Clone, Default)]
pub struct PluginSnapshot {
    pub lib_rs: String,
    pub hash: u64,
}

/// Compressed context for LLM retry (only error-relevant code)
#[derive(Clone, Default)]
pub struct CompressedContext {
    pub error_snippets: Vec<ErrorSnippet>, // Code around error locations
    pub user_code_region: String,          // Full USER_CODE for reference
    pub total_errors: usize,
}

#[derive(Clone)]
pub struct ErrorSnippet {
    pub file: String,
    pub line: u32,
    pub context: String,
    pub error_msg: String,
}

pub struct AgentExecutor {
    tool_registry: Arc<ToolRegistry>,
    max_iterations: usize,
    failure_budget: usize,
    snapshots: HashMap<String, PluginSnapshot>,
}

pub enum ExecutorResult {
    Success {
        output: ToolOutput,
        iterations: usize,
        hash_before: u64,
        hash_after: u64,
    },
    NeedsRetry {
        error_feedback: String,
        iteration: usize,
        hash_mismatch: bool,
        compressed_ctx: Option<CompressedContext>,
    },
    MaxIterationsExceeded {
        last_error: String,
    },
    RolledBack {
        plugin_name: String,
        restored_hash: u64,
    },
}

impl AgentExecutor {
    pub fn new(tool_registry: Arc<ToolRegistry>, max_iterations: usize) -> Self {
        Self {
            tool_registry,
            max_iterations,
            failure_budget: 3,
            snapshots: HashMap::new(),
        }
    }

    fn read_lib_rs(plugin_name: &str) -> Option<String> {
        let path = PathBuf::from("plugins/extra_node")
            .join(plugin_name)
            .join("src/lib.rs");
        std::fs::read_to_string(&path).ok()
    }

    fn write_lib_rs(plugin_name: &str, content: &str) -> bool {
        let path = PathBuf::from("plugins/extra_node")
            .join(plugin_name)
            .join("src/lib.rs");
        std::fs::write(&path, content).is_ok()
    }

    fn extract_user_region(s: &str) -> Option<String> {
        let (a, b) = ("// === USER_CODE_BEGIN ===", "// === USER_CODE_END ===");
        let i = s.find(a)? + a.len();
        let j = s.find(b)?;
        if j <= i {
            return None;
        }
        Some(s[i..j].to_string())
    }

    /// Fast pre-check using cargo check (no codegen, faster feedback)
    pub fn fast_check(plugin_name: &str) -> Vec<CompilerError> {
        let crate_dir = PathBuf::from("plugins/extra_node").join(plugin_name);
        check_fast(&crate_dir).unwrap_or_default()
    }

    /// Build compressed context from compiler errors (smart context compression)
    pub fn compress_context(plugin_name: &str, errors: &[CompilerError]) -> CompressedContext {
        let code = Self::read_lib_rs(plugin_name).unwrap_or_default();
        let lines: Vec<&str> = code.lines().collect();
        let user_code = Self::extract_user_region(&code).unwrap_or_default();

        let mut snippets = Vec::new();
        for err in errors.iter().take(5) {
            if err.line == 0 {
                continue;
            }
            let line_idx = (err.line as usize).saturating_sub(1);
            let start = line_idx.saturating_sub(3);
            let end = (line_idx + 4).min(lines.len());
            let ctx: String = lines[start..end]
                .iter()
                .enumerate()
                .map(|(i, l)| {
                    let ln = start + i + 1;
                    let marker = if ln == err.line as usize {
                        " >>> "
                    } else {
                        "     "
                    };
                    format!("{:4}|{}{}\n", ln, marker, l)
                })
                .collect();
            snippets.push(ErrorSnippet {
                file: err.file.clone(),
                line: err.line,
                context: ctx,
                error_msg: format!("[{}] {}", err.code, err.message),
            });
        }

        CompressedContext {
            error_snippets: snippets,
            user_code_region: user_code,
            total_errors: errors.len(),
        }
    }

    /// Format compressed context for LLM (token-efficient)
    pub fn format_compressed_context(ctx: &CompressedContext) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "[ERRORS: {} total, showing {}]\n\n",
            ctx.total_errors,
            ctx.error_snippets.len()
        ));
        for (i, s) in ctx.error_snippets.iter().enumerate() {
            out.push_str(&format!(
                "=== Error {} at {}:{} ===\n{}\n{}\n\n",
                i + 1,
                s.file,
                s.line,
                s.error_msg,
                s.context
            ));
        }
        if !ctx.user_code_region.is_empty() {
            let uc_lines = ctx.user_code_region.lines().count();
            if uc_lines > 50 {
                let preview: String = ctx
                    .user_code_region
                    .lines()
                    .take(30)
                    .collect::<Vec<_>>()
                    .join("\n");
                out.push_str(&format!(
                    "[USER_CODE: {} lines, showing first 30]\n{}\n...\n",
                    uc_lines, preview
                ));
            } else {
                out.push_str(&format!(
                    "[USER_CODE: {} lines]\n{}\n",
                    uc_lines, ctx.user_code_region
                ));
            }
        }
        out
    }

    /// Snapshot current lib.rs before modification
    pub fn snapshot_before(&mut self, plugin_name: &str) {
        if let Some(code) = Self::read_lib_rs(plugin_name) {
            let h = fnv1a64(code.as_bytes());
            self.snapshots.entry(plugin_name.to_string()).or_default();
            if h != 0 && !code.trim().is_empty() {
                let e = self.snapshots.get_mut(plugin_name).unwrap();
                if e.lib_rs.is_empty() {
                    e.lib_rs = code.clone();
                    e.hash = h;
                }
            }
        }
    }

    /// Record successful state
    pub fn record_success(&mut self, plugin_name: &str) {
        if let Some(code) = Self::read_lib_rs(plugin_name) {
            let h = fnv1a64(code.as_bytes());
            self.snapshots.insert(
                plugin_name.to_string(),
                PluginSnapshot {
                    lib_rs: code,
                    hash: h,
                },
            );
        }
    }

    /// Rollback to last successful state
    pub fn rollback(&mut self, plugin_name: &str) -> Option<u64> {
        let snap = self.snapshots.get(plugin_name)?;
        if snap.lib_rs.is_empty() {
            return None;
        }
        Self::write_lib_rs(plugin_name, &snap.lib_rs);
        Some(snap.hash)
    }

    pub fn try_apply_nodespec(&mut self, nodespec: Value) -> ExecutorResult {
        let plugin_name = nodespec
            .get("plugin_name")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        if !plugin_name.is_empty() {
            self.snapshot_before(&plugin_name);
        }
        let hash_before = Self::read_lib_rs(&plugin_name)
            .map(|s| fnv1a64(s.as_bytes()))
            .unwrap_or(0);
        let mut consecutive_failures = 0usize;

        for i in 0..self.max_iterations {
            let tool = match self.tool_registry.get("apply_rust_nodespec") {
                Some(t) => t,
                None => {
                    return ExecutorResult::MaxIterationsExceeded {
                        last_error: "Tool 'apply_rust_nodespec' not found".to_string(),
                    }
                }
            };
            let args = serde_json::json!({ "nodespec": nodespec });
            match tool.execute(args) {
                Ok(output) => {
                    let hash_after = Self::read_lib_rs(&plugin_name)
                        .map(|s| fnv1a64(s.as_bytes()))
                        .unwrap_or(0);

                    if let Some(diag) = extract_ai_diag(output.text()) {
                        let b = diag
                            .get("build_status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        let s = diag
                            .get("smoke_status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("");
                        if b == "ok" && (s == "passed" || s == "none") {
                            self.record_success(&plugin_name);
                            return ExecutorResult::Success {
                                output,
                                iterations: i + 1,
                                hash_before,
                                hash_after,
                            };
                        }
                        consecutive_failures += 1;
                        if consecutive_failures >= self.failure_budget {
                            if let Some(h) = self.rollback(&plugin_name) {
                                return ExecutorResult::RolledBack {
                                    plugin_name,
                                    restored_hash: h,
                                };
                            }
                        }
                        // Use fast_check + compressed context for better LLM feedback
                        let fast_errors = Self::fast_check(&plugin_name);
                        let compressed = if !fast_errors.is_empty() {
                            Some(Self::compress_context(&plugin_name, &fast_errors))
                        } else {
                            None
                        };
                        let fb = minimal_retry_feedback(&diag, &nodespec);
                        return ExecutorResult::NeedsRetry {
                            error_feedback: fb,
                            iteration: i + 1,
                            hash_mismatch: hash_before != 0 && hash_after == hash_before,
                            compressed_ctx: compressed,
                        };
                    }

                    if output.text().contains("PASSED")
                        || output.text().contains("Applied NodeSpec (no smoke_test)")
                    {
                        self.record_success(&plugin_name);
                        return ExecutorResult::Success {
                            output,
                            iterations: i + 1,
                            hash_before,
                            hash_after,
                        };
                    }
                    if output.text().contains("FAILED") || output.text().contains("error") {
                        consecutive_failures += 1;
                        if consecutive_failures >= self.failure_budget {
                            if let Some(h) = self.rollback(&plugin_name) {
                                return ExecutorResult::RolledBack {
                                    plugin_name,
                                    restored_hash: h,
                                };
                            }
                        }
                        let fast_errors = Self::fast_check(&plugin_name);
                        let compressed = if !fast_errors.is_empty() {
                            Some(Self::compress_context(&plugin_name, &fast_errors))
                        } else {
                            None
                        };
                        return ExecutorResult::NeedsRetry {
                            error_feedback: output.text().to_string(),
                            iteration: i + 1,
                            hash_mismatch: false,
                            compressed_ctx: compressed,
                        };
                    }
                    self.record_success(&plugin_name);
                    return ExecutorResult::Success {
                        output,
                        iterations: i + 1,
                        hash_before,
                        hash_after,
                    };
                }
                Err(ToolError(msg)) => {
                    consecutive_failures += 1;
                    if consecutive_failures >= self.failure_budget {
                        if let Some(h) = self.rollback(&plugin_name) {
                            return ExecutorResult::RolledBack {
                                plugin_name,
                                restored_hash: h,
                            };
                        }
                    }
                    if i + 1 >= self.max_iterations {
                        return ExecutorResult::MaxIterationsExceeded { last_error: msg };
                    }
                    let fast_errors = Self::fast_check(&plugin_name);
                    let compressed = if !fast_errors.is_empty() {
                        Some(Self::compress_context(&plugin_name, &fast_errors))
                    } else {
                        None
                    };
                    return ExecutorResult::NeedsRetry {
                        error_feedback: msg,
                        iteration: i + 1,
                        hash_mismatch: false,
                        compressed_ctx: compressed,
                    };
                }
            }
        }
        ExecutorResult::MaxIterationsExceeded {
            last_error: "Unknown".to_string(),
        }
    }

    pub fn format_retry_feedback(result: &ExecutorResult) -> Option<String> {
        match result {
            ExecutorResult::NeedsRetry {
                error_feedback,
                iteration,
                hash_mismatch,
                compressed_ctx,
            } => {
                let mut msg = format!(
                    "[Agent Iteration {}] The previous attempt failed.",
                    iteration
                );
                if *hash_mismatch {
                    msg.push_str(" WARNING: Patch did not modify lib.rs (hash unchanged). Ensure old text matches exactly.");
                }
                // Use compressed context if available (more token-efficient)
                if let Some(ctx) = compressed_ctx {
                    msg.push_str("\n\n[COMPRESSED ERROR CONTEXT]\n");
                    msg.push_str(&Self::format_compressed_context(ctx));
                } else {
                    msg.push_str(&format!("\n\n{}", error_feedback));
                }
                Some(msg)
            }
            ExecutorResult::MaxIterationsExceeded { last_error } => Some(format!(
                "[Agent] Max iterations exceeded. Last error:\n{}",
                last_error
            )),
            ExecutorResult::RolledBack {
                plugin_name,
                restored_hash,
            } => Some(format!(
                "[Agent] Failure budget exceeded. Rolled back {} to previous state (hash {:016x}).",
                plugin_name, restored_hash
            )),
            ExecutorResult::Success { .. } => None,
        }
    }
}

fn extract_ai_diag(text: &str) -> Option<Value> {
    let m = "AI_DIAG_JSON:";
    let i = text.find(m)? + m.len();
    let s = text[i..].trim();
    let s = if let Some(j) = s.find("Build log") {
        &s[..j]
    } else {
        s
    };
    serde_json::from_str::<Value>(s.trim()).ok()
}

fn minimal_retry_feedback(diag: &Value, nodespec: &Value) -> String {
    let mut out = String::new();
    let build = diag
        .get("build_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let smoke = diag
        .get("smoke_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    out.push_str("[AI_DIAG]\n");
    out.push_str(&format!("build_status={}\nsmoke_status={}\n", build, smoke));

    if build != "ok" {
        out.push_str("\n[Cargo Errors]\n");
        if let Some(arr) = diag.get("cargo_errors").and_then(|v| v.as_array()) {
            for (idx, e) in arr.iter().take(8).enumerate() {
                let file = e.get("file").and_then(|v| v.as_str()).unwrap_or("");
                let line = e
                    .get("line")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let col = e
                    .get("col")
                    .and_then(|v| v.as_u64())
                    .map(|v| v.to_string())
                    .unwrap_or_default();
                let code = e.get("code").and_then(|v| v.as_str()).unwrap_or("");
                let msg = e.get("message").and_then(|v| v.as_str()).unwrap_or("");
                out.push_str(&format!(
                    "{}. {}:{}:{} {} {}\n",
                    idx + 1,
                    file,
                    line,
                    col,
                    code,
                    msg
                ));
            }
        }
        out.push_str("\nFix the compile errors. Prefer PATCH mode and modify only USER_CODE.\n");
    }

    if smoke == "failed" {
        out.push_str("\n[Smoke Failures]\n");
        if let Some(arr) = diag.get("smoke_fails").and_then(|v| v.as_array()) {
            for (idx, f) in arr.iter().take(8).enumerate() {
                out.push_str(&format!("{}. {}\n", idx + 1, f.as_str().unwrap_or("")));
            }
        }
        out.push_str("\nFix logic or NodeSpec asserts so smoke_test passes.\n");
    }

    let mode = nodespec
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("create");
    if mode != "patch" {
        out.push_str("\nSuggested NodeSpec change: set \"mode\":\"patch\" and use \"user_code_patch\" for minimal edits.\n");
    }

    out
}

pub fn create_tool_result_message(result: &ExecutorResult) -> Message {
    match result {
        ExecutorResult::Success { output, iterations, hash_before, hash_after } => {
            Message::new_user(format!("[[_tool_result_internal_]]\nNodeSpec applied successfully after {} iteration(s). Hash: {:016x} -> {:016x}\n\n{}", iterations, hash_before, hash_after, output.text()))
        }
        ExecutorResult::NeedsRetry { error_feedback, iteration, hash_mismatch, compressed_ctx } => {
            let warn = if *hash_mismatch { " (WARN: hash unchanged - patch may not have applied)" } else { "" };
            let ctx_str = compressed_ctx.as_ref().map(|c| AgentExecutor::format_compressed_context(c)).unwrap_or_default();
            let body = if ctx_str.is_empty() { error_feedback.clone() } else { ctx_str };
            Message::new_user(format!("[[_tool_result_internal_]]\n[Iteration {}{}] Error:\n{}", iteration, warn, body))
        }
        ExecutorResult::MaxIterationsExceeded { last_error } => {
            Message::new_user(format!("[[_tool_result_internal_]]\nMax iterations exceeded. Last error:\n{}", last_error))
        }
        ExecutorResult::RolledBack { plugin_name, restored_hash } => {
            Message::new_user(format!("[[_tool_result_internal_]]\nFailure budget exceeded. Rolled back '{}' to last successful state (hash {:016x}). Please revise your approach.", plugin_name, restored_hash))
        }
    }
}
