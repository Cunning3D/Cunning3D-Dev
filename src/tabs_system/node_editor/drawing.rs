use crate::{
    nodes::{InputStyle, Node, NodeStyle},
    tabs_system::node_editor::state::NodeInteraction,
    theme::ModernTheme,
};
use bevy_egui::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};

use crate::cunning_core::command::basic::CmdInsertNodeOnConnection;
use crate::cunning_core::command::basic::CmdSetConnectionWaypoints;
use crate::cunning_core::command::basic::{CmdBatch, CmdMoveNodes, CmdSetNetworkBoxRect, CmdSetStickyNoteRect};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::NodeId;
use crate::nodes::NodeGraph;
use crate::tabs_system::node_editor::interactions::handle_node_insertion;
use crate::tabs_system::node_editor::mathematic::{
    check_and_apply_snap, point_to_bezier_distance_sq,
};
use crate::tabs_system::node_editor::state::NodeHit;
use crate::tabs_system::node_editor::state::NodeSnapshot;
use crate::tabs_system::node_editor::{icons, state::NodeEditorTab};
use crate::tabs_system::EditorTabContext;
use std::collections::{HashMap, HashSet, VecDeque};

use egui_wgpu::sdf::{create_sdf_circle_callback, SdfCircleUniform}; // [NEW] SDF Circle (Ports)
use egui_wgpu::sdf::{create_sdf_curve_callback, SdfCurveUniform}; // [NEW] SDF Curve Rendering
use egui_wgpu::sdf::{create_sdf_dashed_curve_callback, SdfDashedCurveUniform};
use egui_wgpu::sdf::{create_sdf_flow_curve_callback, SdfFlowCurveUniform};
use egui_wgpu::sdf::{create_sdf_grid_callback, SdfGridUniform}; // [NEW] SDF Grid
use egui_wgpu::sdf::{create_sdf_rect_callback, SdfRectUniform}; // [NEW] SDF Rendering // [NEW] SDF Dashed Curve
                                                                                       // NOTE: Node Editor text now uses egui native text rendering for consistency.
use crate::invalidator::{RepaintCause, UiInvalidator};

fn mul_alpha(mut rgba: [f32; 4], m: f32) -> [f32; 4] {
    rgba[3] *= m;
    rgba
}

#[inline]
fn measure_text_x(ctx: &egui::Context, font: egui::FontId, text: &str) -> f32 {
    ctx.fonts_mut(|f| {
        f.layout_no_wrap(text.to_string(), font, Color32::WHITE)
            .rect
            .width()
    })
}

#[inline]
fn ellipsize(ctx: &egui::Context, font: egui::FontId, text: &str, max_w: f32) -> String {
    if max_w <= 1.0 || text.is_empty() {
        return String::new();
    }
    if measure_text_x(ctx, font.clone(), text) <= max_w {
        return text.to_string();
    }
    let ell = "…";
    let ell_w = measure_text_x(ctx, font.clone(), ell);
    if ell_w >= max_w {
        return ell.to_string();
    }
    let mut lo = 0usize;
    let mut hi = text.chars().count();
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        let s: String = text.chars().take(mid).collect();
        if measure_text_x(ctx, font.clone(), &(s.clone() + ell)) <= max_w {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let s: String = text.chars().take(lo).collect();
    s + ell
}

#[inline]
fn wrap_two_lines(ctx: &egui::Context, font: egui::FontId, text: &str, max_w: f32) -> (String, u8) {
    if text.is_empty() {
        return (String::new(), 1);
    }
    if measure_text_x(ctx, font.clone(), text) <= max_w {
        return (text.to_string(), 1);
    }
    // Prefer splitting on spaces; fallback to char split.
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() >= 2 {
        let mut best = 1usize;
        for i in 1..words.len() {
            let a = words[..i].join(" ");
            if measure_text_x(ctx, font.clone(), &a) <= max_w {
                best = i;
            } else {
                break;
            }
        }
        let a = words[..best].join(" ");
        let b = words[best..].join(" ");
        let a2 = ellipsize(ctx, font.clone(), &a, max_w);
        let b2 = ellipsize(ctx, font, &b, max_w);
        return (format!("{}\n{}", a2, b2), 2);
    }
    // No spaces: split by chars roughly to fit first line.
    let mut lo = 1usize;
    let mut hi = text.chars().count();
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        let s: String = text.chars().take(mid).collect();
        if measure_text_x(ctx, font.clone(), &s) <= max_w {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let a: String = text.chars().take(lo).collect();
    let b: String = text.chars().skip(lo).collect();
    (
        format!(
            "{}\n{}",
            ellipsize(ctx, font.clone(), &a, max_w),
            ellipsize(ctx, font, &b, max_w)
        ),
        2,
    )
}

#[inline]
fn port_idx(pid: &crate::nodes::PortId, fallback: usize) -> usize {
    pid.as_str()
        .rsplit_once(':')
        .and_then(|(_, t)| t.parse::<usize>().ok())
        .unwrap_or(fallback)
}

pub fn compute_auto_node_size(
    ctx: &egui::Context,
    s: &crate::node_editor_settings::NodeEditorSettings,
    reg: &crate::cunning_core::registries::node_registry::NodeRegistry,
    name: &str,
    inputs: &[crate::nodes::PortId],
    outputs: &[crate::nodes::PortId],
    input_style: crate::nodes::InputStyle,
) -> Vec2 {
    let min = crate::node_editor_settings::resolved_node_size(s);
    let title_font = egui::FontId::proportional(12.0);
    let port_font = egui::FontId::proportional(9.0);
    let pad_x = 14.0;
    let pad_y = 10.0;
    let row_h = 18.0;
    let header_h = s.node_header_h_base.max(10.0);
    let port_pad = s.port_radius_base.max(0.0) + 4.0;
    let desc = reg.get_descriptor(name);

    // Height strictly follows number of ports (programmatic modeling expectation).
    let rows = inputs.len().max(outputs.len()).max(1) as f32;

    let title_w = measure_text_x(ctx, title_font, name);
    let max_w = s.node_auto_max_w_base.max(160.0);

    // Width is primarily driven by title, but must never shrink so far that left/right IO columns overlap.
    // We intentionally do NOT grow width based on long port labels; those ellipsize instead.
    let io_inset_x = s.node_io_inset_x_base.max(0.0);
    let min_io_label = measure_text_x(ctx, port_font.clone(), "Output 0")
        .max(measure_text_x(ctx, port_font, "Input 0"));
    let mid_pad = 6.0;
    let min_half = io_inset_x + port_pad + min_io_label + mid_pad;
    let min_io_w = (min_half * 2.0).max(min[0]);
    let w = (title_w + pad_x * 2.0).max(min_io_w).max(min[0]).min(max_w);
    let io_inset_y = s.node_io_inset_y_base.max(0.0);
    let h = (pad_y * 2.0 + header_h + rows * row_h + io_inset_y)
        .max(min[1])
        .min(900.0);
    Vec2::new(w, h)
}

pub fn compute_auto_node_layout(
    ctx: &egui::Context,
    s: &crate::node_editor_settings::NodeEditorSettings,
    reg: &crate::cunning_core::registries::node_registry::NodeRegistry,
    name: &str,
    inputs: &[crate::nodes::PortId],
    outputs: &[crate::nodes::PortId],
    input_style: crate::nodes::InputStyle,
) -> (Vec2, f32) {
    let min = crate::node_editor_settings::resolved_node_size(s);
    let title_font = egui::FontId::proportional(12.0);
    let port_font = egui::FontId::proportional(9.0);
    let pad_x = 14.0;
    let pad_y = 10.0;
    let row_h = 18.0;
    let port_pad = s.port_radius_base.max(0.0) + 4.0;
    let icon_w = 14.0 + 6.0; // icon + gap

    let rows = inputs.len().max(outputs.len()).max(1) as f32;
    let title_w = measure_text_x(ctx, title_font.clone(), name);
    let max_w = s.node_auto_max_w_base.max(160.0);
    let io_inset_x = s.node_io_inset_x_base.max(0.0);
    let min_io_label = measure_text_x(ctx, port_font.clone(), "Output 0")
        .max(measure_text_x(ctx, port_font, "Input 0"));
    let mid_pad = 6.0;
    let min_half = io_inset_x + port_pad + min_io_label + mid_pad;
    let min_io_w = (min_half * 2.0).max(min[0]);
    let w = (title_w + pad_x * 2.0 + icon_w)
        .max(min_io_w)
        .max(min[0])
        .min(max_w);

    // Two-line title: if it doesn't fit in the computed width, increase header height.
    let avail_title_w = (w - pad_x * 2.0 - icon_w).max(1.0);
    let title_lines = if title_w > avail_title_w { 2.0 } else { 1.0 };
    let title_line_h = 14.0;
    let header_h = (s.node_header_h_base.max(10.0)).max(title_lines * title_line_h + (6.0));

    let io_inset_y = s.node_io_inset_y_base.max(0.0);
    let h = (pad_y * 2.0 + header_h + rows * row_h + io_inset_y)
        .max(min[1])
        .min(900.0);
    (Vec2::new(w, h), header_h)
}

#[inline]
pub(crate) fn autofit_foreach_boxes(g: &mut NodeGraph) {
    #[inline]
    fn block_id_of(n: &Node) -> Option<&str> {
        n.parameters
            .iter()
            .find(|p| p.name == "block_id")
            .and_then(|p| {
                if let ParameterValue::String(s) = &p.value {
                    Some(s.as_str())
                } else {
                    None
                }
            })
    }
    #[inline]
    fn is_begin(n: &Node) -> bool {
        matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Begin")
    }
    #[inline]
    fn is_end(n: &Node) -> bool {
        matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach End")
    }
    #[inline]
    fn is_meta(n: &Node) -> bool {
        matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Meta")
    }

    let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
    for c in g.connections.values() {
        out.entry(c.from_node).or_default().push(c.to_node);
    }

    let pad = Vec2::new(30.0, 30.0);
    let header_h = 28.0;
    for b in g.network_boxes.values_mut() {
        let Some(bid) = b
            .title
            .strip_prefix("ForEach ")
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };

        let mut begin: Option<NodeId> = None;
        let mut end: Option<NodeId> = None;
        let mut meta: Option<NodeId> = None;
        for (id, n) in &g.nodes {
            if block_id_of(n) != Some(bid) {
                continue;
            }
            if begin.is_none() && is_begin(n) {
                begin = Some(*id);
            }
            if end.is_none() && is_end(n) {
                end = Some(*id);
            }
            if meta.is_none() && is_meta(n) {
                meta = Some(*id);
            }
        }
        let (Some(begin), Some(end)) = (begin, end) else {
            continue;
        };

        // Houdini-like: any node reachable from Begin's Loop output is considered "inside",
        // even if it doesn't (yet) connect to End. Traversal stops at End/Meta.
        let mut reach: HashSet<NodeId> = HashSet::new();
        let mut q: VecDeque<NodeId> = VecDeque::new();
        q.push_back(begin);
        while let Some(nid) = q.pop_front() {
            if !reach.insert(nid) {
                continue;
            }
            if nid == end || Some(nid) == meta {
                continue;
            }
            if let Some(v) = out.get(&nid) {
                for &to in v {
                    q.push_back(to);
                }
            }
        }
        reach.insert(begin);
        reach.insert(end);
        if let Some(m) = meta {
            reach.insert(m);
        }
        b.nodes_inside = reach;

        // Fit rect to members + existing stickies.
        let mut min = Vec2::new(f32::INFINITY, f32::INFINITY);
        let mut max = Vec2::new(f32::NEG_INFINITY, f32::NEG_INFINITY);
        let mut any = false;
        for nid in &b.nodes_inside {
            let Some(n) = g.nodes.get(nid) else {
                continue;
            };
            let p = n.position.to_vec2();
            min.x = min.x.min(p.x);
            min.y = min.y.min(p.y);
            max.x = max.x.max(p.x + n.size.x);
            max.y = max.y.max(p.y + n.size.y);
            any = true;
        }
        for sid in &b.stickies_inside {
            let Some(s) = g.sticky_notes.get(sid) else {
                continue;
            };
            min.x = min.x.min(s.rect.min.x);
            min.y = min.y.min(s.rect.min.y);
            max.x = max.x.max(s.rect.max.x);
            max.y = max.y.max(s.rect.max.y);
            any = true;
        }
        if any {
            let mut mn = min - pad;
            mn.y -= header_h;
            b.rect = Rect::from_min_max(mn.to_pos2(), (max + pad).to_pos2());
        }
    }
}

pub fn draw_port_feedback(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    s: &crate::node_editor_settings::NodeEditorSettings,
    inv: &mut UiInvalidator,
) {
    let zoom = editor.zoom;
    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
    let stroke_w = s.port_stroke_width_base.max(0.0) * zoom.sqrt();
    let screen_size = ui.ctx().screen_rect().size();
    let frame_id = ui.ctx().cumulative_frame_nr();
    let painter = ui.painter();
    let green = Color32::from_rgb(80, 255, 120);

    let draw_port = |pos: Pos2, width_opt: Option<f32>, mul: f32, fill: Color32| {
        if let Some(w) = width_opt {
            // Bar/collection style input: highlight as a rounded rect.
            let h = (port_radius * 1.5 * mul).max(1.0);
            let rect = Rect::from_center_size(pos, Vec2::new(w, h));
            let u = SdfRectUniform {
                center: [pos.x, pos.y],
                half_size: [w * 0.5, h * 0.5],
                corner_radii: [2.0; 4],
                fill_color: bevy_egui::egui::Rgba::from(fill).to_array(),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: stroke_w.max(1.0),
                _pad2: [0.0; 3],
                border_color: bevy_egui::egui::Rgba::from(Color32::WHITE).to_array(),
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(rect.expand(6.0), u, frame_id));
        } else {
            // Point style ports: highlight as a filled circle that grows slightly.
            let r = (port_radius * mul).max(1.0);
            let rect = Rect::from_center_size(pos, Vec2::splat(r * 4.0));
            let u = SdfCircleUniform {
                center: [pos.x, pos.y],
                radius: r,
                border_width: stroke_w.max(1.0),
                fill_color: bevy_egui::egui::Rgba::from(fill).to_array(),
                border_color: bevy_egui::egui::Rgba::from(Color32::WHITE).to_array(),
                softness: 1.0,
                _pad0: 0.0,
                screen_size: [screen_size.x, screen_size.y],
                _pad1: [0.0; 2],
                _pad2: [0.0; 2],
            };
            painter.add(create_sdf_circle_callback(
                rect,
                painter.clip_rect(),
                u,
                frame_id,
            ));
        }
    };

    // Hover highlight (matches old behavior: solid port grows + turns green).
    if let Some((nid, pid, _)) = &editor.hovered_port {
        if let Some((pos, w)) = editor.port_locations.get(&(*nid, pid.clone())) {
            draw_port(*pos, *w, 1.25, green);
        }
    }

    // Active (clicked) source port highlight: solid port stays a bit larger + green.
    if let Some((nid, pid)) = &editor.pending_connection_from {
        if let Some((pos, w)) = editor.port_locations.get(&(*nid, pid.clone())) {
            draw_port(*pos, *w, 1.35, green);
        }
        // While a connection is "armed", we only need to repaint on real pointer movement.
        // This preserves the click-move-click workflow WITHOUT idle 60Hz spinning (whitepaper D1/D2).
        let moved = ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::PointerMoved(_) | egui::Event::MouseMoved(_)))
        });
        if moved {
            inv.request_repaint_after_tagged(
                "node_editor/armed_move",
                std::time::Duration::ZERO,
                RepaintCause::Input,
            );
        }
    }

    // Optional: highlight snapped target port as well.
    if let Some((nid, pid)) = &editor.snapped_to_port {
        if let Some((pos, w)) = editor.port_locations.get(&(*nid, pid.clone())) {
            draw_port(*pos, *w, 1.25, green.linear_multiply(0.9));
        }
    }
}

pub fn draw_and_interact_with_node(
    ui: &mut egui::Ui,
    node: &mut Node,
    node_rect_screen: Rect,
    theme: &ModernTheme,
    is_selected: bool,
    zoom: f32,
    s: &crate::node_editor_settings::NodeEditorSettings,
    port_locations: &mut std::collections::HashMap<
        (crate::nodes::NodeId, crate::nodes::PortId),
        (egui::Pos2, Option<f32>),
    >,
    last_clicked_port: Option<(crate::nodes::PortId, f64)>,
) -> (NodeInteraction, Option<(crate::nodes::PortId, f64)>) {
    let painter = ui.painter();
    let param_changed = false;

    // --- Draw State Backgrounds ---
    let max_dim = node_rect_screen.width().max(node_rect_screen.height());
    let base_square = Rect::from_center_size(node_rect_screen.center(), Vec2::splat(max_dim));
    let state_rounding = CornerRadius::from(6.0 * zoom.sqrt());

    if node.is_display_node {
        let padding = (10.0 * zoom).max(5.0);
        let bg_rect = base_square.expand(padding);
        let screen_size = ui.ctx().screen_rect().size();
        let mut fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_display).to_array();
        fill_rgba[3] = 38.0 / 255.0;
        let uniform = SdfRectUniform {
            center: [bg_rect.center().x, bg_rect.center().y],
            half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
            corner_radii: [
                state_rounding.nw as f32,
                state_rounding.ne as f32,
                state_rounding.se as f32,
                state_rounding.sw as f32,
            ],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
    }

    if node.is_template {
        let padding = (2.0 * zoom).max(1.0);
        let bg_rect = base_square.expand(padding);
        let screen_size = ui.ctx().screen_rect().size();
        let mut fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_template).to_array();
        fill_rgba[3] = 38.0 / 255.0;
        let uniform = SdfRectUniform {
            center: [bg_rect.center().x, bg_rect.center().y],
            half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
            corner_radii: [
                state_rounding.nw as f32,
                state_rounding.ne as f32,
                state_rounding.se as f32,
                state_rounding.sw as f32,
            ],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
    }

    let rounding = CornerRadius::from(s.style_node_rounding_base.max(0.0) * zoom.sqrt());

    // --- Calculate Visual BBox ---
    let visual_bbox = match node.style {
        NodeStyle::Layered => {
            let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
            let background_offset = shadow_offset * 2.0;
            let background_rect = node_rect_screen.translate(background_offset);
            Rect::from_min_max(node_rect_screen.min, background_rect.max)
        }
        NodeStyle::Normal | NodeStyle::Large => node_rect_screen,
    };

    // --- Draw Node Body ---
    match node.style {
        NodeStyle::Layered => {
            let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
            let background_offset = shadow_offset * 2.0;
            let top_rect = node_rect_screen;
            let shadow_rect = top_rect.translate(shadow_offset);
            let background_rect = top_rect.translate(background_offset);

            let screen_size = ui.ctx().screen_rect().size();
            let rounding4 = [
                rounding.nw as f32,
                rounding.ne as f32,
                rounding.se as f32,
                rounding.sw as f32,
            ];
            let frame_id = ui.ctx().cumulative_frame_nr();

            // background
            {
                let c = background_rect.center();
                let s = background_rect.size();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s.x * 0.5, s.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: [1.0, 1.0, 1.0, 1.0],
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(background_rect, uniform, frame_id));
            }
            // shadow-ish middle layer
            {
                let c = shadow_rect.center();
                let s = shadow_rect.size();
                let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.divider_color).to_array();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s.x * 0.5, s.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: fill_rgba,
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(shadow_rect, uniform, frame_id));
            }
            // top
            {
                let c = top_rect.center();
                let s = top_rect.size();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s.x * 0.5, s.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: [1.0, 1.0, 1.0, 1.0],
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(top_rect, uniform, frame_id));
            }
        }
        NodeStyle::Normal | NodeStyle::Large => {
            // [SDF Replacement]
            // We use the SDF Renderer to draw the node body with perfect anti-aliased corners and shadow.
            let screen_size = ui.ctx().screen_rect().size();
            let center = node_rect_screen.center();
            let size = node_rect_screen.size();

            // Fill Color (using theme background if available, or white as before)
            let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_background).to_array();
            let border_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_border).to_array(); // Changed from divider_color to match theme intent

            let uniform = SdfRectUniform {
                center: [center.x, center.y],
                half_size: [size.x / 2.0, size.y / 2.0],
                corner_radii: [
                    rounding.nw as f32,
                    rounding.ne as f32,
                    rounding.se as f32,
                    rounding.sw as f32,
                ],
                fill_color: fill_rgba,
                shadow_color: [0.0, 0.0, 0.0, 0.3], // Soft shadow
                shadow_blur: 15.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 5.0],
                border_width: s.style_node_border_width_base.max(0.0) * zoom.sqrt(),
                _pad2: [0.0; 3],
                border_color: border_rgba,
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            let frame_id = ui.ctx().cumulative_frame_nr();

            painter.add(create_sdf_rect_callback(
                node_rect_screen.expand(30.0), // Ensure enough space for shadow
                uniform,
                frame_id,
            ));
        }
    }

    // --- Status Indicators ---
    let sidebar_width = node_rect_screen.width() * s.layout_sidebar_width_ratio.clamp(0.0, 0.5);

    if node.is_bypassed {
        let bypass_rect = node_rect_screen.with_max_x(node_rect_screen.min.x + sidebar_width);
        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_bypass).to_array();
        let uniform = SdfRectUniform {
            center: [bypass_rect.center().x, bypass_rect.center().y],
            half_size: [bypass_rect.width() * 0.5, bypass_rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bypass_rect, uniform, frame_id));
    }

    if node.is_locked {
        let lock_rect = node_rect_screen.with_min_x(node_rect_screen.max.x - sidebar_width);
        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_lock).to_array();
        let uniform = SdfRectUniform {
            center: [lock_rect.center().x, lock_rect.center().y],
            half_size: [lock_rect.width() * 0.5, lock_rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(lock_rect, uniform, frame_id));
    }

    let divider_stroke = Stroke::new(1.0, theme.colors.divider_color);
    let left_divider_x = node_rect_screen.min.x + sidebar_width;
    {
        let screen_size = ui.ctx().screen_rect().size();
        let w = divider_stroke.width.max(1.0);
        let rect = Rect::from_min_max(
            Pos2::new(left_divider_x - w * 0.5, node_rect_screen.min.y),
            Pos2::new(left_divider_x + w * 0.5, node_rect_screen.max.y),
        );
        let fill_rgba = bevy_egui::egui::Rgba::from(divider_stroke.color).to_array();
        let uniform = SdfRectUniform {
            center: [rect.center().x, rect.center().y],
            half_size: [rect.width() * 0.5, rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            rect.expand(2.0),
            uniform,
            frame_id,
        ));
    }

    let right_divider_x = node_rect_screen.max.x - sidebar_width;
    {
        let screen_size = ui.ctx().screen_rect().size();
        let w = divider_stroke.width.max(1.0);
        let rect = Rect::from_min_max(
            Pos2::new(right_divider_x - w * 0.5, node_rect_screen.min.y),
            Pos2::new(right_divider_x + w * 0.5, node_rect_screen.max.y),
        );
        let fill_rgba = bevy_egui::egui::Rgba::from(divider_stroke.color).to_array();
        let uniform = SdfRectUniform {
            center: [rect.center().x, rect.center().y],
            half_size: [rect.width() * 0.5, rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            rect.expand(2.0),
            uniform,
            frame_id,
        ));
    }

    // --- Selection Outline ---
    if is_selected {
        let screen_size = ui.ctx().screen_rect().size();
        let center = node_rect_screen.center();
        let size = node_rect_screen.size();
        let border_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_selected).to_array();
        let uniform = SdfRectUniform {
            center: [center.x, center.y],
            half_size: [size.x * 0.5, size.y * 0.5],
            corner_radii: [
                rounding.nw as f32,
                rounding.ne as f32,
                rounding.se as f32,
                rounding.sw as f32,
            ],
            fill_color: [1.0, 1.0, 1.0, 0.0],
            shadow_color: [0.0, 0.0, 0.0, 0.0],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: s.style_node_border_width_base.max(0.0) * zoom.sqrt(),
            _pad2: [0.0; 3],
            border_color: border_rgba,
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            node_rect_screen.expand(6.0),
            uniform,
            frame_id,
        ));
    }

    // --- Node Name ---
    {
        let font_px = 14.0 * zoom;
        let lod_hide = s.title_text_lod_hide_px.max(0.0);
        let lod_fade = s.title_text_lod_fade_px.max(1e-3);
        if font_px > lod_hide {
            let pad = (6.0 * zoom).max(2.0);
            let text_pos = Pos2::new(node_rect_screen.center().x, node_rect_screen.top() - pad);
            let opacity = ((font_px - lod_hide) / lod_fade).clamp(0.0, 1.0);
            let color = theme.colors.primary_text.linear_multiply(opacity);
            if opacity > 0.05 {
                painter.text(
                    text_pos,
                    egui::Align2::CENTER_BOTTOM,
                    node.name.clone(),
                    egui::FontId::proportional(font_px),
                    color,
                );
            }
        }
    }

    // --- Node Icon (center) ---
    {
        let sz = (18.0 * zoom).clamp(10.0, 28.0);
        let icon = icons::icon_for_node_name(&node.name, true);
        let r = Rect::from_center_size(node_rect_screen.center(), Vec2::splat(sz));
        egui::Image::new(icon)
            .fit_to_exact_size(r.size())
            .paint_at(ui, r);
    }

    // --- Interaction ---
    let response = ui.interact(
        node_rect_screen,
        ui.make_persistent_id(node.id).with("interaction"),
        Sense::click(),
    );

    let mut interaction = NodeInteraction {
        hovered: response.hovered(),
        clicked: response.clicked(),
        clicked_on_output_port: None,
        dragged_on_output_port: None,
        parameter_changed: param_changed,
    };
    let mut new_last_clicked_port = last_clicked_port;

    // --- Draw Ports ---
    let pointer_pos = ui.ctx().pointer_interact_pos();
    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
    let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
    let port_stroke = Stroke::new(
        s.port_stroke_width_base.max(0.0) * zoom.sqrt(),
        Color32::from_gray(120),
    );
    let hover_radius = port_radius * s.port_hover_radius_mul.max(1.0);
    let hover_radius_sq = hover_radius.powi(2);

    // 1. Inputs
    match node.input_style {
        InputStyle::Individual => {
            let mut input_ports: Vec<&crate::nodes::PortId> = node.inputs.keys().collect();
            input_ports.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            let input_count = input_ports.len();
            for (i, port_id) in input_ports.into_iter().enumerate() {
                let x = node_rect_screen.left() - port_offset;
                let y = node_rect_screen.top()
                    + node_rect_screen.height() * (i as f32 + 1.0) / (input_count as f32 + 1.0);
                let port_pos = Pos2::new(x, y);

                let mut is_hovered = false;
                if let Some(p_pos) = pointer_pos {
                    if p_pos.distance_sq(port_pos) < hover_radius_sq
                        && p_pos.x < node_rect_screen.left()
                    {
                        is_hovered = true;
                    }
                }

                let mut is_flashing = false;
                if let Some((clicked_port_id, click_time)) = new_last_clicked_port.as_ref() {
                    if clicked_port_id == port_id && ui.ctx().input(|i| i.time) - click_time < 0.15
                    {
                        is_flashing = true;
                    }
                }

                let (radius, fill, stroke) = if is_flashing {
                    (
                        port_radius * 1.7,
                        Color32::YELLOW,
                        Stroke::new(port_stroke.width, Color32::WHITE),
                    )
                } else if is_hovered {
                    (
                        port_radius * 1.5,
                        Color32::GREEN,
                        Stroke::new(port_stroke.width, Color32::DARK_GREEN),
                    )
                } else {
                    (port_radius, Color32::WHITE, port_stroke)
                };

                let port_rect = Rect::from_center_size(port_pos, Vec2::splat(hover_radius * 2.0));
                let port_response = ui
                    .interact(
                        port_rect,
                        ui.make_persistent_id(node.id).with(port_id),
                        Sense::click(),
                    )
                    .on_hover_text(port_id.as_str());

                if port_response.clicked() {
                    new_last_clicked_port = Some((port_id.clone(), ui.ctx().input(|i| i.time)));
                }

                let screen_size = ui.ctx().screen_rect().size();
                let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
                let stroke_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
                let uniform = SdfCircleUniform {
                    center: [port_pos.x, port_pos.y],
                    radius,
                    border_width: stroke.width,
                    fill_color: fill_rgba,
                    border_color: stroke_rgba,
                    softness: 1.0,
                    _pad0: 0.0,
                    screen_size: [screen_size.x, screen_size.y],
                    _pad1: [0.0; 2],
                    _pad2: [0.0; 2],
                };
                let frame_id = ui.ctx().cumulative_frame_nr();
                painter.add(create_sdf_circle_callback(
                    port_rect,
                    painter.clip_rect(),
                    uniform,
                    frame_id,
                ));
                port_locations.insert((node.id, port_id.clone()), (port_pos, None));
            }
        }
        InputStyle::Bar | InputStyle::Collection => {
            let mut input_ports: Vec<&crate::nodes::PortId> = node.inputs.keys().collect();
            input_ports.sort_by(|a, b| a.as_str().cmp(b.as_str()));
            if let Some(port_id) = input_ports.into_iter().next() {
                let bar_height =
                    node_rect_screen.height() * s.layout_bar_width_ratio.clamp(0.1, 1.0);
                let bar_width = port_radius * 1.5;
                let bar_center_x = node_rect_screen.left() - port_offset;
                let bar_center_y = node_rect_screen.center().y;
                let bar_pos = Pos2::new(bar_center_x, bar_center_y);
                let bar_rect = Rect::from_center_size(bar_pos, Vec2::new(bar_width, bar_height));

                port_locations.insert((node.id, port_id.clone()), (bar_pos, Some(bar_height)));

                let bar_response = ui
                    .interact(
                        bar_rect.expand(5.0),
                        ui.make_persistent_id(node.id).with(port_id),
                        Sense::hover(),
                    )
                    .on_hover_text(port_id.as_str());

                let is_hovered = bar_response.hovered();

                let (fill, stroke) = if is_hovered {
                    (
                        Color32::GREEN,
                        Stroke::new(port_stroke.width, Color32::DARK_GREEN),
                    )
                } else {
                    (Color32::WHITE, port_stroke)
                };

                let screen_size = ui.ctx().screen_rect().size();
                let center = bar_rect.center();
                let size = bar_rect.size();
                let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
                let border_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
                let rounding = CornerRadius::from(2.0);
                let uniform = SdfRectUniform {
                    center: [center.x, center.y],
                    half_size: [size.x * 0.5, size.y * 0.5],
                    corner_radii: [
                        rounding.nw as f32,
                        rounding.ne as f32,
                        rounding.se as f32,
                        rounding.sw as f32,
                    ],
                    fill_color: fill_rgba,
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: stroke.width,
                    _pad2: [0.0; 3],
                    border_color: border_rgba,
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                let frame_id = ui.ctx().cumulative_frame_nr();
                painter.add(create_sdf_rect_callback(
                    bar_rect.expand(6.0),
                    uniform,
                    frame_id,
                ));
            }
        }
    }

    // 2. Outputs
    let mut output_ports: Vec<&crate::nodes::PortId> = node.outputs.keys().collect();
    output_ports.sort_by(|a, b| a.as_str().cmp(b.as_str()));
    let output_count = output_ports.len();
    for (i, port_id) in output_ports.into_iter().enumerate() {
        let x = node_rect_screen.right() + port_offset;
        let y = node_rect_screen.top()
            + node_rect_screen.height() * (i as f32 + 1.0) / (output_count as f32 + 1.0);
        let port_pos = Pos2::new(x, y);

        let mut is_hovered = false;
        if let Some(p_pos) = pointer_pos {
            if p_pos.distance_sq(port_pos) < hover_radius_sq && p_pos.x > node_rect_screen.right() {
                is_hovered = true;
            }
        }

        let mut is_flashing = false;
        if let Some((clicked_port_id, click_time)) = new_last_clicked_port.as_ref() {
            if clicked_port_id == port_id && ui.ctx().input(|i| i.time) - click_time < 0.15 {
                is_flashing = true;
            }
        }

        let (radius, fill, stroke) = if is_flashing {
            (
                port_radius * 1.7,
                Color32::YELLOW,
                Stroke::new(port_stroke.width, Color32::WHITE),
            )
        } else if is_hovered {
            (
                port_radius * 1.5,
                Color32::GREEN,
                Stroke::new(port_stroke.width, Color32::DARK_GREEN),
            )
        } else {
            (port_radius, Color32::WHITE, port_stroke)
        };

        let port_rect = Rect::from_center_size(port_pos, Vec2::splat(hover_radius * 2.0));
        let port_response = ui
            .interact(
                port_rect,
                ui.make_persistent_id(node.id).with(port_id),
                Sense::click_and_drag(),
            )
            .on_hover_text(port_id.as_str());

        if port_response.clicked() {
            new_last_clicked_port = Some((port_id.clone(), ui.ctx().input(|i| i.time)));
            interaction.clicked_on_output_port = Some((node.id, port_id.clone()));
        }

        if port_response.dragged() {
            interaction.dragged_on_output_port = Some((node.id, port_id.clone()));
        }

        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
        let stroke_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
        let uniform = SdfCircleUniform {
            center: [port_pos.x, port_pos.y],
            radius,
            border_width: stroke.width,
            fill_color: fill_rgba,
            border_color: stroke_rgba,
            softness: 1.0,
            _pad0: 0.0,
            screen_size: [screen_size.x, screen_size.y],
            _pad1: [0.0; 2],
            _pad2: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_circle_callback(
            port_rect,
            painter.clip_rect(),
            uniform,
            frame_id,
        ));
        port_locations.insert((node.id, port_id.clone()), (port_pos, None));
    }

    (interaction, new_last_clicked_port)
}

pub fn draw_and_interact_with_node_snapshot(
    ui: &mut egui::Ui,
    node: &NodeSnapshot,
    node_rect_screen: Rect,
    theme: &ModernTheme,
    is_selected: bool,
    zoom: f32,
    s: &crate::node_editor_settings::NodeEditorSettings,
    port_locations: &mut std::collections::HashMap<
        (crate::nodes::NodeId, crate::nodes::PortId),
        (egui::Pos2, Option<f32>),
    >,
    last_clicked_port: Option<(crate::nodes::PortId, f64)>,
) -> (NodeInteraction, Option<(crate::nodes::PortId, f64)>) {
    let painter = ui.painter();
    let param_changed = false;

    // --- Draw State Backgrounds ---
    let max_dim = node_rect_screen.width().max(node_rect_screen.height());
    let base_square = Rect::from_center_size(node_rect_screen.center(), Vec2::splat(max_dim));
    let state_rounding = CornerRadius::from(6.0 * zoom.sqrt());

    if node.is_display_node {
        let padding = (10.0 * zoom).max(5.0);
        let bg_rect = base_square.expand(padding);
        let screen_size = ui.ctx().screen_rect().size();
        let mut fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_display).to_array();
        fill_rgba[3] = 38.0 / 255.0;
        let uniform = SdfRectUniform {
            center: [bg_rect.center().x, bg_rect.center().y],
            half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
            corner_radii: [
                state_rounding.nw as f32,
                state_rounding.ne as f32,
                state_rounding.se as f32,
                state_rounding.sw as f32,
            ],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
    }

    if node.is_template {
        let padding = (2.0 * zoom).max(1.0);
        let bg_rect = base_square.expand(padding);
        let screen_size = ui.ctx().screen_rect().size();
        let mut fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_template).to_array();
        fill_rgba[3] = 38.0 / 255.0;
        let uniform = SdfRectUniform {
            center: [bg_rect.center().x, bg_rect.center().y],
            half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
            corner_radii: [
                state_rounding.nw as f32,
                state_rounding.ne as f32,
                state_rounding.se as f32,
                state_rounding.sw as f32,
            ],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
    }

    let rounding = CornerRadius::from(s.style_node_rounding_base.max(0.0) * zoom.sqrt());

    // --- Calculate Visual BBox ---
    let visual_bbox = match node.style {
        NodeStyle::Layered => {
            let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
            let background_offset = shadow_offset * 2.0;
            let background_rect = node_rect_screen.translate(background_offset);
            Rect::from_min_max(node_rect_screen.min, background_rect.max)
        }
        NodeStyle::Normal | NodeStyle::Large => node_rect_screen,
    };

    // --- Draw Node Body ---
    match node.style {
        NodeStyle::Layered => {
            let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
            let background_offset = shadow_offset * 2.0;
            let top_rect = node_rect_screen;
            let shadow_rect = top_rect.translate(shadow_offset);
            let background_rect = top_rect.translate(background_offset);

            let screen_size = ui.ctx().screen_rect().size();
            let rounding4 = [
                rounding.nw as f32,
                rounding.ne as f32,
                rounding.se as f32,
                rounding.sw as f32,
            ];
            let frame_id = ui.ctx().cumulative_frame_nr();

            // background
            {
                let c = background_rect.center();
                let s2 = background_rect.size();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s2.x * 0.5, s2.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: [1.0, 1.0, 1.0, 1.0],
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(background_rect, uniform, frame_id));
            }
            // shadow-ish middle layer
            {
                let c = shadow_rect.center();
                let s2 = shadow_rect.size();
                let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.divider_color).to_array();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s2.x * 0.5, s2.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: fill_rgba,
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(shadow_rect, uniform, frame_id));
            }
            // top
            {
                let c = top_rect.center();
                let s2 = top_rect.size();
                let uniform = SdfRectUniform {
                    center: [c.x, c.y],
                    half_size: [s2.x * 0.5, s2.y * 0.5],
                    corner_radii: rounding4,
                    fill_color: [1.0, 1.0, 1.0, 1.0],
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: 0.0,
                    _pad2: [0.0; 3],
                    border_color: [0.0; 4],
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(top_rect, uniform, frame_id));
            }
        }
        NodeStyle::Normal | NodeStyle::Large => {
            let screen_size = ui.ctx().screen_rect().size();
            let center = node_rect_screen.center();
            let size = node_rect_screen.size();
            let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_background).to_array();
            let border_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_border).to_array();

            let uniform = SdfRectUniform {
                center: [center.x, center.y],
                half_size: [size.x / 2.0, size.y / 2.0],
                corner_radii: [
                    rounding.nw as f32,
                    rounding.ne as f32,
                    rounding.se as f32,
                    rounding.sw as f32,
                ],
                fill_color: fill_rgba,
                shadow_color: [0.0, 0.0, 0.0, 0.3],
                shadow_blur: 15.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 5.0],
                border_width: s.style_node_border_width_base.max(0.0) * zoom.sqrt(),
                _pad2: [0.0; 3],
                border_color: border_rgba,
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            let frame_id = ui.ctx().cumulative_frame_nr();
            painter.add(create_sdf_rect_callback(
                node_rect_screen.expand(30.0),
                uniform,
                frame_id,
            ));
        }
    }

    // --- Status Indicators ---
    let sidebar_width = node_rect_screen.width() * s.layout_sidebar_width_ratio.clamp(0.0, 0.5);
    if node.is_bypassed {
        let bypass_rect = node_rect_screen.with_max_x(node_rect_screen.min.x + sidebar_width);
        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_bypass).to_array();
        let uniform = SdfRectUniform {
            center: [bypass_rect.center().x, bypass_rect.center().y],
            half_size: [bypass_rect.width() * 0.5, bypass_rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(bypass_rect, uniform, frame_id));
    }

    if node.is_locked {
        let lock_rect = node_rect_screen.with_min_x(node_rect_screen.max.x - sidebar_width);
        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_lock).to_array();
        let uniform = SdfRectUniform {
            center: [lock_rect.center().x, lock_rect.center().y],
            half_size: [lock_rect.width() * 0.5, lock_rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(lock_rect, uniform, frame_id));
    }

    let divider_stroke = Stroke::new(1.0, theme.colors.divider_color);
    let left_divider_x = node_rect_screen.min.x + sidebar_width;
    {
        let screen_size = ui.ctx().screen_rect().size();
        let w = divider_stroke.width.max(1.0);
        let rect = Rect::from_min_max(
            Pos2::new(left_divider_x - w * 0.5, node_rect_screen.min.y),
            Pos2::new(left_divider_x + w * 0.5, node_rect_screen.max.y),
        );
        let fill_rgba = bevy_egui::egui::Rgba::from(divider_stroke.color).to_array();
        let uniform = SdfRectUniform {
            center: [rect.center().x, rect.center().y],
            half_size: [rect.width() * 0.5, rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            rect.expand(2.0),
            uniform,
            frame_id,
        ));
    }

    let right_divider_x = node_rect_screen.max.x - sidebar_width;
    {
        let screen_size = ui.ctx().screen_rect().size();
        let w = divider_stroke.width.max(1.0);
        let rect = Rect::from_min_max(
            Pos2::new(right_divider_x - w * 0.5, node_rect_screen.min.y),
            Pos2::new(right_divider_x + w * 0.5, node_rect_screen.max.y),
        );
        let fill_rgba = bevy_egui::egui::Rgba::from(divider_stroke.color).to_array();
        let uniform = SdfRectUniform {
            center: [rect.center().x, rect.center().y],
            half_size: [rect.width() * 0.5, rect.height() * 0.5],
            corner_radii: [0.0; 4],
            fill_color: fill_rgba,
            shadow_color: [0.0; 4],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: 0.0,
            _pad2: [0.0; 3],
            border_color: [0.0; 4],
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            rect.expand(2.0),
            uniform,
            frame_id,
        ));
    }

    // --- Selection Outline ---
    if is_selected {
        let screen_size = ui.ctx().screen_rect().size();
        let center = node_rect_screen.center();
        let size = node_rect_screen.size();
        let border_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_selected).to_array();
        let uniform = SdfRectUniform {
            center: [center.x, center.y],
            half_size: [size.x * 0.5, size.y * 0.5],
            corner_radii: [
                rounding.nw as f32,
                rounding.ne as f32,
                rounding.se as f32,
                rounding.sw as f32,
            ],
            fill_color: [1.0, 1.0, 1.0, 0.0],
            shadow_color: [0.0, 0.0, 0.0, 0.0],
            shadow_blur: 0.0,
            _pad1: 0.0,
            shadow_offset: [0.0, 0.0],
            border_width: s.style_node_border_width_base.max(0.0) * zoom.sqrt(),
            _pad2: [0.0; 3],
            border_color: border_rgba,
            screen_size: [screen_size.x, screen_size.y],
            _pad3: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_rect_callback(
            node_rect_screen.expand(6.0),
            uniform,
            frame_id,
        ));
    }

    // --- Node Name ---
    {
        let font_px = 16.0 * zoom;
        let lod_hide = s.title_text_lod_hide_px.max(0.0);
        let lod_fade = s.title_text_lod_fade_px.max(1e-3);
        if font_px > lod_hide {
            let pad = (6.0 * zoom).max(2.0);
            let text_pos = Pos2::new(node_rect_screen.center().x, node_rect_screen.top() - pad);
            let opacity = ((font_px - lod_hide) / lod_fade).clamp(0.0, 1.0);
            let color = theme.colors.primary_text.linear_multiply(opacity);
            if opacity > 0.05 {
                painter.text(
                    text_pos,
                    egui::Align2::CENTER_BOTTOM,
                    node.name.clone(),
                    egui::FontId::proportional(font_px),
                    color,
                );
            }
        }
    }

    // --- Node Icon (center) ---
    {
        let sz = (18.0 * zoom).clamp(10.0, 28.0);
        let icon = icons::icon_for_node_name(&node.name, true);
        let r = Rect::from_center_size(node_rect_screen.center(), Vec2::splat(sz));
        egui::Image::new(icon)
            .fit_to_exact_size(r.size())
            .paint_at(ui, r);
    }

    // --- Interaction ---
    let response = ui.interact(
        node_rect_screen,
        ui.make_persistent_id(node.id).with("interaction"),
        Sense::click(),
    );
    let mut interaction = NodeInteraction {
        hovered: response.hovered(),
        clicked: response.clicked(),
        clicked_on_output_port: None,
        dragged_on_output_port: None,
        parameter_changed: param_changed,
    };
    let mut new_last_clicked_port = last_clicked_port;

    // --- Draw Ports ---
    let pointer_pos = ui.ctx().pointer_interact_pos();
    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
    let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
    let port_stroke = Stroke::new(
        s.port_stroke_width_base.max(0.0) * zoom.sqrt(),
        Color32::from_gray(120),
    );
    let hover_radius = port_radius * s.port_hover_radius_mul.max(1.0);
    let hover_radius_sq = hover_radius.powi(2);

    // 1. Inputs
    match node.input_style {
        InputStyle::Individual => {
            let input_count = node.inputs.len();
            for (i, port_id) in node.inputs.iter().enumerate() {
                let x = node_rect_screen.left() - port_offset;
                let y = node_rect_screen.top()
                    + node_rect_screen.height() * (i as f32 + 1.0) / (input_count as f32 + 1.0);
                let port_pos = Pos2::new(x, y);

                let mut is_hovered = false;
                if let Some(p_pos) = pointer_pos {
                    if p_pos.distance_sq(port_pos) < hover_radius_sq
                        && p_pos.x < node_rect_screen.left()
                    {
                        is_hovered = true;
                    }
                }

                let mut is_flashing = false;
                if let Some((clicked_port_id, click_time)) = new_last_clicked_port.as_ref() {
                    if clicked_port_id == port_id && ui.ctx().input(|i| i.time) - click_time < 0.15
                    {
                        is_flashing = true;
                    }
                }

                let (radius, fill, stroke) = if is_flashing {
                    (
                        port_radius * 1.7,
                        Color32::YELLOW,
                        Stroke::new(port_stroke.width, Color32::WHITE),
                    )
                } else if is_hovered {
                    (
                        port_radius * 1.5,
                        Color32::GREEN,
                        Stroke::new(port_stroke.width, Color32::DARK_GREEN),
                    )
                } else {
                    (port_radius, Color32::WHITE, port_stroke)
                };

                let port_rect = Rect::from_center_size(port_pos, Vec2::splat(hover_radius * 2.0));
                let port_response = ui.interact(
                    port_rect,
                    ui.make_persistent_id(node.id).with(port_id),
                    Sense::click(),
                );
                if port_response.clicked() {
                    new_last_clicked_port = Some((port_id.clone(), ui.ctx().input(|i| i.time)));
                }

                let screen_size = ui.ctx().screen_rect().size();
                let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
                let stroke_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
                let uniform = SdfCircleUniform {
                    center: [port_pos.x, port_pos.y],
                    radius,
                    border_width: stroke.width,
                    fill_color: fill_rgba,
                    border_color: stroke_rgba,
                    softness: 1.0,
                    _pad0: 0.0,
                    screen_size: [screen_size.x, screen_size.y],
                    _pad1: [0.0; 2],
                    _pad2: [0.0; 2],
                };
                let frame_id = ui.ctx().cumulative_frame_nr();
                painter.add(create_sdf_circle_callback(
                    port_rect,
                    painter.clip_rect(),
                    uniform,
                    frame_id,
                ));
                port_locations.insert((node.id, port_id.clone()), (port_pos, None));
            }
        }
        InputStyle::Bar | InputStyle::Collection => {
            if let Some(port_id) = node.inputs.first() {
                let bar_height =
                    node_rect_screen.height() * s.layout_bar_width_ratio.clamp(0.1, 1.0);
                let bar_width = port_radius * 1.5;
                let bar_center_x = node_rect_screen.left() - port_offset;
                let bar_center_y = node_rect_screen.center().y;
                let bar_pos = Pos2::new(bar_center_x, bar_center_y);
                let bar_rect = Rect::from_center_size(bar_pos, Vec2::new(bar_width, bar_height));

                port_locations.insert((node.id, port_id.clone()), (bar_pos, Some(bar_height)));
                let bar_response = ui.interact(
                    bar_rect.expand(5.0),
                    ui.make_persistent_id(node.id).with(port_id),
                    Sense::hover(),
                );
                let is_hovered = bar_response.hovered();

                let (fill, stroke) = if is_hovered {
                    (
                        Color32::GREEN,
                        Stroke::new(port_stroke.width, Color32::DARK_GREEN),
                    )
                } else {
                    (Color32::WHITE, port_stroke)
                };

                let screen_size = ui.ctx().screen_rect().size();
                let center = bar_rect.center();
                let size = bar_rect.size();
                let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
                let border_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
                let rounding = CornerRadius::from(2.0);
                let uniform = SdfRectUniform {
                    center: [center.x, center.y],
                    half_size: [size.x * 0.5, size.y * 0.5],
                    corner_radii: [
                        rounding.nw as f32,
                        rounding.ne as f32,
                        rounding.se as f32,
                        rounding.sw as f32,
                    ],
                    fill_color: fill_rgba,
                    shadow_color: [0.0; 4],
                    shadow_blur: 0.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 0.0],
                    border_width: stroke.width,
                    _pad2: [0.0; 3],
                    border_color: border_rgba,
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                let frame_id = ui.ctx().cumulative_frame_nr();
                painter.add(create_sdf_rect_callback(
                    bar_rect.expand(6.0),
                    uniform,
                    frame_id,
                ));
            }
        }
    }

    // 2. Outputs
    let mut output_ports = node.outputs.clone();
    output_ports.sort();
    let output_count = output_ports.len();
    for (i, port_id) in output_ports.iter().enumerate() {
        let x = node_rect_screen.right() + port_offset;
        let y = node_rect_screen.top()
            + node_rect_screen.height() * (i as f32 + 1.0) / (output_count as f32 + 1.0);
        let port_pos = Pos2::new(x, y);

        let mut is_hovered = false;
        if let Some(p_pos) = pointer_pos {
            if p_pos.distance_sq(port_pos) < hover_radius_sq && p_pos.x > node_rect_screen.right() {
                is_hovered = true;
            }
        }

        let mut is_flashing = false;
        if let Some((clicked_port_id, click_time)) = new_last_clicked_port.as_ref() {
            if clicked_port_id == port_id && ui.ctx().input(|i| i.time) - click_time < 0.15 {
                is_flashing = true;
            }
        }

        let (radius, fill, stroke) = if is_flashing {
            (
                port_radius * 1.7,
                Color32::YELLOW,
                Stroke::new(port_stroke.width, Color32::WHITE),
            )
        } else if is_hovered {
            (
                port_radius * 1.5,
                Color32::GREEN,
                Stroke::new(port_stroke.width, Color32::DARK_GREEN),
            )
        } else {
            (port_radius, Color32::WHITE, port_stroke)
        };

        let port_rect = Rect::from_center_size(port_pos, Vec2::splat(hover_radius * 2.0));
        let port_response = ui
            .interact(
                port_rect,
                ui.make_persistent_id(node.id).with(port_id),
                Sense::click_and_drag(),
            )
            .on_hover_text(port_id.as_str());

        if port_response.clicked() {
            new_last_clicked_port = Some((port_id.clone(), ui.ctx().input(|i| i.time)));
            interaction.clicked_on_output_port = Some((node.id, port_id.clone()));
        }
        if port_response.dragged() {
            interaction.dragged_on_output_port = Some((node.id, port_id.clone()));
        }

        let screen_size = ui.ctx().screen_rect().size();
        let fill_rgba = bevy_egui::egui::Rgba::from(fill).to_array();
        let stroke_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
        let uniform = SdfCircleUniform {
            center: [port_pos.x, port_pos.y],
            radius,
            border_width: stroke.width,
            fill_color: fill_rgba,
            border_color: stroke_rgba,
            softness: 1.0,
            _pad0: 0.0,
            screen_size: [screen_size.x, screen_size.y],
            _pad1: [0.0; 2],
            _pad2: [0.0; 2],
        };
        let frame_id = ui.ctx().cumulative_frame_nr();
        painter.add(create_sdf_circle_callback(
            port_rect,
            painter.clip_rect(),
            uniform,
            frame_id,
        ));
        port_locations.insert((node.id, port_id.clone()), (port_pos, None));
    }

    (interaction, new_last_clicked_port)
}

pub fn draw_dashed_bezier(ui: &mut egui::Ui, p1: Pos2, p4: Pos2, stroke: Stroke) {
    let control_offset = Vec2::new((p4.x - p1.x).abs() * 0.4, 0.0);
    let p2 = p1 + control_offset;
    let p3 = p4 - control_offset;
    let painter = ui.painter();
    let screen_size = ui.ctx().screen_rect().size();
    let min_x = p1.x.min(p2.x).min(p3.x).min(p4.x);
    let max_x = p1.x.max(p2.x).max(p3.x).max(p4.x);
    let min_y = p1.y.min(p2.y).min(p3.y).min(p4.y);
    let max_y = p1.y.max(p2.y).max(p3.y).max(p4.y);
    let curve_rect = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
        .expand(stroke.width * 2.0 + 12.0)
        .intersect(ui.clip_rect());
    if curve_rect.width() <= 0.0 || curve_rect.height() <= 0.0 {
        return;
    }
    let color_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
    let uniform = SdfDashedCurveUniform {
        p01: [p1.x, p1.y, p2.x, p2.y],
        p23: [p3.x, p3.y, p4.x, p4.y],
        color: color_rgba,
        params: [stroke.width, 1.0, 6.0, 4.0],
        screen_params: [screen_size.x, screen_size.y, 0.0, 0.0],
    };
    let frame_id = ui.ctx().cumulative_frame_nr();
    painter.add(create_sdf_dashed_curve_callback(
        curve_rect, uniform, frame_id,
    ));
}

pub fn draw_connections(
    ui: &mut egui::Ui,
    node_graph: &crate::nodes::structs::NodeGraph,
    context: &EditorTabContext,
    port_locations: &std::collections::HashMap<
        (crate::nodes::NodeId, crate::nodes::PortId),
        (egui::Pos2, Option<f32>),
    >,
    editor_rect: Rect,
    pan: Vec2,
    zoom: f32,
    opacity_mul: f32,
) {
    // --- Pre-process to find all connections going into each merge bar ---
    let mut merge_port_connections: std::collections::HashMap<
        (crate::nodes::NodeId, crate::nodes::PortId),
        Vec<(i32, crate::nodes::ConnectionId)>,
    > = std::collections::HashMap::new();
    for (_conn_id, connection) in &node_graph.connections {
        if let Some((_, Some(_))) =
            port_locations.get(&(connection.to_node, connection.to_port.clone()))
        {
            merge_port_connections
                .entry((connection.to_node, connection.to_port.clone()))
                .or_default()
                .push((connection.order, connection.id));
        }
    }
    for v in merge_port_connections.values_mut() {
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    }

    let painter = ui.painter();
    let default_stroke = Stroke::new(
        2.0,
        Color32::from_gray(150).linear_multiply(opacity_mul.clamp(0.0, 1.0)),
    );
    let selected_stroke = Stroke::new(
        3.0,
        context
            .theme
            .colors
            .node_selected
            .linear_multiply(opacity_mul.clamp(0.0, 1.0)),
    );

    let s = context.node_editor_settings;
    let cook = node_graph.cook_viz.as_ref();
    let cook_active = cook
        .map(|v| v.active.load(std::sync::atomic::Ordering::Relaxed))
        .unwrap_or(false);
    let mut ui_scope: std::collections::HashSet<crate::nodes::NodeId> = std::collections::HashSet::new();
    if cook_active {
        if let Some(did) = node_graph.display_node {
        ui_scope.extend(crate::nodes::runtime::cook::compute_upstream_scope(node_graph, did));
        }
        for &sid in context.ui_state.selected_nodes.iter().take(4) {
            ui_scope.extend(crate::nodes::runtime::cook::compute_upstream_scope(node_graph, sid));
        }
    }

    for (conn_id, connection) in &node_graph.connections {
        let is_selected = context.ui_state.selected_connections.contains(conn_id);
        let stroke = if is_selected {
            selected_stroke
        } else {
            default_stroke
        };

        if let (Some((start_pos, _)), Some((end_center_pos, end_bar_width_opt))) = (
            port_locations.get(&(connection.from_node, connection.from_port.clone())),
            port_locations.get(&(connection.to_node, connection.to_port.clone())),
        ) {
            let mut final_end_pos = *end_center_pos;

            // If the target is a merge bar, calculate the distributed position.
            // Horizontal-flow: bar is vertical, so distribute along Y.
            if let Some(bar_h) = end_bar_width_opt {
                if let Some(conns) =
                    merge_port_connections.get(&(connection.to_node, connection.to_port.clone()))
                {
                    let count = conns.len();
                    if count > 1 {
                        if let Some(index) = conns.iter().position(|(_, id)| *id == connection.id) {
                            let fraction = (index as f32 + 1.0) / (count as f32 + 1.0);
                            let offset_y = (fraction * *bar_h) - (*bar_h / 2.0);
                            final_end_pos.y = end_center_pos.y + offset_y;
                        }
                    }
                }
            }

            let mut pts: Vec<Pos2> = Vec::with_capacity(2 + connection.waypoints.len());
            pts.push(*start_pos);
            for wp in &connection.waypoints {
                pts.push(editor_rect.min + wp.to_vec2() * zoom + pan);
            }
            pts.push(final_end_pos);

            let screen_size = ui.ctx().screen_rect().size();
            let frame_id = ui.ctx().cumulative_frame_nr();

            let cook_in_scope = cook_active
                && cook
                    .map(|v| v.in_scope(connection.from_node) && v.in_scope(connection.to_node))
                    .unwrap_or(false);
            let ui_in_scope =
                cook_active && ui_scope.contains(&connection.from_node) && ui_scope.contains(&connection.to_node);
            // When not cooking, do NOT dim anything (and do NOT animate).
            let mut in_scope = !cook_active || cook_in_scope || ui_in_scope;
            let mut to_state = if cook_active {
                cook.and_then(|v| v.states.get(&connection.to_node).map(|s| *s))
                    .unwrap_or(crate::nodes::runtime::cook::NodeCookState::Idle)
            } else {
                crate::nodes::runtime::cook::NodeCookState::Idle
            };

            let busy_to = node_graph
                .nodes
                .get(&connection.to_node)
                .and_then(|n| n.parameters.iter().find(|p| p.name == "busy"))
                .is_some_and(|p| {
                    matches!(
                        &p.value,
                        crate::nodes::parameter::ParameterValue::Bool(true)
                    )
                });
            in_scope = if busy_to { true } else { in_scope };
            if busy_to {
                to_state = crate::nodes::runtime::cook::NodeCookState::Running;
            } else if cook_active
                && ui_in_scope
                && !cook_in_scope
                && matches!(to_state, crate::nodes::runtime::cook::NodeCookState::Idle)
            {
                // No active cook state: visualize the chain as queued.
                to_state = crate::nodes::runtime::cook::NodeCookState::Queued;
            }
            let mut base_rgba = bevy_egui::egui::Rgba::from(stroke.color).to_array();
            if matches!(to_state, crate::nodes::runtime::cook::NodeCookState::Failed) {
                base_rgba = bevy_egui::egui::Rgba::from(Color32::from_rgb(220, 60, 60)).to_array();
            }
            let idx = to_state.as_u8() as usize;
            let (mut speed, mut flow_k, mut blink_hz, mut thick_mul, pulse_w, pulse_spacing) = (
                s.wire_cook_speed[idx],
                s.wire_cook_flow_k[idx],
                s.wire_cook_blink_hz[idx],
                s.wire_cook_thick_mul[idx],
                s.wire_cook_pulse_w[idx],
                s.wire_cook_spacing[idx],
            );
            // Hard stop: if cook is not active, never animate.
            if !cook_active && !busy_to {
                speed = 0.0;
                flow_k = 0.0;
                blink_hz = 0.0;
            }
            let phase = {
                let u = connection.id.as_u128();
                let x = (u as u64) ^ ((u >> 64) as u64);
                (x as f32) * (1.0 / (u64::MAX as f32))
            };
            if is_selected {
                flow_k = flow_k.max(s.wire_cook_selected_flow_k_min);
                thick_mul = thick_mul.max(s.wire_cook_selected_thick_mul_min);
            }
            if !in_scope {
                base_rgba = mul_alpha(base_rgba, s.wire_cook_scope_dim_alpha);
                flow_k = 0.0;
                speed = 0.0;
                blink_hz = 0.0;
            }
            base_rgba = mul_alpha(base_rgba, opacity_mul);

            for w in pts.windows(2) {
                let (p0, p3) = (w[0], w[1]);
                let control_offset = Vec2::new((p3.x - p0.x).abs() * 0.4, 0.0);
                let p1 = p0 + control_offset;
                let p2 = p3 - control_offset;
                let min_x = p0.x.min(p1.x).min(p2.x).min(p3.x);
                let max_x = p0.x.max(p1.x).max(p2.x).max(p3.x);
                let min_y = p0.y.min(p1.y).min(p2.y).min(p3.y);
                let max_y = p0.y.max(p1.y).max(p2.y).max(p3.y);
                let curve_rect =
                    Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
                        .expand(stroke.width * 2.0 + 10.0)
                        .intersect(ui.clip_rect());
                if curve_rect.width() <= 0.0 || curve_rect.height() <= 0.0 {
                    continue;
                }
                let uniform = SdfFlowCurveUniform {
                    p01: [p0.x, p0.y, p1.x, p1.y],
                    p23: [p2.x, p2.y, p3.x, p3.y],
                    color: base_rgba,
                    params0: [
                        stroke.width * thick_mul,
                        s.wire_cook_softness[idx],
                        pulse_w,
                        pulse_spacing,
                    ],
                    params1: [speed, phase, flow_k, blink_hz],
                    screen_params: [screen_size.x, screen_size.y, 0.0, 0.0],
                };
                painter.add(create_sdf_flow_curve_callback(curve_rect, uniform, frame_id));
            }

            // Houdini-like relay points (always visible; brighter when selected).
            if !connection.waypoints.is_empty() {
                let r = if is_selected { 5.0 } else { 4.0 };
                let fill = if is_selected {
                    Color32::from_rgba_unmultiplied(235, 235, 240, (220.0 * opacity_mul) as u8)
                } else {
                    Color32::from_rgba_unmultiplied(210, 210, 215, (140.0 * opacity_mul) as u8)
                };
                let st = if is_selected {
                    Stroke::new(1.0, Color32::from_gray(30).linear_multiply(opacity_mul))
                } else {
                    Stroke::new(1.0, Color32::from_gray(50).linear_multiply(opacity_mul))
                };
                for wp in &connection.waypoints {
                    let sp = editor_rect.min + wp.to_vec2() * zoom + pan;
                    painter.circle_filled(sp, r, fill);
                    painter.circle_stroke(sp, r, st);
                }
            }
        }
    }
}

pub fn draw_ghost_links(
    ui: &mut egui::Ui,
    editor: &NodeEditorTab,
    ghost: &crate::tabs_system::node_editor::state::GhostGraph,
    editor_rect: Rect,
    opacity_mul: f32,
) {
    let stroke = Stroke::new(
        2.0,
        Color32::from_gray(180).linear_multiply(opacity_mul.clamp(0.0, 1.0)),
    );
    let painter = ui.painter();
    let screen_size = ui.ctx().screen_rect().size();
    let frame_id = ui.ctx().cumulative_frame_nr();
    for (from_id, to_id) in &ghost.links {
        let Some(from) = ghost.nodes.iter().find(|n| &n.id == from_id) else {
            continue;
        };
        let Some(to) = ghost.nodes.iter().find(|n| &n.id == to_id) else {
            continue;
        };

        let port_offset = 10.0 * editor.zoom.sqrt();
        let from_min =
            editor_rect.min.to_vec2() + editor.pan + from.position.to_vec2() * editor.zoom;
        let to_min = editor_rect.min.to_vec2() + editor.pan + to.position.to_vec2() * editor.zoom;
        let from_rect = Rect::from_min_size(from_min.to_pos2(), from.size * editor.zoom);
        let to_rect = Rect::from_min_size(to_min.to_pos2(), to.size * editor.zoom);
        let from_n = from.outputs.len().max(1) as f32;
        let to_n = to.inputs.len().max(1) as f32;
        let start_y = from_rect.top() + from_rect.height() * (1.0 / (from_n + 1.0));
        let end_y = match to.input_style {
            InputStyle::Individual => to_rect.top() + to_rect.height() * (1.0 / (to_n + 1.0)),
            InputStyle::Bar | InputStyle::Collection => to_rect.center().y,
        };
        let start = egui::pos2(from_rect.right() + port_offset, start_y);
        let end = egui::pos2(to_rect.left() - port_offset, end_y);

        let control_offset = Vec2::new((end.x - start.x).abs() * 0.4, 0.0);
        let start_control = start + control_offset;
        let end_control = end - control_offset;

        let min_x = start.x.min(start_control.x).min(end_control.x).min(end.x);
        let max_x = start.x.max(start_control.x).max(end_control.x).max(end.x);
        let min_y = start.y.min(start_control.y).min(end_control.y).min(end.y);
        let max_y = start.y.max(start_control.y).max(end_control.y).max(end.y);
        let curve_rect = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
            .expand(stroke.width * 2.0 + 10.0)
            .intersect(ui.clip_rect());
        if curve_rect.width() <= 0.0 || curve_rect.height() <= 0.0 {
            continue;
        }

        let color_rgba = mul_alpha(
            bevy_egui::egui::Rgba::from(stroke.color).to_array(),
            opacity_mul,
        );
        let uniform = SdfCurveUniform {
            p0: [start.x, start.y],
            p1: [start_control.x, start_control.y],
            p2: [end_control.x, end_control.y],
            p3: [end.x, end.y],
            color: color_rgba,
            thickness: stroke.width,
            softness: 1.0,
            screen_size: [screen_size.x, screen_size.y],
            _pad: [0.0; 2],
        };
        painter.add(create_sdf_curve_callback(curve_rect, uniform, frame_id));
    }
}

pub fn draw_insertion_preview(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let dragged_node_id = match context.ui_state.dragged_node_id {
        Some(id) => id,
        None => return,
    };

    let is_primary_down = ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    if !is_primary_down {
        // Keep previous insertion_target so drop logic can still see it on release frame
        return;
    }

    // Recalculate insertion target while dragging
    editor.insertion_target = None;

    let pointer_pos = if let Some(p) = ui.ctx().pointer_interact_pos() {
        p
    } else {
        return;
    };

    let snap_radius: f32 = 25.0;
    let snap_radius_sq = snap_radius * snap_radius;

    let root_graph = &context.node_graph_res.0;
    let node_graph = crate::tabs_system::node_editor::cda::navigation::graph_snapshot_by_path(
        &root_graph,
        &editor.cda_state.breadcrumb(),
    );

    // Determine which ports we will use on the dragged node (same rule as handle_node_insertion)
    let (dragged_input_port, dragged_output_port) =
        if let Some(dragged_node) = node_graph.nodes.get(&dragged_node_id) {
            (
                dragged_node.inputs.keys().next().cloned(),
                dragged_node.outputs.keys().next().cloned(),
            )
        } else {
            return;
        };

    let mut best_dist = snap_radius_sq;
    let mut best_conn_id = None;
    let mut best_segment = None;

    for (conn_id, conn) in &node_graph.connections {
        if conn.from_node == dragged_node_id || conn.to_node == dragged_node_id {
            continue;
        }

        let from_key = (conn.from_node, conn.from_port.clone());
        let to_key = (conn.to_node, conn.to_port.clone());

        let (start_pos, _) = if let Some(v) = editor.port_locations.get(&from_key) {
            v
        } else {
            continue;
        };
        let (end_pos, _) = if let Some(v) = editor.port_locations.get(&to_key) {
            v
        } else {
            continue;
        };

        let mut pts: Vec<Pos2> = Vec::with_capacity(2 + conn.waypoints.len());
        pts.push(*start_pos);
        for wp in &conn.waypoints {
            pts.push(editor_rect.min + wp.to_vec2() * editor.zoom + editor.pan);
        }
        pts.push(*end_pos);
        let mut dist_sq = f32::INFINITY;
        for w in pts.windows(2) {
            dist_sq = dist_sq.min(point_to_bezier_distance_sq(pointer_pos, w[0], w[1]));
        }
        if dist_sq < best_dist {
            best_dist = dist_sq;
            best_conn_id = Some(*conn_id);
            best_segment = Some((*start_pos, *end_pos));
        }
    }

    if let (Some(conn_id), Some((from_pos, to_pos)), Some(in_port), Some(out_port)) = (
        best_conn_id,
        best_segment,
        dragged_input_port,
        dragged_output_port,
    ) {
        // Look up the dragged node's input/output port positions in screen space
        let in_key = (dragged_node_id, in_port);
        let out_key = (dragged_node_id, out_port);

        let (insert_in_pos, _) = match editor.port_locations.get(&in_key) {
            Some(v) => v,
            None => return,
        };
        let (insert_out_pos, _) = match editor.port_locations.get(&out_key) {
            Some(v) => v,
            None => return,
        };

        editor.insertion_target = Some(conn_id);
        let stroke = Stroke::new(2.0, Color32::from_gray(230));

        // Preview the two new curved connections: source -> dragged input, dragged output -> dest
        draw_dashed_bezier(ui, from_pos, *insert_in_pos, stroke);
        draw_dashed_bezier(ui, *insert_out_pos, to_pos, stroke);
    }
}

pub struct GridRippleState {
    pub active: bool,
    pub center: Pos2, // screen coords
    pub start_time: f32,
}

pub fn draw_grid(
    ui: &mut egui::Ui,
    rect: Rect,
    pan: Vec2,
    zoom: f32,
    s: &crate::node_editor_settings::NodeEditorSettings,
    inv: &mut UiInvalidator,
    ripple: Option<&GridRippleState>,
) {
    // Fill background to prevent 3D viewport bleed-through
    ui.painter().rect_filled(rect, 0.0, ui.visuals().panel_fill);

    let painter = ui.painter();
    let screen_size = ui.ctx().screen_rect().size();
    let grid_size = (s.grid_size_base.max(1.0) * zoom).max(1.0);
    let line_width = (s.grid_line_width_base.max(0.0) * zoom.sqrt()).max(0.0);
    let c = s.grid_color;
    let color =
        bevy_egui::egui::Rgba::from(Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3]))
            .to_array();

    // Grid animation trigger: deep-think only (not mouse hover).
    // When deep-think is active we pass a non-empty `ripple` from the node editor (even if we don't use the ripple wave itself).
    let target = if ripple.is_some_and(|r| r.active) {
        1.0
    } else {
        0.0
    };
    let dt = ui
        .input(|i| {
            if i.unstable_dt > 0.0 {
                i.unstable_dt
            } else {
                1.0 / 60.0
            }
        })
        .clamp(0.0, 0.1) as f32;
    let speed = 8.0;
    let a = 1.0 - (-dt * speed).exp();
    let state_id = ui.make_persistent_id("grid_hover_state");
    let hover_state = ui.ctx().data_mut(|d| {
        let v = d.get_temp::<f32>(state_id).unwrap_or(target);
        let nv = v + (target - v) * a;
        d.insert_temp(state_id, nv);
        nv
    });

    // Ripple params for deep thinking mode
    let time = ui.input(|i| i.time) as f32;
    let (ripple_phase, ripple_center, _ripple_intensity) =
        if let Some(r) = ripple.filter(|r| r.active) {
            let phase = time - r.start_time;
            let center = r.center - rect.min; // local coords
            (phase, [center.x, center.y], 1.0f32)
        } else {
            (0.0, [0.0, 0.0], 0.0f32)
        };
    let ripple_intensity = 0.0f32; // disable ripple wave; use grid's built-in hover animation as deep-think indicator

    let uniform = SdfGridUniform {
        rect_min_size: [rect.min.x, rect.min.y, rect.width(), rect.height()],
        pan_grid: [pan.x, pan.y, grid_size, line_width],
        color,
        time_hover: [
            time,
            hover_state,
            s.grid_major_alpha_mul.max(0.0),
            ripple_phase,
        ],
        screen_params: [
            screen_size.x,
            screen_size.y,
            ripple_center[0],
            ripple_center[1],
        ],
        ripple_params: [ripple_intensity, 400.0, 60.0, 0.8], // intensity, speed, wavelength, decay
    };
    let frame_id = (time * 1000.0) as u64;
    painter.add(create_sdf_grid_callback(rect, uniform, frame_id));

    // Repaint only during deep-think transition (avoid hover-driven repaints).
    let needs_repaint = (hover_state - target).abs() > 0.01;
    if needs_repaint {
        inv.request_repaint_after_tagged(
            "node_editor/grid_anim",
            std::time::Duration::from_secs_f32(1.0 / 60.0),
            RepaintCause::Animation,
        );
    }
}

pub fn draw_dashed_line(
    ui: &mut egui::Ui,
    points: [Pos2; 2],
    color: Color32,
    dash_length: f32,
    gap_length: f32,
) {
    let p0 = points[0];
    let p3 = points[1];
    if (p3 - p0).length_sq() < 1e-6 {
        return;
    }
    let v = (p3 - p0) / 3.0;
    let p1 = p0 + v;
    let p2 = p0 + v * 2.0;

    let painter = ui.painter();
    let screen_size = ui.ctx().screen_rect().size();
    let min_x = p0.x.min(p1.x).min(p2.x).min(p3.x);
    let max_x = p0.x.max(p1.x).max(p2.x).max(p3.x);
    let min_y = p0.y.min(p1.y).min(p2.y).min(p3.y);
    let max_y = p0.y.max(p1.y).max(p2.y).max(p3.y);
    let rect = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
        .expand(8.0)
        .intersect(ui.clip_rect());
    if rect.width() <= 0.0 || rect.height() <= 0.0 {
        return;
    }
    let color_rgba = bevy_egui::egui::Rgba::from(color).to_array();
    let uniform = SdfDashedCurveUniform {
        p01: [p0.x, p0.y, p1.x, p1.y],
        p23: [p2.x, p2.y, p3.x, p3.y],
        color: color_rgba,
        params: [1.0, 1.0, dash_length, gap_length],
        screen_params: [screen_size.x, screen_size.y, 0.0, 0.0],
    };
    let frame_id = ui.ctx().cumulative_frame_nr();
    painter.add(create_sdf_dashed_curve_callback(rect, uniform, frame_id));
}

pub fn draw_nodes(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let nodes_len = editor.cached_nodes.len();

    let mut clicked_node = None;
    let mut currently_hovered_node = None;

    for idx in 0..nodes_len {
        let node_id = editor.cached_nodes[idx].id;
        let mut pos = editor.cached_nodes[idx].position;
        let size = editor.cached_nodes[idx].size;
        let is_selected = context.ui_state.selected_nodes.contains(&node_id);
        let mut node_rect_screen = Rect::from_min_size(
            editor_rect.min + pos.to_vec2() * editor.zoom + editor.pan,
            size * editor.zoom,
        );

        // Move node response logic outside draw_and_interact to handle selection state centrally if needed
        let node_response = ui.interact(
            node_rect_screen,
            ui.make_persistent_id(node_id),
            Sense::click_and_drag(),
        );

        if node_response.drag_started_by(egui::PointerButton::Primary) {
            editor.selection_start = None; // Cancel box selection if dragging node
            if is_selected {
                context.ui_state.dragged_node_id = Some(node_id);
            } else {
                if !ui.input(|i| i.modifiers.shift) {
                    context.ui_state.selected_nodes.clear();
                }
                context.ui_state.selected_nodes.insert(node_id);
                context.ui_state.dragged_node_id = Some(node_id);
            }
            // Capture drag-start positions for undo (only once per drag gesture)
            editor.drag_start_positions.clear();
            editor.box_rect_start.clear();
            editor.sticky_rect_start.clear();
            {
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        for id in &context.ui_state.selected_nodes {
                            if let Some(n) = node_graph.nodes.get(id) {
                                editor.drag_start_positions.insert(*id, n.position);
                            }
                        }
                        for bid in &context.ui_state.selected_network_boxes {
                            if let Some(b) = node_graph.network_boxes.get(bid) {
                                editor.box_rect_start.insert(*bid, b.rect);
                            }
                        }
                        for sid in &context.ui_state.selected_sticky_notes {
                            if let Some(s) = node_graph.sticky_notes.get(sid) {
                                editor.sticky_rect_start.insert(*sid, s.rect);
                            }
                        }
                    },
                );
            }
        }

        // Move Nodes
        if context.ui_state.dragged_node_id == Some(node_id)
            && node_response.dragged_by(egui::PointerButton::Primary)
        {
            let mut drag_delta = node_response.drag_delta() / editor.zoom;

            // Restore Snapping
            if context.ui_state.selected_nodes.len() == 1 {
                editor.snap_lines.clear();
                let mut proposed_pos = pos + drag_delta;
                let dragged_size = size;
                for other in &editor.cached_nodes {
                    if other.id == node_id {
                        continue;
                    }
                    let other_rect = Rect::from_min_size(other.position, other.size);
                    let current_rect = Rect::from_min_size(proposed_pos, dragged_size);
                    let snap = check_and_apply_snap(
                        current_rect,
                        other_rect,
                        editor.zoom,
                        10.0,
                        editor_rect.min,
                        editor.pan,
                        &mut editor.snap_lines,
                    );
                    if snap.x != 0.0 {
                        drag_delta.x += snap.x;
                        proposed_pos.x += snap.x;
                    }
                    if snap.y != 0.0 {
                        drag_delta.y += snap.y;
                        proposed_pos.y += snap.y;
                    }
                }
            } else {
                editor.snap_lines.clear();
            }

            {
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        for selected_id in &context.ui_state.selected_nodes {
                            if let Some(n) = node_graph.nodes.get_mut(selected_id) {
                                n.position += drag_delta;
                            }
                        }
                        for bid in &context.ui_state.selected_network_boxes {
                            if let Some(b) = node_graph.network_boxes.get_mut(bid) {
                                b.rect.min += drag_delta;
                                b.rect.max += drag_delta;
                            }
                        }
                        for sid in &context.ui_state.selected_sticky_notes {
                            if let Some(s) = node_graph.sticky_notes.get_mut(sid) {
                                s.rect.min += drag_delta;
                                s.rect.max += drag_delta;
                            }
                        }
                        // Houdini-style: auto-fit ForEach network boxes to their contained nodes while dragging.
                        autofit_foreach_boxes(node_graph);
                    },
                );
            }
            // Keep local cache in sync for immediate visuals.
            for n in editor.cached_nodes.iter_mut() {
                if context.ui_state.selected_nodes.contains(&n.id) {
                    n.position += drag_delta;
                }
            }
            editor.geometry_rev = editor.geometry_rev.wrapping_add(1);

            // Recompute current node rect from updated cache (avoid stale rect during this frame).
            pos = editor.cached_nodes[idx].position;
            node_rect_screen = Rect::from_min_size(
                editor_rect.min + pos.to_vec2() * editor.zoom + editor.pan,
                size * editor.zoom,
            );
        }

        // Interaction + hitboxes only (no painting): keep retained nodes cheap on idle.
        {
            let snap: &NodeSnapshot = &editor.cached_nodes[idx];
            let (interaction, new_last_clicked) = interact_node_ports(
                ui,
                snap,
                node_rect_screen,
                editor.zoom,
                context.node_editor_settings,
                &mut editor.port_locations,
                editor.last_clicked_port.clone(),
                node_response.hovered(),
                node_response.clicked(),
            );
            editor.last_clicked_port = new_last_clicked;
            if let Some((nid, pid)) = interaction.dragged_on_output_port {
                if context.ui_state.selected_nodes.len() > 1
                    && context.ui_state.selected_nodes.contains(&nid)
                {
                    let mut v = Vec::new();
                    for sid in &context.ui_state.selected_nodes {
                        let port = editor
                            .cached_nodes
                            .iter()
                            .find(|n| n.id == *sid)
                            .and_then(|n| {
                                if n.outputs.iter().any(|p| p == &pid) {
                                    Some(pid.clone())
                                } else {
                                    n.outputs.first().cloned()
                                }
                            })
                            .unwrap_or_else(|| pid.clone());
                        v.push((*sid, port));
                    }
                    editor.pending_connections_from = v;
                    editor.pending_connection_from = None;
                    editor.snapped_to_port = None;
                } else {
                    editor.pending_connections_from.clear();
                    editor.pending_connection_from = Some((nid, pid));
                }
                editor.did_start_connection_this_frame = true;
            }
            if let Some((nid, pid)) = interaction.clicked_on_output_port {
                if context.ui_state.selected_nodes.len() > 1
                    && context.ui_state.selected_nodes.contains(&nid)
                {
                    let mut v = Vec::new();
                    for sid in &context.ui_state.selected_nodes {
                        let port = editor
                            .cached_nodes
                            .iter()
                            .find(|n| n.id == *sid)
                            .and_then(|n| {
                                if n.outputs.iter().any(|p| p == &pid) {
                                    Some(pid.clone())
                                } else {
                                    n.outputs.first().cloned()
                                }
                            })
                            .unwrap_or_else(|| pid.clone());
                        v.push((*sid, port));
                    }
                    editor.pending_connections_from = v;
                    editor.pending_connection_from = None;
                    editor.snapped_to_port = None;
                } else {
                    editor.pending_connections_from.clear();
                    editor.pending_connection_from = Some((nid, pid));
                    editor.snapped_to_port = None;
                }
                editor.did_start_connection_this_frame = true;
            }
            if interaction.hovered {
                currently_hovered_node = Some(node_id);
            }
            if interaction.clicked {
                clicked_node = Some(node_id);
            }
        }
    }

    // Handle clicks logic after loop
    if let Some(node_id) = clicked_node {
        if editor.pending_connection_from.is_none() && editor.pending_connections_from.is_empty() {
            if !ui.input(|i| i.modifiers.shift) {
                context.ui_state.selected_nodes.clear();
            }
            context.ui_state.selected_nodes.insert(node_id);
            context.ui_state.last_selected_node_id = Some(node_id);
        }
    }

    // Radial Menu Trigger (avoid writing state if unchanged; cursor-move frames are common).
    if let Some(node_id) = currently_hovered_node {
        if context.ui_state.radial_menu_state.node_id != Some(node_id) {
            context.ui_state.radial_menu_state.node_id = Some(node_id);
        }
    }

    // Handle node drag release and auto-insert into connection
    let any_released = ui.input(|i| i.pointer.any_released());
    if any_released {
        if let Some(dragged_id) = context.ui_state.dragged_node_id {
            if let Some(target_conn) = editor.insertion_target {
                let current_time = ui.ctx().input(|i| i.time);
                {
                    let root_graph = &mut context.node_graph_res.0;
                    crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                        root_graph,
                        &editor.cda_state,
                        |node_graph| {
                            handle_node_insertion(
                                node_graph,
                                target_conn,
                                dragged_id,
                                &mut editor.node_animations,
                                current_time,
                            );
                            let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> =
                                Vec::new();
                            if !editor.drag_start_positions.is_empty() {
                                let mut items = Vec::new();
                                for (id, old) in editor.drag_start_positions.iter() {
                                    if let Some(n) = node_graph.nodes.get(id) {
                                        let new = n.position;
                                        if *old != new {
                                            items.push((*id, *old, new));
                                        }
                                    }
                                }
                                if !items.is_empty() {
                                    cmds.push(Box::new(CmdMoveNodes::new(items)));
                                }
                                editor.drag_start_positions.clear();
                            }
                            if let Some(n) = node_graph.nodes.get(&dragged_id) {
                                let mut ins: Vec<_> = n.inputs.keys().cloned().collect();
                                ins.sort();
                                let mut outs: Vec<_> = n.outputs.keys().cloned().collect();
                                outs.sort();
                                if let (Some(in_port), Some(out_port)) =
                                    (ins.into_iter().next(), outs.into_iter().next())
                                {
                                    let mut cmd = CmdInsertNodeOnConnection::new(
                                        target_conn,
                                        dragged_id,
                                        in_port,
                                        out_port,
                                    );
                                    crate::cunning_core::command::Command::apply(
                                        &mut cmd, node_graph,
                                    );
                                    cmds.push(Box::new(cmd));
                                }
                            }
                            if !cmds.is_empty() {
                                context
                                    .node_editor_state
                                    .record(Box::new(CmdBatch::new("Drag Insert", cmds)));
                            }
                        },
                    );
                }
                context.graph_changed_writer.write_default();
            } else {
                // Normal drag: one gesture => one undo entry.
                if !editor.drag_start_positions.is_empty() {
                    {
                        let root_graph = &mut context.node_graph_res.0;
                        crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                            root_graph,
                            &editor.cda_state,
                            |node_graph| {
                            let mut items = Vec::new();
                            for (id, old) in editor.drag_start_positions.iter() {
                                if let Some(n) = node_graph.nodes.get(id) {
                                    let new = n.position;
                                    if *old != new {
                                        items.push((*id, *old, new));
                                    }
                                }
                            }
                            if !items.is_empty() {
                                context
                                    .node_editor_state
                                    .record(Box::new(CmdMoveNodes::new(items)));
                                context.graph_changed_writer.write_default();
                            }
                            editor.drag_start_positions.clear();
                            },
                        );
                    }
                }
            }

            // Clear drag state regardless of whether an insertion happened
            context.ui_state.dragged_node_id = None;
            editor.insertion_target = None;
        }
    }
}

pub fn rebuild_port_locations(
    editor: &mut NodeEditorTab,
    s: &crate::node_editor_settings::NodeEditorSettings,
    editor_rect: Rect,
) {
    editor.port_locations.clear();
    let zoom = editor.zoom;
    let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
    let io_inset_y = s.node_io_inset_y_base.max(0.0) * zoom;
    for n in &editor.cached_nodes {
        let header_h = n.header_h.max(10.0) * zoom;
        let r = Rect::from_min_size(
            editor_rect.min + n.position.to_vec2() * zoom + editor.pan,
            n.size * zoom,
        );
        let io_top = r.top() + header_h;
        let io_h = (r.height() - header_h - io_inset_y).max(1.0);
        let row_h = 18.0 * zoom;
        let row_top = io_top + (6.0 * zoom).max(2.0);
        match n.input_style {
            InputStyle::Individual => {
                for (i, pid) in n.inputs.iter().enumerate() {
                    let p = egui::pos2(r.left() - port_offset, row_top + row_h * (i as f32 + 0.5));
                    editor.port_locations.insert((n.id, pid.clone()), (p, None));
                }
            }
            InputStyle::Bar | InputStyle::Collection => {
                if let Some(pid) = n.inputs.first() {
                    let h = io_h * s.layout_bar_width_ratio.clamp(0.1, 1.0);
                    let p = egui::pos2(r.left() - port_offset, io_top + io_h * 0.5);
                    editor
                        .port_locations
                        .insert((n.id, pid.clone()), (p, Some(h)));
                }
            }
        }
        for (i, pid) in n.outputs.iter().enumerate() {
            let p = egui::pos2(r.right() + port_offset, row_top + row_h * (i as f32 + 0.5));
            editor.port_locations.insert((n.id, pid.clone()), (p, None));
        }
    }
}

pub fn rebuild_hit_cache(
    editor: &mut NodeEditorTab,
    s: &crate::node_editor_settings::NodeEditorSettings,
    editor_rect: Rect,
    key: u64,
) {
    if editor.hit_cache_key == key && !editor.hit_cache.nodes.is_empty() {
        return;
    }
    editor.hit_cache_key = key;
    editor.hit_cache.nodes.clear();
    editor.hit_cache.buckets.clear();

    // Graph-space cache: invariant under zoom/pan (prevents wheel-induced rebuild storms).
    let bucket_size = editor.hit_cache.bucket_size;
    let zoom_min = 0.1f32;
    let port_offset_graph_max = s.port_offset_base.max(0.0) / zoom_min.sqrt();
    let hover_r_graph_max =
        (s.port_radius_base.max(0.0) * s.port_hover_radius_mul.max(1.0)) / zoom_min.sqrt();
    let margin_graph = (port_offset_graph_max + hover_r_graph_max * 2.0 + 2.0).max(8.0);

    for (idx, n) in editor.cached_nodes.iter().enumerate() {
        let body = Rect::from_min_size(n.position, n.size);
        let mut visual_rect = body.expand(margin_graph);
        if n.is_bypassed || n.is_locked {
            let sidebar_w = body.width() * s.layout_sidebar_width_ratio.clamp(0.0, 0.5);
            visual_rect = visual_rect.expand2(Vec2::new(sidebar_w, 0.0));
        }

        editor.hit_cache.nodes.push(NodeHit {
            id: n.id,
            rect: visual_rect,
            logical_rect: body,
            input_style: n.input_style,
            inputs: n.inputs.clone(),
            outputs: n.outputs.clone(),
        });

        let min_x = (visual_rect.min.x / bucket_size).floor() as i32;
        let max_x = (visual_rect.max.x / bucket_size).floor() as i32;
        let min_y = (visual_rect.min.y / bucket_size).floor() as i32;
        let max_y = (visual_rect.max.y / bucket_size).floor() as i32;
        for x in min_x..=max_x {
            for y in min_y..=max_y {
                editor
                    .hit_cache
                    .buckets
                    .entry((x, y))
                    .or_default()
                    .push(idx);
            }
        }
    }
}

pub fn handle_node_input(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let pointer = ui.ctx().pointer_interact_pos();
    let pointer_graph =
        pointer.map(|p| ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2());
    let canvas_hovered = pointer.is_some_and(|p| editor_rect.contains(p));
    // If the node editor's own popup menu is open, never run canvas hit-tests or pointer actions.
    // (Don't use global `any_popup_open()`: other panels may open popups and would break node interaction.)
    if !matches!(
        editor.menu_state,
        crate::tabs_system::node_editor::state::MenuState::None
    ) {
        editor.hovered_port = None;
        return;
    }

    // Optimization: Skip expensive hit tests if pointer is static and not interacting.
    // BUT always run if we are in a dragging state (node or connection) to ensure consistency.
    let is_interacting = context.ui_state.dragged_node_id.is_some()
        || editor.pending_connection_from.is_some()
        || !editor.pending_connections_from.is_empty()
        || editor.selection_start.is_some();

    let popup_open = ui.ctx().memory(|m| {
        #[allow(deprecated)]
        {
            m.any_popup_open()
        }
    });
    let pointer_moved = ui.input(|i| {
        i.pointer.velocity() != Vec2::ZERO
            || i.pointer.any_click()
            || i.pointer.any_down()
            || i.pointer.any_released()
            || (!popup_open && canvas_hovered && i.raw_scroll_delta != Vec2::ZERO)
    });

    if !pointer_moved && !is_interacting {
        return;
    }

    let primary_pressed = ui.input(|i| i.pointer.primary_pressed());
    let primary_down = ui.input(|i| i.pointer.primary_down());
    let primary_released = ui.input(|i| i.pointer.primary_released());
    let shift = ui.input(|i| i.modifiers.shift);
    let s = context.node_editor_settings;
    let zoom = editor.zoom;
    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
    let hover_r = port_radius * s.port_hover_radius_mul.max(1.0);
    let _hover_r_sq = hover_r * hover_r;
    let bucket_size = editor.hit_cache.bucket_size;
    let candidates = |p: Pos2| -> Option<&[usize]> {
        editor
            .hit_cache
            .buckets
            .get(&(
                (p.x / bucket_size).floor() as i32,
                (p.y / bucket_size).floor() as i32,
            ))
            .map(|v| v.as_slice())
    };

    // Hover for radial menu AND port highlighting
    if let (Some(p), Some(pg)) = (pointer, pointer_graph) {
        if editor_rect.contains(p) {
            // Port Hover Logic
            let mut hit_port: Option<(crate::nodes::NodeId, crate::nodes::PortId, Rect)> = None;
            if let Some(v) = candidates(pg) {
                for &i in v.iter().rev() {
                    if let Some(n) = editor.hit_cache.nodes.get(i) {
                        // Compute per-node port hit boxes in SCREEN space (depends on zoom/pan).
                        let zoom = editor.zoom;
                        let port_radius =
                            context.node_editor_settings.port_radius_base.max(0.0) * zoom.sqrt();
                        let port_offset =
                            context.node_editor_settings.port_offset_base.max(0.0) * zoom.sqrt();
                        let hover_radius = port_radius
                            * context.node_editor_settings.port_hover_radius_mul.max(1.0);
                        let r = Rect::from_min_size(
                            editor_rect.min + n.logical_rect.min.to_vec2() * zoom + editor.pan,
                            n.logical_rect.size() * zoom,
                        );
                        // Check Outputs (right side)
                        {
                            let cnt = n.outputs.len().max(1) as f32;
                            for (k, pid) in n.outputs.iter().enumerate() {
                                let pp = egui::pos2(
                                    r.right() + port_offset,
                                    r.top() + r.height() * (k as f32 + 1.0) / (cnt + 1.0),
                                );
                                let rect =
                                    Rect::from_center_size(pp, Vec2::splat(hover_radius * 2.0));
                                if rect.contains(p) {
                                    hit_port = Some((n.id, pid.clone(), rect));
                                    break;
                                }
                            }
                        }
                        if hit_port.is_some() {
                            break;
                        }
                        // Check Inputs (left side)
                        match n.input_style {
                            InputStyle::Individual => {
                                let cnt = n.inputs.len().max(1) as f32;
                                for (k, pid) in n.inputs.iter().enumerate() {
                                    let pp = egui::pos2(
                                        r.left() - port_offset,
                                        r.top() + r.height() * (k as f32 + 1.0) / (cnt + 1.0),
                                    );
                                    let rect =
                                        Rect::from_center_size(pp, Vec2::splat(hover_radius * 2.0));
                                    if rect.contains(p) {
                                        hit_port = Some((n.id, pid.clone(), rect));
                                        break;
                                    }
                                }
                            }
                            InputStyle::Bar | InputStyle::Collection => {
                                if let Some(pid) = n.inputs.first() {
                                    let h = r.height()
                                        * context
                                            .node_editor_settings
                                            .layout_bar_width_ratio
                                            .clamp(0.1, 1.0);
                                    let w = port_radius * 1.5;
                                    let pp = egui::pos2(r.left() - port_offset, r.center().y);
                                    let rect =
                                        Rect::from_center_size(pp, Vec2::new(w, h)).expand(5.0);
                                    if rect.contains(p) {
                                        hit_port = Some((n.id, pid.clone(), rect));
                                    }
                                }
                            }
                        }
                        if hit_port.is_some() {
                            break;
                        }
                    }
                }
            }
            editor.hovered_port = hit_port;

            // Hovered node for radial menu: body-only hit in graph space (prevents ring-area from being treated as node hit).
            let mut hovered = None;
            if let Some(v) = candidates(pg) {
                for &i in v.iter().rev() {
                    if let Some(n) = editor.hit_cache.nodes.get(i) {
                        if n.logical_rect.contains(pg) {
                            hovered = Some(n.id);
                            break;
                        }
                    }
                }
            }

            // Keep radial menu alive when moving from the node to the radial ring area.
            if hovered.is_none() {
                if let Some(prev) = context.ui_state.radial_menu_state.node_id {
                    if let Some(prev_node) = editor.cached_nodes.iter().find(|n| n.id == prev) {
                        let node_rect = Rect::from_min_size(
                            editor_rect.min + prev_node.position.to_vec2() * zoom + editor.pan,
                            prev_node.size * zoom,
                        );
                        let menu_rect =
                            crate::tabs_system::node_editor::mathematic::calculate_actual_menu_rect(
                                node_rect, zoom, s,
                            );
                        if menu_rect.contains(p) {
                            hovered = Some(prev);
                        }
                    }
                }
            }
            if context.ui_state.radial_menu_state.node_id != hovered {
                context.ui_state.radial_menu_state.node_id = hovered;
            }
        }
    } else {
        editor.hovered_port = None;
    }

    // Begin actions.
    if primary_pressed {
        if let Some(p) = pointer {
            if editor_rect.contains(p) {
                // 1) output port hits first (start connection)
                let mut hit_out: Option<(crate::nodes::NodeId, crate::nodes::PortId)> = None;
                let pg = ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                if let Some(v) = candidates(pg) {
                    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
                    let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
                    let hover_radius = port_radius * s.port_hover_radius_mul.max(1.0);
                    for &i in v.iter().rev() {
                        if let Some(n) = editor.hit_cache.nodes.get(i) {
                            let r = Rect::from_min_size(
                                editor_rect.min + n.logical_rect.min.to_vec2() * zoom + editor.pan,
                                n.logical_rect.size() * zoom,
                            );
                            let cnt = n.outputs.len().max(1) as f32;
                            for (k, pid) in n.outputs.iter().enumerate() {
                                let pp = egui::pos2(
                                    r.right() + port_offset,
                                    r.top() + r.height() * (k as f32 + 1.0) / (cnt + 1.0),
                                );
                                let rect =
                                    Rect::from_center_size(pp, Vec2::splat(hover_radius * 2.0));
                                if rect.contains(p) {
                                    hit_out = Some((n.id, pid.clone()));
                                    break;
                                }
                            }
                        }
                        if hit_out.is_some() {
                            break;
                        }
                    }
                }
                if let Some((nid, pid)) = hit_out {
                    if context.ui_state.selected_nodes.len() > 1
                        && context.ui_state.selected_nodes.contains(&nid)
                    {
                        let mut v = Vec::new();
                        for sid in &context.ui_state.selected_nodes {
                            let port = editor
                                .cached_nodes
                                .iter()
                                .find(|n| n.id == *sid)
                                .and_then(|n| {
                                    if n.outputs.iter().any(|p| p == &pid) {
                                        Some(pid.clone())
                                    } else {
                                        n.outputs.first().cloned()
                                    }
                                })
                                .unwrap_or_else(|| pid.clone());
                            v.push((*sid, port));
                        }
                        editor.pending_connections_from = v;
                        editor.pending_connection_from = None;
                        editor.snapped_to_port = None;
                        editor.pending_wire_waypoints.clear();
                        editor.single_alt_was_down = false;
                        editor.selected_waypoint = None;
                        editor.waypoint_drag_old = None;
                        editor.did_start_connection_this_frame = true;
                        editor.last_clicked_port = Some((pid, ui.ctx().input(|i| i.time)));
                        return;
                    }
                    editor.pending_connections_from.clear();
                    editor.pending_connection_from = Some((nid, pid.clone()));
                    editor.snapped_to_port = None;
                    editor.pending_wire_waypoints.clear();
                    editor.single_alt_was_down = false;
                    editor.selected_waypoint = None;
                    editor.waypoint_drag_old = None;
                    editor.did_start_connection_this_frame = true;
                    editor.last_clicked_port = Some((pid, ui.ctx().input(|i| i.time)));
                    return;
                }

                // 2) connection / waypoint hit (select wire / move waypoint)
                {
                    let root_graph = &context.node_graph_res.0;
                    let g = crate::tabs_system::node_editor::cda::navigation::graph_snapshot_by_path(
                        &root_graph,
                        &editor.cda_state.breadcrumb(),
                    );
                    // Same merge-bar distribution rule as draw_connections.
                    let mut merge_port_connections: std::collections::HashMap<
                        (crate::nodes::NodeId, crate::nodes::PortId),
                        Vec<(i32, crate::nodes::ConnectionId)>,
                    > = std::collections::HashMap::new();
                    for (_cid, c) in &g.connections {
                        if let Some((_, Some(_))) =
                            editor.port_locations.get(&(c.to_node, c.to_port.clone()))
                        {
                            merge_port_connections
                                .entry((c.to_node, c.to_port.clone()))
                                .or_default()
                                .push((c.order, c.id));
                        }
                    }
                    for v in merge_port_connections.values_mut() {
                        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
                    }
                    let mut best_conn: Option<crate::nodes::ConnectionId> = None;
                    let mut best_dist = (10.0f32).powi(2);
                    let mut best_wp: Option<(crate::nodes::ConnectionId, usize)> = None;
                    let wp_r_sq = (7.0f32).powi(2);
                    for (cid, c) in &g.connections {
                        let (Some((start_pos, _)), Some((end_center_pos, end_bar_w))) = (
                            editor.port_locations.get(&(c.from_node, c.from_port.clone())),
                            editor.port_locations.get(&(c.to_node, c.to_port.clone())),
                        ) else {
                            continue;
                        };
                        let mut end_pos = *end_center_pos;
                        if let Some(bar_h) = end_bar_w {
                            if let Some(conns) =
                                merge_port_connections.get(&(c.to_node, c.to_port.clone()))
                            {
                                let count = conns.len();
                                if count > 1 {
                                    if let Some(index) =
                                        conns.iter().position(|(_, id)| *id == c.id)
                                    {
                                        let fraction = (index as f32 + 1.0) / (count as f32 + 1.0);
                                        let offset_y = (fraction * *bar_h) - (*bar_h / 2.0);
                                        end_pos.y = end_center_pos.y + offset_y;
                                    }
                                }
                            }
                        }
                        let mut pts: Vec<Pos2> = Vec::with_capacity(2 + c.waypoints.len());
                        pts.push(*start_pos);
                        for (i, wp) in c.waypoints.iter().enumerate() {
                            let sp = editor_rect.min + wp.to_vec2() * zoom + editor.pan;
                            if sp.distance_sq(p) <= wp_r_sq {
                                best_wp = Some((*cid, i));
                                break;
                            }
                            pts.push(sp);
                        }
                        if best_wp.is_some() {
                            break;
                        }
                        pts.push(end_pos);
                        for w in pts.windows(2) {
                            let d = point_to_bezier_distance_sq(p, w[0], w[1]);
                            if d < best_dist {
                                best_dist = d;
                                best_conn = Some(*cid);
                            }
                        }
                    }
                    if let Some((cid, wi)) = best_wp {
                        editor.selection_start = None;
                        if !shift {
                            context.ui_state.selected_nodes.clear();
                            context.ui_state.selected_network_boxes.clear();
                            context.ui_state.selected_promote_notes.clear();
                            context.ui_state.selected_sticky_notes.clear();
                            context.ui_state.selected_connections.clear();
                        }
                        context.ui_state.selected_connections.insert(cid);
                        editor.selected_waypoint = Some((cid, wi));
                        editor.waypoint_drag_old = g
                            .connections
                            .get(&cid)
                            .map(|c| (cid, c.waypoints.clone()));
                        return;
                    }
                    if let Some(cid) = best_conn {
                        editor.selection_start = None;
                        if !shift {
                            context.ui_state.selected_nodes.clear();
                            context.ui_state.selected_network_boxes.clear();
                            context.ui_state.selected_promote_notes.clear();
                            context.ui_state.selected_sticky_notes.clear();
                            context.ui_state.selected_connections.clear();
                        }
                        if shift && context.ui_state.selected_connections.contains(&cid) {
                            context.ui_state.selected_connections.remove(&cid);
                        } else {
                            context.ui_state.selected_connections.insert(cid);
                        }
                        editor.selected_waypoint = None;
                        editor.waypoint_drag_old = None;
                        return;
                    }
                }

                // 2) node body hit (select / arm drag)
                let mut hit_node = None;
                if let Some(v) = candidates(pg) {
                    for &i in v.iter().rev() {
                        if let Some(n) = editor.hit_cache.nodes.get(i) {
                            if n.logical_rect.contains(pg) {
                                hit_node = Some(n.id);
                                break;
                            }
                        }
                    }
                }
                if let Some(nid) = hit_node {
                    editor.selection_start = None;
                    let already_selected = context.ui_state.selected_nodes.contains(&nid);
                    if !shift && !already_selected {
                        context.ui_state.selected_nodes.clear();
                    }
                    context.ui_state.selected_nodes.insert(nid);
                    context.ui_state.last_selected_node_id = Some(nid);
                    context.ui_state.dragged_node_id = Some(nid);
                    editor.drag_pointer_last = Some(p);
                    editor.drag_start_positions.clear();
                }
            }
        }
    }

    // Dragging nodes (event-driven: only runs during input frames).
    if primary_down {
        if let (Some((cid, wi)), Some(pg), Some((cid0, _old))) =
            (editor.selected_waypoint, pointer_graph, editor.waypoint_drag_old.as_ref())
        {
            if *cid0 == cid {
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |g| {
                        let to = if let Some(c) = g.connections.get_mut(&cid) {
                            if wi < c.waypoints.len() {
                                c.waypoints[wi] = pg;
                                Some(c.to_node)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        if let Some(to) = to {
                            g.mark_dirty(to);
                        }
                    },
                );
                context.ui_invalidator.request_repaint_after_tagged(
                    "node_editor/waypoint_drag",
                    std::time::Duration::ZERO,
                    RepaintCause::Input,
                );
                rebuild_port_locations(editor, s, editor_rect);
                return;
            }
        }
        if let (Some(drag_id), Some(p), Some(last)) = (
            context.ui_state.dragged_node_id,
            pointer,
            editor.drag_pointer_last,
        ) {
            let delta_screen = p - last;
            if delta_screen != Vec2::ZERO {
                // Drag threshold: avoid turning small click jitter into a drag (fixes radial "Visible" button being unclickable).
                if editor.drag_start_positions.is_empty() && delta_screen.length_sq() < 4.0 {
                    return;
                }
                // First real drag frame: capture undo start positions and reset pointer anchor to avoid a jump.
                if editor.drag_start_positions.is_empty() {
                    // Capture undo start positions from the local cache to avoid locking the shared graph.
                    // The authoritative commit into the NodeGraph will be done once on release.
                    editor.pending_node_move_delta = Vec2::ZERO;
                    for id in &context.ui_state.selected_nodes {
                        if let Some(n) = editor.cached_nodes.iter().find(|n| n.id == *id) {
                            editor.drag_start_positions.insert(*id, n.position);
                        }
                    }
                    editor.drag_pointer_last = Some(p);
                    context.ui_invalidator.request_repaint_after_tagged(
                        "node_editor/node_drag_start",
                        std::time::Duration::ZERO,
                        RepaintCause::Input,
                    );
                    return;
                }
                let mut drag_delta = delta_screen / zoom;
                if context.ui_state.selected_nodes.len() == 1 {
                    editor.snap_lines.clear();
                    let pos = editor
                        .cached_nodes
                        .iter()
                        .find(|n| n.id == drag_id)
                        .map(|n| n.position)
                        .unwrap_or_default();
                    let size = editor
                        .cached_nodes
                        .iter()
                        .find(|n| n.id == drag_id)
                        .map(|n| n.size)
                        .unwrap_or_default();
                    let mut proposed = pos + drag_delta;

                    // Optimization: Use HitCache for snapping instead of iterating all nodes.
                    // We check buckets covering the proposed rect + snap margin.
                    let snap_margin = 20.0;
                    let search_rect = Rect::from_min_size(proposed, size).expand(snap_margin);
                    let min_bucket_x = (search_rect.min.x / bucket_size).floor() as i32;
                    let max_bucket_x = (search_rect.max.x / bucket_size).floor() as i32;
                    let min_bucket_y = (search_rect.min.y / bucket_size).floor() as i32;
                    let max_bucket_y = (search_rect.max.y / bucket_size).floor() as i32;

                    let mut snap_candidates = std::collections::HashSet::new();
                    for bx in min_bucket_x..=max_bucket_x {
                        for by in min_bucket_y..=max_bucket_y {
                            if let Some(indices) = editor.hit_cache.buckets.get(&(bx, by)) {
                                snap_candidates.extend(indices.iter().copied());
                            }
                        }
                    }

                    for idx in snap_candidates {
                        let other = &editor.hit_cache.nodes[idx]; // Use hit_cache nodes which are synchronized with cached_nodes
                        if other.id == drag_id {
                            continue;
                        }
                        let other_rect = other.logical_rect; // Use LOGICAL rect for snapping (body only)
                        let cur_rect = Rect::from_min_size(proposed, size);
                        let snap = check_and_apply_snap(
                            cur_rect,
                            other_rect,
                            zoom,
                            10.0,
                            editor_rect.min,
                            editor.pan,
                            &mut editor.snap_lines,
                        );
                        if snap.x != 0.0 {
                            drag_delta.x += snap.x;
                            proposed.x += snap.x;
                        }
                        if snap.y != 0.0 {
                            drag_delta.y += snap.y;
                            proposed.y += snap.y;
                        }
                    }
                } else {
                    editor.snap_lines.clear();
                }

                // Update visuals immediately using cached nodes (cheap, no locks).
                for n in editor.cached_nodes.iter_mut() {
                    if context.ui_state.selected_nodes.contains(&n.id) {
                        n.position += drag_delta;
                    }
                }
                // Accumulate to commit on release (avoid per-frame graph lock).
                editor.pending_node_move_delta += drag_delta;
                editor.geometry_rev = editor.geometry_rev.wrapping_add(1);
                editor.drag_pointer_last = Some(p);
                // Deterministic repaint: if we changed positions this step, request a repaint immediately.
                context.ui_invalidator.request_repaint_after_tagged(
                    "node_editor/node_drag",
                    std::time::Duration::ZERO,
                    RepaintCause::Input,
                );
                rebuild_port_locations(editor, s, editor_rect);
            }
        }
    }

    if primary_released {
        editor.drag_pointer_last = None;
    }

    // Keep existing insertion-on-release behavior (depends on insertion_target).
    let any_released = ui.input(|i| i.pointer.any_released());
    if any_released {
        if let Some((cid, old)) = editor.waypoint_drag_old.take() {
            let root = &context.node_graph_res.0;
            let g = crate::tabs_system::node_editor::cda::navigation::graph_snapshot_by_path(
                &root,
                &editor.cda_state.breadcrumb(),
            );
            if let Some(c) = g.connections.get(&cid) {
                let new = c.waypoints.clone();
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |gg| {
                        context.node_editor_state.execute(
                            Box::new(CmdSetConnectionWaypoints::new(cid, old, new)),
                            gg,
                        );
                    },
                );
                context.graph_changed_writer.write_default();
            }
        }
        if let Some(dragged_id) = context.ui_state.dragged_node_id {
            if let Some(target_conn) = editor.insertion_target {
                let current_time = ui.ctx().input(|i| i.time);
                {
                    let root_graph = &mut context.node_graph_res.0;
                    crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                        root_graph,
                        &editor.cda_state,
                        |node_graph| {
                            // Commit pending move before insertion so graph state matches what user saw.
                            if editor.pending_node_move_delta != Vec2::ZERO {
                                let d = editor.pending_node_move_delta;
                                for id in &context.ui_state.selected_nodes {
                                    if let Some(n) = node_graph.nodes.get_mut(id) {
                                        n.position += d;
                                    }
                                }
                            // Keep ForEach network boxes aligned after a move gesture.
                            autofit_foreach_boxes(node_graph);
                                editor.pending_node_move_delta = Vec2::ZERO;
                            }
                            handle_node_insertion(
                                node_graph,
                                target_conn,
                                dragged_id,
                                &mut editor.node_animations,
                                current_time,
                            );
                        },
                    );
                }
                // Force cache rebuild next frame to show new node immediately
                editor.cached_nodes_rev = 0;
                context.graph_changed_writer.write_default();
            } else if !editor.drag_start_positions.is_empty() {
                // Normal move: one gesture => one undo entry.
                let root_graph = &mut context.node_graph_res.0;
                crate::tabs_system::node_editor::cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        // Commit pending move once (avoid per-frame lock contention).
                        if editor.pending_node_move_delta != Vec2::ZERO {
                            let d = editor.pending_node_move_delta;
                            for id in &context.ui_state.selected_nodes {
                                if let Some(n) = node_graph.nodes.get_mut(id) {
                                    n.position += d;
                                }
                            }
                            // Keep ForEach network boxes aligned after a move gesture.
                            autofit_foreach_boxes(node_graph);
                            editor.pending_node_move_delta = Vec2::ZERO;
                        }
                        let mut items = Vec::new();
                        for (id, old) in editor.drag_start_positions.iter() {
                            if let Some(n) = node_graph.nodes.get(id) {
                                let new = n.position;
                                if *old != new {
                                    items.push((*id, *old, new));
                                }
                            }
                        }
                        let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                        if !items.is_empty() { cmds.push(Box::new(CmdMoveNodes::new(items))); }
                        for (bid, old) in editor.box_rect_start.drain() {
                            if let Some(b) = node_graph.network_boxes.get(&bid) {
                                let new = b.rect;
                                if old != new { cmds.push(Box::new(CmdSetNetworkBoxRect::new(bid, old, new))); }
                            }
                        }
                        for (sid, old) in editor.sticky_rect_start.drain() {
                            if let Some(s) = node_graph.sticky_notes.get(&sid) {
                                let new = s.rect;
                                if old != new { cmds.push(Box::new(CmdSetStickyNoteRect::new(sid, old, new))); }
                            }
                        }
                        if !cmds.is_empty() {
                            context.node_editor_state.record(Box::new(CmdBatch::new("Move Selection", cmds)));
                            context.graph_changed_writer.write_default();
                        }
                    },
                );
                editor.drag_start_positions.clear();
            }
            context.ui_state.dragged_node_id = None;
            editor.insertion_target = None;
        }
    }
}

fn interact_node_ports(
    ui: &mut egui::Ui,
    node: &NodeSnapshot,
    node_rect_screen: Rect,
    zoom: f32,
    s: &crate::node_editor_settings::NodeEditorSettings,
    port_locations: &mut std::collections::HashMap<
        (crate::nodes::NodeId, crate::nodes::PortId),
        (egui::Pos2, Option<f32>),
    >,
    last_clicked_port: Option<(crate::nodes::PortId, f64)>,
    hovered: bool,
    clicked: bool,
) -> (NodeInteraction, Option<(crate::nodes::PortId, f64)>) {
    let mut interaction = NodeInteraction {
        hovered,
        clicked,
        clicked_on_output_port: None,
        dragged_on_output_port: None,
        parameter_changed: false,
    };
    let mut new_last = last_clicked_port;
    let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
    let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
    let hover_radius = port_radius * s.port_hover_radius_mul.max(1.0);
    let header_h = node.header_h.max(10.0) * zoom;
    let io_inset_y = s.node_io_inset_y_base.max(0.0) * zoom;
    let io_top = node_rect_screen.top() + header_h;
    let io_h = (node_rect_screen.height() - header_h - io_inset_y).max(1.0);
    let row_h = 18.0 * zoom;
    let row_top = io_top + (6.0 * zoom).max(2.0);

    match node.input_style {
        InputStyle::Individual => {
            for (i, pid) in node.inputs.iter().enumerate() {
                let pos = egui::pos2(
                    node_rect_screen.left() - port_offset,
                    row_top + row_h * (i as f32 + 0.5),
                );
                let rect = Rect::from_center_size(pos, Vec2::splat(hover_radius * 2.0));
                ui.interact(
                    rect,
                    ui.make_persistent_id(node.id).with(("in", pid.as_str())),
                    Sense::hover(),
                );
                port_locations.insert((node.id, pid.clone()), (pos, None));
            }
        }
        InputStyle::Bar | InputStyle::Collection => {
            if let Some(pid) = node.inputs.first() {
                let h = io_h * s.layout_bar_width_ratio.clamp(0.1, 1.0);
                let w = port_radius * 1.5;
                let pos = egui::pos2(node_rect_screen.left() - port_offset, io_top + io_h * 0.5);
                let rect = Rect::from_center_size(pos, Vec2::new(w, h));
                ui.interact(
                    rect.expand(5.0),
                    ui.make_persistent_id(node.id)
                        .with(("in_bar", pid.as_str())),
                    Sense::hover(),
                );
                port_locations.insert((node.id, pid.clone()), (pos, Some(h)));
            }
        }
    }

    for (i, pid) in node.outputs.iter().enumerate() {
        let pos = egui::pos2(
            node_rect_screen.right() + port_offset,
            row_top + row_h * (i as f32 + 0.5),
        );
        let rect = Rect::from_center_size(pos, Vec2::splat(hover_radius * 2.0));
        let r = ui.interact(
            rect,
            ui.make_persistent_id(node.id).with(("out", pid.as_str())),
            Sense::click_and_drag(),
        );
        if r.clicked() {
            new_last = Some((pid.clone(), ui.ctx().input(|i| i.time)));
            interaction.clicked_on_output_port = Some((node.id, pid.clone()));
        }
        if r.dragged() {
            interaction.dragged_on_output_port = Some((node.id, pid.clone()));
        }
        port_locations.insert((node.id, pid.clone()), (pos, None));
    }

    (interaction, new_last)
}

pub fn paint_nodes_retained(
    ui: &mut egui::Ui,
    editor: &NodeEditorTab,
    context: &EditorTabContext,
    editor_rect: Rect,
    nodes: &[NodeSnapshot],
    opacity_mul: f32,
    selected_nodes: Option<&std::collections::HashSet<crate::nodes::NodeId>>,
) {
    let painter = ui.painter();
    let s = context.node_editor_settings;
    let zoom = editor.zoom;
    for node in nodes {
        let node_rect_screen = Rect::from_min_size(
            editor_rect.min + node.position.to_vec2() * zoom + editor.pan,
            node.size * zoom,
        );
        let screen_size = ui.ctx().screen_rect().size();
        let frame_id = ui.ctx().cumulative_frame_nr();
        let theme = context.theme;
        let is_selected = selected_nodes.map_or(false, |s| s.contains(&node.id));

        // --- Draw State Backgrounds ---
        let max_dim = node_rect_screen.width().max(node_rect_screen.height());
        let base_square = Rect::from_center_size(node_rect_screen.center(), Vec2::splat(max_dim));
        let state_rounding = CornerRadius::from(6.0 * zoom.sqrt());
        if node.is_display_node {
            let bg_rect = base_square.expand((10.0 * zoom).max(5.0));
            let mut fill_rgba =
                bevy_egui::egui::Rgba::from(theme.colors.indicator_display).to_array();
            fill_rgba[3] = 38.0 / 255.0;
            let uniform = SdfRectUniform {
                center: [bg_rect.center().x, bg_rect.center().y],
                half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
                corner_radii: [
                    state_rounding.nw as f32,
                    state_rounding.ne as f32,
                    state_rounding.se as f32,
                    state_rounding.sw as f32,
                ],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
        }
        if node.is_template {
            let bg_rect = base_square.expand((2.0 * zoom).max(1.0));
            let mut fill_rgba =
                bevy_egui::egui::Rgba::from(theme.colors.indicator_template).to_array();
            fill_rgba[3] = 38.0 / 255.0;
            let uniform = SdfRectUniform {
                center: [bg_rect.center().x, bg_rect.center().y],
                half_size: [bg_rect.width() * 0.5, bg_rect.height() * 0.5],
                corner_radii: [
                    state_rounding.nw as f32,
                    state_rounding.ne as f32,
                    state_rounding.se as f32,
                    state_rounding.sw as f32,
                ],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(bg_rect, uniform, frame_id));
        }

        let rounding = CornerRadius::from(s.style_node_rounding_base.max(0.0) * zoom.sqrt());

        // --- Calculate Visual BBox ---
        let visual_bbox = match node.style {
            NodeStyle::Layered => {
                let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
                let background_offset = shadow_offset * 2.0;
                let background_rect = node_rect_screen.translate(background_offset);
                Rect::from_min_max(node_rect_screen.min, background_rect.max)
            }
            NodeStyle::Normal | NodeStyle::Large => node_rect_screen,
        };

        // --- Draw Node Body ---
        match node.style {
            NodeStyle::Layered => {
                let shadow_offset = Vec2::new(4.0, 4.0) * zoom.sqrt();
                let background_offset = shadow_offset * 2.0;
                let top_rect = node_rect_screen;
                let shadow_rect = top_rect.translate(shadow_offset);
                let background_rect = top_rect.translate(background_offset);
                let rounding4 = [
                    rounding.nw as f32,
                    rounding.ne as f32,
                    rounding.se as f32,
                    rounding.sw as f32,
                ];

                // background
                {
                    let c = background_rect.center();
                    let s2 = background_rect.size();
                    let uniform = SdfRectUniform {
                        center: [c.x, c.y],
                        half_size: [s2.x * 0.5, s2.y * 0.5],
                        corner_radii: rounding4,
                        fill_color: [1.0, 1.0, 1.0, 1.0],
                        shadow_color: [0.0; 4],
                        shadow_blur: 0.0,
                        _pad1: 0.0,
                        shadow_offset: [0.0, 0.0],
                        border_width: 0.0,
                        _pad2: [0.0; 3],
                        border_color: [0.0; 4],
                        screen_size: [screen_size.x, screen_size.y],
                        _pad3: [0.0; 2],
                    };
                    painter.add(create_sdf_rect_callback(background_rect, uniform, frame_id));
                }
                // shadow-ish middle layer
                {
                    let c = shadow_rect.center();
                    let s2 = shadow_rect.size();
                    let fill_rgba =
                        bevy_egui::egui::Rgba::from(theme.colors.divider_color).to_array();
                    let uniform = SdfRectUniform {
                        center: [c.x, c.y],
                        half_size: [s2.x * 0.5, s2.y * 0.5],
                        corner_radii: rounding4,
                        fill_color: fill_rgba,
                        shadow_color: [0.0; 4],
                        shadow_blur: 0.0,
                        _pad1: 0.0,
                        shadow_offset: [0.0, 0.0],
                        border_width: 0.0,
                        _pad2: [0.0; 3],
                        border_color: [0.0; 4],
                        screen_size: [screen_size.x, screen_size.y],
                        _pad3: [0.0; 2],
                    };
                    painter.add(create_sdf_rect_callback(shadow_rect, uniform, frame_id));
                }
                // top
                {
                    let c = top_rect.center();
                    let s2 = top_rect.size();
                    let uniform = SdfRectUniform {
                        center: [c.x, c.y],
                        half_size: [s2.x * 0.5, s2.y * 0.5],
                        corner_radii: rounding4,
                        fill_color: [1.0, 1.0, 1.0, 1.0],
                        shadow_color: [0.0; 4],
                        shadow_blur: 0.0,
                        _pad1: 0.0,
                        shadow_offset: [0.0, 0.0],
                        border_width: 0.0,
                        _pad2: [0.0; 3],
                        border_color: [0.0; 4],
                        screen_size: [screen_size.x, screen_size.y],
                        _pad3: [0.0; 2],
                    };
                    painter.add(create_sdf_rect_callback(top_rect, uniform, frame_id));
                }
            }
            NodeStyle::Normal | NodeStyle::Large => {
                let center = node_rect_screen.center();
                let size = node_rect_screen.size();
                let fill_rgba =
                    bevy_egui::egui::Rgba::from(theme.colors.node_background).to_array();
                let border_rgba = bevy_egui::egui::Rgba::from(theme.colors.node_border).to_array();
                let uniform = SdfRectUniform {
                    center: [center.x, center.y],
                    half_size: [size.x * 0.5, size.y * 0.5],
                    corner_radii: [
                        rounding.nw as f32,
                        rounding.ne as f32,
                        rounding.se as f32,
                        rounding.sw as f32,
                    ],
                    fill_color: mul_alpha(fill_rgba, opacity_mul),
                    shadow_color: mul_alpha([0.0, 0.0, 0.0, 0.3], opacity_mul),
                    shadow_blur: 15.0,
                    _pad1: 0.0,
                    shadow_offset: [0.0, 5.0],
                    border_width: s.style_node_border_width_base.max(0.0) * zoom.sqrt(),
                    _pad2: [0.0; 3],
                    border_color: mul_alpha(border_rgba, opacity_mul),
                    screen_size: [screen_size.x, screen_size.y],
                    _pad3: [0.0; 2],
                };
                painter.add(create_sdf_rect_callback(
                    node_rect_screen.expand(30.0),
                    uniform,
                    frame_id,
                ));
            }
        }

        // --- Status Indicators ---
        let sidebar_width = node_rect_screen.width() * s.layout_sidebar_width_ratio.clamp(0.0, 0.5);
        if node.is_bypassed {
            let bypass_rect = node_rect_screen.with_max_x(node_rect_screen.min.x + sidebar_width);
            let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_bypass).to_array();
            let uniform = SdfRectUniform {
                center: [bypass_rect.center().x, bypass_rect.center().y],
                half_size: [bypass_rect.width() * 0.5, bypass_rect.height() * 0.5],
                corner_radii: [0.0; 4],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(bypass_rect, uniform, frame_id));
        }
        if node.is_locked {
            let lock_rect = node_rect_screen.with_min_x(node_rect_screen.max.x - sidebar_width);
            let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.indicator_lock).to_array();
            let uniform = SdfRectUniform {
                center: [lock_rect.center().x, lock_rect.center().y],
                half_size: [lock_rect.width() * 0.5, lock_rect.height() * 0.5],
                corner_radii: [0.0; 4],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(lock_rect, uniform, frame_id));
        }

        // NOTE: Removed vertical side dividers (bypass/lock separators) per UX request.

        // --- Selected Node Outline (animated along border; not a static yellow box) ---
        if is_selected {
            let w0 = (s.style_node_border_width_base.max(0.0) * zoom.sqrt()).max(1.0);
            let expand = (2.0 * zoom.sqrt()).max(1.0);
            let r = node_rect_screen.expand(expand);
            let mut rgba = bevy_egui::egui::Rgba::from(theme.colors.node_selected).to_array();
            rgba = mul_alpha(rgba, opacity_mul);
            let phase0 = {
                let u = node.id.as_u128();
                let x = (u as u64) ^ ((u >> 64) as u64);
                (x as f32) * (1.0 / (u64::MAX as f32))
            };
            let p_tl = Pos2::new(r.min.x, r.min.y);
            let p_tr = Pos2::new(r.max.x, r.min.y);
            let p_br = Pos2::new(r.max.x, r.max.y);
            let p_bl = Pos2::new(r.min.x, r.max.y);
            let segs = [(p_tl, p_tr), (p_tr, p_br), (p_br, p_bl), (p_bl, p_tl)];
            for (i, (a, b)) in segs.into_iter().enumerate() {
                let d = b - a;
                let p1 = a + d / 3.0;
                let p2 = a + d * (2.0 / 3.0);
                let min_x = a.x.min(p1.x).min(p2.x).min(b.x);
                let max_x = a.x.max(p1.x).max(p2.x).max(b.x);
                let min_y = a.y.min(p1.y).min(p2.y).min(b.y);
                let max_y = a.y.max(p1.y).max(p2.y).max(b.y);
                let rr = Rect::from_min_max(Pos2::new(min_x, min_y), Pos2::new(max_x, max_y))
                    .expand(w0 * 2.0 + 8.0)
                    .intersect(ui.clip_rect());
                if rr.width() <= 0.0 || rr.height() <= 0.0 {
                    continue;
                }
                let uniform = SdfFlowCurveUniform {
                    p01: [a.x, a.y, p1.x, p1.y],
                    p23: [p2.x, p2.y, b.x, b.y],
                    color: rgba,
                    params0: [w0, 1.0, 0.10, 0.08],
                    params1: [2.0, phase0 + (i as f32) * 0.25, 1.0, 0.0],
                    screen_params: [screen_size.x, screen_size.y, 0.0, 0.0],
                };
                painter.add(create_sdf_flow_curve_callback(rr, uniform, frame_id));
            }
        }

        // --- Node Title (header) ---
        let font_px = 12.0 * zoom;
        let lod_hide = s.title_text_lod_hide_px.max(0.0);
        let lod_fade = s.title_text_lod_fade_px.max(1e-3);
        let header_h = node.header_h.max(10.0) * zoom;
        if font_px > lod_hide {
            let pad_x = (10.0 * zoom).max(4.0);
            let pad_y = (header_h * 0.5).max(1.0);
            let icon_px = (14.0 * zoom).max(4.0);
            let icon_rect = Rect::from_min_size(
                Pos2::new(
                    node_rect_screen.left() + pad_x,
                    node_rect_screen.top() + (4.0 * zoom).max(2.0),
                ),
                Vec2::splat(icon_px),
            );
            let text_pos = Pos2::new(
                icon_rect.max.x + (6.0 * zoom).max(2.0),
                node_rect_screen.top() + (6.0 * zoom).max(2.0),
            );
            let opacity = ((font_px - lod_hide) / lod_fade).clamp(0.0, 1.0);
            let a = (255.0 * (opacity * opacity_mul).clamp(0.0, 1.0)) as u8;
            let color = Color32::from_black_alpha(a);
            if opacity > 0.05 {
                let tint = Color32::from_black_alpha(a);
                let icon = icons::icon_for_node_name(&node.name, true);
                egui::Image::new(icon)
                    .tint(tint)
                    .fit_to_exact_size(icon_rect.size())
                    .paint_at(ui, icon_rect);
                let font = egui::FontId::proportional(font_px);
                let max_w = (node_rect_screen.right() - text_pos.x - pad_x).max(1.0);
                let (txt, _lines) = wrap_two_lines(ui.ctx(), font.clone(), &node.name, max_w);
                painter.text(text_pos, egui::Align2::LEFT_TOP, txt, font, color);
            }
        }

        // --- Header Divider ---
        {
            let y = node_rect_screen.top() + header_h;
            let w = 1.0f32.max(1.0 * zoom.sqrt());
            let rect = Rect::from_min_max(
                Pos2::new(node_rect_screen.left(), y - w * 0.5),
                Pos2::new(node_rect_screen.right(), y + w * 0.5),
            );
            let fill_rgba = bevy_egui::egui::Rgba::from(theme.colors.divider_color).to_array();
            let u = SdfRectUniform {
                center: [rect.center().x, rect.center().y],
                half_size: [rect.width() * 0.5, rect.height() * 0.5],
                corner_radii: [0.0; 4],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(rect.expand(1.0), u, frame_id));
        }

        // --- IO Panel (inset, dark, top corners square) ---
        let io_inset_x = s.node_io_inset_x_base.max(0.0) * zoom;
        let io_inset_y = s.node_io_inset_y_base.max(0.0) * zoom;
        let io_rect = Rect::from_min_max(
            Pos2::new(
                node_rect_screen.left() + io_inset_x,
                node_rect_screen.top() + header_h,
            ),
            Pos2::new(
                node_rect_screen.right() - io_inset_x,
                node_rect_screen.bottom() - io_inset_y,
            ),
        );
        if io_rect.width() > 1.0 && io_rect.height() > 1.0 {
            let mut fill_rgba =
                bevy_egui::egui::Rgba::from(theme.colors.panel_background).to_array();
            fill_rgba[3] = s.node_io_panel_alpha.clamp(0.0, 1.0);
            let u = SdfRectUniform {
                center: [io_rect.center().x, io_rect.center().y],
                half_size: [io_rect.width() * 0.5, io_rect.height() * 0.5],
                corner_radii: [0.0, 0.0, rounding.se as f32, rounding.sw as f32],
                fill_color: mul_alpha(fill_rgba, opacity_mul),
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 0.0,
                _pad2: [0.0; 3],
                border_color: [0.0; 4],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            painter.add(create_sdf_rect_callback(io_rect.expand(2.0), u, frame_id));
        }

        // Node icon is rendered in the header (before title).
        // Nano Heightmap preview (file:// URI via egui_extras loaders).
        if node.name == crate::nodes::ai_texture::NODE_NANO_HEIGHTMAP && io_rect.width() > 4.0 && io_rect.height() > 4.0 {
            let abs_uri = context
                .node_graph_res
                .0
                .nodes
                .get(&node.id)
                .and_then(|n| {
                    n.parameters.iter().find(|p| p.name == crate::nodes::ai_texture::PARAM_IMAGE_PATH).and_then(|p| {
                        if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.trim().to_string()) } else { None }
                    })
                })
                .filter(|s| !s.is_empty())
                .and_then(|rel| std::env::current_dir().ok().map(|cwd| cwd.join("assets").join(rel)))
                .and_then(|abs| abs.canonicalize().ok())
                .map(|abs| {
                    let p = abs.to_string_lossy().replace('\\', "/");
                    if p.starts_with('/') { format!("file://{}", p) } else { format!("file:///{}", p) }
                });
            if let Some(uri) = abs_uri {
                let pad = (6.0 * zoom).max(2.0);
                let r = Rect::from_min_max(
                    Pos2::new(io_rect.left() + pad, io_rect.top() + pad),
                    Pos2::new(io_rect.right() - pad, io_rect.bottom() - pad),
                );
                if r.width() > 4.0 && r.height() > 4.0 {
                    egui::Image::new(uri)
                        .fit_to_exact_size(r.size())
                        .rounding(4.0)
                        .paint_at(ui, r);
                }
            }
        }

        // Nano HexPlanar Baker preview (BaseColor UV if available).
        if node.name == crate::nodes::ai_texture::NODE_NANO_HEXPLANAR_BAKER && io_rect.width() > 4.0 && io_rect.height() > 4.0 {
            let rel = context
                .node_graph_res
                .0
                .nodes
                .get(&node.id)
                .and_then(|n| {
                    let s = |name: &str| n.parameters.iter().find(|p| p.name == name).and_then(|p| {
                        if let crate::nodes::parameter::ParameterValue::String(s) = &p.value { Some(s.trim().to_string()) } else { None }
                    });
                    s(crate::nodes::ai_texture::PARAM_BASE_UV_FIXED).filter(|s| !s.is_empty()).or_else(|| s(crate::nodes::ai_texture::PARAM_BASE_UV).filter(|s| !s.is_empty()))
                })
                .and_then(|rel| std::env::current_dir().ok().map(|cwd| cwd.join("assets").join(rel)))
                .and_then(|abs| abs.canonicalize().ok())
                .map(|abs| {
                    let p = abs.to_string_lossy().replace('\\', "/");
                    if p.starts_with('/') { format!("file://{}", p) } else { format!("file:///{}", p) }
                });
            if let Some(uri) = rel {
                let pad = (6.0 * zoom).max(2.0);
                let r = Rect::from_min_max(
                    Pos2::new(io_rect.left() + pad, io_rect.top() + pad),
                    Pos2::new(io_rect.right() - pad, io_rect.bottom() - pad),
                );
                if r.width() > 4.0 && r.height() > 4.0 {
                    egui::Image::new(uri)
                        .fit_to_exact_size(r.size())
                        .rounding(4.0)
                        .paint_at(ui, r);
                }
            }
        }

        // --- Ports (static visuals + labels) ---
        let port_radius = s.port_radius_base.max(0.0) * zoom.sqrt();
        let port_offset = s.port_offset_base.max(0.0) * zoom.sqrt();
        let stroke_w = s.port_stroke_width_base.max(0.0) * zoom.sqrt();
        let fill_rgba = bevy_egui::egui::Rgba::from(Color32::WHITE).to_array();
        let stroke_rgba = bevy_egui::egui::Rgba::from(Color32::from_gray(120)).to_array();
        let port_font_px = (9.0 * zoom).clamp(6.0, 14.0);
        let port_label_pad = port_radius + (4.0 * zoom).max(2.0);
        let port_label_color =
            Color32::from_white_alpha((255.0 * opacity_mul.clamp(0.0, 1.0)) as u8);
        let desc = context.node_registry.get_descriptor(&node.name);
        let port_font = egui::FontId::proportional(port_font_px);
        let io_inset_x = s.node_io_inset_x_base.max(0.0) * zoom;
        let io_inset_y = s.node_io_inset_y_base.max(0.0) * zoom;
        let io_top = node_rect_screen.top() + header_h;
        let io_h = (node_rect_screen.height() - header_h - io_inset_y).max(1.0);
        let io_mid_x = node_rect_screen.center().x;
        let max_l = (io_mid_x
            - (node_rect_screen.left() + io_inset_x + port_label_pad)
            - (6.0 * zoom).max(2.0))
        .max(0.0);
        let max_r = ((node_rect_screen.right() - io_inset_x - port_label_pad)
            - io_mid_x
            - (6.0 * zoom).max(2.0))
        .max(0.0);

        match node.input_style {
            InputStyle::Individual => {
                let row_h = 18.0 * zoom;
                let row_top = io_top + (6.0 * zoom).max(2.0);
                for (i, pid) in node.inputs.iter().enumerate() {
                    let p = egui::pos2(
                        node_rect_screen.left() - port_offset,
                        row_top + row_h * (i as f32 + 0.5),
                    );
                    let rect = Rect::from_center_size(p, Vec2::splat(port_radius * 4.0));
                    let u = SdfCircleUniform {
                        center: [p.x, p.y],
                        radius: port_radius,
                        border_width: stroke_w,
                        fill_color: fill_rgba,
                        border_color: stroke_rgba,
                        softness: 1.0,
                        _pad0: 0.0,
                        screen_size: [screen_size.x, screen_size.y],
                        _pad1: [0.0; 2],
                        _pad2: [0.0; 2],
                    };
                    painter.add(create_sdf_circle_callback(
                        rect,
                        painter.clip_rect(),
                        u,
                        frame_id,
                    ));
                    // Port label (left-aligned inside node)
                    if font_px > lod_hide {
                        let idx = pid
                            .as_str()
                            .rsplit_once(':')
                            .and_then(|(_, t)| t.parse::<usize>().ok())
                            .unwrap_or(i);
                        let label = desc
                            .as_ref()
                            .and_then(|d| d.inputs.get(idx).map(|s| s.as_str()))
                            .unwrap_or("");
                        let label = if label.is_empty() {
                            format!("Input {}", idx)
                        } else {
                            label.to_string()
                        };
                        let lpos = Pos2::new(node_rect_screen.left() + port_label_pad, p.y);
                        painter.text(
                            lpos,
                            egui::Align2::LEFT_CENTER,
                            ellipsize(ui.ctx(), port_font.clone(), &label, max_l),
                            port_font.clone(),
                            port_label_color,
                        );
                    }
                }
            }
            InputStyle::Bar | InputStyle::Collection => {
                if !node.inputs.is_empty() {
                    let h = node_rect_screen.height() * s.layout_bar_width_ratio.clamp(0.1, 1.0);
                    let w = port_radius * 1.5;
                    let p = egui::pos2(
                        node_rect_screen.left() - port_offset,
                        node_rect_screen.center().y,
                    );
                    let rect = Rect::from_center_size(p, Vec2::new(w, h));
                    let u = SdfRectUniform {
                        center: [p.x, p.y],
                        half_size: [w * 0.5, h * 0.5],
                        corner_radii: [2.0; 4],
                        fill_color: fill_rgba,
                        shadow_color: [0.0; 4],
                        shadow_blur: 0.0,
                        _pad1: 0.0,
                        shadow_offset: [0.0, 0.0],
                        border_width: stroke_w,
                        _pad2: [0.0; 3],
                        border_color: stroke_rgba,
                        screen_size: [screen_size.x, screen_size.y],
                        _pad3: [0.0; 2],
                    };
                    painter.add(create_sdf_rect_callback(rect.expand(6.0), u, frame_id));
                }
            }
        }

        let row_h = 18.0 * zoom;
        let row_top = io_top + (6.0 * zoom).max(2.0);
        for (i, pid) in node.outputs.iter().enumerate() {
            let p = egui::pos2(
                node_rect_screen.right() + port_offset,
                row_top + row_h * (i as f32 + 0.5),
            );
            let rect = Rect::from_center_size(p, Vec2::splat(port_radius * 4.0));
            let u = SdfCircleUniform {
                center: [p.x, p.y],
                radius: port_radius,
                border_width: stroke_w,
                fill_color: fill_rgba,
                border_color: stroke_rgba,
                softness: 1.0,
                _pad0: 0.0,
                screen_size: [screen_size.x, screen_size.y],
                _pad1: [0.0; 2],
                _pad2: [0.0; 2],
            };
            painter.add(create_sdf_circle_callback(
                rect,
                painter.clip_rect(),
                u,
                frame_id,
            ));
            // Port label (right-aligned inside node)
            if font_px > lod_hide {
                let idx = pid
                    .as_str()
                    .rsplit_once(':')
                    .and_then(|(_, t)| t.parse::<usize>().ok())
                    .unwrap_or(i);
                let label = desc
                    .as_ref()
                    .and_then(|d| d.outputs.get(idx).map(|s| s.as_str()))
                    .unwrap_or("");
                let label = if label.is_empty() {
                    format!("Output {}", idx)
                } else {
                    label.to_string()
                };
                let lpos = Pos2::new(node_rect_screen.right() - port_label_pad, p.y);
                painter.text(
                    lpos,
                    egui::Align2::RIGHT_CENTER,
                    ellipsize(ui.ctx(), port_font.clone(), &label, max_r),
                    port_font.clone(),
                    port_label_color,
                );
            }
        }
    }
}

pub fn draw_connection_preview(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    _context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let mut preview_end_pos = ui.ctx().pointer_interact_pos();
    let dragging_any =
        editor.pending_connection_from.is_some() || !editor.pending_connections_from.is_empty();
    let wants_spinner =
        dragging_any && editor.snapped_to_port.is_none() && editor.ghost_request_id.is_some();
    let wants_burst_hint = dragging_any
        && editor.snapped_to_port.is_none()
        && editor.ghost_request_id.is_none()
        && editor.ghost_tab_last_time.is_some();
    let wants_tab_hint = dragging_any
        && editor.snapped_to_port.is_none()
        && editor.ghost_request_id.is_none()
        && editor.ghost_graph.is_none()
        && editor.ghost_tab_last_time.is_none();
    let wants_apply_hint = dragging_any
        && editor.snapped_to_port.is_none()
        && editor.ghost_request_id.is_none()
        && editor.ghost_graph.is_some();
    let wants_deep_spinner = wants_spinner && editor.deep_mode;

    // Multi-wire preview: render multiple dashed wires (no snapping).
    if !editor.pending_connections_from.is_empty() {
        // Keep ghost anchored to the wire tail (so it follows your hand), even for multi-wire.
        if ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary)) {
            if let Some(end_pos) = preview_end_pos {
                let new_anchor = ((end_pos - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                if let (Some(old), Some(g)) =
                    (editor.ghost_anchor_graph_pos, editor.ghost_graph.as_mut())
                {
                    let d = new_anchor - old;
                    if d.x != 0.0 || d.y != 0.0 {
                        for n in &mut g.nodes {
                            n.position += d;
                        }
                    }
                }
                editor.ghost_anchor_graph_pos = Some(new_anchor);
            }
        }
        if let Some(end_pos) = preview_end_pos {
            for (nid, pid) in editor.pending_connections_from.iter().cloned() {
                if let Some((start_pos, _)) = editor.port_locations.get(&(nid, pid)) {
                    draw_dashed_bezier(
                        ui,
                        *start_pos,
                        end_pos,
                        Stroke::new(2.0, Color32::from_gray(200)),
                    );
                }
            }
            if wants_tab_hint {
                ui.painter().text(
                    end_pos + Vec2::new(12.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    "Tab: Copilot  `: Apply",
                    egui::FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            }
            if wants_burst_hint {
                let n = editor.ghost_tab_burst.max(1);
                ui.painter().text(
                    end_pos + Vec2::new(12.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    format!("Copilot x{} …", n),
                    egui::FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            }
            if wants_apply_hint {
                ui.painter().text(
                    end_pos + Vec2::new(12.0, 10.0),
                    egui::Align2::LEFT_TOP,
                    "`: Apply",
                    egui::FontId::proportional(14.0),
                    Color32::from_gray(220),
                );
            }
            if wants_spinner {
                let t = ui.input(|i| i.time) as f32;
                let r = 10.0;
                let n = 8;
                for k in 0..n {
                    let a = (k as f32 / n as f32) * std::f32::consts::TAU + t * 6.0;
                    let p = end_pos + Vec2::new(a.cos() * r, a.sin() * r);
                    let alpha = (((k as f32) / n as f32) * 0.6 + 0.2).clamp(0.0, 1.0);
                    ui.painter().circle_filled(
                        p,
                        2.0,
                        Color32::from_white_alpha((255.0 * alpha) as u8),
                    );
                }
                if wants_deep_spinner {
                    let label = format!(
                        "🧠 Deep {}/{}",
                        editor.deep_skill_turns,
                        crate::libs::ai_service::copilot_skill::MAX_SKILL_TURNS
                    );
                    ui.painter().text(
                        end_pos + Vec2::new(20.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        label,
                        egui::FontId::proportional(13.0),
                        Color32::from_rgb(180, 220, 255),
                    );
                }
                _context.ui_invalidator.request_repaint_after_tagged(
                    "node_editor/ghost_spinner",
                    std::time::Duration::from_millis(16),
                    crate::invalidator::RepaintCause::Animation,
                );
            }
        }
        return;
    }

    // If a wire is "parked", freeze preview end at the last waypoint and allow resuming by dragging the dot.
    if editor.pending_wire_parked && editor.pending_connection_from.is_some() && !editor.pending_wire_waypoints.is_empty() {
        let last_wp = *editor.pending_wire_waypoints.last().unwrap_or(&egui::Pos2::ZERO);
        let last_wp_screen = editor_rect.min + last_wp.to_vec2() * editor.zoom + editor.pan;
        preview_end_pos = Some(last_wp_screen);

        let r = (9.0 * editor.zoom.sqrt()).clamp(8.0, 14.0);
        let rect = Rect::from_center_size(last_wp_screen, Vec2::splat(r * 2.0));
        let id = ui.make_persistent_id((
            "pending_wire_relay_dot",
            editor_rect.min.x.to_bits(),
            editor_rect.min.y.to_bits(),
            editor_rect.max.x.to_bits(),
            editor_rect.max.y.to_bits(),
        ));
        let resp = ui.interact(rect, id, Sense::click_and_drag());
        ui.painter().circle_filled(
            last_wp_screen,
            (6.0 * editor.zoom.sqrt()).clamp(4.0, 10.0),
            Color32::from_rgba_unmultiplied(235, 235, 240, 220),
        );
        ui.painter().circle_stroke(
            last_wp_screen,
            (6.0 * editor.zoom.sqrt()).clamp(4.0, 10.0),
            Stroke::new(1.0, Color32::from_gray(40)),
        );
        if resp.dragged() || resp.clicked() {
            // Resume: un-park and continue previewing to the pointer.
            editor.pending_wire_parked = false;
            preview_end_pos = ui.ctx().pointer_interact_pos();
        }
    }

    // While dragging, keep ghost anchored to the wire tail (so it follows your hand).
    if editor.pending_connection_from.is_some()
        && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary))
    {
        if let Some(end_pos) = preview_end_pos {
            let new_anchor = ((end_pos - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
            if let (Some(old), Some(g)) =
                (editor.ghost_anchor_graph_pos, editor.ghost_graph.as_mut())
            {
                let d = new_anchor - old;
                if d.x != 0.0 || d.y != 0.0 {
                    for n in &mut g.nodes {
                        n.position += d;
                    }
                }
            }
            editor.ghost_anchor_graph_pos = Some(new_anchor);
        }
    }

    // Snapping Logic
    if let Some((start_node_id, _)) = &editor.pending_connection_from {
        if let Some(mouse_pos) = preview_end_pos {
            let snap_radius_sq = (25.0 as f32).powi(2);
            let mut best_dist = snap_radius_sq;
            let mut best_target = None;

            // Iterate inputs to snap
            for n in &editor.cached_nodes {
                if n.id == *start_node_id {
                    continue;
                }
                for port_id in &n.inputs {
                    if let Some((port_pos, width_opt)) =
                        editor.port_locations.get(&(n.id, port_id.clone()))
                    {
                        let d = if let Some(width) = width_opt {
                            // For Bar style (vertical bar on the left): check vertical bounds and horizontal distance.
                            let half_h = width / 2.0;
                            let dx = (mouse_pos.x - port_pos.x).abs();
                            let dy = (mouse_pos.y - port_pos.y).abs();
                            if dy <= half_h + 10.0 {
                                dx.powi(2)
                            } else {
                                f32::INFINITY
                            }
                        } else {
                            // For Point style: standard distance
                            port_pos.distance_sq(mouse_pos)
                        };

                        if d < best_dist {
                            best_dist = d;
                            best_target = Some((n.id, port_id.clone()));
                        }
                    }
                }
            }
            editor.snapped_to_port = best_target;
        }
    }

    if let Some((_, _)) = &editor.snapped_to_port {
        // snapped_to_port is (NodeId, PortId), which matches our key structure
        if let Some((port_pos, _)) = editor
            .port_locations
            .get(editor.snapped_to_port.as_ref().unwrap())
        {
            preview_end_pos = Some(*port_pos);
        }
    }

    if let Some((node_id, port_id)) = &editor.pending_connection_from {
        if let Some((start_pos, _)) = editor.port_locations.get(&(*node_id, port_id.clone())) {
            if let Some(end_pos) = preview_end_pos {
                let mut pts: Vec<Pos2> =
                    Vec::with_capacity(2 + editor.pending_wire_waypoints.len());
                pts.push(*start_pos);
                for wp in &editor.pending_wire_waypoints {
                    pts.push(editor_rect.min + wp.to_vec2() * editor.zoom + editor.pan);
                }
                pts.push(end_pos);
                for w in pts.windows(2) {
                    draw_dashed_bezier(ui, w[0], w[1], Stroke::new(2.0, Color32::from_gray(200)));
                }
                // Houdini-like relay points: show a small "port dot" at each pending waypoint.
                if !editor.pending_wire_waypoints.is_empty() {
                    let painter = ui.painter();
                    let r = (5.0 * editor.zoom.sqrt()).clamp(3.0, 8.0);
                    let fill = Color32::from_rgba_unmultiplied(235, 235, 240, 230);
                    let stroke = Stroke::new(1.0, Color32::from_gray(40));
                    for wp in &pts[1..pts.len().saturating_sub(1)] {
                        painter.circle_filled(*wp, r, fill);
                        painter.circle_stroke(*wp, r, stroke);
                    }
                }
                if wants_tab_hint {
                    ui.painter().text(
                        end_pos + Vec2::new(12.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        "Tab: Copilot  `: Apply",
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(220),
                    );
                }
                if wants_burst_hint {
                    let n = editor.ghost_tab_burst.max(1);
                    ui.painter().text(
                        end_pos + Vec2::new(12.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        format!("Copilot x{} …", n),
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(220),
                    );
                }
                if wants_apply_hint {
                    ui.painter().text(
                        end_pos + Vec2::new(12.0, 10.0),
                        egui::Align2::LEFT_TOP,
                        "`: Apply",
                        egui::FontId::proportional(14.0),
                        Color32::from_gray(220),
                    );
                }
                if wants_spinner {
                    // Simple spinner: 8 dots rotating around the wire tail.
                    let t = ui.input(|i| i.time) as f32;
                    let r = 10.0;
                    let n = 8;
                    for k in 0..n {
                        let a = (k as f32 / n as f32) * std::f32::consts::TAU + t * 6.0;
                        let p = end_pos + Vec2::new(a.cos() * r, a.sin() * r);
                        let alpha = (((k as f32) / n as f32) * 0.6 + 0.2).clamp(0.0, 1.0);
                        ui.painter().circle_filled(
                            p,
                            2.0,
                            Color32::from_white_alpha((255.0 * alpha) as u8),
                        );
                    }
                    // Deep mode indicator
                    if wants_deep_spinner {
                        let label = format!(
                            "🧠 Deep {}/{}",
                            editor.deep_skill_turns,
                            crate::libs::ai_service::copilot_skill::MAX_SKILL_TURNS
                        );
                        ui.painter().text(
                            end_pos + Vec2::new(20.0, 10.0),
                            egui::Align2::LEFT_TOP,
                            label,
                            egui::FontId::proportional(13.0),
                            Color32::from_rgb(180, 220, 255),
                        );
                    }
                    _context.ui_invalidator.request_repaint_after_tagged(
                        "node_editor/ghost_spinner",
                        std::time::Duration::ZERO,
                        RepaintCause::Animation,
                    );
                }
            }
        }
    }
}

pub fn draw_copilot_relays(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let mut sessions: Vec<_> = editor.copilot_relays.values().cloned().collect();
    sessions.sort_by(|a, b| a.created_at.cmp(&b.created_at));
    for s in sessions {
        // Virtual wire + ghost preview: keep both, not just a UI panel.
        let anchor_screen = editor_rect.min + s.anchor_graph_pos.to_vec2() * editor.zoom + editor.pan;
        let dot_r = (6.0 * editor.zoom.sqrt()).clamp(4.0, 10.0);
        ui.painter().circle_filled(
            anchor_screen,
            dot_r,
            Color32::from_rgba_unmultiplied(235, 235, 240, 160),
        );
        ui.painter().circle_stroke(
            anchor_screen,
            dot_r,
            Stroke::new(1.0, Color32::from_gray(40)),
        );

        // If we have a ghost graph, render it and route virtual wires into it.
        if let Some(g) = &s.ghost {
            // Ghost nodes + links (translucent)
            paint_nodes_retained(
                ui,
                editor,
                context,
                editor_rect,
                &g.nodes,
                0.55,
                None,
            );
            draw_ghost_links(ui, editor, g, editor_rect, 0.55);
        }

        // Virtual wire(s): from sources -> anchor or first ghost node (if present).
        let mut target_end = anchor_screen;
        if let Some(g) = &s.ghost {
            if let Some(first) = g.nodes.first() {
                let node_rect_screen = Rect::from_min_size(
                    editor_rect.min + first.position.to_vec2() * editor.zoom + editor.pan,
                    first.size * editor.zoom,
                );
                let in_n = first.inputs.len().max(1) as f32;
                let end_y = match first.input_style {
                    InputStyle::Individual => {
                        node_rect_screen.top() + node_rect_screen.height() * (1.0 / (in_n + 1.0))
                    }
                    InputStyle::Bar | InputStyle::Collection => node_rect_screen.center().y,
                };
                let port_offset = 10.0 * editor.zoom.sqrt();
                target_end = egui::pos2(node_rect_screen.left() - port_offset, end_y);
            }
        }
        let nsrc = s.sources.len().max(1) as f32;
        for (i, (nid, pid)) in s.sources.iter().cloned().enumerate() {
            let Some((start_pos, _)) = editor.port_locations.get(&(nid, pid)) else {
                continue;
            };
            // Slightly fan-in multiple sources so the preview is readable.
            let k = i as f32;
            let offset_y = (k - (nsrc - 1.0) * 0.5) * (6.0 * editor.zoom.sqrt());
            draw_dashed_bezier(
                ui,
                *start_pos,
                target_end + Vec2::new(0.0, offset_y),
                Stroke::new(2.0, Color32::from_gray(200)),
            );
        }

        let sp = editor_rect.min + s.anchor_graph_pos.to_vec2() * editor.zoom + editor.pan;
        let id = ui.make_persistent_id(("copilot_relay", s.session_id));
        egui::Area::new(id)
            .fixed_pos(sp + Vec2::new(12.0, 12.0))
            .order(egui::Order::Foreground)
            .show(ui.ctx(), |ui| {
                let selected = editor.copilot_relay_selected == Some(s.session_id);
                let bg = if selected {
                    Color32::from_rgba_unmultiplied(40, 60, 90, 210)
                } else {
                    Color32::from_rgba_unmultiplied(20, 20, 30, 190)
                };
                egui::Frame::none()
                    .fill(bg)
                    .stroke(Stroke::new(1.0, Color32::from_gray(80)))
                    .rounding(egui::Rounding::same(6))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        let hdr = format!("{:?}", s.backend);
                        let status = match s.status {
                            crate::tabs_system::node_editor::state::CopilotRelayStatus::Generating => "Generating…",
                            crate::tabs_system::node_editor::state::CopilotRelayStatus::Ready => "Ready",
                            crate::tabs_system::node_editor::state::CopilotRelayStatus::Error => "Error",
                        };
                        let label = format!("{} · {}", hdr, status);
                        let r = ui.interact(ui.min_rect(), id.with("hit"), Sense::click());
                        if r.clicked() {
                            editor.copilot_relay_selected = Some(s.session_id);
                        }
                        ui.label(label);
                        if let Some(g) = &s.ghost {
                            let mut chain = String::new();
                            for (i, n) in g.nodes.iter().take(4).enumerate() {
                                if i > 0 {
                                    chain.push_str(" → ");
                                }
                                chain.push_str(&n.name);
                            }
                            if g.nodes.len() > 4 {
                                chain.push_str(" …");
                            }
                            if !chain.is_empty() {
                                ui.colored_label(Color32::from_gray(180), chain);
                            }
                        }
                        if let Some(e) = &s.error {
                            ui.colored_label(Color32::LIGHT_RED, e);
                        }
                        ui.horizontal(|ui| {
                            let mut mk = |kind| {
                                editor.copilot_relay_actions.push(crate::tabs_system::node_editor::state::CopilotRelayAction { session_id: s.session_id, kind });
                            };
                            match s.status {
                                crate::tabs_system::node_editor::state::CopilotRelayStatus::Generating => {
                                    if ui.small_button("Cancel").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Cancel); }
                                }
                                crate::tabs_system::node_editor::state::CopilotRelayStatus::Ready => {
                                    if ui.small_button("Apply (`)").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Apply); }
                                    if ui.small_button("Reroll (Tab)").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Reroll); }
                                    if ui.small_button("Cancel").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Cancel); }
                                }
                                crate::tabs_system::node_editor::state::CopilotRelayStatus::Error => {
                                    if ui.small_button("Reroll").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Reroll); }
                                    if ui.small_button("Cancel").clicked() { mk(crate::tabs_system::node_editor::state::CopilotRelayActionKind::Cancel); }
                                }
                            }
                        });
                    });
            });
    }
}

pub fn draw_ghost_reason_note(
    ui: &mut egui::Ui,
    editor: &NodeEditorTab,
    ghost: &crate::tabs_system::node_editor::state::GhostGraph,
    editor_rect: Rect,
) {
    let Some(reason) = editor
        .ghost_reason
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let Some(first) = ghost.nodes.first() else {
        return;
    };
    let node_rect = Rect::from_min_size(first.position, first.size);
    let node_screen = Rect::from_min_max(
        editor_rect.min + node_rect.min.to_vec2() * editor.zoom + editor.pan,
        editor_rect.min + node_rect.max.to_vec2() * editor.zoom + editor.pan,
    );
    let pad = 10.0;
    let note_w = 260.0;
    let note_h = 130.0;
    let mut x = node_screen.max.x + 16.0;
    if x + note_w > editor_rect.max.x {
        x = node_screen.min.x - 16.0 - note_w;
    }
    let mut y = node_screen.min.y;
    if y + note_h > editor_rect.max.y {
        y = (editor_rect.max.y - note_h).max(editor_rect.min.y);
    }
    if y < editor_rect.min.y {
        y = editor_rect.min.y;
    }
    let r = Rect::from_min_size(egui::pos2(x, y), egui::vec2(note_w, note_h));
    let bg = Color32::from_rgba_unmultiplied(255, 243, 138, 120);
    ui.painter().rect_filled(r, 6.0, bg);
    ui.painter().rect_stroke(
        r,
        6.0,
        Stroke::new(1.0, Color32::from_gray(40)),
        egui::StrokeKind::Inside,
    );
    let title = editor.ghost_reason_title.as_deref().unwrap_or("Why");
    ui.painter().text(
        r.min + egui::vec2(pad, pad),
        egui::Align2::LEFT_TOP,
        title,
        egui::FontId::proportional(14.0),
        Color32::from_gray(20),
    );
    let galley = ui.fonts_mut(|f| {
        f.layout(
            reason.to_string(),
            egui::FontId::proportional(13.0),
            Color32::from_gray(10),
            note_w - pad * 2.0,
        )
    });
    ui.painter().galley(
        r.min + egui::vec2(pad, pad + 18.0),
        galley,
        Color32::from_gray(10),
    );
}
