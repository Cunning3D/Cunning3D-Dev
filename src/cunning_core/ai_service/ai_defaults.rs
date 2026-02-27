//! AI default configuration (centralized; avoid scattered hardcoding).

pub const DEFAULT_QWEN_GGUF: &str = "Lmodels/Qwen3-4B-Thinking-2507-Q4_K_M.gguf";
pub const DEFAULT_QWEN_TOKENIZER: &str = "Lmodels/tokenizer.json";

// Inference parameters (aligned with LM Studio defaults: context=100K, batch=512).
pub const DEFAULT_LOCAL_MAX_TOKENS: usize = 8192; // Max generation length (aligned with LM Studio).
pub const DEFAULT_TEMPERATURE: f64 = 0.7; // Sampling temperature.
pub const DEFAULT_TOP_P: f64 = 0.95; // Top-P sampling.
pub const LOCAL_STREAM_LOG_VERBOSE: bool = true; // Local streaming debug logs (very noisy; disable when needed).

// Tiny Copilot (Ghost Path Completion) context budget
pub const TINY_GHOST_MAX_NODES: usize = 12; // Full info nodes (reduced for token efficiency)
pub const TINY_GHOST_MAX_LINKS: usize = 24; // Max connections to include
pub const TINY_GHOST_MAX_HOPS: usize = 6; // BFS depth for better context understanding
pub const TINY_GHOST_MAX_FAR_NODES: usize = 16; // Summary-only nodes (increased for broader context)

// Ghost completion UX
pub const GHOST_TAB_INTER_TAP_S: f64 = 0.28;

// Auto-retry config for Ghost Path
pub const GHOST_MAX_RETRIES: u8 = 1; // Max retry attempts before giving up
pub const GHOST_TIMEOUT_SECS: f32 = 20.0; // Timeout threshold for LocalTiny
#[allow(dead_code)]
pub const GHOST_TAB_BURST_WINDOW_S: f64 = GHOST_TAB_INTER_TAP_S;
#[allow(dead_code)]
pub const GHOST_TAB_SINGLE_GRACE_S: f64 = GHOST_TAB_INTER_TAP_S;
