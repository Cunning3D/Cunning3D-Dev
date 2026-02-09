use bevy::log::{error, info};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::Duration;

/// 客户端连接本地运行的 LLM 服务 (兼容 OpenAI API，如 llama-server / vLLM)
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
                .timeout(Duration::from_secs(5)) // 本地模型响应应该很快，5秒超时防止卡死
                .build()
                .unwrap_or_default(),
            base_url: format!("http://localhost:{}", port),
        }
    }

    /// 核心功能：根据当前节点预测下一个节点
    /// 返回可能性最高的 3 个节点类型名称
    pub async fn predict_next_nodes(&self, current_node: &str) -> Vec<String> {
        let url = format!("{}/v1/chat/completions", self.base_url);

        // 构造 Prompt
        // 对于 0.5B 小模型，Prompt 要极其直接和简单。
        let system_prompt = "You are a helper for Cunning3D node graph. Predict the next node.";
        let user_prompt = format!(
            "Current node: {}. Suggest 3 likely next nodes. Output JSON array only.",
            current_node
        );

        let body = json!({
            "model": "qwen-0.5b", // 模型名通常不重要，llama.cpp 会忽略或只看加载的模型
            "messages": [
                { "role": "system", "content": system_prompt },
                { "role": "user", "content": user_prompt }
            ],
            "temperature": 0.1, // 低温，追求确定性
            "max_tokens": 64,   // 只需要几个词
            "stream": false,
            "response_format": { "type": "json_object" } // 强制 JSON 模式（如果支持）
        });

        match self.client.post(&url).json(&body).send().await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    error!("Local LLM HTTP error: {}", resp.status());
                    return vec![];
                }

                match resp.json::<Value>().await {
                    Ok(json) => {
                        // 解析 OpenAI 格式的返回
                        if let Some(content) = json["choices"][0]["message"]["content"].as_str() {
                            // 尝试解析 JSON 数组
                            // 0.5B 模型可能输出不完美的 JSON，这里做一个简单的容错解析
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
                // 连接失败通常意味着服务没跑，这是正常情况，不报错或只报 debug
                info!("Local LLM not available: {}", e);
                vec![]
            }
        }
    }

    /// 简单的解析器，从字符串中提取节点列表
    fn parse_node_list(content: &str) -> Vec<String> {
        // 尝试标准 JSON 解析
        if let Ok(list) = serde_json::from_str::<Vec<String>>(content) {
            return list;
        }

        // 如果 JSON 解析失败，尝试简单的正则或分割（针对小模型可能胡言乱语的情况）
        // 比如它可能返回: "1. Transform\n2. Bevel"
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
        results.truncate(3); // 只取前3个
        results
    }
}
