use egui::{Color32, FontId, Response, Sense, Vec2};

#[derive(Clone, Copy, Debug, Default)]
pub struct Cfg { pub sdf: bool, pub gpu_text: bool, pub retained: bool }

#[inline]
fn cfg_id() -> egui::Id { egui::Id::new("cunning_demo_cfg") }

pub fn set_cfg(ctx: &egui::Context, cfg: Cfg) { ctx.data_mut(|d| d.insert_temp(cfg_id(), cfg)); }

pub fn get_cfg(ctx: &egui::Context) -> Cfg { ctx.data(|d| d.get_temp::<Cfg>(cfg_id()).unwrap_or_default()) }

#[inline]
fn c32(c: Color32) -> [f32; 4] { egui::Rgba::from(c).to_array() }

pub fn text_size(ui: &egui::Ui, text: &str, font: FontId) -> Vec2 {
    ui.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font, Color32::WHITE).size())
}

pub fn gpu_label(ui: &mut egui::Ui, text: &str, font_px: f32, color: Color32) {
    ui.label(egui::RichText::new(text).color(color).size(font_px));
}

pub fn sdf_button(ui: &mut egui::Ui, text: &str, selected: bool) -> Response {
    let cfg = get_cfg(ui.ctx());
    let font_px = ui.style().text_styles.get(&egui::TextStyle::Button).map_or(14.0, |f| f.size);
    let pad = Vec2::new(10.0, 6.0);
    let sz = text_size(ui, text, FontId::proportional(font_px)) + pad * 2.0;
    let (rect, resp) = ui.allocate_exact_size(sz, Sense::click());
    let v = ui.style().interact_selectable(&resp, selected);

    #[cfg(feature = "wgpu")]
    if cfg.sdf {
        use egui::epaint::Shape;
        if v.bg_fill.a() > 0 || v.bg_stroke.width > 0.0 {
            let r = v.rounding.nw;
            let u = egui_wgpu::sdf::SdfRectUniform {
                center: [rect.center().x, rect.center().y],
                half_size: [rect.width() * 0.5, rect.height() * 0.5],
                corner_radii: [r, r, r, r],
                fill_color: c32(v.bg_fill),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0; 2],
                border_width: v.bg_stroke.width,
                _pad2: [0.0; 3],
                border_color: c32(v.bg_stroke.color),
                screen_size: [ui.ctx().screen_rect().width(), ui.ctx().screen_rect().height()],
                _pad3: [0.0; 2],
            };
            ui.painter().add(Shape::Callback(egui_wgpu::sdf::create_sdf_rect_callback(ui.painter().clip_rect(), u, ui.ctx().frame_nr())));
        }
    } else {
        ui.painter().rect(rect, v.rounding, v.bg_fill, v.bg_stroke);
    }

    let tcol = v.text_color();
    let font = FontId::proportional(font_px);
    let tsz = text_size(ui, text, font.clone());
    let text_pos = egui::pos2(rect.min.x + pad.x, rect.center().y - tsz.y * 0.5);
    let _text_rect = egui::Rect::from_min_size(text_pos, tsz);
    ui.painter().text(text_pos, egui::Align2::LEFT_TOP, text, font, tcol);
    resp
}

pub fn toggle(ui: &mut egui::Ui, on: &mut bool, text: &str) -> Response {
    let s = if *on { format!("✅ {text}") } else { format!("⬜ {text}") };
    let r = sdf_button(ui, &s, *on);
    if r.clicked() { *on = !*on; }
    r
}


