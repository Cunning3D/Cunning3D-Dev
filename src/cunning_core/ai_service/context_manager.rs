//! Context assembly with token budgeting.

use super::context_config as cfg;
use super::prefix_cache::StablePrefixCache;
use super::truncate_util;
use crate::tabs_registry::ai_workspace::session::message::Message;
use crate::tabs_registry::ai_workspace::session::thread_entry::{ThreadEntry, ToolCallStatus};
use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

pub struct ContextManager;

impl ContextManager {
    pub fn build_contents_from_entries(
        entries: &[ThreadEntry],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        images: &[crate::tabs_registry::ai_workspace::session::message::ImageAttachment],
        system_instruction: String,
    ) -> serde_json::Value {
        let mut contents: Vec<serde_json::Value> = Vec::new();

        if entries.is_empty() {
            if let Some(doc) = StablePrefixCache::project_doc() {
                contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": format!("[Project Docs]\n{}", doc) }] }));
            }
        }

        if let Some(summary) = entries.iter().rev().find_map(|e| match e {
            ThreadEntry::Assistant { content, .. }
                if content.starts_with("[Previous context summary]") =>
            {
                Some(content.as_str())
            }
            _ => None,
        }) {
            contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": Self::pin_summary(summary) }] }));
        }

        let mut all: Vec<serde_json::Value> = Vec::new();
        let mut pending_tool_results: Vec<serde_json::Value> = Vec::new();
        let mut flush_tool_results =
            |all: &mut Vec<serde_json::Value>, pending: &mut Vec<serde_json::Value>| {
                if pending.is_empty() {
                    return;
                }
                all.push(serde_json::json!({ "role": "user", "parts": std::mem::take(pending) }));
            };

        for e in entries.iter() {
            match e {
                ThreadEntry::ToolCall(c) => {
                    use ToolCallStatus as S;
                    let terminal = matches!(c.status, S::Completed | S::Canceled | S::Failed(_));
                    if !terminal {
                        continue;
                    }
                    let raw_in = c.raw_input.as_deref().unwrap_or("{}");
                    let args: serde_json::Value = serde_json::from_str(raw_in)
                        .unwrap_or_else(|_| serde_json::json!({ "_raw": raw_in }));
                    all.push(serde_json::json!({ "role": "model", "parts": [{ "functionCall": { "name": c.tool_name, "args": args } }] }));

                    let tr = c.to_tool_result().unwrap_or_else(|| {
                        crate::tabs_registry::ai_workspace::session::thread_entry::ToolResult {
                            tool_use_id: c.tool_use_id(),
                            tool_name: c.tool_name.clone(),
                            is_error: true,
                            content: "Tool canceled by user.".into(),
                            debug_output: c.raw_output.clone(),
                        }
                    });
                    let mut resp = serde_json::json!({ "tool_use_id": tr.tool_use_id, "is_error": tr.is_error, "content": Self::compress(&tr.content, cfg::TOOL_LLM_MAX_CHARS) });
                    if let Some(dbg) = tr.debug_output {
                        resp["debug_output"] = serde_json::Value::String(Self::compress(
                            &dbg,
                            cfg::TOOL_LLM_MAX_CHARS,
                        ));
                    }
                    pending_tool_results.push(serde_json::json!({ "functionResponse": { "name": tr.tool_name, "response": resp } }));
                }
                ThreadEntry::User {
                    text,
                    images: msg_images,
                    ..
                } => {
                    flush_tool_results(&mut all, &mut pending_tool_results);
                    let img_note = if msg_images.is_empty() {
                        String::new()
                    } else {
                        format!("[{} image(s)] ", msg_images.len())
                    };
                    let t = format!(
                        "{}{}",
                        img_note,
                        Self::compress(text, cfg::HISTORY_USER_CHARS)
                    );
                    all.push(serde_json::json!({ "role": "user", "parts": [{ "text": t }] }));
                }
                ThreadEntry::Assistant { content, .. } => {
                    flush_tool_results(&mut all, &mut pending_tool_results);
                    if content.starts_with("[Previous context summary]") {
                        continue;
                    }
                    let t = Self::compress(content, cfg::HISTORY_AI_CHARS);
                    all.push(serde_json::json!({ "role": "model", "parts": [{ "text": t }] }));
                }
            }
        }
        flush_tool_results(&mut all, &mut pending_tool_results);

        // Token-budget: keep the most recent suffix of history (like Codex), preserving order.
        let mut used = 0usize;
        let mut kept_rev: Vec<serde_json::Value> = Vec::new();
        for m in all.iter().rev() {
            let est = Self::estimate_tokens(&m.to_string());
            if used + est > cfg::HISTORY_TOKEN_BUDGET && !kept_rev.is_empty() {
                break;
            }
            kept_rev.push(m.clone());
            used += est;
        }
        kept_rev.reverse();
        contents.extend(kept_rev);

        let final_input = match context {
            Some(ctx) => format!(
                "{}\n\n[Context]\n{}",
                new_user_input,
                Self::compress(ctx, cfg::CONTEXT_MAX_CHARS)
            ),
            None => new_user_input.to_string(),
        };
        let mut final_parts: Vec<serde_json::Value> = images
            .iter()
            .map(|img| serde_json::json!({ "inlineData": { "mimeType": img.mime_type, "data": img.data_b64 } }))
            .collect();
        final_parts.push(serde_json::json!({ "text": final_input }));
        contents.push(serde_json::json!({ "role": "user", "parts": final_parts }));

        let mut body = serde_json::json!({
            "systemInstruction": { "parts": [{ "text": system_instruction }] },
            "contents": contents
        });

        if let Some(mut tool_defs) = tools {
            body["tools"] = StablePrefixCache::tool_declarations(tool_defs);
        }
        body
    }

    pub fn build_contents(
        history: &[Message],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        images: &[crate::tabs_registry::ai_workspace::session::message::ImageAttachment],
        system_instruction: String,
    ) -> serde_json::Value {
        let mut contents: Vec<serde_json::Value> = Vec::new();

        if history.is_empty() {
            if let Some(doc) = StablePrefixCache::project_doc() {
                contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": format!("[Project Docs]\n{}", doc) }] }));
            }
        }

        if let Some(summary) = history.iter().rev().find_map(|m| match m {
            Message::Ai { content, .. } if content.starts_with("[Previous context summary]") => {
                Some(content.as_str())
            }
            _ => None,
        }) {
            contents.push(serde_json::json!({ "role": "user", "parts": [{ "text": Self::pin_summary(summary) }] }));
        }

        let mut used = 0usize;
        let mut hist: Vec<serde_json::Value> = Vec::new();
        for msg in history.iter().rev() {
            let (role, parts, token_est) = match msg {
                Message::User {
                    text,
                    images: msg_images,
                } => {
                    if let Some(fr) = Self::parse_tool_result_to_function_response(text) {
                        (
                            "user",
                            vec![serde_json::json!({ "functionResponse": fr })],
                            cfg::TOOL_OUTPUT_TOKEN_LIMIT.min(Self::estimate_tokens(text)),
                        )
                    } else {
                        let img_note = if msg_images.is_empty() {
                            String::new()
                        } else {
                            format!("[{} image(s)] ", msg_images.len())
                        };
                        let t = format!(
                            "{}{}",
                            img_note,
                            Self::compress(text, cfg::HISTORY_USER_CHARS)
                        );
                        let est = Self::estimate_tokens(&t);
                        ("user", vec![serde_json::json!({ "text": t })], est)
                    }
                }
                Message::Ai { content, .. } => {
                    if content.starts_with("[Previous context summary]") {
                        continue;
                    }
                    if let Some(fc) = Self::parse_tool_use_to_function_call(content) {
                        (
                            "model",
                            vec![serde_json::json!({ "functionCall": fc })],
                            cfg::TOOL_OUTPUT_TOKEN_LIMIT.min(Self::estimate_tokens(content)),
                        )
                    } else {
                        let t = Self::compress(content, cfg::HISTORY_AI_CHARS);
                        let est = Self::estimate_tokens(&t);
                        ("model", vec![serde_json::json!({ "text": t })], est)
                    }
                }
            };
            let t = token_est;
            if used + t > cfg::HISTORY_TOKEN_BUDGET && !hist.is_empty() {
                break;
            }
            hist.push(serde_json::json!({ "role": role, "parts": parts }));
            used += t;
        }
        hist.reverse();
        contents.extend(hist);

        let final_input = match context {
            Some(ctx) => format!(
                "{}\n\n[Context]\n{}",
                new_user_input,
                Self::compress(ctx, cfg::CONTEXT_MAX_CHARS)
            ),
            None => new_user_input.to_string(),
        };
        let mut final_parts: Vec<serde_json::Value> = images
            .iter()
            .map(|img| serde_json::json!({ "inlineData": { "mimeType": img.mime_type, "data": img.data_b64 } }))
            .collect();
        final_parts.push(serde_json::json!({ "text": final_input }));
        contents.push(serde_json::json!({ "role": "user", "parts": final_parts }));

        let mut body = serde_json::json!({
            "systemInstruction": { "parts": [{ "text": system_instruction }] },
            "contents": contents
        });

        if let Some(mut tool_defs) = tools {
            body["tools"] = StablePrefixCache::tool_declarations(tool_defs);
        }
        body
    }

    fn estimate_tokens(text: &str) -> usize {
        ((text.chars().count() as f32) / cfg::TOKEN_EST_CHARS_PER_TOKEN).ceil() as usize
    }

    fn compress(text: &str, max_chars: usize) -> String {
        let t = text.trim();
        if t.starts_with("[ToolUse]") || t.starts_with("[ToolResult]") {
            if t.len() <= max_chars {
                return t.to_string();
            }
            return format!("{}...", &t.chars().take(max_chars).collect::<String>());
        }
        if t.starts_with("[[_tool_result_internal_]]")
            || t.starts_with("OK:")
            || t.starts_with("FAILED:")
        {
            let b = t.trim_start_matches("[[_tool_result_internal_]]").trim();
            if b.contains("PASSED") {
                return "[OK: built]".into();
            }
            if b.contains("FAILED") || b.contains("error[E") {
                return "[FAIL]".into();
            }
            if b.contains("CHostApi") || b.contains("CHudCmd") || b.contains("CGizmoCmd") {
                return "[OK: ABI]".into();
            }
            if b.contains("gizmo_primitives") || b.contains("hud_commands") {
                return "[OK: interaction]".into();
            }
            if b.contains("node_count") {
                return "[OK: graph]".into();
            }
            return "[OK]".into();
        }
        if t.len() <= max_chars {
            t.to_string()
        } else {
            format!("{}...", &t.chars().take(max_chars).collect::<String>())
        }
    }

    fn pin_summary(summary: &str) -> String {
        let s = summary.trim();
        if truncate_util::approx_token_count(s) > 2000 {
            truncate_util::truncate_mid_tokens(s, 2000)
        } else {
            s.to_string()
        }
    }

    fn parse_tool_use_to_function_call(s: &str) -> Option<serde_json::Value> {
        let t = s.trim();
        if !t.starts_with("[ToolUse]") {
            return None;
        }
        let head = t.lines().next().unwrap_or("");
        let name = head
            .split_whitespace()
            .find_map(|p| p.strip_prefix("name="))?
            .to_string();
        let args_txt = t
            .split("```json")
            .nth(1)
            .and_then(|rest| rest.split("```").next())
            .map(|s| s.trim())
            .unwrap_or("{}");
        let mut args: serde_json::Value = serde_json::from_str(args_txt)
            .unwrap_or_else(|_| serde_json::json!({ "_raw": args_txt }));
        if !args.is_object() {
            args = serde_json::json!({ "_": args });
        }
        Some(serde_json::json!({ "name": name, "args": args }))
    }

    fn parse_tool_result_to_function_response(s: &str) -> Option<serde_json::Value> {
        let t = s.trim();
        if !t.starts_with("[ToolResult]") {
            return None;
        }
        let mut lines = t.lines();
        let head = lines.next().unwrap_or("");
        let name = head
            .split_whitespace()
            .find_map(|p| p.strip_prefix("name="))?
            .to_string();
        let id = head
            .split_whitespace()
            .find_map(|p| p.strip_prefix("id="))
            .unwrap_or("")
            .to_string();
        let status = head
            .split_whitespace()
            .find_map(|p| p.strip_prefix("status="))
            .unwrap_or("")
            .to_string();
        let is_error = head
            .split_whitespace()
            .find_map(|p| p.strip_prefix("is_error="))
            .map(|v| v == "true")
            .unwrap_or(false);
        let rest = lines.collect::<Vec<_>>().join("\n");
        let (content, debug_output) = if let Some((a, b)) = rest.split_once("\n\n[DebugOutput]\n") {
            (a.trim().to_string(), Some(b.trim().to_string()))
        } else {
            (rest.trim().to_string(), None)
        };
        let mut resp = serde_json::json!({ "tool_use_id": id, "status": status, "is_error": is_error, "content": content });
        if let Some(dbg) = debug_output {
            resp["debug_output"] = serde_json::Value::String(dbg);
        }
        Some(serde_json::json!({ "name": name, "response": resp }))
    }
}

#[cfg(all(test, feature = "ai_ws_gemini_structured_tests"))]
mod gemini_structured_tests {
    use super::*;
    use crate::tabs_registry::ai_workspace::session::message::{ImageAttachment, MessageState};
    use crate::tabs_registry::ai_workspace::session::thread_entry::{
        ThreadEntry, ToolCall, ToolCallStatus,
    };

    fn tc(id: u64, name: &str, raw: &str) -> ToolCall {
        let mut c = ToolCall::new(id, name.to_string(), raw.to_string());
        c.raw_input = Some(raw.to_string());
        c.llm_result = Some("OK".into());
        c.raw_output = Some("RAW".into());
        c.status = ToolCallStatus::Completed;
        c
    }

    #[test]
    fn gemini_entries_emit_functioncall_then_aggregated_functionresponses() {
        let entries = vec![
            ThreadEntry::User {
                text: "hi".into(),
                images: vec![],
                mentions: vec![],
                timestamp: 0,
            },
            ThreadEntry::Assistant {
                thinking: None,
                content: "ok".into(),
                state: MessageState::Done,
                timestamp: 0,
            },
            ThreadEntry::ToolCall(tc(1, "read_file", "{\"file_path\":\"a\"}")),
            ThreadEntry::ToolCall(tc(2, "search_workspace", "{\"pattern\":\"x\"}")),
        ];
        let body = ContextManager::build_contents_from_entries(
            &entries,
            "next",
            None,
            None,
            &[],
            "sys".into(),
        );
        let contents = body.get("contents").and_then(|v| v.as_array()).unwrap();
        let mut saw_call = 0usize;
        let mut saw_resp_parts = 0usize;
        for c in contents {
            if let Some(parts) = c.get("parts").and_then(|p| p.as_array()) {
                for p in parts {
                    if p.get("functionCall").is_some() {
                        saw_call += 1;
                    }
                    if p.get("functionResponse").is_some() {
                        saw_resp_parts += 1;
                    }
                }
            }
        }
        assert_eq!(saw_call, 2);
        assert_eq!(saw_resp_parts, 2);
        // Aggregation: there should be exactly one user content containing both functionResponse parts.
        let user_resp_msgs = contents
            .iter()
            .filter(|c| c.get("role").and_then(|r| r.as_str()) == Some("user"))
            .filter(|c| {
                c.get("parts")
                    .and_then(|p| p.as_array())
                    .is_some_and(|ps| ps.iter().any(|x| x.get("functionResponse").is_some()))
            })
            .count();
        assert_eq!(user_resp_msgs, 1);
    }
}
