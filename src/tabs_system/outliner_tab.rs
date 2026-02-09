//! `outliner_tab.rs` - A dockable tab that displays a scene outliner.

use bevy_egui::egui;

use crate::tabs_system::{EditorTab, EditorTabContext};

#[derive(Default)]
pub struct OutlinerTab;

impl EditorTab for OutlinerTab {
    fn title(&self) -> egui::WidgetText {
        "Outliner".into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, _context: &mut EditorTabContext) {
        // --- Outliner content goes here in the future ---
        ui.label("Scene Objects...");
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
