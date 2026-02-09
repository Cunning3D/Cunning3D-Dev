//! Input widgets for coverlay panels.
use super::style::{animate_toggle, paint_gpu_text, paint_sdf_circle, paint_sdf_rect, OverlayTheme};
use bevy_egui::egui::{self, CursorIcon, Response, Sense, Ui};
use std::ops::RangeInclusive;

#[inline]
pub fn styled_slider(ui: &mut Ui, val: &mut f32, range: RangeInclusive<f32>, label: &str) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let mn = *range.start();
    let mx = *range.end();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0 * s), Sense::click_and_drag());
    let lab_r = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 84.0 * s, rect.max.y));
    let bar_r = egui::Rect::from_min_max(egui::pos2(lab_r.max.x + 6.0 * s, rect.min.y + 11.0 * s), egui::pos2(rect.max.x - 44.0 * s, rect.max.y - 11.0 * s));
    let w = theme.input_box_w.max(36.0 * s);
    let h = theme.input_box_h.max(20.0 * s);
    let val_r = egui::Rect::from_center_size(egui::pos2(rect.max.x - w * 0.5, rect.center().y), egui::vec2(w, h));
    let bar_r = egui::Rect::from_min_max(bar_r.min, egui::pos2(val_r.min.x - 8.0 * s, bar_r.max.y));

    if resp.dragged() || resp.clicked() {
        if let Some(p) = resp.interact_pointer_pos() {
            let t = ((p.x - bar_r.min.x) / bar_r.width().max(1e-3)).clamp(0.0, 1.0);
            *val = (mn + (mx - mn) * t).clamp(mn.min(mx), mn.max(mx));
        }
    }

    paint_gpu_text(ui, lab_r.left_center() + egui::vec2(2.0 * s, 0.0), egui::Align2::LEFT_CENTER, label, 13.0 * s, theme.fg_secondary);
    paint_sdf_rect(ui, bar_r, theme.radius_md, theme.bg_input, egui::Color32::TRANSPARENT, 0.0);
    let t = if (mx - mn).abs() < 1e-6 { 0.0 } else { ((*val - mn) / (mx - mn)).clamp(0.0, 1.0) };
    let fill = egui::Rect::from_min_max(bar_r.min, egui::pos2(bar_r.min.x + bar_r.width() * t, bar_r.max.y));
    paint_sdf_rect(ui, fill, theme.radius_md, theme.accent, egui::Color32::TRANSPARENT, 0.0);
    let kx = bar_r.min.x + bar_r.width() * t;
    paint_sdf_circle(ui, egui::pos2(kx, bar_r.center().y), 7.5 * s, theme.fg_primary, egui::Color32::TRANSPARENT, 0.0);
    paint_sdf_rect(ui, val_r, theme.radius_md, theme.bg_input, egui::Color32::TRANSPARENT, 0.0);
    paint_gpu_text(ui, val_r.center(), egui::Align2::CENTER_CENTER, format!("{:.3}", *val), 12.0 * s, theme.fg_primary);
    resp
}

#[inline]
pub fn styled_drag<T: egui::emath::Numeric>(ui: &mut Ui, val: &mut T, speed: f64, label: &str) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 34.0 * s), Sense::click_and_drag());
    let lab_r = egui::Rect::from_min_max(rect.min, egui::pos2(rect.min.x + 120.0 * s, rect.max.y));
    let w = theme.input_box_w.max(36.0 * s);
    let h = theme.input_box_h.max(20.0 * s);
    let box_r = egui::Rect::from_center_size(egui::pos2(rect.max.x - w * 0.5, rect.center().y), egui::vec2(w, h));

    if resp.dragged() {
        let dx = ui.input(|i| i.pointer.delta().x) as f64;
        let mut v = val.to_f64() + dx * speed;
        if T::INTEGRAL { v = v.round(); }
        *val = T::from_f64(v.clamp(T::MIN.to_f64(), T::MAX.to_f64()));
    }

    paint_gpu_text(ui, lab_r.left_center() + egui::vec2(2.0 * s, 0.0), egui::Align2::LEFT_CENTER, label, 13.0 * s, theme.fg_secondary);
    paint_sdf_rect(ui, box_r, theme.radius_md, theme.bg_input, egui::Color32::TRANSPARENT, 0.0);
    paint_gpu_text(ui, box_r.center(), egui::Align2::CENTER_CENTER, format!("{:.3}", val.to_f64()), 12.5 * s, theme.fg_primary);
    resp
}

#[inline]
pub fn toggle_switch(ui: &mut Ui, on: &mut bool) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id("toggle_switch");
    let size = egui::vec2(46.0 * s, 24.0 * s);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.clicked() { *on = !*on; }
    if resp.hovered() { ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand); }
    let t = animate_toggle(ui.ctx(), id, *on);
    let bg = OverlayTheme::lerp_color(theme.bg_input, theme.accent, t);
    paint_sdf_rect(ui, rect, theme.radius_lg, bg, egui::Color32::TRANSPARENT, 0.0);
    let knob_r = 9.0 * s;
    let knob_x = rect.left() + knob_r + 3.0 * s + t * (rect.width() - knob_r * 2.0 - 6.0 * s);
    let knob_center = egui::pos2(knob_x, rect.center().y);
    paint_sdf_circle(ui, knob_center, knob_r, theme.fg_primary, egui::Color32::TRANSPARENT, 0.0);
    resp
}

#[inline]
pub fn radio_row<T: Copy + PartialEq>(ui: &mut Ui, cur: &mut T, options: &[(T, &str)]) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    ui.horizontal(|ui| {
        for (val, label) in options {
            let selected = *cur == *val;
            let (r, resp) = ui.allocate_exact_size(egui::vec2(110.0 * s, 34.0 * s), Sense::click());
            paint_sdf_rect(ui, r, theme.radius_md, if selected { theme.accent } else { theme.bg_input }, egui::Color32::TRANSPARENT, 0.0);
            paint_gpu_text(ui, r.center(), egui::Align2::CENTER_CENTER, *label, 12.5 * s, theme.fg_primary);
            if resp.clicked() { *cur = *val; }
        }
    });
}

#[inline]
pub fn checkbox_row(ui: &mut Ui, flags: &mut [(&mut bool, &str)]) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    ui.horizontal(|ui| {
        for (on, label) in flags.iter_mut() {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(110.0 * s, 34.0 * s), Sense::click());
            paint_sdf_rect(ui, r, theme.radius_md, if **on { theme.accent } else { theme.bg_input }, egui::Color32::TRANSPARENT, 0.0);
            paint_gpu_text(ui, r.center(), egui::Align2::CENTER_CENTER, *label, 12.5 * s, theme.fg_primary);
            if resp.clicked() { **on = !**on; }
        }
    });
}

#[inline]
pub fn axis_toggle(ui: &mut Ui, x: &mut bool, y: &mut bool, z: &mut bool) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    ui.horizontal(|ui| {
        let axes = [
            ("X", x, egui::Color32::from_rgb(220, 80, 80)),
            ("Y", y, egui::Color32::from_rgb(80, 180, 80)),
            ("Z", z, egui::Color32::from_rgb(80, 120, 220)),
        ];
        for (label, on, col) in axes {
            let (r, resp) = ui.allocate_exact_size(egui::vec2(44.0 * s, 34.0 * s), Sense::click());
            paint_sdf_rect(ui, r, theme.radius_md, if *on { col } else { theme.bg_input }, egui::Color32::TRANSPARENT, 0.0);
            paint_gpu_text(ui, r.center(), egui::Align2::CENTER_CENTER, label, 12.5 * s, theme.fg_primary);
            if resp.clicked() { *on = !*on; }
        }
    });
}

