pub use crate::settings::node_editor_settings::*;

use crate::settings::{
    SettingMeta, SettingScope, SettingValue, SettingsMerge, SettingsRegistry, SettingsStores,
};
use bevy::prelude::*;

#[derive(Resource, Clone)]
pub struct NodeEditorSettings {
    pub node_default_size: [f32; 2],
    pub node_min_size: [f32; 2],
    pub node_header_h_base: f32,
    pub node_io_inset_x_base: f32,
    pub node_io_inset_y_base: f32,
    pub node_io_panel_alpha: f32,
    pub node_auto_max_w_base: f32,
    pub port_radius_base: f32,
    pub port_offset_base: f32,
    pub port_hover_radius_mul: f32,
    pub port_stroke_width_base: f32,
    pub layout_sidebar_width_ratio: f32,
    pub layout_bar_width_ratio: f32,
    pub layout_title_text_offset_x: f32,
    pub style_node_rounding_base: f32,
    pub style_node_border_width_base: f32,
    pub network_box_fill_alpha: f32,
    pub aux_selected_border_width_mul: f32,
    pub grid_size_base: f32,
    pub grid_line_width_base: f32,
    pub grid_color: [u8; 4],
    pub grid_major_ratio: f32,
    pub grid_major_line_width_mul: f32,
    pub grid_major_alpha_mul: f32,
    pub radial_inner_padding_base: f32,
    pub radial_inner_padding_min: f32,
    pub radial_inner_padding_ratio: f32,
    pub radial_thickness_base: f32,
    pub radial_thickness_min: f32,
    pub radial_thickness_ratio: f32,
    pub radial_gap_factor: f32,
    pub radial_gap_min: f32,
    pub radial_activation_margin: f32,
    pub title_text_lod_hide_px: f32,
    pub title_text_lod_fade_px: f32,

    pub cda_io_gap_x: f32,
    pub cda_io_margin_y: f32,

    /// Cook visualization on wires (SDF flow).
    pub wire_cook_scope_dim_alpha: f32, // 0..1
    pub wire_cook_speed: [f32; 5],      // NodeCookState index
    pub wire_cook_flow_k: [f32; 5],     // 0..1
    pub wire_cook_blink_hz: [f32; 5],   // 0..inf
    pub wire_cook_thick_mul: [f32; 5],  // >= 0
    pub wire_cook_softness: [f32; 5],   // >= 0 (AA width)
    pub wire_cook_pulse_w: [f32; 5],    // 0..0.5
    pub wire_cook_spacing: [f32; 5],    // 0.02..inf (normalized)
    pub wire_cook_selected_flow_k_min: f32,
    pub wire_cook_selected_thick_mul_min: f32,

    pub node_cook_border_width_mul: f32,
    pub node_cook_border_expand_px: f32,
    pub node_cook_border_speed_mul: f32,
    pub node_cook_border_flow_k_mul: f32,

    /// AI auto-layout (engine-side, does not rely on model formatting).
    pub ai_layout_gap_x: f32,
    pub ai_layout_gap_y: f32,
    pub ai_layout_max_cols: u32,
    pub ai_layout_box_pad: f32,
    pub ai_layout_avoid_pad: f32,
    pub ai_layout_box_rgba: [u8; 4],
    pub ai_layout_sticky_rgba: [u8; 4],
    pub ai_layout_sticky_min_w: f32,
    pub ai_layout_sticky_max_w: f32,
    pub ai_layout_sticky_min_h: f32,
    pub ai_layout_sticky_line_h: f32,
    pub ai_layout_sticky_wrap_cols: u32,
}

impl Default for NodeEditorSettings {
    fn default() -> Self {
        Self {
            node_default_size: [200.0, 92.0],
            node_min_size: [170.0, 80.0],
            node_header_h_base: 26.0,
            node_io_inset_x_base: 1.0,
            node_io_inset_y_base: 1.0,
            node_io_panel_alpha: 0.85,
            node_auto_max_w_base: 260.0,
            port_radius_base: 5.0,
            port_offset_base: 7.5,
            port_hover_radius_mul: 3.5,
            port_stroke_width_base: 1.5,
            layout_sidebar_width_ratio: 0.15,
            layout_bar_width_ratio: 0.8,
            layout_title_text_offset_x: 10.0,
            style_node_rounding_base: 4.0,
            style_node_border_width_base: 2.0,
            network_box_fill_alpha: 0.5,
            aux_selected_border_width_mul: 1.8,
            grid_size_base: 50.0,
            grid_line_width_base: 1.0,
            grid_color: [80, 80, 80, 255],
            grid_major_ratio: 5.0,
            grid_major_line_width_mul: 2.0,
            grid_major_alpha_mul: 1.5,
            radial_inner_padding_base: 12.0,
            radial_inner_padding_min: 10.0,
            radial_inner_padding_ratio: 0.03,
            radial_thickness_base: 30.0,
            radial_thickness_min: 20.0,
            radial_thickness_ratio: 0.05,
            radial_gap_factor: 1.85,
            radial_gap_min: 5.0,
            radial_activation_margin: 5.0,
            title_text_lod_hide_px: 5.0,
            title_text_lod_fade_px: 4.0,

            cda_io_gap_x: 50.0,
            cda_io_margin_y: 200.0,

            wire_cook_scope_dim_alpha: 0.22,
            // Index: 0 Idle, 1 Queued, 2 Running, 3 Blocked, 4 Failed
            wire_cook_speed: [0.0, 0.65, 2.25, 0.0, 0.0],
            wire_cook_flow_k: [0.0, 0.75, 1.0, 0.0, 0.0],
            wire_cook_blink_hz: [0.0, 0.0, 0.0, 2.2, 3.0],
            wire_cook_thick_mul: [1.0, 1.05, 1.15, 1.0, 1.0],
            wire_cook_softness: [1.0, 1.0, 1.0, 1.0, 1.8],
            wire_cook_pulse_w: [0.0, 0.12, 0.10, 0.0, 0.0],
            wire_cook_spacing: [0.10, 0.10, 0.075, 0.10, 0.10],
            wire_cook_selected_flow_k_min: 0.85,
            wire_cook_selected_thick_mul_min: 1.2,

            node_cook_border_width_mul: 1.35,
            node_cook_border_expand_px: 5.0,
            node_cook_border_speed_mul: 1.0,
            node_cook_border_flow_k_mul: 1.0,

            ai_layout_gap_x: 80.0,
            ai_layout_gap_y: 70.0,
            ai_layout_max_cols: 4,
            ai_layout_box_pad: 46.0,
            ai_layout_avoid_pad: 26.0,
            ai_layout_box_rgba: [80, 120, 255, 26],
            ai_layout_sticky_rgba: [255, 243, 138, 255],
            ai_layout_sticky_min_w: 260.0,
            ai_layout_sticky_max_w: 620.0,
            ai_layout_sticky_min_h: 120.0,
            ai_layout_sticky_line_h: 16.0,
            ai_layout_sticky_wrap_cols: 72,
        }
    }
}

const HARD_MIN_NODE_SIZE: [f32; 2] = [160.0, 80.0];

pub fn resolved_node_size(s: &NodeEditorSettings) -> [f32; 2] {
    let ds = s.node_default_size;
    let ms = s.node_min_size;
    [
        ds[0].max(ms[0]).max(HARD_MIN_NODE_SIZE[0]).max(0.0),
        ds[1].max(ms[1]).max(HARD_MIN_NODE_SIZE[1]).max(0.0),
    ]
}

pub fn apply_from_settings(
    reg: &SettingsRegistry,
    stores: &SettingsStores,
    s: &mut NodeEditorSettings,
) {
    let get = |id: &str| {
        reg.get(id).and_then(|m| {
            Some(SettingsMerge::resolve(m, stores.project.get(id), stores.user.get(id)).1)
        })
    };
    if let Some(SettingValue::Vec2(v)) = get("node_editor.node.default_size") {
        s.node_default_size = v;
    }
    if let Some(SettingValue::Vec2(v)) = get("node_editor.node.min_size") {
        s.node_min_size = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.node.header_h_base") {
        s.node_header_h_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.node.io_inset_x_base") {
        s.node_io_inset_x_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.node.io_inset_y_base") {
        s.node_io_inset_y_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.node.io_panel_alpha") {
        s.node_io_panel_alpha = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.node.auto_max_w_base") {
        s.node_auto_max_w_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.ports.radius_base") {
        s.port_radius_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.ports.offset_base") {
        s.port_offset_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.ports.hover_radius_mul") {
        s.port_hover_radius_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.ports.stroke_width_base") {
        s.port_stroke_width_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.layout.sidebar_width_ratio") {
        s.layout_sidebar_width_ratio = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.layout.bar_width_ratio") {
        s.layout_bar_width_ratio = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.layout.title_text_offset_x") {
        s.layout_title_text_offset_x = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.style.rounding_base") {
        s.style_node_rounding_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.style.border_width_base") {
        s.style_node_border_width_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.grid.size_base") {
        s.grid_size_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.grid.line_width_base") {
        s.grid_line_width_base = v;
    }
    if let Some(SettingValue::Color32(v)) = get("node_editor.grid.color") {
        s.grid_color = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.grid.major_ratio") {
        s.grid_major_ratio = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.grid.major_line_width_mul") {
        s.grid_major_line_width_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.grid.major_alpha_mul") {
        s.grid_major_alpha_mul = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.inner_padding_base") {
        s.radial_inner_padding_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.inner_padding_min") {
        s.radial_inner_padding_min = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.inner_padding_ratio") {
        s.radial_inner_padding_ratio = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.thickness_base") {
        s.radial_thickness_base = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.thickness_min") {
        s.radial_thickness_min = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.thickness_ratio") {
        s.radial_thickness_ratio = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.gap_factor") {
        s.radial_gap_factor = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.gap_min") {
        s.radial_gap_min = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.menus.radial.activation_margin") {
        s.radial_activation_margin = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.typography.title_lod_hide_px") {
        s.title_text_lod_hide_px = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.nodes.typography.title_lod_fade_px") {
        s.title_text_lod_fade_px = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.cda.layout.io_gap_x") {
        s.cda_io_gap_x = v;
    }
    if let Some(SettingValue::F32(v)) = get("node_editor.cda.layout.io_margin_y") {
        s.cda_io_margin_y = v;
    }
}

pub fn sync_from_settings_stores(
    reg: Res<SettingsRegistry>,
    stores: Res<SettingsStores>,
    mut s: ResMut<NodeEditorSettings>,
) {
    if !(reg.is_changed() || stores.is_changed() || s.is_added()) {
        return;
    }
    apply_from_settings(&reg, &stores, &mut s);
}

pub fn apply_to_all_nodes(
    mut node_graph_res: ResMut<crate::NodeGraphResource>,
    s: Res<NodeEditorSettings>,
    mut last_len: Local<usize>,
    mut last_size: Local<[f32; 2]>,
) {
    let len = node_graph_res.0.nodes.len();
    let size = resolved_node_size(&s);
    if *last_len == len && *last_size == size {
        return;
    }
    node_graph_res.0.graph_revision = node_graph_res.0.graph_revision.wrapping_add(1); // UI/layout change; forces cached snapshot refresh.
    *last_len = len;
    *last_size = size;
}

fn register_node_editor_settings(reg: &mut SettingsRegistry) {
    let d = NodeEditorSettings::default();
    reg.upsert(SettingMeta {
        id: "node_editor.node.default_size".into(),
        path: "Node Editor/Nodes".into(),
        label: "Default Size".into(),
        help: "Default node size (w,h) for new nodes".into(),
        scope: SettingScope::User,
        default: SettingValue::Vec2(d.node_default_size),
        min: Some(40.0),
        max: Some(2000.0),
        step: Some(1.0),
        keywords: vec![
            "node".into(),
            "size".into(),
            "width".into(),
            "height".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.min_size".into(),
        path: "Node Editor/Nodes".into(),
        label: "Min Size".into(),
        help: "Minimum node size clamp (w,h)".into(),
        scope: SettingScope::User,
        default: SettingValue::Vec2(d.node_min_size),
        min: Some(0.0),
        max: Some(2000.0),
        step: Some(1.0),
        keywords: vec!["node".into(), "min".into(), "size".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.header_h_base".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "Header Height Base".into(),
        help: "Node header height (px) for title area".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.node_header_h_base),
        min: Some(10.0),
        max: Some(120.0),
        step: Some(1.0),
        keywords: vec![
            "node".into(),
            "header".into(),
            "title".into(),
            "height".into(),
        ],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.io_inset_x_base".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "IO Inset X Base".into(),
        help: "Inner IO panel horizontal inset (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.node_io_inset_x_base),
        min: Some(0.0),
        max: Some(80.0),
        step: Some(0.5),
        keywords: vec!["node".into(), "io".into(), "inset".into(), "x".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.io_inset_y_base".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "IO Inset Y Base".into(),
        help: "Inner IO panel bottom inset (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.node_io_inset_y_base),
        min: Some(0.0),
        max: Some(80.0),
        step: Some(0.5),
        keywords: vec!["node".into(), "io".into(), "inset".into(), "y".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.io_panel_alpha".into(),
        path: "Node Editor/Nodes/Style".into(),
        label: "IO Panel Alpha".into(),
        help: "Inner IO panel alpha (0..1)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.node_io_panel_alpha),
        min: Some(0.0),
        max: Some(1.0),
        step: Some(0.01),
        keywords: vec!["node".into(), "io".into(), "panel".into(), "alpha".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.node.auto_max_w_base".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "Auto Max Width Base".into(),
        help: "Auto-sized node max width (px). Labels will ellipsize beyond this.".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.node_auto_max_w_base),
        min: Some(160.0),
        max: Some(1200.0),
        step: Some(1.0),
        keywords: vec!["node".into(), "auto".into(), "width".into(), "max".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.ports.radius_base".into(),
        path: "Node Editor/Nodes/Ports".into(),
        label: "Port Radius Base".into(),
        help: "Port circle radius base (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.port_radius_base),
        min: Some(0.5),
        max: Some(64.0),
        step: Some(0.25),
        keywords: vec!["port".into(), "radius".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.ports.offset_base".into(),
        path: "Node Editor/Nodes/Ports".into(),
        label: "Port Offset Base".into(),
        help: "Port offset from node body (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.port_offset_base),
        min: Some(0.0),
        max: Some(256.0),
        step: Some(0.25),
        keywords: vec!["port".into(), "offset".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.ports.hover_radius_mul".into(),
        path: "Node Editor/Nodes/Ports".into(),
        label: "Port Hover Radius Mul".into(),
        help: "Hover detection radius multiplier".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.port_hover_radius_mul),
        min: Some(1.0),
        max: Some(16.0),
        step: Some(0.1),
        keywords: vec!["port".into(), "hover".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.ports.stroke_width_base".into(),
        path: "Node Editor/Nodes/Ports".into(),
        label: "Port Stroke Width Base".into(),
        help: "Port stroke width base (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.port_stroke_width_base),
        min: Some(0.0),
        max: Some(16.0),
        step: Some(0.1),
        keywords: vec!["port".into(), "stroke".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.layout.sidebar_width_ratio".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "Sidebar Width Ratio".into(),
        help: "Node side strip width = node_width * ratio".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.layout_sidebar_width_ratio),
        min: Some(0.0),
        max: Some(0.5),
        step: Some(0.01),
        keywords: vec!["node".into(), "sidebar".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.layout.bar_width_ratio".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "Bar Width Ratio".into(),
        help: "Node bar width = node_width * ratio".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.layout_bar_width_ratio),
        min: Some(0.1),
        max: Some(1.0),
        step: Some(0.01),
        keywords: vec!["node".into(), "bar".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.layout.title_text_offset_x".into(),
        path: "Node Editor/Nodes/Layout".into(),
        label: "Title Text Offset X".into(),
        help: "Title text left padding (in px, scaled by zoom)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.layout_title_text_offset_x),
        min: Some(0.0),
        max: Some(128.0),
        step: Some(0.25),
        keywords: vec!["title".into(), "text".into(), "padding".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.style.rounding_base".into(),
        path: "Node Editor/Nodes/Style".into(),
        label: "Rounding Base".into(),
        help: "Node rounding base (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.style_node_rounding_base),
        min: Some(0.0),
        max: Some(64.0),
        step: Some(0.25),
        keywords: vec!["rounding".into(), "corner".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.style.border_width_base".into(),
        path: "Node Editor/Nodes/Style".into(),
        label: "Border Width Base".into(),
        help: "Node border width base (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.style_node_border_width_base),
        min: Some(0.0),
        max: Some(16.0),
        step: Some(0.1),
        keywords: vec!["border".into(), "stroke".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.size_base".into(),
        path: "Node Editor/Grid".into(),
        label: "Grid Size Base".into(),
        help: "Grid cell size base (scaled by zoom)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.grid_size_base),
        min: Some(1.0),
        max: Some(512.0),
        step: Some(1.0),
        keywords: vec!["grid".into(), "size".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.line_width_base".into(),
        path: "Node Editor/Grid".into(),
        label: "Line Width Base".into(),
        help: "Grid line width base (scaled by sqrt(zoom))".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.grid_line_width_base),
        min: Some(0.0),
        max: Some(16.0),
        step: Some(0.1),
        keywords: vec!["grid".into(), "width".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.color".into(),
        path: "Node Editor/Grid".into(),
        label: "Color".into(),
        help: "Grid color".into(),
        scope: SettingScope::User,
        default: SettingValue::Color32(d.grid_color),
        min: None,
        max: None,
        step: None,
        keywords: vec!["grid".into(), "color".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.major_ratio".into(),
        path: "Node Editor/Grid".into(),
        label: "Major Ratio".into(),
        help: "Major grid step = minor * ratio".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.grid_major_ratio),
        min: Some(1.0),
        max: Some(32.0),
        step: Some(1.0),
        keywords: vec!["grid".into(), "major".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.major_line_width_mul".into(),
        path: "Node Editor/Grid".into(),
        label: "Major Width Mul".into(),
        help: "Major line width multiplier".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.grid_major_line_width_mul),
        min: Some(0.0),
        max: Some(8.0),
        step: Some(0.1),
        keywords: vec!["grid".into(), "major".into(), "width".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.grid.major_alpha_mul".into(),
        path: "Node Editor/Grid".into(),
        label: "Major Alpha Mul".into(),
        help: "Major line alpha multiplier".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.grid_major_alpha_mul),
        min: Some(0.0),
        max: Some(8.0),
        step: Some(0.1),
        keywords: vec!["grid".into(), "major".into(), "alpha".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.inner_padding_base".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Inner Padding Base".into(),
        help: "Inner padding base (scaled by zoom)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_inner_padding_base),
        min: Some(0.0),
        max: Some(128.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "padding".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.inner_padding_min".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Inner Padding Min".into(),
        help: "Inner padding minimum (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_inner_padding_min),
        min: Some(0.0),
        max: Some(128.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "padding".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.inner_padding_ratio".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Inner Padding Ratio".into(),
        help: "Inner padding = base + ratio * max(node_dim)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_inner_padding_ratio),
        min: Some(0.0),
        max: Some(0.5),
        step: Some(0.005),
        keywords: vec!["radial".into(), "padding".into(), "ratio".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.thickness_base".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Thickness Base".into(),
        help: "Ring thickness base (scaled by zoom)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_thickness_base),
        min: Some(0.0),
        max: Some(256.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "thickness".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.thickness_min".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Thickness Min".into(),
        help: "Ring thickness minimum (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_thickness_min),
        min: Some(0.0),
        max: Some(256.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "thickness".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.thickness_ratio".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Thickness Ratio".into(),
        help: "Thickness = base + ratio * max(node_dim)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_thickness_ratio),
        min: Some(0.0),
        max: Some(0.5),
        step: Some(0.005),
        keywords: vec!["radial".into(), "thickness".into(), "ratio".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.gap_factor".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Gap Factor".into(),
        help: "Gap = thickness * factor".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_gap_factor),
        min: Some(0.0),
        max: Some(8.0),
        step: Some(0.05),
        keywords: vec!["radial".into(), "gap".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.gap_min".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Gap Min".into(),
        help: "Gap minimum (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_gap_min),
        min: Some(0.0),
        max: Some(64.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "gap".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.menus.radial.activation_margin".into(),
        path: "Node Editor/Menus/Radial".into(),
        label: "Activation Margin".into(),
        help: "Extra margin for hover keep-open (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.radial_activation_margin),
        min: Some(0.0),
        max: Some(64.0),
        step: Some(0.5),
        keywords: vec!["radial".into(), "hover".into()],
    });

    reg.upsert(SettingMeta {
        id: "node_editor.nodes.typography.title_lod_hide_px".into(),
        path: "Node Editor/Nodes/Typography".into(),
        label: "Title LOD Hide (px)".into(),
        help: "When title font_px <= this, hide/fade out title text during zoom-out".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.title_text_lod_hide_px),
        min: Some(0.0),
        max: Some(64.0),
        step: Some(0.25),
        keywords: vec!["title".into(), "text".into(), "lod".into(), "hide".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.nodes.typography.title_lod_fade_px".into(),
        path: "Node Editor/Nodes/Typography".into(),
        label: "Title LOD Fade Range (px)".into(),
        help: "Opacity ramps from 0..1 over this px range above the hide threshold".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.title_text_lod_fade_px),
        min: Some(0.01),
        max: Some(64.0),
        step: Some(0.25),
        keywords: vec!["title".into(), "text".into(), "lod".into(), "fade".into()],
    });

    reg.upsert(SettingMeta {
        id: "node_editor.cda.layout.io_gap_x".into(),
        path: "Node Editor/CDA/Layout".into(),
        label: "IO Gap X".into(),
        help: "Horizontal spacing between generated CDA IO nodes (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.cda_io_gap_x),
        min: Some(0.0),
        max: Some(2000.0),
        step: Some(1.0),
        keywords: vec!["cda".into(), "io".into(), "gap".into(), "layout".into()],
    });
    reg.upsert(SettingMeta {
        id: "node_editor.cda.layout.io_margin_y".into(),
        path: "Node Editor/CDA/Layout".into(),
        label: "IO Margin Y".into(),
        help: "Vertical distance from AABB to generated CDA IO nodes (px)".into(),
        scope: SettingScope::User,
        default: SettingValue::F32(d.cda_io_margin_y),
        min: Some(0.0),
        max: Some(5000.0),
        step: Some(1.0),
        keywords: vec!["cda".into(), "io".into(), "margin".into(), "layout".into()],
    });
}

crate::register_settings_provider!("node_editor", register_node_editor_settings);
