use bevy::prelude::*;
use bevy_egui::egui;
use std::ops::RangeInclusive;

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

pub fn draw_int_slider_with_input(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut i32,
    range: RangeInclusive<i32>,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed |= ui
            .add(egui::Slider::new(value, range.clone()).integer())
            .changed();
        changed |= ui.add(egui::DragValue::new(value).speed(1.0)).changed();
    });
    changed
}

pub fn draw_float_slider_with_input(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut f32,
    range: RangeInclusive<f32>,
    speed: f64,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(label);
        changed |= ui.add(egui::Slider::new(value, range.clone())).changed();
        changed |= ui.add(egui::DragValue::new(value).speed(speed)).changed();
    });
    changed
}
