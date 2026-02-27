use crate::cunning_core::traits::pane_interface::PaneTab;
use crate::register_pane;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui;

pub mod client;
pub mod context;
pub mod session;
pub mod tools;

#[derive(Default)]
pub struct GpuiWorkspaceLauncherTab;

impl PaneTab for GpuiWorkspaceLauncherTab {
    fn ui(&mut self, ui: &mut egui::Ui, _context: &mut EditorTabContext) {
        ui.label("AI Workspace (GPUI) launches as native window.");
    }

    fn title(&self) -> egui::WidgetText {
        "AI Workspace (GPUI)".into()
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

register_pane!("AI Workspace (GPUI)", GpuiWorkspaceLauncherTab);
