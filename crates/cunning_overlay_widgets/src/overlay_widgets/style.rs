//! Overlay theme and animation utilities.
use bevy_egui::egui::{self, Align2, Color32, Context, CornerRadius, FontId, Id, Pos2, Rect, Stroke, Ui, Vec2};
use egui_wgpu::sdf::{create_sdf_circle_callback, create_sdf_rect_callback, SdfCircleUniform, SdfRectUniform};
#[cfg(not(target_arch = "wasm32"))]
use egui_wgpu::sdf::{create_gpu_text_callback, GpuTextUniform};

#[derive(Clone, Copy)]
pub struct OverlayTheme {
    pub scale: f32,
    pub bg_panel: Color32,
    pub bg_group: Color32,
    pub bg_input: Color32,
    pub fg_primary: Color32,
    pub fg_secondary: Color32,
    pub accent: Color32,
    pub hover: Color32,
    pub press: Color32,
    pub stroke: Stroke,
    pub radius_sm: CornerRadius,
    pub radius_md: CornerRadius,
    pub radius_lg: CornerRadius,
    pub icon_size: Vec2,
    pub spacing: f32,
    pub palette_gap: f32,
    pub palette_cell_min: f32,
    pub palette_cell_max: f32,
    pub palette_strip_cols: u8,
    pub palette_strip_cell_px: f32,
    pub sep_opacity: f32,
    pub sep_thickness: f32,
    pub palette_grid_opacity: f32,
    pub palette_grid_thickness: f32,
    pub text_opacity: f32,
    pub input_box_w: f32,
    pub input_box_h: f32,
}

impl OverlayTheme {
    #[inline]
    pub fn from_ui(ui: &egui::Ui) -> Self {
        let v = &ui.style().visuals;
        let dark = v.dark_mode;
        let scale = 1.00;
        let s = |px: f32| px * scale;
        let rad = |r: u8| CornerRadius::same(((r as f32) * scale).round().clamp(1.0, 255.0) as u8);
        let base = v.window_fill();
        let bg_panel = Color32::TRANSPARENT;
        let bg_group = Color32::TRANSPARENT;
        let bg_input = Self::lighten(base, if dark { 0.10 } else { 0.05 });
        let text_opacity = if cfg!(target_arch = "wasm32") { 1.0 } else { 0.90 };
        Self {
            scale,
            bg_panel,
            bg_group,
            bg_input,
            fg_primary: Self::with_alpha_mul(v.text_color(), text_opacity),
            fg_secondary: Self::with_alpha_mul(v.weak_text_color(), text_opacity),
            accent: v.selection.bg_fill,
            hover: Self::lighten(bg_input, if dark { 0.06 } else { 0.04 }),
            press: Self::darken(bg_input, if dark { 0.20 } else { 0.10 }),
            stroke: Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
            radius_sm: rad(2),
            radius_md: rad(3),
            radius_lg: rad(4),
            icon_size: Vec2::splat(s(30.0)),
            spacing: s(7.0),
            palette_gap: s(2.0),
            palette_cell_min: s(12.0),
            palette_cell_max: s(22.0),
            // MagicaVoxel-like palette: tall bars, multi-column fill.
            palette_strip_cols: 8,
            palette_strip_cell_px: s(18.0),
            sep_opacity: 0.20,
            sep_thickness: s(2.0),
            palette_grid_opacity: 0.35,
            palette_grid_thickness: s(1.0),
            text_opacity,
            input_box_w: s(65.0),
            input_box_h: s(30.0),
        }
    }

    #[inline]
    pub fn lerp_color(a: Color32, b: Color32, t: f32) -> Color32 {
        let t = t.clamp(0.0, 1.0);
        Color32::from_rgba_unmultiplied(
            (a.r() as f32 + (b.r() as f32 - a.r() as f32) * t) as u8,
            (a.g() as f32 + (b.g() as f32 - a.g() as f32) * t) as u8,
            (a.b() as f32 + (b.b() as f32 - a.b() as f32) * t) as u8,
            (a.a() as f32 + (b.a() as f32 - a.a() as f32) * t) as u8,
        )
    }

    #[inline] pub fn darken(c: Color32, t: f32) -> Color32 { Self::lerp_color(c, Color32::BLACK, t) }
    #[inline] pub fn lighten(c: Color32, t: f32) -> Color32 { Self::lerp_color(c, Color32::WHITE, t) }
    #[inline]
    pub fn with_alpha_mul(c: Color32, m: f32) -> Color32 {
        let a = ((c.a() as f32) * m.clamp(0.0, 1.0)).round().clamp(0.0, 255.0) as u8;
        Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), a)
    }
}

#[inline] pub fn animate_hover(ctx: &Context, id: Id, hovered: bool) -> f32 { ctx.animate_value_with_time(id.with("hover"), if hovered { 1.0 } else { 0.0 }, 0.12) }
#[inline] pub fn animate_select(ctx: &Context, id: Id, selected: bool) -> f32 { ctx.animate_value_with_time(id.with("sel"), if selected { 1.0 } else { 0.0 }, 0.15) }
#[inline] pub fn animate_toggle(ctx: &Context, id: Id, on: bool) -> f32 { ctx.animate_value_with_time(id.with("tog"), if on { 1.0 } else { 0.0 }, 0.18) }

#[inline]
pub fn paint_sdf_rect(ui: &mut Ui, rect: Rect, rounding: CornerRadius, fill: Color32, border: Color32, border_w: f32) {
    let c = rect.center();
    let s = rect.size();
    let screen = ui.ctx().content_rect().size();
    let uniform = SdfRectUniform {
        center: [c.x, c.y],
        half_size: [s.x * 0.5, s.y * 0.5],
        corner_radii: [rounding.nw as f32, rounding.ne as f32, rounding.se as f32, rounding.sw as f32],
        fill_color: egui::Rgba::from(fill).to_array(),
        shadow_color: [0.0; 4],
        shadow_blur: 0.0,
        _pad1: 0.0,
        shadow_offset: [0.0, 0.0],
        border_width: border_w.max(0.0),
        _pad2: [0.0; 3],
        border_color: egui::Rgba::from(border).to_array(),
        screen_size: [screen.x, screen.y],
        _pad3: [0.0; 2],
    };
    let frame_id = ui.ctx().cumulative_frame_nr();
    ui.painter().add(create_sdf_rect_callback(rect, uniform, frame_id));
}

#[inline]
pub fn paint_sdf_circle(ui: &mut Ui, center: Pos2, radius: f32, fill: Color32, border: Color32, border_w: f32) {
    let screen = ui.ctx().content_rect().size();
    let u = SdfCircleUniform {
        center: [center.x, center.y],
        radius: radius.max(0.0),
        border_width: border_w.max(0.0),
        fill_color: egui::Rgba::from(fill).to_array(),
        border_color: egui::Rgba::from(border).to_array(),
        softness: 1.0,
        _pad0: 0.0,
        screen_size: [screen.x, screen.y],
        _pad1: [0.0; 2],
        _pad2: [0.0; 2],
    };
    let frame_id = ui.ctx().cumulative_frame_nr();
    let r = Rect::from_center_size(center, Vec2::splat((radius + border_w + 2.0) * 2.0));
    ui.painter().add(create_sdf_circle_callback(r, ui.painter().clip_rect(), u, frame_id));
}

#[inline]
pub fn paint_gpu_text(ui: &mut Ui, pos: Pos2, align: Align2, text: impl Into<String>, font_px: f32, color: Color32) {
    let text = text.into();
    #[cfg(target_arch = "wasm32")]
    {
        ui.painter().text(pos, align, text, FontId::proportional(font_px), color);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let galley = ui.fonts_mut(|f| f.layout_no_wrap(text.clone(), FontId::proportional(font_px), color));
        let rect = align.anchor_size(pos, galley.size());
        let frame_id = ui.ctx().cumulative_frame_nr();
        ui.painter().add(create_gpu_text_callback(
            ui.painter().clip_rect(),
            GpuTextUniform { text, pos: rect.min, color, font_px, bounds: rect.size(), family: 0 },
            frame_id,
        ));
    }
}
