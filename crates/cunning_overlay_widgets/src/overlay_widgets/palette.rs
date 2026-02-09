//! Palette widgets for coverlay panels.
use super::style::{animate_select, paint_sdf_rect, OverlayTheme};
use bevy_egui::egui::{self, Color32, CursorIcon, Response, Sense, Ui};

#[inline]
pub fn palette_strip(ui: &mut Ui, selected: &mut u8, mut color: impl FnMut(u8) -> Color32) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let avail = ui.available_size_before_wrap();
    let avail_w = avail.x.max(1.0);
    let avail_h = avail.y.max(1.0);
    // MagicaVoxel-like: tall bars (height > width), laid out in a multi-column grid.
    // Scroll is vertical (top->bottom), and selection supports click + scrubbing.
    let cell_w = theme.palette_strip_cell_px.clamp(4.0, 48.0);
    let cell_h = (cell_w * 4.0).clamp(12.0, 240.0);
    let gap = theme.palette_gap.max(0.0);
    let step_x = (cell_w + gap).max(1.0);
    let step_y = (cell_h + gap).max(1.0);
    let cols = ((avail_w / step_x.max(1e-3)).floor() as i32)
        .clamp(1, theme.palette_strip_cols.max(1) as i32) as u8;
    let cols = cols.max(1);
    let rows = (((255usize + cols as usize - 1) / cols as usize).max(1)) as u16;
    let total_h = (rows as f32) * step_y;
    let grid_a = (255.0 * theme.palette_grid_opacity.clamp(0.0, 1.0)) as u8;
    let grid_c = Color32::from_rgba_unmultiplied(theme.stroke.color.r(), theme.stroke.color.g(), theme.stroke.color.b(), grid_a);
    let grid_w = theme.palette_grid_thickness.max(0.0);
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail_w, total_h.max(avail_h)), Sense::click_and_drag());
            if resp.hovered() { ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand); }
            for i in 1u16..=255u16 {
                let k = (i - 1) as u32;
                let x = (k % (cols as u32)) as f32;
                let y = (k / (cols as u32)) as f32;
                let x0 = rect.min.x + x * step_x;
                let y0 = rect.min.y + y * step_y;
                let mut r = egui::Rect::from_min_size(egui::pos2(x0, y0), egui::vec2(cell_w, cell_h));
                if gap > 0.0 { r = r.shrink(gap * 0.35); }
                let sel = (i as u8) == *selected;
                let (bc, bw) = if sel { (theme.accent, 2.0 * s) } else { (grid_c, grid_w) };
                paint_sdf_rect(ui, r, egui::CornerRadius::ZERO, color(i as u8), bc, bw);
            }
            // Update selection on click OR while dragging/scrubbing.
            let scrubbing = resp.dragged() || (resp.hovered() && ui.input(|i| i.pointer.primary_down()));
            if (resp.clicked() || scrubbing) && resp.interact_pointer_pos().is_some() {
                if let Some(p) = resp.interact_pointer_pos() {
                    let x = ((p.x - rect.min.x) / step_x.max(1e-3)).floor() as i32;
                    let y = ((p.y - rect.min.y) / step_y.max(1e-3)).floor() as i32;
                    if x >= 0 && y >= 0 {
                        let idx = (y as i32) * (cols as i32) + x + 1;
                        *selected = (idx.clamp(1, 255) as u8);
                    }
                }
            }
        });
}

#[inline]
pub fn palette_cell(ui: &mut Ui, col: Color32, selected: bool) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    palette_cell_sized(ui, col, selected, 14.0 * theme.scale)
}

#[inline]
pub fn palette_cell_sized(ui: &mut Ui, col: Color32, selected: bool, sz: f32) -> Response {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id(("pal_cell", col.to_array()));
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(sz, sz), Sense::click());
    if resp.hovered() { ui.output_mut(|o| o.cursor_icon = CursorIcon::PointingHand); }
    let sel_t = animate_select(ui.ctx(), id, selected);
    let grid_a = (255.0 * theme.palette_grid_opacity.clamp(0.0, 1.0)) as u8;
    let grid_c = Color32::from_rgba_unmultiplied(theme.stroke.color.r(), theme.stroke.color.g(), theme.stroke.color.b(), grid_a);
    let mut bw = theme.palette_grid_thickness.max(0.0);
    let mut bc = grid_c;
    if resp.hovered() { bw = bw.max(1.0 * s); }
    if selected { bw = 2.0 * s; bc = theme.accent; }
    else if sel_t > 0.0 && bw <= 0.0 { bw = 1.0; }
    paint_sdf_rect(ui, rect, theme.radius_sm, col, bc, bw);
    resp
}

#[inline]
pub fn palette_grid(ui: &mut Ui, palette: &[Color32; 256], selected: &mut u8) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    for y in 0..16u8 {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(2.0 * s, 2.0 * s);
            for x in 0..16u8 {
                let idx = y * 16 + x;
                if idx == 0 { ui.add_space(16.0 * s); continue; }
                let col = palette[idx as usize];
                let resp = palette_cell(ui, col, idx == *selected).on_hover_text(format!("Palette {}", idx));
                if resp.clicked() { *selected = idx; }
            }
        });
    }
}

#[inline]
pub fn palette_grid_fill(ui: &mut Ui, selected: &mut u8, color: impl FnMut(u8) -> Color32) {
    palette_strip(ui, selected, color);
}

#[inline]
pub fn color_preview(ui: &mut Ui, col: Color32, size: f32) {
    let theme = OverlayTheme::from_ui(ui);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), Sense::hover());
    ui.painter().rect(rect, theme.radius_sm, col, theme.stroke, egui::StrokeKind::Outside);
}

#[inline]
pub fn color_picker_mini(ui: &mut Ui, col: &mut Color32) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let mut rgba = col.to_array();
    ui.horizontal(|ui| {
        color_preview(ui, *col, 24.0 * s);
        ui.vertical(|ui| {
            ui.horizontal(|ui| { ui.label(egui::RichText::new("R").color(theme.fg_secondary).size(10.0 * s)); ui.add(egui::DragValue::new(&mut rgba[0]).speed(1).range(0..=255)); });
            ui.horizontal(|ui| { ui.label(egui::RichText::new("G").color(theme.fg_secondary).size(10.0 * s)); ui.add(egui::DragValue::new(&mut rgba[1]).speed(1).range(0..=255)); });
            ui.horizontal(|ui| { ui.label(egui::RichText::new("B").color(theme.fg_secondary).size(10.0 * s)); ui.add(egui::DragValue::new(&mut rgba[2]).speed(1).range(0..=255)); });
        });
    });
    *col = Color32::from_rgba_unmultiplied(rgba[0], rgba[1], rgba[2], rgba[3]);
}

#[inline]
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let h = (h.fract() + 1.0) % 1.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let i = (h * 6.0).floor() as i32;
    let f = h * 6.0 - (i as f32);
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match i.rem_euclid(6) { 0 => (v, t, p), 1 => (q, v, p), 2 => (p, v, t), 3 => (p, q, v), 4 => (t, p, v), _ => (v, p, q) };
    ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
}

#[inline]
fn rgb_to_hsv(c: Color32) -> (f32, f32, f32) {
    let r = c.r() as f32 / 255.0;
    let g = c.g() as f32 / 255.0;
    let b = c.b() as f32 / 255.0;
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let v = max;
    let s = if max <= 1e-6 { 0.0 } else { d / max };
    let h = if d <= 1e-6 { 0.0 }
    else if max == r { ((g - b) / d + if g < b { 6.0 } else { 0.0 }) / 6.0 }
    else if max == g { ((b - r) / d + 2.0) / 6.0 }
    else { ((r - g) / d + 4.0) / 6.0 };
    (h, s, v)
}

#[inline]
fn grad_bar(ui: &mut Ui, rect: egui::Rect, stops: impl Fn(f32) -> Color32) {
    let n = 48;
    let w = rect.width() / (n as f32);
    for i in 0..n {
        let t0 = (i as f32) / (n as f32);
        let t1 = ((i + 1) as f32) / (n as f32);
        let x0 = rect.min.x + w * (i as f32);
        let x1 = rect.min.x + w * ((i + 1) as f32);
        let r = egui::Rect::from_min_max(egui::pos2(x0, rect.min.y), egui::pos2(x1, rect.max.y));
        ui.painter().rect_filled(r, egui::CornerRadius::ZERO, stops((t0 + t1) * 0.5));
    }
}

#[inline]
fn bar_handle(ui: &mut Ui, rect: egui::Rect, t: f32) {
    let s = OverlayTheme::from_ui(ui).scale;
    let x = rect.min.x + rect.width() * t.clamp(0.0, 1.0);
    let r = egui::Rect::from_center_size(egui::pos2(x, rect.center().y), egui::vec2(8.0 * s, rect.height() + 6.0 * s));
    let rr = ((2.0 * s).round().clamp(1.0, 255.0) as u8);
    ui.painter().rect(r, egui::CornerRadius::same(rr), Color32::from_black_alpha(80), egui::Stroke::new(1.0 * s, Color32::from_white_alpha(140)), egui::StrokeKind::Outside);
}

#[inline]
pub fn hsv_palette_bar(ui: &mut Ui, id_source: impl std::hash::Hash, selected: &mut u8, mut color: impl FnMut(u8) -> Color32) {
    let theme = OverlayTheme::from_ui(ui);
    let s = theme.scale;
    let id = ui.make_persistent_id(id_source);
    let mut hsv = ui.ctx().data(|d| d.get_temp::<(f32, f32, f32)>(id)).unwrap_or_else(|| rgb_to_hsv(color(*selected)));
    let cur_hsv = rgb_to_hsv(color(*selected));
    if (cur_hsv.0 - hsv.0).abs() > 0.35 || (cur_hsv.1 - hsv.1).abs() > 0.35 || (cur_hsv.2 - hsv.2).abs() > 0.35 { hsv = cur_hsv; }
    let bar_h = 12.0 * s;
    let pick = |ui: &mut Ui, label: &str, _rect: egui::Rect, t: &mut f32, draw: &dyn Fn(&mut Ui, egui::Rect)| {
        ui.label(egui::RichText::new(label).color(theme.fg_secondary).size(10.0 * s));
        let (r, resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), bar_h), Sense::click_and_drag());
        let bar_rect = egui::Rect::from_min_size(r.min, egui::vec2(r.width(), bar_h));
        draw(ui, bar_rect);
        if resp.dragged() || resp.clicked() { if let Some(p) = resp.interact_pointer_pos() { *t = ((p.x - bar_rect.min.x) / bar_rect.width()).clamp(0.0, 1.0); } }
        bar_handle(ui, bar_rect, *t);
    };
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing = egui::vec2(6.0 * s, 4.0 * s);
        pick(ui, "H", ui.max_rect(), &mut hsv.0, &|ui, rect| { grad_bar(ui, rect, |t| { let (r, g, b) = hsv_to_rgb(t, 1.0, 1.0); Color32::from_rgb(r, g, b) }); });
        pick(ui, "S", ui.max_rect(), &mut hsv.1, &|ui, rect| { let h = hsv.0; grad_bar(ui, rect, |t| { let (r, g, b) = hsv_to_rgb(h, t, hsv.2); Color32::from_rgb(r, g, b) }); });
        pick(ui, "V", ui.max_rect(), &mut hsv.2, &|ui, rect| { let h = hsv.0; let s = hsv.1; grad_bar(ui, rect, |t| { let (r, g, b) = hsv_to_rgb(h, s, t); Color32::from_rgb(r, g, b) }); });
    });
    let (r, g, b) = hsv_to_rgb(hsv.0, hsv.1, hsv.2);
    let want = Color32::from_rgb(r, g, b);
    let mut best = *selected;
    let mut best_d = i32::MAX;
    for idx in 1u16..=255u16 {
        let c = color(idx as u8);
        let dr = c.r() as i32 - want.r() as i32;
        let dg = c.g() as i32 - want.g() as i32;
        let db = c.b() as i32 - want.b() as i32;
        let d = dr * dr + dg * dg + db * db;
        if d < best_d { best_d = d; best = idx as u8; }
    }
    *selected = best;
    ui.ctx().data_mut(|d| d.insert_temp(id, hsv));
}

