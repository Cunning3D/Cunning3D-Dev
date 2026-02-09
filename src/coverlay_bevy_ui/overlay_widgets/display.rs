//! Display widgets for coverlay panels.
use super::style::{paint_gpu_text, paint_sdf_rect, OverlayTheme};
use bevy_egui::egui::{self, Align2, Color32, Sense, Ui};

#[inline]
pub fn label_primary(ui: &mut Ui, text: &str) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0 * s), Sense::hover());
    paint_gpu_text(ui, r.left_center() + egui::vec2(2.0 * s, 0.0), Align2::LEFT_CENTER, text, 13.5 * s, theme.fg_primary);
}

#[inline]
pub fn label_secondary(ui: &mut Ui, text: &str) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0 * s), Sense::hover());
    paint_gpu_text(ui, r.left_center() + egui::vec2(2.0 * s, 0.0), Align2::LEFT_CENTER, text, 12.5 * s, theme.fg_secondary);
}

#[inline]
pub fn label_value(ui: &mut Ui, label: &str, value: &str) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0 * s), Sense::hover());
    paint_gpu_text(ui, r.left_center() + egui::vec2(2.0 * s, 0.0), Align2::LEFT_CENTER, label, 12.0 * s, theme.fg_secondary);
    paint_gpu_text(ui, r.right_center() - egui::vec2(2.0 * s, 0.0), Align2::RIGHT_CENTER, value, 12.5 * s, theme.fg_primary);
}

#[inline]
pub fn badge(ui: &mut Ui, text: &str, color: Color32) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let font_px = 11.0 * s;
    let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.to_string(), egui::FontId::proportional(font_px), theme.fg_primary));
    let size = galley.size() + egui::vec2(12.0 * s, 6.0 * s);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    paint_sdf_rect(ui, rect, theme.radius_sm, color, Color32::TRANSPARENT, 0.0);
    paint_gpu_text(ui, rect.center(), Align2::CENTER_CENTER, text, font_px, theme.fg_primary);
}

#[inline]
pub fn progress_bar(ui: &mut Ui, progress: f32) {
    let theme = OverlayTheme::from_ui(ui);
    let height = 6.0 * theme.scale;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), height), Sense::hover());
    let fill_w = rect.width() * progress.clamp(0.0, 1.0);
    let fill_rect = egui::Rect::from_min_size(rect.min, egui::vec2(fill_w, height));
    paint_sdf_rect(ui, rect, theme.radius_sm, theme.bg_input, Color32::TRANSPARENT, 0.0);
    paint_sdf_rect(ui, fill_rect, theme.radius_sm, theme.accent, Color32::TRANSPARENT, 0.0);
}

#[inline]
pub fn info_row(ui: &mut Ui, key: &str, value: &str) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let (r, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 18.0 * s), Sense::hover());
    paint_gpu_text(ui, r.left_center() + egui::vec2(2.0 * s, 0.0), Align2::LEFT_CENTER, key, 12.0 * s, theme.fg_secondary);
    paint_gpu_text(ui, r.right_center() - egui::vec2(2.0 * s, 0.0), Align2::RIGHT_CENTER, value, 12.0 * s, theme.fg_primary);
}
