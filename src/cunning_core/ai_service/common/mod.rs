//! Shared components (prompts/defaults + model utilities)
pub mod ai_defaults;
pub mod workspace_prompt;

// Common utilities for model backends - ported from Oxide-Lab
pub mod flash_helpers;
pub use flash_helpers::is_flash_attention_available;
