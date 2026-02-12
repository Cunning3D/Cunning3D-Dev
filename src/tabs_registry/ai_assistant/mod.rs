use crate::register_pane;

pub mod pane;
pub mod prompt;

pub use pane::AiAssistantPane;

register_pane!("AI Assistant", AiAssistantPane);
