//! 通用组件（Prompt/默认配置 + 模型通用工具）
pub mod ai_defaults;
pub mod workspace_prompt;

// Common utilities for model backends - 移植自 Oxide-Lab
pub mod flash_helpers;
pub use flash_helpers::is_flash_attention_available;
