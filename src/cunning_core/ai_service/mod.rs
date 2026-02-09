pub mod ai_defaults;
pub mod completion;
pub mod context_config;
pub mod context_manager;
pub mod gemini;
pub mod local_llama;
pub mod local_prompt; // 本地模型专用提示词 (与 Gemini 分离)
pub mod native_candle;
pub mod openai_compat;
pub mod parser;
pub mod prefix_cache;
pub mod thinking_parser; // Oxide-Lab 移植的完整 thinking 解析器
pub mod truncate_util;
pub mod workspace_prompt;

// 移植自 Oxide-Lab 的 Qwen3 完整实现
pub mod common;
pub mod models_api;
pub mod native_tiny_model;
pub mod qwen3;

// 按模型/通用分组的"更清晰入口"（不破坏旧路径）
pub mod copilot_skill;
pub mod qwen;
