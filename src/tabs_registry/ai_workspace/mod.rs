use crate::cunning_core::traits::pane_interface::PaneTab;
use crate::register_pane;
use bevy_egui::egui::{self, Ui};
use inventory; // Macro is exported at crate root or we use full path

pub mod agent;
pub mod client;
pub mod context;
pub mod pane;
pub mod session;
pub mod tools;


pub use pane::AiWorkspacePane;

register_pane!("AI Workspace", AiWorkspacePane);
