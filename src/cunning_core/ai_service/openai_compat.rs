use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

#[derive(Debug, Clone)]
pub enum OpenAiCompatChatOutput {
    Text(String),
    ToolCalls(Vec<(String, Value)>),
}

pub struct OpenAiCompatClient {
    client: Client,
    base_url: String,
    model: String,
    api_key: String,
}

impl OpenAiCompatClient {
    pub fn new(base_url: String, model: String, api_key: String) -> Self {
        Self {
            client: Client::builder()
                .connect_timeout(Duration::from_secs(2))
                .timeout(Duration::from_secs(30))
                .build()
                .unwrap_or_default(),
            base_url,
            model,
            api_key,
        }
    }

    pub async fn chat_once(&self, prompt: &str, max_tokens: u32) -> Result<String, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/chat/completions", base);
        let mut req = self.client.post(&url);
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(self.api_key.trim());
        }
        let body = json!({
            "model": if self.model.trim().is_empty() { "local-model" } else { self.model.trim() },
            "messages": [{ "role": "user", "content": prompt }],
            "temperature": 0.1,
            "max_tokens": max_tokens,
            "stream": false
        });
        let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!(
                "HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            ));
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;
        v.get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .and_then(|m| m.get("content"))
            .and_then(|c| c.as_str())
            .map(|s| s.to_string())
            .ok_or_else(|| {
                "OpenAI-compatible response missing choices[0].message.content".to_string()
            })
    }

    pub async fn chat_once_with_tools(
        &self,
        messages: Vec<Value>,
        tools: Vec<ToolDefinition>,
        max_tokens: u32,
    ) -> Result<OpenAiCompatChatOutput, String> {
        let base = self.base_url.trim_end_matches('/');
        let url = format!("{}/chat/completions", base);
        let mut req = self.client.post(&url);
        if !self.api_key.trim().is_empty() {
            req = req.bearer_auth(self.api_key.trim());
        }

        let tool_defs: Vec<Value> = tools
            .into_iter()
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters
                    }
                })
            })
            .collect();

        let body = json!({
            "model": if self.model.trim().is_empty() { "local-model" } else { self.model.trim() },
            "messages": messages,
            "temperature": 0.1,
            "max_tokens": max_tokens,
            "stream": false,
            "tools": tool_defs,
            "tool_choice": "auto"
        });

        let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;
        if !resp.status().is_success() {
            return Err(format!(
                "HTTP {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            ));
        }
        let v: Value = resp.json().await.map_err(|e| e.to_string())?;

        let msg = v
            .get("choices")
            .and_then(|c| c.get(0))
            .and_then(|c| c.get("message"))
            .ok_or_else(|| "OpenAI-compatible response missing choices[0].message".to_string())?;

        // New-style tool calls
        if let Some(calls) = msg.get("tool_calls").and_then(|x| x.as_array()) {
            let mut out = Vec::new();
            for c in calls {
                let name = c
                    .get("function")
                    .and_then(|f| f.get("name"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("")
                    .to_string();
                let args_raw = c
                    .get("function")
                    .and_then(|f| f.get("arguments"))
                    .and_then(|x| x.as_str())
                    .unwrap_or("{}");
                if name.trim().is_empty() {
                    continue;
                }
                let args: Value =
                    serde_json::from_str(args_raw).unwrap_or_else(|_| json!({ "_raw": args_raw }));
                out.push((name, args));
            }
            if !out.is_empty() {
                return Ok(OpenAiCompatChatOutput::ToolCalls(out));
            }
        }

        // Legacy single function_call
        if let Some(fc) = msg.get("function_call").and_then(|x| x.as_object()) {
            let name = fc
                .get("name")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let args_raw = fc.get("arguments").and_then(|x| x.as_str()).unwrap_or("{}");
            if !name.trim().is_empty() {
                let args: Value =
                    serde_json::from_str(args_raw).unwrap_or_else(|_| json!({ "_raw": args_raw }));
                return Ok(OpenAiCompatChatOutput::ToolCalls(vec![(name, args)]));
            }
        }

        // Plain text
        let text = msg
            .get("content")
            .and_then(|c| c.as_str())
            .unwrap_or("")
            .to_string();
        Ok(OpenAiCompatChatOutput::Text(text))
    }
}
