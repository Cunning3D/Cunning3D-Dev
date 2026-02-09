use crate::settings::{
    SettingMeta, SettingScope, SettingValue, SettingsMerge, SettingsRegistry, SettingsStores,
};
use crate::theme::ThemeMode;
use bevy::prelude::*;

#[derive(Resource, Clone, PartialEq)]
pub struct UiSettings {
    pub theme_mode: ThemeMode,
    pub rounding: f32,
    pub rounding_window_mul: f32,
    pub rounding_menu_mul: f32,
    pub rounding_widget_mul: f32,
    pub density: f32,
    pub font_scale: f32,
    pub font_heading_mul: f32,
    pub font_body_mul: f32,
    pub font_button_mul: f32,
    pub font_small_mul: f32,
    pub font_monospace_mul: f32,
    pub spacing_item_mul: [f32; 2],
    pub spacing_button_mul: [f32; 2],
    pub spacing_interact_mul: [f32; 2],
    pub spacing_window_margin_mul: f32,
    pub spacing_menu_margin_mul: f32,
    pub spacing_indent_mul: f32,
    pub spacing_slider_width_mul: f32,
    pub spacing_slider_rail_height_mul: f32,
    pub spacing_combo_width_mul: f32,
    pub spacing_text_edit_width_mul: f32,
    pub spacing_icon_width_mul: f32,
    pub spacing_icon_width_inner_mul: f32,
    pub spacing_icon_spacing_mul: f32,
    pub spacing_tooltip_width_mul: f32,
    pub spacing_menu_width_mul: f32,
    pub spacing_menu_spacing_mul: f32,
    pub spacing_combo_height_mul: f32,
    pub scroll_floating: bool,
    pub scroll_bar_width: f32,
    pub scroll_handle_min_length: f32,
    pub scroll_bar_inner_margin: f32,
    pub scroll_bar_outer_margin: f32,
    pub scroll_floating_width: f32,
    pub scroll_floating_allocated_width: f32,
    pub scroll_foreground_color: bool,
    pub scroll_dormant_background_opacity: f32,
    pub scroll_active_background_opacity: f32,
    pub scroll_interact_background_opacity: f32,
    pub scroll_dormant_handle_opacity: f32,
    pub scroll_active_handle_opacity: f32,
    pub scroll_interact_handle_opacity: f32,
    pub interaction_interact_radius: f32,
    pub interaction_resize_grab_radius_side: f32,
    pub interaction_resize_grab_radius_corner: f32,
    pub interaction_show_tooltips_only_when_still: bool,
    pub interaction_tooltip_delay: f32,
    pub interaction_selectable_labels: bool,
    pub interaction_multi_widget_text_select: bool,
    pub stroke_window: f32,
    pub stroke_widget: f32,
    pub stroke_selection: f32,
    pub color_accent: [u8; 4],
    pub color_selection_bg: [u8; 4],
    pub color_selection_stroke: [u8; 4],
    pub color_hyperlink: [u8; 4],
    pub color_warn: [u8; 4],
    pub color_error: [u8; 4],
    pub color_override_text: bool,
    pub color_text: [u8; 4],
    pub shadow_blur: f32,
    pub shadow_spread: f32,
    pub shadow_alpha: f32,
    pub shadow_offset: [f32; 2],
}

impl Default for UiSettings {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::Dark,
            rounding: 3.0,
            rounding_window_mul: 1.0,
            rounding_menu_mul: 1.0,
            rounding_widget_mul: 1.0,
            density: 1.0,
            font_scale: 1.0,
            font_heading_mul: 1.0,
            font_body_mul: 1.0,
            font_button_mul: 1.0,
            font_small_mul: 1.0,
            font_monospace_mul: 1.0,
            spacing_item_mul: [1.0, 1.0],
            spacing_button_mul: [1.0, 1.0],
            spacing_interact_mul: [1.0, 1.0],
            spacing_window_margin_mul: 1.0,
            spacing_menu_margin_mul: 1.0,
            spacing_indent_mul: 1.0,
            spacing_slider_width_mul: 1.0,
            spacing_slider_rail_height_mul: 1.0,
            spacing_combo_width_mul: 1.0,
            spacing_text_edit_width_mul: 1.0,
            spacing_icon_width_mul: 1.0,
            spacing_icon_width_inner_mul: 1.0,
            spacing_icon_spacing_mul: 1.0,
            spacing_tooltip_width_mul: 1.0,
            spacing_menu_width_mul: 1.0,
            spacing_menu_spacing_mul: 1.0,
            spacing_combo_height_mul: 1.0,
            scroll_floating: true,
            scroll_bar_width: 10.0,
            scroll_handle_min_length: 12.0,
            scroll_bar_inner_margin: 4.0,
            scroll_bar_outer_margin: 0.0,
            scroll_floating_width: 2.0,
            scroll_floating_allocated_width: 0.0,
            scroll_foreground_color: true,
            scroll_dormant_background_opacity: 0.0,
            scroll_active_background_opacity: 0.4,
            scroll_interact_background_opacity: 0.7,
            scroll_dormant_handle_opacity: 0.0,
            scroll_active_handle_opacity: 0.6,
            scroll_interact_handle_opacity: 1.0,
            interaction_interact_radius: 5.0,
            interaction_resize_grab_radius_side: 5.0,
            interaction_resize_grab_radius_corner: 10.0,
            interaction_show_tooltips_only_when_still: true,
            interaction_tooltip_delay: 0.3,
            interaction_selectable_labels: true,
            interaction_multi_widget_text_select: true,
            stroke_window: 1.0,
            stroke_widget: 1.0,
            stroke_selection: 1.0,
            color_accent: [90, 170, 255, 255],
            color_selection_bg: [90, 170, 255, 64],
            color_selection_stroke: [90, 170, 255, 255],
            color_hyperlink: [90, 170, 255, 255],
            color_warn: [255, 143, 0, 255],
            color_error: [255, 0, 0, 255],
            color_override_text: false,
            color_text: [240, 240, 240, 255],
            shadow_blur: 23.0,
            shadow_spread: 0.0,
            shadow_alpha: 0.35,
            shadow_offset: [0.0, 6.0],
        }
    }
}

pub fn apply_from_settings(reg: &SettingsRegistry, stores: &SettingsStores, s: &mut UiSettings) {
    let get = |id: &str| {
        reg.get(id).and_then(|m| {
            Some(SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id)).1)
        })
    };
    if let Some(SettingValue::Enum(v)) = get("ui.theme") {
        s.theme_mode = if v.eq_ignore_ascii_case("light") {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
    }
    if let Some(SettingValue::F32(v)) = get("ui.rounding") {
        s.rounding = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.rounding.window_mul") {
        s.rounding_window_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.rounding.menu_mul") {
        s.rounding_menu_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.rounding.widget_mul") {
        s.rounding_widget_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.density") {
        s.density = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.scale") {
        s.font_scale = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.heading_mul") {
        s.font_heading_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.body_mul") {
        s.font_body_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.button_mul") {
        s.font_button_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.small_mul") {
        s.font_small_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.typography.monospace_mul") {
        s.font_monospace_mul = v;
    }
    if let Some(SettingValue::Vec2(v)) = get("ui.spacing.item_mul") {
        s.spacing_item_mul = v;
    }
    if let Some(SettingValue::Vec2(v)) = get("ui.spacing.button_mul") {
        s.spacing_button_mul = v;
    }
    if let Some(SettingValue::Vec2(v)) = get("ui.spacing.interact_mul") {
        s.spacing_interact_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.window_margin_mul") {
        s.spacing_window_margin_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.menu_margin_mul") {
        s.spacing_menu_margin_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.indent_mul") {
        s.spacing_indent_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.slider_width_mul") {
        s.spacing_slider_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.slider_rail_height_mul") {
        s.spacing_slider_rail_height_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.combo_width_mul") {
        s.spacing_combo_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.text_edit_width_mul") {
        s.spacing_text_edit_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.icon_width_mul") {
        s.spacing_icon_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.icon_width_inner_mul") {
        s.spacing_icon_width_inner_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.icon_spacing_mul") {
        s.spacing_icon_spacing_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.tooltip_width_mul") {
        s.spacing_tooltip_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.menu_width_mul") {
        s.spacing_menu_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.menu_spacing_mul") {
        s.spacing_menu_spacing_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.spacing.combo_height_mul") {
        s.spacing_combo_height_mul = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.scroll.floating") {
        s.scroll_floating = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.bar_width") {
        s.scroll_bar_width = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.handle_min_length") {
        s.scroll_handle_min_length = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.bar_inner_margin") {
        s.scroll_bar_inner_margin = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.bar_outer_margin") {
        s.scroll_bar_outer_margin = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.floating_width") {
        s.scroll_floating_width = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.floating_allocated_width") {
        s.scroll_floating_allocated_width = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.scroll.foreground_color") {
        s.scroll_foreground_color = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.dormant_background_opacity") {
        s.scroll_dormant_background_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.active_background_opacity") {
        s.scroll_active_background_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.interact_background_opacity") {
        s.scroll_interact_background_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.dormant_handle_opacity") {
        s.scroll_dormant_handle_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.active_handle_opacity") {
        s.scroll_active_handle_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.scroll.interact_handle_opacity") {
        s.scroll_interact_handle_opacity = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.interaction.interact_radius") {
        s.interaction_interact_radius = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.interaction.resize_grab_radius_side") {
        s.interaction_resize_grab_radius_side = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.interaction.resize_grab_radius_corner") {
        s.interaction_resize_grab_radius_corner = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.interaction.show_tooltips_only_when_still") {
        s.interaction_show_tooltips_only_when_still = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.interaction.tooltip_delay") {
        s.interaction_tooltip_delay = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.interaction.selectable_labels") {
        s.interaction_selectable_labels = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.interaction.multi_widget_text_select") {
        s.interaction_multi_widget_text_select = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.stroke.window") {
        s.stroke_window = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.stroke.widget") {
        s.stroke_widget = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.stroke.selection") {
        s.stroke_selection = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.accent") {
        s.color_accent = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.selection_bg") {
        s.color_selection_bg = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.selection_stroke") {
        s.color_selection_stroke = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.hyperlink") {
        s.color_hyperlink = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.warn") {
        s.color_warn = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.error") {
        s.color_error = v;
    }
    if let Some(SettingValue::Bool(v)) = get("ui.colors.override_text") {
        s.color_override_text = v;
    }
    if let Some(SettingValue::Color32(v)) = get("ui.colors.text") {
        s.color_text = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.shadow.blur") {
        s.shadow_blur = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.shadow.spread") {
        s.shadow_spread = v;
    }
    if let Some(SettingValue::F32(v)) = get("ui.shadow.alpha") {
        s.shadow_alpha = v;
    }
    if let Some(SettingValue::Vec2(v)) = get("ui.shadow.offset") {
        s.shadow_offset = v;
    }
}

pub fn sync_from_settings_stores(
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut s: ResMut<UiSettings>,
) {
    if !(reg.is_changed() || stores.is_changed() || s.is_added()) {
        return;
    }
    let mut next = (*s).clone();
    apply_from_settings(&reg, &stores, &mut next);
    if next != *s {
        *s = next;
    }
}

fn register_ui_settings(reg: &mut SettingsRegistry) {
    let d = UiSettings::default();
    fn f32m(
        reg: &mut SettingsRegistry,
        id: &str,
        path: &str,
        label: &str,
        help: &str,
        def: f32,
        min: f32,
        max: f32,
        step: f32,
        kw: &[&str],
    ) {
        reg.upsert(SettingMeta {
            id: id.into(),
            path: path.into(),
            label: label.into(),
            help: help.into(),
            scope: SettingScope::Both,
            default: SettingValue::F32(def),
            min: Some(min),
            max: Some(max),
            step: Some(step),
            keywords: kw.iter().map(|s| (*s).into()).collect(),
        })
    }
    fn v2m(
        reg: &mut SettingsRegistry,
        id: &str,
        path: &str,
        label: &str,
        help: &str,
        def: [f32; 2],
        min: f32,
        max: f32,
        step: f32,
        kw: &[&str],
    ) {
        reg.upsert(SettingMeta {
            id: id.into(),
            path: path.into(),
            label: label.into(),
            help: help.into(),
            scope: SettingScope::Both,
            default: SettingValue::Vec2(def),
            min: Some(min),
            max: Some(max),
            step: Some(step),
            keywords: kw.iter().map(|s| (*s).into()).collect(),
        })
    }
    fn col(
        reg: &mut SettingsRegistry,
        id: &str,
        path: &str,
        label: &str,
        help: &str,
        def: [u8; 4],
        kw: &[&str],
    ) {
        reg.upsert(SettingMeta {
            id: id.into(),
            path: path.into(),
            label: label.into(),
            help: help.into(),
            scope: SettingScope::Both,
            default: SettingValue::Color32(def),
            min: None,
            max: None,
            step: None,
            keywords: kw.iter().map(|s| (*s).into()).collect(),
        })
    }
    fn b(
        reg: &mut SettingsRegistry,
        id: &str,
        path: &str,
        label: &str,
        help: &str,
        def: bool,
        kw: &[&str],
    ) {
        reg.upsert(SettingMeta {
            id: id.into(),
            path: path.into(),
            label: label.into(),
            help: help.into(),
            scope: SettingScope::Both,
            default: SettingValue::Bool(def),
            min: None,
            max: None,
            step: None,
            keywords: kw.iter().map(|s| (*s).into()).collect(),
        })
    }

    reg.upsert(SettingMeta {
        id: "ui.theme".into(),
        path: "General/UI/Theme".into(),
        label: "Theme".into(),
        help: "Dark/Light".into(),
        scope: SettingScope::Both,
        default: SettingValue::Enum("Dark".into()),
        min: None,
        max: None,
        step: None,
        keywords: vec!["theme".into(), "dark".into(), "light".into()],
    });
    f32m(
        reg,
        "ui.rounding",
        "General/UI/Rounding",
        "Rounding",
        "Global corner rounding",
        d.rounding,
        0.0,
        32.0,
        0.25,
        &["round", "radius"],
    );
    f32m(
        reg,
        "ui.rounding.window_mul",
        "General/UI/Rounding",
        "Window Mul",
        "window_rounding multiplier",
        d.rounding_window_mul,
        0.0,
        3.0,
        0.05,
        &["round", "window"],
    );
    f32m(
        reg,
        "ui.rounding.menu_mul",
        "General/UI/Rounding",
        "Menu Mul",
        "menu_rounding multiplier",
        d.rounding_menu_mul,
        0.0,
        3.0,
        0.05,
        &["round", "menu"],
    );
    f32m(
        reg,
        "ui.rounding.widget_mul",
        "General/UI/Rounding",
        "Widget Mul",
        "widget rounding multiplier",
        d.rounding_widget_mul,
        0.0,
        3.0,
        0.05,
        &["round", "widget"],
    );

    f32m(
        reg,
        "ui.typography.scale",
        "General/UI/Typography",
        "Global Scale",
        "Multiply all font sizes",
        d.font_scale,
        0.5,
        3.0,
        0.05,
        &["font", "scale"],
    );
    f32m(
        reg,
        "ui.typography.heading_mul",
        "General/UI/Typography",
        "Heading Mul",
        "Heading size multiplier",
        d.font_heading_mul,
        0.5,
        3.0,
        0.05,
        &["heading", "font"],
    );
    f32m(
        reg,
        "ui.typography.body_mul",
        "General/UI/Typography",
        "Body Mul",
        "Body size multiplier",
        d.font_body_mul,
        0.5,
        3.0,
        0.05,
        &["body", "font"],
    );
    f32m(
        reg,
        "ui.typography.button_mul",
        "General/UI/Typography",
        "Button Mul",
        "Button text size multiplier",
        d.font_button_mul,
        0.5,
        3.0,
        0.05,
        &["button", "font"],
    );
    f32m(
        reg,
        "ui.typography.small_mul",
        "General/UI/Typography",
        "Small Mul",
        "Small text size multiplier",
        d.font_small_mul,
        0.5,
        3.0,
        0.05,
        &["small", "font"],
    );
    f32m(
        reg,
        "ui.typography.monospace_mul",
        "General/UI/Typography",
        "Monospace Mul",
        "Monospace size multiplier",
        d.font_monospace_mul,
        0.5,
        3.0,
        0.05,
        &["mono", "code", "font"],
    );

    f32m(
        reg,
        "ui.density",
        "General/UI/Spacing",
        "Global Density",
        "Multiply all spacing",
        d.density,
        0.5,
        2.5,
        0.05,
        &["spacing", "density"],
    );
    v2m(
        reg,
        "ui.spacing.item_mul",
        "General/UI/Spacing",
        "Item Spacing Mul",
        "Multiply item_spacing (x,y)",
        d.spacing_item_mul,
        0.25,
        3.0,
        0.05,
        &["spacing", "item"],
    );
    v2m(
        reg,
        "ui.spacing.button_mul",
        "General/UI/Spacing",
        "Button Padding Mul",
        "Multiply button_padding (x,y)",
        d.spacing_button_mul,
        0.25,
        3.0,
        0.05,
        &["padding", "button"],
    );
    v2m(
        reg,
        "ui.spacing.interact_mul",
        "General/UI/Spacing",
        "Interact Size Mul",
        "Multiply interact_size (x,y)",
        d.spacing_interact_mul,
        0.25,
        3.0,
        0.05,
        &["interact", "hit"],
    );
    f32m(
        reg,
        "ui.spacing.window_margin_mul",
        "General/UI/Spacing/Frames",
        "Window Margin Mul",
        "Multiply window_margin",
        d.spacing_window_margin_mul,
        0.25,
        3.0,
        0.05,
        &["margin", "window"],
    );
    f32m(
        reg,
        "ui.spacing.menu_margin_mul",
        "General/UI/Spacing/Frames",
        "Menu Margin Mul",
        "Multiply menu_margin",
        d.spacing_menu_margin_mul,
        0.25,
        3.0,
        0.05,
        &["margin", "menu"],
    );
    f32m(
        reg,
        "ui.spacing.indent_mul",
        "General/UI/Spacing",
        "Indent Mul",
        "Multiply indent",
        d.spacing_indent_mul,
        0.25,
        3.0,
        0.05,
        &["indent"],
    );
    f32m(
        reg,
        "ui.spacing.slider_width_mul",
        "General/UI/Spacing",
        "Slider Width Mul",
        "Multiply slider_width",
        d.spacing_slider_width_mul,
        0.25,
        3.0,
        0.05,
        &["slider", "width"],
    );
    f32m(
        reg,
        "ui.spacing.slider_rail_height_mul",
        "General/UI/Spacing/Widgets",
        "Slider Rail Height Mul",
        "Multiply slider_rail_height",
        d.spacing_slider_rail_height_mul,
        0.25,
        3.0,
        0.05,
        &["slider", "rail"],
    );
    f32m(
        reg,
        "ui.spacing.combo_width_mul",
        "General/UI/Spacing/Widgets",
        "Combo Width Mul",
        "Multiply combo_width",
        d.spacing_combo_width_mul,
        0.25,
        3.0,
        0.05,
        &["combo", "width"],
    );
    f32m(
        reg,
        "ui.spacing.text_edit_width_mul",
        "General/UI/Spacing/Widgets",
        "TextEdit Width Mul",
        "Multiply text_edit_width",
        d.spacing_text_edit_width_mul,
        0.25,
        3.0,
        0.05,
        &["textedit", "width"],
    );
    f32m(
        reg,
        "ui.spacing.icon_width_mul",
        "General/UI/Spacing/Icons",
        "Icon Width Mul",
        "Multiply icon_width",
        d.spacing_icon_width_mul,
        0.25,
        3.0,
        0.05,
        &["icon", "checkbox"],
    );
    f32m(
        reg,
        "ui.spacing.icon_width_inner_mul",
        "General/UI/Spacing/Icons",
        "Icon Inner Mul",
        "Multiply icon_width_inner",
        d.spacing_icon_width_inner_mul,
        0.25,
        3.0,
        0.05,
        &["icon", "inner"],
    );
    f32m(
        reg,
        "ui.spacing.icon_spacing_mul",
        "General/UI/Spacing/Icons",
        "Icon Spacing Mul",
        "Multiply icon_spacing",
        d.spacing_icon_spacing_mul,
        0.25,
        3.0,
        0.05,
        &["icon", "spacing"],
    );
    f32m(
        reg,
        "ui.spacing.tooltip_width_mul",
        "General/UI/Spacing/Menus",
        "Tooltip Width Mul",
        "Multiply tooltip_width",
        d.spacing_tooltip_width_mul,
        0.25,
        3.0,
        0.05,
        &["tooltip", "width"],
    );
    f32m(
        reg,
        "ui.spacing.menu_width_mul",
        "General/UI/Spacing/Menus",
        "Menu Width Mul",
        "Multiply menu_width",
        d.spacing_menu_width_mul,
        0.25,
        3.0,
        0.05,
        &["menu", "width"],
    );
    f32m(
        reg,
        "ui.spacing.menu_spacing_mul",
        "General/UI/Spacing/Menus",
        "Menu Spacing Mul",
        "Multiply menu_spacing",
        d.spacing_menu_spacing_mul,
        0.25,
        3.0,
        0.05,
        &["menu", "spacing"],
    );
    f32m(
        reg,
        "ui.spacing.combo_height_mul",
        "General/UI/Spacing/Menus",
        "Combo Height Mul",
        "Multiply combo_height",
        d.spacing_combo_height_mul,
        0.25,
        3.0,
        0.05,
        &["combo", "height"],
    );

    b(
        reg,
        "ui.scroll.floating",
        "General/UI/Scroll",
        "Floating",
        "Floating scrollbars",
        d.scroll_floating,
        &["scroll", "floating"],
    );
    f32m(
        reg,
        "ui.scroll.bar_width",
        "General/UI/Scroll",
        "Bar Width",
        "Scrollbar width",
        d.scroll_bar_width,
        0.0,
        64.0,
        0.5,
        &["scroll", "width"],
    );
    f32m(
        reg,
        "ui.scroll.handle_min_length",
        "General/UI/Scroll",
        "Handle Min",
        "Minimum handle length",
        d.scroll_handle_min_length,
        0.0,
        128.0,
        0.5,
        &["scroll", "handle"],
    );
    f32m(
        reg,
        "ui.scroll.bar_inner_margin",
        "General/UI/Scroll",
        "Inner Margin",
        "Margin between content and bar",
        d.scroll_bar_inner_margin,
        0.0,
        64.0,
        0.5,
        &["scroll", "margin"],
    );
    f32m(
        reg,
        "ui.scroll.bar_outer_margin",
        "General/UI/Scroll",
        "Outer Margin",
        "Margin between bar and container",
        d.scroll_bar_outer_margin,
        0.0,
        64.0,
        0.5,
        &["scroll", "margin"],
    );
    f32m(
        reg,
        "ui.scroll.floating_width",
        "General/UI/Scroll",
        "Floating Width",
        "Thin width when not hovered",
        d.scroll_floating_width,
        0.0,
        64.0,
        0.5,
        &["scroll", "floating"],
    );
    f32m(
        reg,
        "ui.scroll.floating_allocated_width",
        "General/UI/Scroll",
        "Floating Alloc",
        "Allocated width for floating bar",
        d.scroll_floating_allocated_width,
        0.0,
        64.0,
        0.5,
        &["scroll", "alloc"],
    );
    b(
        reg,
        "ui.scroll.foreground_color",
        "General/UI/Scroll",
        "Foreground Color",
        "High-contrast scroll colors",
        d.scroll_foreground_color,
        &["scroll", "contrast"],
    );
    f32m(
        reg,
        "ui.scroll.dormant_background_opacity",
        "General/UI/Scroll/Opacity",
        "Dormant BG",
        "Background opacity (dormant)",
        d.scroll_dormant_background_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );
    f32m(
        reg,
        "ui.scroll.active_background_opacity",
        "General/UI/Scroll/Opacity",
        "Active BG",
        "Background opacity (active)",
        d.scroll_active_background_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );
    f32m(
        reg,
        "ui.scroll.interact_background_opacity",
        "General/UI/Scroll/Opacity",
        "Interact BG",
        "Background opacity (interact)",
        d.scroll_interact_background_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );
    f32m(
        reg,
        "ui.scroll.dormant_handle_opacity",
        "General/UI/Scroll/Opacity",
        "Dormant Handle",
        "Handle opacity (dormant)",
        d.scroll_dormant_handle_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );
    f32m(
        reg,
        "ui.scroll.active_handle_opacity",
        "General/UI/Scroll/Opacity",
        "Active Handle",
        "Handle opacity (active)",
        d.scroll_active_handle_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );
    f32m(
        reg,
        "ui.scroll.interact_handle_opacity",
        "General/UI/Scroll/Opacity",
        "Interact Handle",
        "Handle opacity (interact)",
        d.scroll_interact_handle_opacity,
        0.0,
        1.0,
        0.01,
        &["scroll", "opacity"],
    );

    f32m(
        reg,
        "ui.interaction.interact_radius",
        "General/UI/Interaction",
        "Interact Radius",
        "Easier hit-testing (touch)",
        d.interaction_interact_radius,
        0.0,
        32.0,
        0.25,
        &["interaction", "touch"],
    );
    f32m(
        reg,
        "ui.interaction.resize_grab_radius_side",
        "General/UI/Interaction",
        "Resize Side",
        "Window resize grab radius (side)",
        d.interaction_resize_grab_radius_side,
        0.0,
        32.0,
        0.25,
        &["resize", "window"],
    );
    f32m(
        reg,
        "ui.interaction.resize_grab_radius_corner",
        "General/UI/Interaction",
        "Resize Corner",
        "Window resize grab radius (corner)",
        d.interaction_resize_grab_radius_corner,
        0.0,
        32.0,
        0.25,
        &["resize", "window"],
    );
    b(
        reg,
        "ui.interaction.show_tooltips_only_when_still",
        "General/UI/Interaction/Tooltips",
        "Only When Still",
        "Only show tooltip when mouse stops",
        d.interaction_show_tooltips_only_when_still,
        &["tooltip"],
    );
    f32m(
        reg,
        "ui.interaction.tooltip_delay",
        "General/UI/Interaction/Tooltips",
        "Tooltip Delay",
        "Delay before tooltip shows",
        d.interaction_tooltip_delay,
        0.0,
        3.0,
        0.01,
        &["tooltip", "delay"],
    );
    b(
        reg,
        "ui.interaction.selectable_labels",
        "General/UI/Interaction/Text",
        "Selectable Labels",
        "Text selection in labels",
        d.interaction_selectable_labels,
        &["text", "select"],
    );
    b(
        reg,
        "ui.interaction.multi_widget_text_select",
        "General/UI/Interaction/Text",
        "Multi Widget Select",
        "Selection across multiple labels",
        d.interaction_multi_widget_text_select,
        &["text", "select"],
    );

    f32m(
        reg,
        "ui.stroke.window",
        "General/UI/Stroke",
        "Window Stroke",
        "Window stroke width",
        d.stroke_window,
        0.0,
        4.0,
        0.1,
        &["stroke", "border"],
    );
    f32m(
        reg,
        "ui.stroke.widget",
        "General/UI/Stroke",
        "Widget Stroke",
        "Widget stroke width",
        d.stroke_widget,
        0.0,
        4.0,
        0.1,
        &["stroke", "border"],
    );
    f32m(
        reg,
        "ui.stroke.selection",
        "General/UI/Stroke",
        "Selection Stroke",
        "Selection stroke width",
        d.stroke_selection,
        0.0,
        4.0,
        0.1,
        &["selection", "stroke"],
    );

    col(
        reg,
        "ui.colors.accent",
        "General/UI/Colors/Accent",
        "Accent",
        "Accent color (borders/highlights)",
        d.color_accent,
        &["accent", "primary"],
    );
    col(
        reg,
        "ui.colors.selection_bg",
        "General/UI/Colors/Selection",
        "Selection BG",
        "Selection background",
        d.color_selection_bg,
        &["selection", "bg"],
    );
    col(
        reg,
        "ui.colors.selection_stroke",
        "General/UI/Colors/Selection",
        "Selection Stroke",
        "Selection outline",
        d.color_selection_stroke,
        &["selection", "stroke"],
    );
    col(
        reg,
        "ui.colors.hyperlink",
        "General/UI/Colors/Feedback",
        "Hyperlink",
        "Hyperlink color",
        d.color_hyperlink,
        &["link"],
    );
    col(
        reg,
        "ui.colors.warn",
        "General/UI/Colors/Feedback",
        "Warning",
        "Warning text color",
        d.color_warn,
        &["warn"],
    );
    col(
        reg,
        "ui.colors.error",
        "General/UI/Colors/Feedback",
        "Error",
        "Error text color",
        d.color_error,
        &["error"],
    );
    b(
        reg,
        "ui.colors.override_text",
        "General/UI/Colors/Text",
        "Override Text",
        "Use custom text color",
        d.color_override_text,
        &["text"],
    );
    col(
        reg,
        "ui.colors.text",
        "General/UI/Colors/Text",
        "Text Color",
        "Override text color",
        d.color_text,
        &["text"],
    );

    f32m(
        reg,
        "ui.shadow.blur",
        "General/UI/Shadow",
        "Blur",
        "Shadow blur",
        d.shadow_blur,
        0.0,
        256.0,
        0.5,
        &["shadow", "blur"],
    );
    f32m(
        reg,
        "ui.shadow.spread",
        "General/UI/Shadow",
        "Spread",
        "Shadow spread",
        d.shadow_spread,
        -128.0,
        128.0,
        0.5,
        &["shadow", "spread"],
    );
    f32m(
        reg,
        "ui.shadow.alpha",
        "General/UI/Shadow",
        "Alpha",
        "Shadow alpha",
        d.shadow_alpha,
        0.0,
        1.0,
        0.01,
        &["shadow", "opacity"],
    );
    v2m(
        reg,
        "ui.shadow.offset",
        "General/UI/Shadow",
        "Offset",
        "Shadow offset (x,y)",
        d.shadow_offset,
        -256.0,
        256.0,
        0.5,
        &["shadow", "offset"],
    );
}

crate::register_settings_provider!("ui", register_ui_settings);
