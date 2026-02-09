//! GPUI-based AI Workspace window (Zed-compatible architecture).
//! Host drives Session/Tool/LLM; UI consumes Snapshot/Event via channel.

pub mod protocol;
pub mod host;
pub mod app;
pub mod ui;
pub mod components;
pub mod ide;

pub use host::AiWorkspaceHost;
pub use protocol::{HostToUi, UiToHost};
