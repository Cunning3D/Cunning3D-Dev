//! Button widgets for coverlay panels.
use super::style::{animate_hover, animate_select, paint_gpu_text, paint_sdf_rect, OverlayTheme};
use bevy_egui::egui::{self, Color32, CursorIcon, Response, Sense, Ui};

#[inline]
pub fn icon_button(ui: &mut Ui, icon: &str, selected: bool) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id(icon);
    let (rect, resp) = ui.allocate_exact_size(theme.icon_size, Sense::click());
    if resp.hovered() { ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand); }
    let hover_t = animate_hover(ui.ctx(), id, resp.hovered());
    let sel_t = animate_select(ui.ctx(), id, selected);
    let base = if selected { OverlayTheme::lerp_color(theme.bg_input, theme.accent, 0.82) } else { theme.bg_input };
    let mut fill = OverlayTheme::lerp_color(base, theme.hover, hover_t * (1.0 - sel_t * 0.35));
    if resp.is_pointer_button_down_on() { fill = OverlayTheme::lerp_color(fill, theme.press, 0.92); }
    paint_sdf_rect(ui, rect, theme.radius_md, fill, Color32::TRANSPARENT, 0.0);
    paint_gpu_text(ui, rect.center(), egui::Align2::CENTER_CENTER, icon, 16.0 * s, theme.fg_primary);
    resp
}

#[inline]
pub fn icon_select<T: Copy + PartialEq>(ui: &mut Ui, cur: &mut T, val: T, icon: &str, tip: &str) -> bool {
    let resp = icon_button(ui, icon, *cur == val).on_hover_text(tip);
    let clicked = resp.clicked();
    if clicked { *cur = val; }
    clicked
}

#[inline]
pub fn tool_button(ui: &mut Ui, icon: &str, label: &str, selected: bool) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id((icon, label));
    let size = egui::vec2(190.0 * s, 34.0 * s);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() { ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand); }
    let hover_t = animate_hover(ui.ctx(), id, resp.hovered());
    let sel_t = animate_select(ui.ctx(), id, selected);
    let base = if selected { OverlayTheme::lerp_color(theme.bg_input, theme.accent, 0.82) } else { theme.bg_input };
    let mut fill = OverlayTheme::lerp_color(base, theme.hover, hover_t * (1.0 - sel_t * 0.35));
    if resp.is_pointer_button_down_on() { fill = OverlayTheme::lerp_color(fill, theme.press, 0.92); }
    paint_sdf_rect(ui, rect, theme.radius_md, fill, Color32::TRANSPARENT, 0.0);
    let icon_rect = egui::Rect::from_min_size(rect.min + egui::vec2(10.0 * s, (rect.height() - theme.icon_size.y) * 0.5), theme.icon_size);
    paint_gpu_text(ui, icon_rect.center(), egui::Align2::CENTER_CENTER, icon, 16.0 * s, theme.fg_primary);
    let text_pos = egui::pos2(icon_rect.max.x + 10.0 * s, rect.center().y);
    paint_gpu_text(ui, text_pos, egui::Align2::LEFT_CENTER, label, 13.5 * s, theme.fg_primary);
    resp
}

#[inline]
pub fn pill_button(ui: &mut Ui, label: &str, selected: bool) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id(label);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(160.0 * s, 34.0 * s), Sense::click());
    let fill = if selected { OverlayTheme::lerp_color(theme.bg_input, theme.accent, 0.82) } else { theme.bg_input };
    let hover_t = animate_hover(ui.ctx(), id, resp.hovered());
    let mut fill = OverlayTheme::lerp_color(fill, theme.hover, hover_t * if selected { 0.12 } else { 0.28 });
    if resp.is_pointer_button_down_on() { fill = OverlayTheme::lerp_color(fill, theme.press, 0.92); }
    paint_sdf_rect(ui, rect, theme.radius_lg, fill, Color32::TRANSPARENT, 0.0);
    paint_gpu_text(ui, rect.center(), egui::Align2::CENTER_CENTER, label, 13.0 * s, theme.fg_primary);
    resp
}

#[inline]
pub fn toggle_button(ui: &mut Ui, on: &mut bool, label: &str) -> Response {
    let resp = pill_button(ui, label, *on);
    if resp.clicked() { *on = !*on; }
    resp
}

#[inline]
pub fn action_button(ui: &mut Ui, label: &str) -> Response {
    pill_button(ui, label, false)
}
