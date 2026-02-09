use egui::{self, pos2, vec2, Align, Id, RichText, Sense, Stroke, Ui};
use std::hash::Hash;

#[inline]
fn paint_header_line(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
    if r.width() <= 0.0 { return; }
    let y = r.center().y;
    let stroke = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color.linear_multiply(0.9));
    ui.painter().line_segment([pos2(r.left(), y), pos2(r.right(), y)], stroke);
}

/// A Houdini-style section header with a long horizontal rule.
/// Uses egui's persistent collapsing state; good for property sheets.
pub fn section(
    ui: &mut Ui,
    id_source: impl Hash,
    title: &str,
    default_open: bool,
    add_body: impl FnOnce(&mut Ui),
) -> bool {
    let id = ui.make_persistent_id(("cui_section", id_source));
    let state = egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, default_open);
    let mut header = state.show_header(ui, |ui| {
        ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
            ui.label(RichText::new(title).strong());
            paint_header_line(ui);
        }).inner
    });
    let open = header.is_open();
    header.body(add_body);
    open
}

