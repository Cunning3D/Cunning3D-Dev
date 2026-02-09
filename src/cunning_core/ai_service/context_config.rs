//! Shared context budget knobs (single source of truth).

pub const MODEL_CONTEXT_WINDOW_TOKENS: usize = 200_000;
pub const HISTORY_TOKEN_BUDGET: usize = 180_000;
pub const TOOL_LLM_MAX_CHARS: usize = 2000;
pub const TOOL_OUTPUT_TOKEN_LIMIT: usize = 800;
pub const PROJECT_DOC_MAX_BYTES: usize = 32 * 1024;
pub const PROJECT_DOC_FILES: &[&str] = &["README.md"];
pub const HISTORY_USER_CHARS: usize = 500;
pub const HISTORY_AI_CHARS: usize = 800;
pub const CONTEXT_MAX_CHARS: usize = 4000;
pub const ENV_CONTEXT_MAX_CHARS: usize = 512;
pub const TOKEN_EST_CHARS_PER_TOKEN: f32 = 3.8;
