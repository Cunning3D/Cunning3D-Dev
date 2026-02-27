use bevy::log::{error, info};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// Client for connecting to a locally running LLM service (OpenAI API compatible, e.g. llama-server / vLLM)
pub struct LocalLlamaClient {
    client: Client,
    base_url: String,
}

impl Default for LocalLlamaClient {
    fn default() -> Self {
        Self::new(8080)
    }
}

impl LocalLlamaClient {
    pub fn new(port: u16) -> Self {
        Self {
            client: Client::builder()
                .timeout(Duration::from_secs(5)) // Local models should respond quickly; 5s timeout prevents hangs
                .build()
                .unwrap_or_default(),
            base_url: format!("http://localhost:{}", port),
        }
    }

    /// Core feature: predict the next node based on the current node
    /// Returns the 3 most likely node type names
    pub async fn predict_next_nodes(&self, current_node: &str) -> Vec<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        // Build the prompt.
        // For ~0.5B small models, the prompt must be extremely direct and simple.
        let system_prompt = "You are a helper for Cunning3D node graph. Predict the next node.";
        let user_prompt = format!(
            "Current node: {}. Suggest 3 likely next nodes. Output JSON array only.",
            current_node
        );

        let body = json!({
            "model": "qwen-0.5b", // Model name is usually not important; llama.cpp may ignore it or use the loaded model
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "temperature": 0.1, // Low temperature for determinism
            "max_tokens": 64,   // Only need a few tokens
            "stream": false,
            "response_format": { "type": "json_object" } // Force JSON mode (if supported)
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    error!("Local LLM HTTP error: {}", resp.status());
                    return vec![];
                }

                match resp.json::<Value>().await {
                    Ok(json) => {
                        // Parse an OpenAI-style response
                        if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                            // Try parsing a JSON array.
                            // Small models may output imperfect JSON; do a simple tolerant parse here.
                            Self::parse_node_list(content)
                        } else {
                            vec![]
                        }
                    }
                    Err(e) => {
                        error!("Failed to parse Local LLM response: {}", e);
                        vec![]
                    }
                }
            }
            Err(e) => {
                // Connection failure usually means the service isn't running; that's normal—log at debug or ignore.
                info!("Local LLM not available: {}", e);
                vec![]
            }
        }
    }

    /// A simple parser that extracts a node list from a string
    fn parse_node_list(content: &str) -> Vec<String> {
        // Try standard JSON parsing
        if let Ok(list) = serde_json::from_str::<Vec<String>>(content) {
            return list;
        }

        // If JSON parsing fails, fall back to a simple regex/split (small models may produce junk).
        // For example, it might return: "1. Transform\n2. Bevel"
        let mut results = Vec::new();
        for line in content.lines() {
            let clean = line
                .trim()
                .trim_start_matches(|c: char| {
                    c.is_numeric() || c == '.' || c == '-' || c == '"' || c == '[' || c == ']'
                })
                .trim_end_matches(|c: char| c == '"' || c == ',' || c == ']');
            if !clean.is_empty() && clean.len() > 2 {
                results.push(clean.to_string());
            }
        }
        results.truncate(3); // Keep only top 3
        results
    }
}
