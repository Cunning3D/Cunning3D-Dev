use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

#[derive(Clone)]
pub struct CompletionClient {
    client: Client,
    api_key: String,
    model_name: String,
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

impl Default for CompletionClient {
    fn default() -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5)) // Fast timeout for completion
                .build()
                .unwrap_or_default(),
            api_key: std::env::var("GEMINI_API_KEY")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .or_else(load_gemini_key_from_settings)
                .unwrap_or_default(),
            model_name: "gemini-2.0-flash".to_string(), // Use Flash for speed
        }
    }
}

impl CompletionClient {
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn request_completion(&self, prefix: String, suffix: String) -> Option<String> {
        if self.api_key.is_empty() {
            return None;
        }

        let url = format!(
            "https://generativelanguage.googleapis.com/v1beta/models/{}:generateContent?key={}",
            self.model_name, self.api_key
        );

        // FIM (Fill-In-the-Middle) Prompt Strategy
        let prompt = format!(
            "Complete the code. Output ONLY the missing code. No markdown. No comments unless necessary.\n\nCode:\n{}<CURSOR>{}",
            prefix, suffix
        );

        let body = json!({
            "contents": [{
                "role": "user",
                "parts": [{ "text": prompt }]
            }],
            "generationConfig": {
                "temperature": 0.2, // Low temp for deterministic code
                "maxOutputTokens": 64, // Short completions only
                "stopSequences": ["\n\n", "```"]
            }
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    return None;
                }
                if let Ok(json) = resp.json::<Value>().await {
                    if let Some(text) =
                        json["candidates"][0]["content"]["parts"][0]["text"].as_str()
                    {
                        let trimmed = text.trim_end();
                        if !trimmed.is_empty() {
                            return Some(trimmed.to_string());
                        }
                    }
                }
            }
            Err(_) => {}
        }
        None
    }
}
