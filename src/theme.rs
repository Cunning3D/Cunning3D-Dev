//! Theme and style definitions.

use crate::ui_settings::UiSettings;
use bevy::prelude::*;
use bevy_egui::egui::{
    Color32, CornerRadius, FontFamily, FontId, Margin, Stroke, Style as EguiStyle, Visuals,
};
use bevy_egui::EguiContexts;

#[derive(Clone, Copy, PartialEq)]
pub enum ThemeMode {
    Light,
    Dark,
}

/// A resource that holds the current theme settings.
#[derive(Resource, Clone)]
pub struct ModernTheme {
    pub mode: ThemeMode,
    pub colors: ThemeColors,
    pub fonts: ThemeFonts,
    pub styles: ThemeStyles,
}

#[derive(Clone, Copy)]
pub struct ThemeColors {
    // Background colors
    pub primary_background: Color32,
    pub secondary_background: Color32,
    pub panel_background: Color32,

    // Text colors
    pub primary_text: Color32,
    pub secondary_text: Color32,
    pub accent_text: Color32,

    // Interaction colors
    pub hover_color: Color32,
    pub pressed_color: Color32,
    pub selected_color: Color32,

    // Borders and separators
    pub border_color: Color32,
    pub divider_color: Color32,

    // Accent colors
    pub accent_blue: Color32,
    pub accent_green: Color32,
    pub accent_orange: Color32,
    pub accent_red: Color32,

    // Node editor specific colors
    pub node_background: Color32,
    pub node_border: Color32,
    pub node_selected: Color32,
    pub connection_line: Color32,

    // Node state colors
    pub node_display: Color32,
    pub node_bypassed: Color32,
    pub node_template: Color32,
    pub node_locked_border: Color32,

    // NEW: Menu Button Colors
    pub menu_button_bypass_active: Color32,
    pub menu_button_visible_active: Color32,
    pub menu_button_template_active: Color32,
    pub menu_button_lock_active: Color32,

    // OLD Indicator Colors - Will be deprecated/removed later if unused
    pub indicator_display: Color32,
    pub indicator_template: Color32,
    pub indicator_bypass: Color32,
    pub indicator_lock: Color32,
}

#[derive(Clone)]
pub struct ThemeFonts {
    pub heading: FontId,
    pub body: FontId,
    pub small: FontId,
    pub monospace: FontId,
}

#[derive(Clone, Copy)]
pub struct ThemeStyles {
    pub rounding: CornerRadius,
    pub margin: Margin,
    pub button_padding: bevy_egui::egui::Vec2,
    pub panel_margin: Margin,
}

impl Default for ModernTheme {
    fn default() -> Self {
        Self::light()
    }
}

impl ModernTheme {
    pub fn light() -> Self {
        Self {
            mode: ThemeMode::Light,
            colors: ThemeColors::light(),
            fonts: ThemeFonts::default(),
            styles: ThemeStyles::default(),
        }
    }

    pub fn dark() -> Self {
        Self {
            mode: ThemeMode::Dark,
            colors: ThemeColors::dark(),
            fonts: ThemeFonts::default(),
            styles: ThemeStyles::default(),
        }
    }

    pub fn toggle_mode(&mut self) {
        match self.mode {
            ThemeMode::Light => {
                self.mode = ThemeMode::Dark;
                self.colors = ThemeColors::dark();
            }
            ThemeMode::Dark => {
                self.mode = ThemeMode::Light;
                self.colors = ThemeColors::light();
            }
        }
    }

    pub fn apply_to_egui_style(&self) -> EguiStyle {
        let mut style = EguiStyle::default();

        // Apply visual style
        style.visuals = if matches!(self.mode, ThemeMode::Dark) {
            Visuals::dark()
        } else {
            Visuals::light()
        };

        // --- Win98 Style Overrides ---
        // Disable Window Shadows for that flat, classic look
        style.visuals.window_shadow = bevy_egui::egui::epaint::Shadow::NONE;
        // Enforce solid, 1px borders for windows
        style.visuals.window_stroke = Stroke::new(1.0, self.colors.border_color);

        // Custom colors
        style.visuals.window_fill = self.colors.primary_background;
        style.visuals.panel_fill = self.colors.panel_background;
        style.visuals.extreme_bg_color = self.colors.secondary_background;

        style.visuals.widgets.noninteractive.bg_fill = self.colors.secondary_background;
        style.visuals.widgets.inactive.bg_fill = self.colors.secondary_background;
        style.visuals.widgets.hovered.bg_fill = self.colors.hover_color;
        style.visuals.widgets.active.bg_fill = self.colors.pressed_color;

        // Use the defined text colors for the widget's foreground (text).
        style.visuals.widgets.noninteractive.fg_stroke =
            Stroke::new(1.0, self.colors.secondary_text);
        // Make strokes slightly thicker for that "chunky" feel
        style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, self.colors.primary_text);
        style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, self.colors.primary_text);
        style.visuals.widgets.active.fg_stroke = Stroke::new(1.0, self.colors.primary_text);

        // Win98 Border Simulation (Simple)
        // We use a 1.0px solid border for all widgets.
        style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, self.colors.border_color);
        style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, self.colors.border_color);
        style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, self.colors.accent_text); // Highlight border on hover
        style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, self.colors.accent_text);

        // Set the global override text color for any text not in a widget.
        style.visuals.override_text_color = Some(self.colors.primary_text);

        // Set rounding (force zero rounding)
        style.visuals.widgets.noninteractive.corner_radius = CornerRadius::ZERO;
        style.visuals.widgets.inactive.corner_radius = CornerRadius::ZERO;
        style.visuals.widgets.hovered.corner_radius = CornerRadius::ZERO;
        style.visuals.widgets.active.corner_radius = CornerRadius::ZERO;
        style.visuals.window_corner_radius = CornerRadius::ZERO;
        style.visuals.menu_corner_radius = CornerRadius::ZERO;

        // Houdini-style: ultra compact spacing
        style.spacing.button_padding = self.styles.button_padding;
        style.spacing.item_spacing = bevy_egui::egui::Vec2::new(3.0, 1.0);
        style.spacing.window_margin = self.styles.panel_margin;
        style.spacing.indent = 14.0;
        style.spacing.interact_size = bevy_egui::egui::Vec2::new(32.0, 16.0);
        style.spacing.icon_width = 12.0;
        style.spacing.icon_width_inner = 8.0;
        style.spacing.icon_spacing = 2.0;
        style.spacing.menu_margin = bevy_egui::egui::Margin::same(2);

        style
    }
}

impl ThemeColors {
    pub fn light() -> Self {
        Self {
            primary_background: Color32::from_rgb(192, 192, 192), // Classic Win98 Grey
            secondary_background: Color32::from_rgb(255, 255, 255), // White for inputs
            panel_background: Color32::from_rgb(192, 192, 192),

            primary_text: Color32::BLACK,
            secondary_text: Color32::from_gray(100),
            accent_text: Color32::BLUE, // Classic selection blue

            hover_color: Color32::from_rgb(220, 220, 220),
            pressed_color: Color32::from_rgb(160, 160, 160),
            selected_color: Color32::BLUE,

            border_color: Color32::BLACK, // Hard black borders
            divider_color: Color32::from_gray(128),

            // Keep accents vibrant
            accent_blue: Color32::BLUE,
            accent_green: Color32::from_rgb(0, 128, 0),
            accent_orange: Color32::from_rgb(255, 128, 0),
            accent_red: Color32::RED,

            node_background: Color32::from_rgb(255, 255, 255),
            node_border: Color32::BLACK,
            node_selected: Color32::BLUE,
            connection_line: Color32::BLACK,

            // Node State Colors (Light)
            node_display: Color32::from_rgb(200, 220, 255),
            node_bypassed: Color32::from_gray(180),
            node_template: Color32::from_gray(180),
            node_locked_border: Color32::from_rgb(255, 0, 0),

            // NEW: Menu Button Colors
            menu_button_bypass_active: Color32::YELLOW,
            menu_button_visible_active: Color32::GREEN,
            menu_button_template_active: Color32::from_rgb(255, 0, 255),
            menu_button_lock_active: Color32::RED,

            // Indicator Colors
            indicator_display: Color32::BLUE,
            indicator_template: Color32::from_rgb(255, 0, 255),
            indicator_bypass: Color32::YELLOW,
            indicator_lock: Color32::RED,
        }
    }

    pub fn dark() -> Self {
        // Dark Win98 (Industrial Dark)
        Self {
            primary_background: Color32::from_rgb(50, 50, 50),
            secondary_background: Color32::from_rgb(30, 30, 30),
            panel_background: Color32::from_rgb(50, 50, 50),

            // Dark-mode text in this app is intentionally closer to white for better perceived contrast (closer to Zed/GPUI feel).
            primary_text: Color32::from_gray(240),
            secondary_text: Color32::from_gray(190),
            accent_text: Color32::from_rgb(100, 150, 255),

            hover_color: Color32::from_rgb(70, 70, 70),
            pressed_color: Color32::from_rgb(30, 30, 30),
            selected_color: Color32::from_rgb(0, 0, 128),

            border_color: Color32::BLACK, // Still black borders
            divider_color: Color32::from_rgb(80, 80, 80),

            accent_blue: Color32::from_rgb(100, 150, 255),
            accent_green: Color32::from_rgb(80, 200, 100),
            accent_orange: Color32::from_rgb(255, 150, 50),
            accent_red: Color32::from_rgb(255, 80, 80),

            // Node bodies should stay clearly visible above the dark canvas/grid (milk-white).
            node_background: Color32::from_rgb(250, 250, 250),
            node_border: Color32::from_rgb(140, 140, 140),
            node_selected: Color32::YELLOW, // Yellow outline on dark is very visible
            connection_line: Color32::from_rgb(180, 180, 180),

            // Node State Colors (Dark)
            node_display: Color32::from_rgb(0, 50, 100),
            node_bypassed: Color32::from_rgb(60, 60, 60),
            node_template: Color32::from_rgb(60, 60, 60),
            node_locked_border: Color32::from_rgb(200, 50, 0),

            // NEW: Menu Button Colors
            menu_button_bypass_active: Color32::YELLOW,
            menu_button_visible_active: Color32::GREEN,
            menu_button_template_active: Color32::from_rgb(200, 80, 255),
            menu_button_lock_active: Color32::RED,

            // Indicator Colors
            indicator_display: Color32::from_rgb(50, 150, 255),
            indicator_template: Color32::from_rgb(255, 80, 150),
            indicator_bypass: Color32::YELLOW,
            indicator_lock: Color32::RED,
        }
    }
}

impl Default for ThemeFonts {
    fn default() -> Self {
        Self {
            heading: FontId::new(18.0, FontFamily::Proportional),
            body: FontId::new(14.0, FontFamily::Proportional),
            small: FontId::new(12.0, FontFamily::Proportional),
            monospace: FontId::new(13.0, FontFamily::Monospace),
        }
    }
}

impl Default for ThemeStyles {
    fn default() -> Self {
        Self {
            rounding: CornerRadius::ZERO,
            margin: Margin::same(2),                              // Houdini-style: minimal
            button_padding: bevy_egui::egui::Vec2::new(4.0, 2.0), // Houdini-style: tiny
            panel_margin: Margin::same(2),
        }
    }
}

pub fn setup_theme(mut contexts: EguiContexts, theme: Res<ModernTheme>) {
    let ctx = contexts.ctx_mut();
    init_egui_context(ctx, &theme);
}

pub fn init_egui_context(ctx: &bevy_egui::egui::Context, theme: &ModernTheme) {
    ctx.set_style(theme.apply_to_egui_style());
    let mut fonts = bevy_egui::egui::FontDefinitions::default();
    let mut any = !fonts.font_data.is_empty();
    #[cfg(target_os = "windows")]
    {
        let fonts_dirs = vec![
            std::env::var("WINDIR")
                .map(|d| std::path::Path::new(&d).join("Fonts"))
                .unwrap_or_else(|_| std::path::PathBuf::from("C:\\Windows\\Fonts")),
            std::path::PathBuf::from("C:\\Windows\\Fonts"),
        ];
        let candidates = ["msyh.ttc", "msyh.ttf", "simhei.ttf", "simsun.ttc"];
        'outer: for dir in fonts_dirs {
            if !dir.exists() {
                continue;
            }
            for file in candidates.iter() {
                let path = dir.join(file);
                if let Ok(data) = std::fs::read(&path) {
                    bevy::prelude::info!("Loading system font for CJK support: {:?}", path);
                    fonts.font_data.insert(
                        "cjk".to_owned(),
                        bevy_egui::egui::FontData::from_owned(data).into(),
                    );
                    fonts
                        .families
                        .entry(bevy_egui::egui::FontFamily::Proportional)
                        .or_default()
                        .insert(0, "cjk".to_owned());
                    fonts
                        .families
                        .entry(bevy_egui::egui::FontFamily::Monospace)
                        .or_default()
                        .insert(0, "cjk".to_owned());
                    any = true;
                    break 'outer;
                }
            }
        }
    }
    if any {
        ctx.set_fonts(fonts);
    }
}

pub fn init_new_window_egui_fonts_system(
    mut commands: Commands,
    mut q: Query<(Entity, &mut bevy_egui::EguiContext), With<crate::ui::NeedsEguiFontsInit>>,
    theme: Res<ModernTheme>,
) {
    for (e, mut c) in q.iter_mut() {
        init_egui_context(c.get_mut(), &*theme);
        commands.entity(e).remove::<crate::ui::NeedsEguiFontsInit>();
    }
}

fn ui_settings_hash(s: &UiSettings) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut h = DefaultHasher::new();
    (if matches!(s.theme_mode, ThemeMode::Dark) {
        0u8
    } else {
        1u8
    })
    .hash(&mut h);
    for f in [
        s.rounding,
        s.rounding_window_mul,
        s.rounding_menu_mul,
        s.rounding_widget_mul,
        s.density,
        s.font_scale,
        s.font_heading_mul,
        s.font_body_mul,
        s.font_button_mul,
        s.font_small_mul,
        s.font_monospace_mul,
        s.spacing_window_margin_mul,
        s.spacing_menu_margin_mul,
        s.spacing_indent_mul,
        s.spacing_slider_width_mul,
        s.spacing_slider_rail_height_mul,
        s.spacing_combo_width_mul,
        s.spacing_text_edit_width_mul,
        s.spacing_icon_width_mul,
        s.spacing_icon_width_inner_mul,
        s.spacing_icon_spacing_mul,
        s.spacing_tooltip_width_mul,
        s.spacing_menu_width_mul,
        s.spacing_menu_spacing_mul,
        s.spacing_combo_height_mul,
        s.scroll_bar_width,
        s.scroll_handle_min_length,
        s.scroll_bar_inner_margin,
        s.scroll_bar_outer_margin,
        s.scroll_floating_width,
        s.scroll_floating_allocated_width,
        s.scroll_dormant_background_opacity,
        s.scroll_active_background_opacity,
        s.scroll_interact_background_opacity,
        s.scroll_dormant_handle_opacity,
        s.scroll_active_handle_opacity,
        s.scroll_interact_handle_opacity,
        s.interaction_interact_radius,
        s.interaction_resize_grab_radius_side,
        s.interaction_resize_grab_radius_corner,
        s.interaction_tooltip_delay,
        s.stroke_window,
        s.stroke_widget,
        s.stroke_selection,
        s.shadow_blur,
        s.shadow_spread,
        s.shadow_alpha,
    ] {
        f.to_bits().hash(&mut h);
    }
    for f in [
        s.spacing_item_mul[0],
        s.spacing_item_mul[1],
        s.spacing_button_mul[0],
        s.spacing_button_mul[1],
        s.spacing_interact_mul[0],
        s.spacing_interact_mul[1],
        s.shadow_offset[0],
        s.shadow_offset[1],
    ] {
        f.to_bits().hash(&mut h);
    }
    for b in [
        s.scroll_floating,
        s.scroll_foreground_color,
        s.interaction_show_tooltips_only_when_still,
        s.interaction_selectable_labels,
        s.interaction_multi_widget_text_select,
        s.color_override_text,
    ] {
        b.hash(&mut h);
    }
    for c in [
        s.color_accent,
        s.color_selection_bg,
        s.color_selection_stroke,
        s.color_hyperlink,
        s.color_warn,
        s.color_error,
        s.color_text,
    ] {
        c.hash(&mut h);
    }
    h.finish()
}

pub fn apply_ui_settings(mut contexts: EguiContexts, s: Res<UiSettings>, mut last: Local<u64>) {
    let h = ui_settings_hash(&s);
    if *last == h {
        return;
    }
    *last = h;
    let theme = if matches!(s.theme_mode, ThemeMode::Dark) {
        ModernTheme::dark()
    } else {
        ModernTheme::light()
    };
    let mut style = theme.apply_to_egui_style();
    let d = s.density.max(0.05);
    let r = s.rounding.max(0.0);
    style.visuals.window_corner_radius = CornerRadius::same(
        (r * s.rounding_window_mul.max(0.0))
            .round()
            .clamp(0.0, 255.0) as u8,
    );
    style.visuals.menu_corner_radius =
        CornerRadius::same((r * s.rounding_menu_mul.max(0.0)).round().clamp(0.0, 255.0) as u8);
    let wr = CornerRadius::same(
        (r * s.rounding_widget_mul.max(0.0))
            .round()
            .clamp(0.0, 255.0) as u8,
    );
    style.visuals.widgets.noninteractive.corner_radius = wr;
    style.visuals.widgets.inactive.corner_radius = wr;
    style.visuals.widgets.hovered.corner_radius = wr;
    style.visuals.widgets.active.corner_radius = wr;
    style.spacing.item_spacing = bevy_egui::egui::vec2(
        style.spacing.item_spacing.x * s.spacing_item_mul[0] * d,
        style.spacing.item_spacing.y * s.spacing_item_mul[1] * d,
    );
    style.spacing.button_padding = bevy_egui::egui::vec2(
        style.spacing.button_padding.x * s.spacing_button_mul[0] * d,
        style.spacing.button_padding.y * s.spacing_button_mul[1] * d,
    );
    style.spacing.interact_size = bevy_egui::egui::vec2(
        style.spacing.interact_size.x * s.spacing_interact_mul[0] * d,
        style.spacing.interact_size.y * s.spacing_interact_mul[1] * d,
    );
    style.spacing.window_margin = Margin::same(
        (style.spacing.window_margin.left as f32 * s.spacing_window_margin_mul.max(0.0))
            .round()
            .clamp(0.0, 127.0) as i8,
    );
    style.spacing.menu_margin = Margin::same(
        (style.spacing.menu_margin.left as f32 * s.spacing_menu_margin_mul.max(0.0))
            .round()
            .clamp(0.0, 127.0) as i8,
    );
    style.spacing.indent = style.spacing.indent * s.spacing_indent_mul.max(0.0);
    style.spacing.slider_width = style.spacing.slider_width * s.spacing_slider_width_mul.max(0.0);
    style.spacing.combo_width = style.spacing.combo_width * s.spacing_combo_width_mul.max(0.0);
    style.spacing.text_edit_width =
        style.spacing.text_edit_width * s.spacing_text_edit_width_mul.max(0.0);
    style.spacing.icon_width = style.spacing.icon_width * s.spacing_icon_width_mul.max(0.0);
    style.spacing.icon_width_inner =
        style.spacing.icon_width_inner * s.spacing_icon_width_inner_mul.max(0.0);
    style.spacing.icon_spacing = style.spacing.icon_spacing * s.spacing_icon_spacing_mul.max(0.0);
    style.spacing.tooltip_width =
        style.spacing.tooltip_width * s.spacing_tooltip_width_mul.max(0.0);
    style.spacing.menu_width = style.spacing.menu_width * s.spacing_menu_width_mul.max(0.0);
    style.spacing.menu_spacing = style.spacing.menu_spacing * s.spacing_menu_spacing_mul.max(0.0);
    style.spacing.combo_height = style.spacing.combo_height * s.spacing_combo_height_mul.max(0.0);
    style.spacing.scroll = bevy_egui::egui::style::ScrollStyle {
        floating: s.scroll_floating,
        bar_width: s.scroll_bar_width.max(0.0),
        handle_min_length: s.scroll_handle_min_length.max(0.0),
        bar_inner_margin: s.scroll_bar_inner_margin.max(0.0),
        bar_outer_margin: s.scroll_bar_outer_margin.max(0.0),
        floating_width: s.scroll_floating_width.max(0.0),
        floating_allocated_width: s.scroll_floating_allocated_width.max(0.0),
        foreground_color: s.scroll_foreground_color,
        dormant_background_opacity: s.scroll_dormant_background_opacity.clamp(0.0, 1.0),
        active_background_opacity: s.scroll_active_background_opacity.clamp(0.0, 1.0),
        interact_background_opacity: s.scroll_interact_background_opacity.clamp(0.0, 1.0),
        dormant_handle_opacity: s.scroll_dormant_handle_opacity.clamp(0.0, 1.0),
        active_handle_opacity: s.scroll_active_handle_opacity.clamp(0.0, 1.0),
        interact_handle_opacity: s.scroll_interact_handle_opacity.clamp(0.0, 1.0),
    };
    style.interaction.interact_radius = s.interaction_interact_radius.max(0.0);
    style.interaction.resize_grab_radius_side = s.interaction_resize_grab_radius_side.max(0.0);
    style.interaction.resize_grab_radius_corner = s.interaction_resize_grab_radius_corner.max(0.0);
    style.interaction.show_tooltips_only_when_still = s.interaction_show_tooltips_only_when_still;
    style.interaction.tooltip_delay = s.interaction_tooltip_delay.max(0.0);
    style.interaction.selectable_labels = s.interaction_selectable_labels;
    style.interaction.multi_widget_text_select = s.interaction_multi_widget_text_select;
    style.visuals.window_stroke.width = s.stroke_window.max(0.0);
    style.visuals.widgets.inactive.bg_stroke.width = s.stroke_widget.max(0.0);
    style.visuals.widgets.hovered.bg_stroke.width = s.stroke_widget.max(0.0);
    style.visuals.widgets.active.bg_stroke.width = s.stroke_widget.max(0.0);
    style.visuals.selection.stroke.width = s.stroke_selection.max(0.0);
    let rgba = |c: [u8; 4]| Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]);
    style.visuals.hyperlink_color = rgba(s.color_hyperlink);
    style.visuals.warn_fg_color = rgba(s.color_warn);
    style.visuals.error_fg_color = rgba(s.color_error);
    style.visuals.selection.bg_fill = rgba(s.color_selection_bg);
    style.visuals.selection.stroke.color = rgba(s.color_selection_stroke);
    if s.color_override_text {
        style.visuals.override_text_color = Some(rgba(s.color_text));
    }
    let mut sh = style.visuals.window_shadow;
    sh.blur = s.shadow_blur.max(0.0).round().clamp(0.0, 255.0) as u8;
    sh.spread = s.shadow_spread.max(0.0).round().clamp(0.0, 255.0) as u8;
    sh.offset = [
        s.shadow_offset[0].round().clamp(-128.0, 127.0) as i8,
        s.shadow_offset[1].round().clamp(-128.0, 127.0) as i8,
    ];
    sh.color =
        Color32::from_rgba_unmultiplied(0, 0, 0, (255.0 * s.shadow_alpha.clamp(0.0, 1.0)) as u8);
    style.visuals.window_shadow = sh;
    style.visuals.popup_shadow = sh;
    for (k, v) in style.text_styles.iter_mut() {
        let m = match k {
            bevy_egui::egui::TextStyle::Heading => s.font_heading_mul,
            bevy_egui::egui::TextStyle::Body => s.font_body_mul,
            bevy_egui::egui::TextStyle::Button => s.font_button_mul,
            bevy_egui::egui::TextStyle::Small => s.font_small_mul,
            bevy_egui::egui::TextStyle::Monospace => s.font_monospace_mul,
            _ => 1.0,
        };
        v.size = (v.size * s.font_scale.max(0.05) * m.max(0.05)).max(1.0);
    }
    contexts.ctx_mut().set_style(style);
}
