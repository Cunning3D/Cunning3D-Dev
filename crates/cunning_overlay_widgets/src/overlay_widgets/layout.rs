//! Layout widgets for coverlay panels.
use super::style::{paint_gpu_text, paint_sdf_rect, OverlayTheme};
use bevy_egui::egui::{self, Ui};

#[inline]
pub fn panel_frame(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    ui.spacing_mut().item_spacing = egui::vec2(theme.spacing, theme.spacing);
    ui.add_space(theme.spacing);
    f(ui);
    ui.add_space(theme.spacing);
}

#[inline]
pub fn group(ui: &mut Ui, title: &str, open: bool, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    egui::CollapsingHeader::new(egui::RichText::new(title).strong().color(theme.fg_primary))
        .default_open(open)
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing = egui::vec2(theme.spacing, theme.spacing);
            f(ui);
        });
}

#[inline]
pub fn group_flat(ui: &mut Ui, title: &str, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    ui.label(egui::RichText::new(title).strong().color(theme.fg_primary).size(13.0 * theme.scale));
    ui.spacing_mut().item_spacing = egui::vec2(theme.spacing, theme.spacing);
    f(ui);
}

#[inline]
pub fn toolbar(ui: &mut Ui, vertical: bool, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    ui.spacing_mut().item_spacing = egui::vec2(theme.spacing, theme.spacing);
    if vertical { ui.vertical(f); } else { ui.horizontal(f); }
}

#[inline]
pub fn toolbar_sep(ui: &mut Ui, vertical: bool) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let c0 = theme.stroke.color;
    let a = ((255.0 * theme.sep_opacity.clamp(0.0, 1.0)) as u8).min(255);
    let c = egui::Color32::from_rgba_unmultiplied(c0.r(), c0.g(), c0.b(), a);
    let t = theme.sep_thickness.max(1.0 * s);
    let h = (theme.icon_size.y - 4.0 * s).max(18.0 * s);
    let sz = if vertical { egui::vec2(ui.available_width().max(1.0), t) } else { egui::vec2(t, h) };
    let (r, _) = ui.allocate_exact_size(sz, egui::Sense::hover());
    paint_sdf_rect(ui, r, egui::CornerRadius::ZERO, c, egui::Color32::TRANSPARENT, 0.0);
}

#[inline]
pub fn card(ui: &mut Ui, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    ui.spacing_mut().item_spacing = egui::vec2(theme.spacing, theme.spacing);
    f(ui);
}

#[inline]
pub fn grid(ui: &mut Ui, cols: usize, f: impl FnOnce(&mut Ui)) {
    let theme = OverlayTheme::from_ui(ui);
    egui::Grid::new(ui.make_persistent_id("grid"))
        .num_columns(cols)
        .spacing(egui::vec2(4.0 * theme.scale, 4.0 * theme.scale))
        .show(ui, |ui| f(ui));
}

#[inline]
pub fn hsep(ui: &mut Ui) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let c0 = theme.stroke.color;
    let a = ((255.0 * theme.sep_opacity.clamp(0.0, 1.0)) as u8).min(255);
    let c = egui::Color32::from_rgba_unmultiplied(c0.r(), c0.g(), c0.b(), a);
    let t = theme.sep_thickness.max(1.0 * s);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), t), egui::Sense::hover());
    paint_sdf_rect(ui, rect, egui::CornerRadius::ZERO, c, egui::Color32::TRANSPARENT, 0.0);
}

#[inline]
pub fn vsep(ui: &mut Ui) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let c0 = theme.stroke.color;
    let a = ((255.0 * theme.sep_opacity.clamp(0.0, 1.0)) as u8).min(255);
    let c = egui::Color32::from_rgba_unmultiplied(c0.r(), c0.g(), c0.b(), a);
    let t = theme.sep_thickness.max(1.0 * s);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(t, ui.available_height().min(20.0 * s)), egui::Sense::hover());
    paint_sdf_rect(ui, rect, egui::CornerRadius::ZERO, c, egui::Color32::TRANSPARENT, 0.0);
}

#[inline]
pub fn segmented_tabs(ui: &mut Ui, id_source: impl std::hash::Hash, cur: &mut u8, labels: &[&str]) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    ui.push_id(id_source, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(4.0 * s, 0.0);
            for (i, &lab) in labels.iter().enumerate() {
                let i = i.min(u8::MAX as usize) as u8;
                let sel = *cur == i;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(90.0 * s, 34.0 * s), egui::Sense::click());
                let fill = if sel { theme.accent } else { theme.bg_input };
                paint_sdf_rect(ui, rect, theme.radius_md, fill, egui::Color32::TRANSPARENT, 0.0);
                paint_gpu_text(ui, rect.center(), egui::Align2::CENTER_CENTER, lab, 12.5 * s, theme.fg_primary);
                if resp.clicked() { *cur = i; }
            }
        });
    });
}

#[inline]
pub fn tab_strip(ui: &mut Ui, id_source: impl std::hash::Hash, cur: &mut u8, labels: &[&str]) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    ui.push_id(id_source, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(8.0 * s, 0.0);
            for (i, &lab) in labels.iter().enumerate() {
                let i = i.min(u8::MAX as usize) as u8;
                let sel = *cur == i;
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(90.0 * s, 30.0 * s), egui::Sense::click());
                paint_gpu_text(ui, rect.center(), egui::Align2::CENTER_CENTER, lab, 12.5 * s, if sel { theme.fg_primary } else { theme.fg_secondary });
                if resp.clicked() { *cur = i; }
                if sel && ui.is_rect_visible(resp.rect) {
                    let y = rect.bottom() - 1.0 * s;
                    let line = egui::Rect::from_min_max(egui::pos2(rect.min.x, y), egui::pos2(rect.max.x, y + 1.0 * s));
                    paint_sdf_rect(ui, line, egui::CornerRadius::ZERO, theme.fg_primary, egui::Color32::TRANSPARENT, 0.0);
                }
            }
        });
    });
}

#[inline]
pub fn pager(ui: &mut Ui, id_source: impl std::hash::Hash, page: &mut u32, page_count: u32) -> bool {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    if page_count <= 1 { *page = 0; return false; }
    let mut changed = false;
    ui.push_id(id_source, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(6.0 * s, 0.0);
            let p = (*page).min(page_count.saturating_sub(1));
            if p != *page { *page = p; changed = true; }
            let prev = ui.add(egui::Button::new(egui::RichText::new("◀").color(theme.fg_primary).size(11.0 * s)).fill(theme.bg_input));
            if prev.clicked() && *page > 0 { *page -= 1; changed = true; }
            ui.label(egui::RichText::new(format!("{}/{}", (*page + 1), page_count)).color(theme.fg_secondary).size(11.0 * s));
            let next = ui.add(egui::Button::new(egui::RichText::new("▶").color(theme.fg_primary).size(11.0 * s)).fill(theme.bg_input));
            if next.clicked() && (*page + 1) < page_count { *page += 1; changed = true; }
        });
    });
    changed
}

