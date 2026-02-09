use bevy::prelude::*;
use bevy_egui::egui;

/// 标准 HUD 实现：显示节点名称和图标
/// Standard HUD implementation: Display node name and icon
pub fn draw_default_hud(ui: &mut egui::Ui, node_name: &str, _icon: Option<&str>) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            // Logo (Placeholder)
            ui.label("🧊");
            // Name
            ui.heading(node_name);
        });
        ui.label(egui::RichText::new("Basic Node").italics().weak());
    });
}
