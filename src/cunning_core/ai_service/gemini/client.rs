use super::parser::StreamParser;
use crate::tabs_registry::ai_workspace::session::event::SessionEvent;
use crate::tabs_registry::ai_workspace::session::prompt::PromptBuilder;
use crate::tabs_registry::ai_workspace::session::thread_entry::ThreadEntry;
use bevy::log::{error, info, warn};
use futures_lite::StreamExt; // for .next()
use reqwest::{Client, StatusCode};
use serde_json::Value;
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::time::sleep;
// Zed-like: short connect timeout + stream idle timeout to avoid "dead" hangs.

pub struct GeminiClient {
    client: Client,
    model: String,
    api_key: String,
}

use crate::tabs_registry::ai_workspace::tools::ToolDefinition;

// Single chat request error types, used for retry decisions
enum ChatError {
    Aborted,
    Network(String),
    Http { status: StatusCode, body: String },
    Stream(String),
    Json(String),
}

// No timeouts; user abort controls cancellation.

impl GeminiClient {
    pub fn new(api_key: String, model: String) -> Self {
        let env_model = std::env::var("CUNNING_GEMINI_MODEL").unwrap_or_default();
        // Single source of truth: UI/settings win; env vars are defaults only.
        let model = if model.trim().is_empty() { env_model } else { model };
        let model = if model.trim().is_empty() { "gemini-3-pro-preview".to_string() } else { model };
        let model = super::normalize_model_name(&model);

        let client = match Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .build()
        {
            Ok(c) => c,
            Err(e) => {
                // Fallback to default client to avoid system-wide failure due to config error
                error!(
                    "Failed to build Gemini HTTP client with custom timeouts: {}",
                    e
                );
                Client::new()
            }
        };

        Self {
            client,
            model,
            api_key,
        }
    }

    /// Extract the next complete JSON value (object or array) from buffer, return it stringified, and remove prefix from buffer.
    fn drain_next_json(buffer: &mut String) -> Option<String> {
        // Remove leading whitespace, commas, and outer '[' if start of stream array
        while let Some(first_char) = buffer.chars().next() {
            if first_char.is_whitespace()
                || first_char == ','
                || first_char == '['
                || first_char == ']'
            {
                buffer.drain(..1);
            } else {
                break;
            }
        }

        if buffer.is_empty() {
            return None;
        }

        let mut chars = buffer.char_indices();
        let (start_idx, _first_ch) = match chars.next() {
            Some((idx, ch)) if ch == '{' => (idx, ch), // Only handle objects as we stripped array brackets
            // Non-'{' content likely stream end marker or garbage, discard
            _ => {
                buffer.drain(..1);
                return None;
            }
        };

        let mut depth = 1i32;
        let mut in_string = false;
        let mut escape = false;

        for (i, ch) in chars {
            if in_string {
                if escape {
                    escape = false;
                    continue;
                }
                match ch {
                    '\\' => escape = true,
                    '"' => in_string = false,
                    _ => {}
                }
            } else {
                match ch {
                    '"' => in_string = true,
                    '{' | '[' => {
                        depth += 1;
                    }
                    '}' | ']' => {
                        depth -= 1;
                        if depth == 0 {
                            let end = i + ch.len_utf8();
                            let json_str = buffer[start_idx..end].to_string();
                            buffer.drain(..end);
                            return Some(json_str);
                        }
                    }
                    _ => {}
                }
            }
        }

        // Not enough content for complete JSON, wait for more bytes
        None
    }

    pub async fn stream_chat(
        &self,
        entries: &[ThreadEntry],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        parser: &mut StreamParser,
        callback: impl Fn(SessionEvent),
        abort_signal: oneshot::Receiver<()>,
    ) {
        self.stream_chat_with_images(
            entries,
            new_user_input,
            context,
            tools,
            &[],
            parser,
            callback,
            abort_signal,
        )
        .await;
    }

    pub async fn stream_chat_with_images(
        &self,
        entries: &[ThreadEntry],
        new_user_input: &str,
        context: Option<&str>,
        tools: Option<Vec<ToolDefinition>>,
        images: &[crate::tabs_registry::ai_workspace::session::message::ImageAttachment],
        parser: &mut StreamParser,
        callback: impl Fn(SessionEvent),
        mut abort_signal: oneshot::Receiver<()>,
    ) {
        if self.api_key.is_empty() {
            let p = crate::runtime_paths::ai_providers_path();
            callback(SessionEvent::Error(format!(
                "Missing Gemini API key (set GEMINI_API_KEY / GOOGLE_API_KEY / GOOGLE_GENERATIVE_AI_API_KEY, or {} -> gemini.api_key).",
                p.display()
            )));
            return;
        }

        let mut models_to_try: Vec<String> = Vec::new();
        let m = self.model.trim();
        if !m.is_empty() { models_to_try.push(m.to_string()); }
        if models_to_try.is_empty() { models_to_try.push("gemini-3-flash-preview".to_string()); }

        info!(
            "[AI Workspace] Gemini models_to_try={:?} (model='{}')",
            models_to_try,
            self.model
        );

        let body = PromptBuilder::build_full_request_gemini_from_entries_with_images(
            entries,
            new_user_input,
            context,
            tools,
            images,
        );
        if let Ok(()) = abort_signal.try_recv() {
            callback(SessionEvent::Error("Current request aborted".to_string()));
            return;
        }
        for (attempt, model) in models_to_try.iter().enumerate() {
            info!("[AI Workspace] Gemini attempt={} model='{}'", attempt + 1, model);
            let stream_url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
                model, self.api_key
            );
            let non_stream_url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
                model, self.api_key
            );
            match self
                .do_single_stream_request(&stream_url, &body, parser, &callback, &mut abort_signal)
                .await
            {
                Ok(()) => return,
                Err(ChatError::Http { status, body: _ }) if status == StatusCode::NOT_FOUND => {
                    // Stream endpoint or model may be missing; fall back to non-stream for this model.
                    match self
                        .do_single_non_stream_request(&non_stream_url, &body, &callback, &mut abort_signal)
                        .await
                    {
                        Ok(()) => return,
                        Err(ChatError::Http { status, body }) if status == StatusCode::NOT_FOUND => {
                            if attempt + 1 == models_to_try.len() {
                                callback(SessionEvent::Error(format!(
                                    "Gemini HTTP {} Not Found for model '{}': {}",
                                    status,
                                    model,
                                    if body.trim().is_empty() { "<empty body>".to_string() } else { body }
                                )));
                                return;
                            }
                            continue;
                        }
                        Err(ChatError::Aborted) => {
                            callback(SessionEvent::Error("Current request aborted".to_string()));
                            return;
                        }
                        Err(ChatError::Http { status, body }) => {
                            callback(SessionEvent::Error(format!(
                                "Gemini HTTP {} for model '{}': {}",
                                status,
                                model,
                                if body.trim().is_empty() { "<empty body>".to_string() } else { body }
                            )));
                            return;
                        }
                        Err(ChatError::Network(e)) => {
                            callback(SessionEvent::Error(format!("Network error: {}", e)));
                            return;
                        }
                        Err(ChatError::Stream(e)) => {
                            callback(SessionEvent::Error(format!("Stream error: {}", e)));
                            return;
                        }
                        Err(ChatError::Json(e)) => {
                            callback(SessionEvent::Error(format!("JSON parse error: {}", e)));
                            return;
                        }
                    }
                }
                Err(ChatError::Aborted) => {
                    callback(SessionEvent::Error("Current request aborted".to_string()));
                    return;
                }
                Err(ChatError::Http { status, body }) => {
                    let can_fallback = attempt + 1 < models_to_try.len();
                    let is_retryable = status == StatusCode::TOO_MANY_REQUESTS
                        || status == StatusCode::SERVICE_UNAVAILABLE
                        || status == StatusCode::INTERNAL_SERVER_ERROR
                        || status == StatusCode::GATEWAY_TIMEOUT;
                    if can_fallback && is_retryable {
                        let ms = (600u64 * (attempt as u64 + 1)).min(3000);
                        warn!(
                            "[AI Workspace] Gemini HTTP {} for model='{}' (will retry/fallback in {}ms)",
                            status, model, ms
                        );
                        sleep(Duration::from_millis(ms)).await;
                        continue;
                    }
                    callback(SessionEvent::Error(format!(
                        "Gemini HTTP {} for model '{}': {}",
                        status,
                        model,
                        if body.trim().is_empty() { "<empty body>".to_string() } else { body }
                    )));
                    return;
                }
                Err(ChatError::Network(e)) => {
                    callback(SessionEvent::Error(format!("Network error: {}", e)));
                    return;
                }
                Err(ChatError::Stream(e)) => {
                    callback(SessionEvent::Error(format!("Stream error: {}", e)));
                    return;
                }
                Err(ChatError::Json(e)) => {
                    callback(SessionEvent::Error(format!("JSON parse error: {}", e)));
                    return;
                }
            }
        }
    }

    async fn do_single_non_stream_request<F>(
        &self,
        url: &str,
        body: &Value,
        callback: &F,
        abort_signal: &mut oneshot::Receiver<()>,
    ) -> Result<(), ChatError>
    where
        F: Fn(SessionEvent),
    {
        let request_fut = self.client.post(url).json(body).send();
        let response = tokio::select! {
            result = &mut *abort_signal => {
                if result.is_ok() { return Err(ChatError::Aborted); }
                return Err(ChatError::Network("request superseded".to_string()));
            }
            result = request_fut => { result.map_err(|e| ChatError::Network(e.to_string()))? }
        };
        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(ChatError::Http { status, body: text });
        }
        let json: Value = response
            .json()
            .await
            .map_err(|e| ChatError::Json(e.to_string()))?;
        let mut full_text = String::new();
        let mut saw_tool = false;
        if let Some(candidates) = json["candidates"].as_array() {
            if let Some(first) = candidates.first() {
                if let Some(parts) = first["content"]["parts"].as_array() {
                    for part in parts {
                        if let Some(func) = part.get("functionCall").and_then(|v| v.as_object()) {
                            if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                let args = func.get("args").cloned().unwrap_or_else(|| serde_json::json!({}));
                                callback(SessionEvent::ToolCallRequest { tool_name: name.to_string(), args });
                                saw_tool = true;
                            }
                        }
                        if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                            if !text.is_empty() {
                                full_text.push_str(text);
                            }
                        }
                    }
                }
            }
        }
        if saw_tool {
            callback(SessionEvent::StreamedCompletion);
            return Ok(());
        }
        // Emulate streaming (Zed-style: send multiple text chunks).
        let chunk_chars = 64usize;
        let mut buf = String::new();
        for ch in full_text.chars() {
            buf.push(ch);
            if buf.chars().count() >= chunk_chars {
                if let Ok(()) = abort_signal.try_recv() {
                    return Err(ChatError::Aborted);
                }
                callback(SessionEvent::Text(std::mem::take(&mut buf)));
            }
        }
        if !buf.is_empty() {
            if let Ok(()) = abort_signal.try_recv() {
                return Err(ChatError::Aborted);
            }
            callback(SessionEvent::Text(buf));
        }
        callback(SessionEvent::StreamedCompletion);
        Ok(())
    }

    async fn do_single_stream_request<F>(
        &self,
        url: &str,
        body: &Value,
        parser: &mut StreamParser,
        callback: &F,
        abort_signal: &mut oneshot::Receiver<()>,
    ) -> Result<(), ChatError>
    where
        F: Fn(SessionEvent),
    {
        let request_fut = self.client.post(url).json(body).send();

        let response = tokio::select! {
            result = &mut *abort_signal => {
                // Only abort if sender explicitly sent () - NOT if sender was dropped (replaced by new request)
                if result.is_ok() { return Err(ChatError::Aborted); }
                return Err(ChatError::Network("request superseded".to_string()));
            }
            result = request_fut => { result.map_err(|e| ChatError::Network(e.to_string()))? }
        };

        if !response.status().is_success() {
            let status = response.status();
            let text = response.text().await.unwrap_or_default();
            return Err(ChatError::Http { status, body: text });
        }

        // Zed-style SSE: response is newline-delimited "data: {json}" lines.
        let mut stream = response.bytes_stream();
        let mut buffer = String::new(); // UTF-8 line buffer
        let idle_timeout = Duration::from_secs(45);
        let mut last_io = tokio::time::Instant::now();

        loop {
            tokio::select! {
                result = &mut *abort_signal => {
                    // Only abort if sender explicitly sent () - NOT if sender was dropped
                    if result.is_ok() { return Err(ChatError::Aborted); }
                    // Sender dropped (new request started) - gracefully exit this stream
                    break;
                }
                _ = tokio::time::sleep_until(last_io + idle_timeout) => {
                    return Err(ChatError::Stream("idle timeout".to_string()));
                }
                item = stream.next() => {
                    let Some(item) = item else { break; };
                    match item {
                        Ok(bytes) => {
                            last_io = tokio::time::Instant::now();
                            buffer.push_str(&String::from_utf8_lossy(&bytes));
                            while let Some(nl) = buffer.find('\n') {
                                let mut line = buffer[..nl].to_string();
                                buffer.drain(..nl + 1);
                                line = line.trim().to_string();
                                if line.is_empty() { continue; }
                                let Some(data) = line.strip_prefix("data: ") else { continue; };
                                let data = data.trim();
                                if data == "[DONE]" { break; }
                                let json: Value = serde_json::from_str(data).map_err(|e| ChatError::Json(e.to_string()))?;
                                if let Some(candidates) = json["candidates"].as_array() {
                                    if let Some(first) = candidates.first() {
                                        if let Some(parts_arr) = first["content"]["parts"].as_array() {
                                            let mut saw_tool = false;
                                            for part in parts_arr {
                                                if let Some(text) = part.get("text").and_then(|v| v.as_str()) {
                                                    for event in parser.parse(text) { callback(event); }
                                                }
                                                if let Some(func) = part.get("functionCall").and_then(|v| v.as_object()) {
                                                    if let Some(name) = func.get("name").and_then(|v| v.as_str()) {
                                                        let args = func.get("args").cloned().unwrap_or_else(|| serde_json::json!({}));
                                                        callback(SessionEvent::ToolCallRequest { tool_name: name.to_string(), args });
                                                        saw_tool = true;
                                                    }
                                                }
                                            }
                                            if saw_tool {
                                                callback(SessionEvent::StreamedCompletion);
                                                return Ok(());
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => { return Err(ChatError::Stream(e.to_string())); }
                    }
                }
            }
        }

        // Normal stream end, notify upper layer of completion (for unified diff/undo ops)
        callback(SessionEvent::StreamedCompletion);
        Ok(())
    }

    pub async fn generate_title(&self, user_input: &str) -> Result<String, String> {
        if self.api_key.is_empty() {
            let p = crate::runtime_paths::ai_providers_path();
            return Err(format!(
                "Missing Gemini API key (set GEMINI_API_KEY / GOOGLE_API_KEY / GOOGLE_GENERATIVE_AI_API_KEY, or {} -> gemini.api_key).",
                p.display()
            ));
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model, self.api_key
        );

        let prompt = format!("Summarize the following request into a very short title (max 5 words, no quotes). Language: same as input.\n\nRequest: {}", user_input);
        let body = serde_json::json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }]
        });

        let response = self
            .client
            .post(&url)
            .json(&body)
            .send()
            .await
            .map_err(|e| format!("Network error: {}", e))?;
        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status()));
        }

        let json: Value = response
            .json()
            .await
            .map_err(|e| format!("JSON error: {}", e))?;
        if let Some(candidates) = json["candidates"].as_array() {
            if let Some(first) = candidates.first() {
                if let Some(parts) = first["content"]["parts"].as_array() {
                    if let Some(text) = parts.first().and_then(|p| p["text"].as_str()) {
                        return Ok(text.trim().to_string());
                    }
                }
            }
        }
        Err("No content generated".to_string())
    }
}
