//! Local backend (Qwen/LM Studio) prompt builder for AI Workspace.
use crate::libs::ai_service::workspace_prompt::PromptBuilder as WorkspacePromptBuilder;
use crate::tabs_registry::ai_workspace::session::message::Message;
use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

pub struct LocalPromptBuilder;

impl LocalPromptBuilder {
    fn build_system_prompt() -> String {
        let mut prompt = WorkspacePromptBuilder::build_system_prompt();
        prompt.push_str(
            r#"

====================================================
LOCAL BACKEND TOOL-CALL FORMAT (MANDATORY)
====================================================
- When you decide to call a tool, output EXACTLY one JSON object and NOTHING ELSE:
  {"tool_name":"...","args":{...}}
- Do not wrap the JSON in markdown fences.
- Do not include <think> or any other text around the JSON.
- If you need multiple tools, call them one at a time across turns."#,
        );
        prompt
    }

    pub fn build_prompt(
        history: &[Message],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
    ) -> String {
        let system = Self::build_system_prompt();
        let mut out = format!("<|im_start|>system\n{}<|im_end|>\n", system);

        if let Some(tool_defs) = &tools {
            let tools_json: Vec<_> = tool_defs.iter().map(|t| serde_json::json!({"name": t.name, "description": t.description, "parameters": t.parameters})).collect();
            out.push_str(&format!("<|im_start|>system\n[Tool Definitions]\n{}\nCall format: {{\"tool_name\":\"...\",\"args\":{{...}}}}<|im_end|>\n", serde_json::to_string_pretty(&tools_json).unwrap_or_default()));
        }

        for msg in history {
            match msg {
                Message::User { text, .. } => {
                    out.push_str(&format!("<|im_start|>user\n{}<|im_end|>\n", text))
                }
                Message::Ai { content, .. } if !content.trim().is_empty() => {
                    out.push_str(&format!("<|im_start|>assistant\n{}<|im_end|>\n", content))
                }
                _ => {}
            }
        }

        let user_msg = match context {
            Some(ctx) => format!("{}\n\n[AI Workspace Context]\n{}", new_user_input, ctx),
            None => new_user_input.to_string(),
        };
        // 仅用模板开启 thinking：不在 system 里额外“教”think mode
        out.push_str(&format!(
            "<|im_start|>user\n{}<|im_end|>\n<|im_start|>assistant\n<think>\n",
            user_msg
        ));
        out
    }

    pub fn parse_thinking(raw: &str) -> (Option<String>, String) {
        if let (Some(s), Some(e)) = (raw.find("<think>"), raw.find("</think>")) {
            (
                Some(raw[s + 7..e].trim().to_string()),
                format!("{}{}", raw[..s].trim(), raw[e + 8..].trim()),
            )
        } else {
            (None, raw.to_string())
        }
    }
}
