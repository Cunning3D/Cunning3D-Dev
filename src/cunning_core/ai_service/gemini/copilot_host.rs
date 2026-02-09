use bevy::prelude::*;
use reqwest::blocking::Client;
use reqwest::header;
use serde_json::Value;
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::Duration;

#[derive(Resource)]
pub struct GeminiCopilotHost {
    tx: Sender<GeminiCopilotRequest>,
    rx: std::sync::Mutex<Receiver<GeminiCopilotResponse>>,
}

pub struct GeminiCopilotRequest {
    pub id: String,
    pub prompt: String,
}
#[derive(Clone, Debug)]
pub struct GeminiCopilotResponse {
    pub id: String,
    pub text: String,
    pub error: Option<String>,
}

impl GeminiCopilotHost {
    pub fn new() -> Self {
        let (tx_req, rx_req) = mpsc::channel::<GeminiCopilotRequest>();
        let (tx_res, rx_res) = mpsc::channel::<GeminiCopilotResponse>();
        let inflight = std::sync::Arc::new((std::sync::Mutex::new(0usize), std::sync::Condvar::new()));
        thread::spawn(move || {
            fn post_json_with_retry(
                client: &Client,
                api_key: &str,
                model: &str,
                body: &serde_json::Value,
                max_attempts: usize,
            ) -> Result<(u16, String), String> {
                let url = format!("https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent", model);
                let mut attempt = 0usize;
                let mut backoff = Duration::from_millis(250);
                loop {
                    attempt += 1;
                    let r = client
                        .post(&url)
                        .header(header::CONTENT_TYPE, "application/json")
                        .header("x-goog-api-key", api_key)
                        .json(body)
                        .send();
                    match r {
                        Ok(resp) => {
                            let status = resp.status().as_u16();
                            let txt = resp.text().unwrap_or_default();
                            return Ok((status, txt));
                        }
                        Err(e) => {
                            let retryable = e.is_connect() || e.is_timeout() || e.is_request();
                            if attempt >= max_attempts || !retryable {
                                return Err(e.to_string());
                            }
                            thread::sleep(backoff);
                            backoff = (backoff * 2).min(Duration::from_millis(2000));
                        }
                    }
                }
            }
            fn load_gemini_key_from_settings() -> Option<String> {
                let cwd = std::env::current_dir().ok()?;
                let path = cwd.join("settings/ai/providers.json");
                let raw = std::fs::read_to_string(path).ok()?;
                let v: Value = serde_json::from_str(&raw).ok()?;
                v.get("gemini")
                    .and_then(|g| g.get("api_key"))
                    .and_then(|x| x.as_str())
                    .map(str::to_string)
                    .filter(|s| !s.trim().is_empty())
            }
            fn load_gemini_models_from_settings() -> (Option<String>, Option<String>) {
                let Some(cwd) = std::env::current_dir().ok() else { return (None, None); };
                let path = cwd.join("settings/ai/providers.json");
                let Ok(raw) = std::fs::read_to_string(path) else { return (None, None); };
                let Ok(v) = serde_json::from_str::<Value>(&raw) else { return (None, None); };
                let Some(g) = v.get("gemini") else { return (None, None); };
                let pro = g
                    .get("model_pro")
                    .or_else(|| g.get("model"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                let flash = g
                    .get("model_flash")
                    .or_else(|| g.get("model_text"))
                    .and_then(|x| x.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                (pro, flash)
            }
            let client = Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(60))
                .build()
                .unwrap_or_else(|e| {
                error!("GeminiCopilotHost: failed to build reqwest client: {}", e);
                Client::new()
            });
            let api_key = std::env::var("GEMINI_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(load_gemini_key_from_settings)
                .unwrap_or_default();
            let (pro_cfg, flash_cfg) = load_gemini_models_from_settings();
            let model_pro = std::env::var("CUNNING_GEMINI_MODEL_PRO")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or(pro_cfg)
                .unwrap_or_else(|| "gemini-3-pro-preview".to_string());
            let model_flash = std::env::var("CUNNING_GEMINI_MODEL_FLASH")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .or(flash_cfg)
                .unwrap_or_else(|| "gemini-3-flash".to_string());
            const MAX_INFLIGHT: usize = 8;
            while let Ok(req) = rx_req.recv() {
                let inflight = inflight.clone();
                let client = client.clone();
                let api_key = api_key.clone();
                let model_pro = model_pro.clone();
                let model_flash = model_flash.clone();
                let tx_res = tx_res.clone();
                {
                    let (m, cv) = &*inflight;
                    let mut n = m.lock().unwrap_or_else(|e| e.into_inner());
                    while *n >= MAX_INFLIGHT {
                        n = cv.wait(n).unwrap_or_else(|e| e.into_inner());
                    }
                    *n += 1;
                }
                thread::spawn(move || {
                    info!(
                        "GeminiCopilotHost: request id={} prompt_len={}",
                        req.id,
                        req.prompt.len()
                    );
                    let do_req = |prompt: &str, max_tokens: i64| -> Result<(Value, String), String> {
                        let body = serde_json::json!({
                            "contents": [{"role": "user", "parts": [{"text": prompt}]}],
                            "generationConfig": {
                                "temperature": 0.2,
                                "topP": 0.9,
                                "maxOutputTokens": max_tokens,
                                "responseMimeType": "application/json"
                            }
                        });
                        let try_model = |m: &str| -> Result<(u16, String), String> {
                            post_json_with_retry(&client, &api_key, m, &body, 3)
                        };
                        // Speed-first: Flash by default; fallback to Pro only when model is missing (404).
                        let (status, txt) = match try_model(&model_flash) {
                            Ok((404, _)) => try_model(&model_pro)?,
                            Ok(v) => v,
                            Err(e) => return Err(e),
                        };
                        if status < 200 || status >= 300 {
                            return Err(format!("HTTP {}: {}", status, txt));
                        }
                        let v = serde_json::from_str::<Value>(&txt).map_err(|e| format!("JSON: {}", e))?;
                        Ok((v, txt))
                    };
                    let extract_text = |v: &Value| -> (String, Option<String>) {
                        let parts = v
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("content"))
                            .and_then(|c| c.get("parts"))
                            .and_then(|p| p.as_array())
                            .cloned()
                            .unwrap_or_default();
                        let mut out = String::new();
                        for p in parts {
                            if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                                out.push_str(t);
                            }
                        }
                        let finish = v
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("finishReason"))
                            .and_then(|x| x.as_str())
                            .map(|s| s.to_string());
                        (out, finish)
                    };
                    let out: Result<(String, Option<String>), String> = do_req(&req.prompt, 2048).and_then(|(v, raw_txt)| {
                        let (out, finish) = extract_text(&v);
                        if !out.trim().is_empty() {
                            return Ok((out, None));
                        }
                        let cand_len = v
                            .get("candidates")
                            .and_then(|c| c.as_array())
                            .map(|a| a.len())
                            .unwrap_or(0);
                        let prompt_feedback = v.get("promptFeedback").map(|x| x.to_string());
                        let safety = v
                            .get("candidates")
                            .and_then(|c| c.get(0))
                            .and_then(|c| c.get("safetyRatings"))
                            .map(|x| x.to_string());
                        let thoughts = v
                            .get("usageMetadata")
                            .and_then(|m| m.get("thoughtsTokenCount"))
                            .and_then(|x| x.as_i64());
                        let raw_preview: String = raw_txt.chars().take(900).collect();
                        warn!(
                            "GeminiCopilotHost: empty text id={} candidates_len={} finishReason={:?} thoughtsTokens={:?} promptFeedback={:?} safetyRatings={:?} raw_preview={:?}",
                            req.id, cand_len, finish, thoughts, prompt_feedback, safety, raw_preview
                        );
                        thread::sleep(Duration::from_millis(120));
                        let retry_prompt = format!("{}\n\nReturn ONLY a JSON array. No extra text.", req.prompt);
                        if let Ok((v2, raw2)) = do_req(&retry_prompt, 4096) {
                            let (out2, finish2) = extract_text(&v2);
                            if !out2.trim().is_empty() {
                                return Ok((out2, None));
                            }
                            let thoughts2 = v2
                                .get("usageMetadata")
                                .and_then(|m| m.get("thoughtsTokenCount"))
                                .and_then(|x| x.as_i64());
                            let raw_preview2: String = raw2.chars().take(600).collect();
                            warn!(
                                "GeminiCopilotHost: retry still empty id={} finishReason={:?} thoughtsTokens={:?} raw_preview={:?}",
                                req.id, finish2, thoughts2, raw_preview2
                            );
                            return Ok((String::new(), Some(format!("Empty Gemini text (finishReason={:?})", finish2))));
                        }
                        Ok((String::new(), Some(format!("Empty Gemini text (finishReason={:?})", finish))))
                    });
                    let _ = tx_res.send(match out {
                        Ok((text, maybe_err)) => {
                            let preview: String = text.chars().take(120).collect();
                            info!(
                                "GeminiCopilotHost: response id={} text_preview={:?}",
                                req.id, preview
                            );
                            GeminiCopilotResponse {
                                id: req.id,
                                text,
                                error: maybe_err,
                            }
                        }
                        Err(e) => {
                            error!("GeminiCopilotHost: response error id={} err={}", req.id, e);
                            GeminiCopilotResponse {
                                id: req.id,
                                text: String::new(),
                                error: Some(e),
                            }
                        }
                    });
                    let (m, cv) = &*inflight;
                    if let Ok(mut n) = m.lock() {
                        *n = n.saturating_sub(1);
                        cv.notify_one();
                    }
                });
            }
        });
        Self {
            tx: tx_req,
            rx: std::sync::Mutex::new(rx_res),
        }
    }

    pub fn request(&self, id: &str, prompt: &str) {
        let _ = self.tx.send(GeminiCopilotRequest {
            id: id.to_string(),
            prompt: prompt.to_string(),
        });
    }
    pub fn poll(&self) -> Vec<GeminiCopilotResponse> {
        let mut out = Vec::new();
        let Ok(rx) = self.rx.lock() else {
            return out;
        };
        while let Ok(r) = rx.try_recv() {
            out.push(r);
        }
        out
    }
}
