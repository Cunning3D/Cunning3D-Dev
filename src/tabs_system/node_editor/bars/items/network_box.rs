use crate::cunning_core::command::basic::{
    CmdBatch, CmdMoveNodes, CmdSetNetworkBoxRect, CmdSetStickyNoteRect,
};
use crate::gpu_text;
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::{node_editor::NodeEditorTab, EditorTabContext};
use bevy_egui::egui::{self, Color32, CornerRadius, Pos2, Rect, Sense, Stroke, Vec2};
use egui_wgpu::sdf::GpuTextUniform;
use egui_wgpu::sdf::{create_sdf_rect_callback, SdfRectUniform};

#[inline]
fn is_foreach_box(t: &str) -> bool {
    t.starts_with("ForEach ")
}

#[inline]
fn cross(o: Vec2, a: Vec2, b: Vec2) -> f32 {
    (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
}

#[inline]
fn signed_area(poly: &[Pos2]) -> f32 {
    let n = poly.len();
    if n < 3 {
        return 0.0;
    }
    let mut a = 0.0;
    for i in 0..n {
        let p = poly[i];
        let q = poly[(i + 1) % n];
        a += p.x * q.y - q.x * p.y;
    }
    a * 0.5
}

fn round_convex(poly: &[Pos2], r: f32, steps: usize) -> Vec<Pos2> {
    let n = poly.len();
    if n < 3 || r <= 0.0 {
        return poly.to_vec();
    }
    let ccw = signed_area(poly) >= 0.0;
    let mut out: Vec<Pos2> = Vec::with_capacity(n * (steps + 2));
    for i in 0..n {
        let p0 = poly[(i + n - 1) % n];
        let p1 = poly[i];
        let p2 = poly[(i + 1) % n];
        let v1 = (p0 - p1).normalized();
        let v2 = (p2 - p1).normalized();
        let s = v1 + v2;
        let cos = v1.dot(v2).clamp(-0.999_9, 0.999_9);
        let sin_half = ((1.0 - cos) * 0.5).sqrt().max(1e-4);
        let tan_half = sin_half / ((1.0 + cos) * 0.5).sqrt().max(1e-4);
        if s.length_sq() < 1e-6 || tan_half <= 1e-4 {
            out.push(p1);
            continue;
        }
        let e1 = (p1 - p0).length();
        let e2 = (p2 - p1).length();
        let rr = r.min(e1 * 0.45).min(e2 * 0.45);
        let t = (rr / tan_half).min(e1).min(e2);
        let a = p1 + v1 * t;
        let b = p1 + v2 * t;
        let bis = s.normalized();
        let c = p1 + bis * (rr / sin_half);
        let mut ang_a = (a - c).angle();
        let mut ang_b = (b - c).angle();
        if ccw {
            while ang_b <= ang_a {
                ang_b += std::f32::consts::TAU;
            }
        } else {
            while ang_b >= ang_a {
                ang_b -= std::f32::consts::TAU;
            }
        }
        out.push(a);
        for s in 1..steps.max(1) {
            let t = s as f32 / steps.max(1) as f32;
            let ang = ang_a + (ang_b - ang_a) * t;
            out.push(c + Vec2::angled(ang) * rr);
        }
        out.push(b);
    }
    out
}

fn convex_hull(mut pts: Vec<Vec2>) -> Vec<Vec2> {
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| a.x == b.x && a.y == b.y);
    if pts.len() <= 2 {
        return pts;
    }
    let mut lo: Vec<Vec2> = Vec::with_capacity(pts.len());
    for p in &pts {
        while lo.len() >= 2 && cross(lo[lo.len() - 2], lo[lo.len() - 1], *p) <= 0.0 {
            lo.pop();
        }
        lo.push(*p);
    }
    let mut hi: Vec<Vec2> = Vec::with_capacity(pts.len());
    for p in pts.iter().rev() {
        while hi.len() >= 2 && cross(hi[hi.len() - 2], hi[hi.len() - 1], *p) <= 0.0 {
            hi.pop();
        }
        hi.push(*p);
    }
    lo.pop();
    hi.pop();
    lo.extend(hi);
    lo
}

pub fn draw_foreach_overlays(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let root_graph = &context.node_graph_res.0;
    let snap = cda::navigation::graph_snapshot_by_path(&root_graph, &editor.cda_state.breadcrumb());
    for box_id in &snap.network_box_draw_order {
        let Some(b) = snap.network_boxes.get(box_id) else {
            continue;
        };
        if !is_foreach_box(&b.title) {
            continue;
        }
        let mut pts: Vec<Vec2> = Vec::new();
        let pad = 30.0;
        let header_h = 28.0;
        for nid in &b.nodes_inside {
            let Some(n) = snap.nodes.get(nid) else {
                continue;
            };
            let r = Rect::from_min_size(n.position, n.size).expand(pad);
            pts.extend([
                r.min.to_vec2(),
                Vec2::new(r.max.x, r.min.y),
                r.max.to_vec2(),
                Vec2::new(r.min.x, r.max.y),
            ]);
        }
        for sid in &b.stickies_inside {
            let Some(s) = snap.sticky_notes.get(sid) else {
                continue;
            };
            let r = s.rect.expand(pad);
            pts.extend([
                r.min.to_vec2(),
                Vec2::new(r.max.x, r.min.y),
                r.max.to_vec2(),
                Vec2::new(r.min.x, r.max.y),
            ]);
        }
        // Ensure the convex hull contains a dedicated header area for the block title/id.
        let hr = Rect::from_min_max(b.rect.min, Pos2::new(b.rect.max.x, b.rect.min.y + header_h));
        pts.extend([
            hr.min.to_vec2(),
            Vec2::new(hr.max.x, hr.min.y),
            hr.max.to_vec2(),
            Vec2::new(hr.min.x, hr.max.y),
        ]);
        let hull = convex_hull(pts);
        if hull.len() < 3 {
            continue;
        }
        let poly: Vec<Pos2> = hull
            .into_iter()
            .map(|p| editor_rect.min + p * editor.zoom + editor.pan)
            .collect();
        let poly = round_convex(&poly, 14.0, 6);
        let sel = context.ui_state.selected_network_boxes.contains(box_id);
        let alpha = (context.node_editor_settings.network_box_fill_alpha.clamp(0.0, 1.0) * 255.0) as u8;
        let fill = Color32::from_rgba_unmultiplied(b.color.r(), b.color.g(), b.color.b(), alpha);
        let stroke = if sel {
            Stroke::new(1.8, context.theme.colors.node_selected)
        } else {
            Stroke::new(1.2, Color32::from_gray(50))
        };
        // Draw fill and stroke separately to ensure outline is always visible.
        ui.painter().add(egui::Shape::convex_polygon(
            poly.clone(),
            fill,
            Stroke::NONE,
        ));
        ui.painter().add(egui::Shape::closed_line(poly, stroke));
    }
}

pub fn draw_network_boxes(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let network_box_ids = {
        let root_graph = &context.node_graph_res.0;
        cda::navigation::graph_snapshot_by_path(&root_graph, &editor.cda_state.breadcrumb())
            .network_box_draw_order
            .clone()
    };

    for box_id in &network_box_ids {
        let root_graph = &mut context.node_graph_res.0;
        let mut box_rect = None;
        let mut box_title = None;
        let mut box_color = None;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |node_graph| {
            if let Some(b) = node_graph.network_boxes.get(box_id) {
                box_rect = Some(b.rect);
                box_title = Some(b.title.clone());
                box_color = Some(b.color);
            }
        });
        let (Some(mut box_rect), Some(mut box_title), Some(box_color)) =
            (box_rect, box_title, box_color)
        else {
            continue;
        };
        let foreach = is_foreach_box(&box_title);
        let box_rect_screen = Rect::from_min_max(
            editor_rect.min + box_rect.min.to_vec2() * editor.zoom + editor.pan,
            editor_rect.min + box_rect.max.to_vec2() * editor.zoom + editor.pan,
        );

        // --- Drawing ---
        let is_selected = context.ui_state.selected_network_boxes.contains(box_id);
        let alpha = (context.node_editor_settings.network_box_fill_alpha.clamp(0.0, 1.0) * 255.0) as u8;
        let fill_color = Color32::from_rgba_unmultiplied(box_color.r(), box_color.g(), box_color.b(), alpha);
        let stroke_color = if is_selected {
            context.theme.colors.node_selected
        } else {
            Color32::from_gray(50)
        };
        if !foreach {
            let screen_size = ui.ctx().screen_rect().size();
            let c = box_rect_screen.center();
            let s = box_rect_screen.size();
            let rounding = CornerRadius::from(5.0);
            let fill_rgba = bevy_egui::egui::Rgba::from(fill_color).to_array();
            let border_rgba = bevy_egui::egui::Rgba::from(stroke_color).to_array();
            let uniform = SdfRectUniform {
                center: [c.x, c.y],
                half_size: [s.x * 0.5, s.y * 0.5],
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
                border_width: if is_selected { 1.5 * context.node_editor_settings.aux_selected_border_width_mul.max(1.0) } else { 1.5 },
                _pad2: [0.0; 3],
                border_color: border_rgba,
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            let frame_id = ui.ctx().cumulative_frame_nr();
            ui.painter().add(create_sdf_rect_callback(
                box_rect_screen.expand(6.0),
                uniform,
                frame_id,
            ));
        }

        let title_bar_height = 28.0 * editor.zoom;
        let title_bar_rect = Rect::from_min_size(
            box_rect_screen.min,
            Vec2::new(box_rect_screen.width(), title_bar_height),
        );

        if !foreach && editor.editing_box_title_id == Some(*box_id) {
            let mut temp_title = box_title.clone();
            let text_edit_response = ui.put(
                title_bar_rect.shrink(4.0),
                egui::TextEdit::singleline(&mut temp_title)
                    .id_source(("network_box_title", box_id)),
            );
            box_title = temp_title.clone();
            {
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        if let Some(b) = node_graph.network_boxes.get_mut(box_id) {
                            b.title = temp_title;
                        }
                    },
                );
            }

            if text_edit_response.lost_focus() || ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                editor.editing_box_title_id = None;
            } else {
                ui.ctx()
                    .memory_mut(|m| m.request_focus(text_edit_response.id));
            }
        } else {
            let painter = ui.painter();
            if foreach {
                // Houdini-like header: "ForEach" + block id separated, never overlaps content.
                let frame_id = ui.ctx().cumulative_frame_nr();
                let bg = Color32::from_rgba_unmultiplied(0, 0, 0, 46);
                ui.painter()
                    .rect_filled(title_bar_rect, 6.0 * editor.zoom, bg);
                let (label, bid) = box_title
                    .strip_prefix("ForEach ")
                    .map(|s| ("ForEach", s.trim()))
                    .unwrap_or(("ForEach", ""));
                let font_px = 14.0 * editor.zoom;
                let color = Color32::from_gray(235);
                let weak = Color32::from_gray(200);
                let pad_x = 8.0 * editor.zoom;
                let left = title_bar_rect.left_center() + Vec2::new(pad_x, 0.0);
                gpu_text::paint(
                    painter,
                    GpuTextUniform {
                        text: label.into(),
                        pos: left,
                        color,
                        font_px,
                        bounds: Vec2::new(title_bar_rect.width() * 0.6, title_bar_rect.height()),
                        family: 0,
                    },
                    frame_id,
                );
                if !bid.is_empty() {
                    let right = title_bar_rect.right_center() - Vec2::new(pad_x, 0.0);
                    let galley = ui.fonts_mut(|f| {
                        f.layout_no_wrap(bid.to_string(), egui::FontId::proportional(font_px), weak)
                    });
                    let r = egui::Align2::RIGHT_CENTER.anchor_size(right, galley.size());
                    gpu_text::paint(
                        painter,
                        GpuTextUniform {
                            text: bid.to_string(),
                            pos: r.min,
                            color: weak,
                            font_px,
                            bounds: r.size(),
                            family: 0,
                        },
                        frame_id,
                    );
                }
            } else {
                let pos = title_bar_rect.left_center() + Vec2::new(5.0 * editor.zoom, 0.0);
                let anchor = egui::Align2::LEFT_CENTER;
                let font_px = 14.0 * editor.zoom;
                let color = Color32::WHITE;
                let text = box_title.clone();
                let galley = ui.fonts_mut(|f| {
                    f.layout_no_wrap(text.clone(), egui::FontId::proportional(font_px), color)
                });
                let r = anchor.anchor_size(pos, galley.size());
                let frame_id = ui.ctx().cumulative_frame_nr();
                gpu_text::paint(
                    painter,
                    GpuTextUniform {
                        text,
                        pos: r.min,
                        color,
                        font_px,
                        bounds: r.size(),
                        family: 0,
                    },
                    frame_id,
                );
            }
        }

        // --- Resize Handle ---
        if !foreach {
            let resize_handle_size = Vec2::splat(10.0);
            let resize_handle_rect = Rect::from_min_size(
                box_rect_screen.right_bottom() - resize_handle_size,
                resize_handle_size,
            );
            let resize_response = ui.interact(
                resize_handle_rect,
                ui.make_persistent_id(box_id).with("resize"),
                Sense::drag(),
            );
            if resize_response.hovered() || resize_response.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
            }
            if resize_response.dragged_by(egui::PointerButton::Primary) {
                let drag_delta_graph = resize_response.drag_delta() / editor.zoom;
                let min_size = Vec2::new(100.0, 80.0);
                let new_max = box_rect.max + drag_delta_graph;
                if new_max.x - box_rect.min.x >= min_size.x {
                    box_rect.max.x = new_max.x;
                }
                if new_max.y - box_rect.min.y >= min_size.y {
                    box_rect.max.y = new_max.y;
                }
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        if let Some(b) = node_graph.network_boxes.get_mut(box_id) {
                            b.rect = box_rect;
                        }
                    },
                );
            }
            if resize_response.drag_started_by(egui::PointerButton::Primary) {
                editor.box_rect_start.insert(*box_id, box_rect);
            }
            if resize_response.drag_stopped_by(egui::PointerButton::Primary) {
                if let Some(old) = editor.box_rect_start.remove(box_id) {
                    let new = box_rect;
                    if old != new {
                        context
                            .node_editor_state
                            .record(Box::new(CmdSetNetworkBoxRect::new(*box_id, old, new)));
                        context.graph_changed_writer.write_default();
                    }
                }
            }
        }

        // --- Interaction ---
        let title_response = ui.interact(
            title_bar_rect,
            ui.make_persistent_id(box_id),
            Sense::click_and_drag(),
        );

        if title_response.double_clicked() {
            editor.editing_box_title_id = Some(*box_id);
        }

        if title_response.drag_started_by(egui::PointerButton::Primary) {
            editor.selection_start = None; // Cancel marquee while dragging.
            if !context.ui_state.selected_network_boxes.contains(box_id) {
                if !ui.input(|i| i.modifiers.shift) {
                    context.ui_state.selected_nodes.clear();
                    context.ui_state.selected_connections.clear();
                    context.ui_state.selected_network_boxes.clear();
                    context.ui_state.selected_promote_notes.clear();
                    context.ui_state.selected_sticky_notes.clear();
                }
                context.ui_state.selected_network_boxes.insert(*box_id);
            }
            // TODO: Bring to front

            editor.box_rect_start.clear();
            editor.sticky_rect_start.clear();
            editor.drag_start_positions.clear();
            {
                let selected_boxes: Vec<_> = context.ui_state.selected_network_boxes.iter().copied().collect();
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        for selected_box_id in selected_boxes {
                            if let Some(b) = node_graph.network_boxes.get(&selected_box_id) {
                                editor.box_rect_start.insert(selected_box_id, b.rect);
                                for nid in &b.nodes_inside {
                                    if let Some(n) = node_graph.nodes.get(nid) {
                                        editor.drag_start_positions.insert(*nid, n.position);
                                    }
                                }
                                for sid in &b.stickies_inside {
                                    if let Some(s) = node_graph.sticky_notes.get(sid) {
                                        editor.sticky_rect_start.insert(*sid, s.rect);
                                    }
                                }
                            }
                        }
                        for sid in context.ui_state.selected_sticky_notes.iter().copied() {
                            if let Some(s) = node_graph.sticky_notes.get(&sid) {
                                editor.sticky_rect_start.insert(sid, s.rect);
                            }
                        }
                        for nid in context.ui_state.selected_nodes.iter().copied() {
                            if let Some(n) = node_graph.nodes.get(&nid) {
                                editor.drag_start_positions.insert(nid, n.position);
                            }
                        }
                    },
                );
            }
        }

        if title_response.dragged_by(egui::PointerButton::Primary) {
            let drag_delta_graph = title_response.drag_delta() / editor.zoom;
            if context.ui_state.selected_network_boxes.contains(box_id) {
                let selected_boxes: Vec<_> = context.ui_state.selected_network_boxes.iter().copied().collect();
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        let mut move_nodes: std::collections::HashSet<crate::nodes::NodeId> =
                            context.ui_state.selected_nodes.iter().copied().collect();
                        let mut move_stickies: std::collections::HashSet<crate::nodes::StickyNoteId> =
                            context.ui_state.selected_sticky_notes.iter().copied().collect();
                        for selected_box_id in &selected_boxes {
                            if let Some(b) = node_graph.network_boxes.get_mut(selected_box_id) {
                                b.rect.min += drag_delta_graph;
                                b.rect.max += drag_delta_graph;
                            }
                            if let Some(b) = node_graph.network_boxes.get(selected_box_id) {
                                for nid in &b.nodes_inside { move_nodes.insert(*nid); }
                                for sid in &b.stickies_inside { move_stickies.insert(*sid); }
                            }
                        }
                        for sticky_id in move_stickies {
                            if let Some(sticky) = node_graph.sticky_notes.get_mut(&sticky_id) {
                                sticky.rect.min += drag_delta_graph;
                                sticky.rect.max += drag_delta_graph;
                            }
                        }
                        for node_id in move_nodes {
                            if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                                node.position += drag_delta_graph;
                            }
                        }
                    },
                );
                // Keep visual cache in sync for any moved nodes (selected + box-contained).
                let root_graph = &context.node_graph_res.0;
                let snap = cda::navigation::graph_snapshot_by_path(&root_graph, &editor.cda_state.breadcrumb());
                let mut moved: std::collections::HashSet<crate::nodes::NodeId> =
                    context.ui_state.selected_nodes.iter().copied().collect();
                for bid in &selected_boxes {
                    if let Some(b) = snap.network_boxes.get(bid) {
                        moved.extend(b.nodes_inside.iter().copied());
                    }
                }
                for n in editor.cached_nodes.iter_mut() {
                    if moved.contains(&n.id) {
                        n.position += drag_delta_graph;
                    }
                }
                editor.geometry_rev = editor.geometry_rev.wrapping_add(1);
            }
        }

        if title_response.drag_stopped_by(egui::PointerButton::Primary) {
            let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
            {
                let root_graph = &mut context.node_graph_res.0;
                cda::navigation::with_current_graph_mut(
                    root_graph,
                    &editor.cda_state,
                    |node_graph| {
                        for (bid, old) in editor.box_rect_start.drain() {
                            if let Some(b) = node_graph.network_boxes.get(&bid) {
                                let new = b.rect;
                                if old != new {
                                    cmds.push(Box::new(CmdSetNetworkBoxRect::new(bid, old, new)));
                                }
                            }
                        }
                        let mut moved: Vec<(
                            crate::nodes::NodeId,
                            bevy_egui::egui::Pos2,
                            bevy_egui::egui::Pos2,
                        )> = Vec::new();
                        for (nid, old) in editor.drag_start_positions.drain() {
                            if let Some(n) = node_graph.nodes.get(&nid) {
                                let new = n.position;
                                if old != new {
                                    moved.push((nid, old, new));
                                }
                            }
                        }
                        if !moved.is_empty() {
                            cmds.push(Box::new(CmdMoveNodes::new(moved)));
                        }
                        for (sid, old) in editor.sticky_rect_start.drain() {
                            if let Some(s) = node_graph.sticky_notes.get(&sid) {
                                let new = s.rect;
                                if old != new {
                                    cmds.push(Box::new(CmdSetStickyNoteRect::new(sid, old, new)));
                                }
                            }
                        }
                    },
                );
            }
            if !cmds.is_empty() {
                context
                    .node_editor_state
                    .record(Box::new(CmdBatch::new("Move Network Box", cmds)));
                context.graph_changed_writer.write_default();
            }
        }
    }
}
