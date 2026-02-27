pub mod ai_defaults;
pub mod completion;
pub mod context_config;
pub mod context_manager;
pub mod gemini;
pub mod local_llama;
pub mod local_prompt; // Local-model-specific prompts (separate from Gemini).
pub mod native_candle;
pub mod openai_compat;
pub mod parser;
pub mod prefix_cache;
pub mod thinking_parser; // Full thinking parser ported from Oxide-Lab.
pub mod truncate_util;
pub mod workspace_prompt;

// Full Qwen3 implementation ported from Oxide-Lab
pub mod common;
pub mod models_api;
pub mod native_tiny_model;
pub mod qwen3;

// A clearer entry point grouped by model/common (keeps old paths intact).
pub mod copilot_skill;
pub mod qwen;
