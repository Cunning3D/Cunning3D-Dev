use crate::tabs_system::node_editor::connection_hint::service::ConnectionHintState;
use bevy::prelude::*;
use bevy_egui::egui;

pub fn show_connection_tooltip(ui: &mut egui::Ui, state: &ConnectionHintState) {
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::new("connection_hint_layer"));
    if let Some(hint) = &state.current_hint {
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            layer,
            egui::Id::new("connection_hint"),
            |ui: &mut egui::Ui| {
                ui.label(egui::RichText::new(hint).color(egui::Color32::LIGHT_BLUE));
            },
        );
    } else if state.pending_request_id.is_some() {
        egui::show_tooltip_at_pointer(
            ui.ctx(),
            layer,
            egui::Id::new("connection_hint"),
            |ui: &mut egui::Ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Checking compatibility...");
                });
            },
        );
    }
}
