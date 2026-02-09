use crate::cunning_core::command::basic::{
    CmdBatch, CmdMoveNodes, CmdSetNetworkBoxRect, CmdSetStickyNoteRect,
};
use crate::gpu_text;
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::{node_editor::NodeEditorTab, EditorTabContext};
use bevy_egui::egui::{self, Color32, CornerRadius, Rect, Sense, Vec2};
use egui_wgpu::sdf::GpuTextUniform;
use egui_wgpu::sdf::{create_sdf_rect_callback, SdfRectUniform};

pub fn draw_sticky_notes(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let sticky_note_ids = {
        let root_graph = &context.node_graph_res.0;
        cda::navigation::graph_snapshot_by_path(&root_graph, &editor.cda_state.breadcrumb())
            .sticky_note_draw_order
            .clone()
    };

    for note_id in &sticky_note_ids {
        let root_graph = &mut context.node_graph_res.0;
        cda::navigation::with_current_graph_mut(root_graph, &editor.cda_state, |node_graph| {
            let Some(note0) = node_graph.sticky_notes.get(note_id).cloned() else { return; };
            let mut note = note0;
                let note_rect_screen = Rect::from_min_max(
                    editor_rect.min + note.rect.min.to_vec2() * editor.zoom + editor.pan,
                    editor_rect.min + note.rect.max.to_vec2() * editor.zoom + editor.pan,
                );
                let select_resp = ui.interact(
                    note_rect_screen,
                    ui.make_persistent_id(note_id).with("select"),
                    Sense::click(),
                );
                if select_resp.clicked() {
                    if !ui.input(|i| i.modifiers.shift) {
                        context.ui_state.selected_nodes.clear();
                        context.ui_state.selected_connections.clear();
                        context.ui_state.selected_network_boxes.clear();
                        context.ui_state.selected_promote_notes.clear();
                        context.ui_state.selected_sticky_notes.clear();
                    }
                    context.ui_state.selected_sticky_notes.insert(*note_id);
                }

                // --- Drawing ---
                {
                    let screen_size = ui.ctx().content_rect().size();
                    let c = note_rect_screen.center();
                    let s = note_rect_screen.size();
                    let rounding = CornerRadius::from(5.0);
                    let fill_rgba = bevy_egui::egui::Rgba::from(note.color).to_array();
                    let sel = context.ui_state.selected_sticky_notes.contains(note_id);
                    let border_c = if sel { context.theme.colors.node_selected } else { Color32::BLACK };
                    let border_w = if sel { 1.0 * context.node_editor_settings.aux_selected_border_width_mul.max(1.0) } else { 1.0 };
                    let border_rgba = bevy_egui::egui::Rgba::from(border_c).to_array();
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
                        border_width: border_w,
                        _pad2: [0.0; 3],
                        border_color: border_rgba,
                        screen_size: [screen_size.x, screen_size.y],
                        _pad3: [0.0; 2],
                    };
                    let frame_id = ui.ctx().cumulative_frame_nr();
                    ui.painter().add(create_sdf_rect_callback(
                        note_rect_screen.expand(6.0),
                        uniform,
                        frame_id,
                    ));
                }

                let title_bar_height = 24.0 * editor.zoom;
                let title_bar_height_graph = 24.0;
                let title_bar_rect = Rect::from_min_size(
                    note_rect_screen.min,
                    Vec2::new(note_rect_screen.width(), title_bar_height),
                );
                let content_rect =
                    Rect::from_min_max(title_bar_rect.left_bottom(), note_rect_screen.max);

                // --- Title ---
                if editor.editing_sticky_note_title_id == Some(*note_id) {
                    let mut temp_title = note.title.clone();
                    let text_edit_response = ui.put(
                        title_bar_rect.shrink(4.0),
                        egui::TextEdit::singleline(&mut temp_title)
                            .id_source(("sticky_note_title", note_id))
                            .text_color(Color32::BLACK),
                    );
                    note.title = temp_title;

                    if text_edit_response.lost_focus()
                        || ui.input(|i| i.key_pressed(egui::Key::Enter))
                    {
                        editor.editing_sticky_note_title_id = None;
                    } else {
                        ui.ctx()
                            .memory_mut(|m| m.request_focus(text_edit_response.id));
                    }
                } else {
                    let painter = ui.painter();
                    let pos = title_bar_rect.left_center() + Vec2::new(5.0 * editor.zoom, 0.0);
                    let anchor = egui::Align2::LEFT_CENTER;
                    let font_px = 14.0 * editor.zoom;
                    let color = Color32::BLACK;
                    let text = note.title.clone();
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

                // --- Separator Line ---
                let line_start = title_bar_rect.left_bottom();
                let line_end = title_bar_rect.right_bottom();
                {
                    let screen_size = ui.ctx().content_rect().size();
                    let y = line_start.y;
                    let h = 1.0;
                    let rect = Rect::from_min_max(
                        egui::pos2(line_start.x, y - h * 0.5),
                        egui::pos2(line_end.x, y + h * 0.5),
                    );
                    let c = rect.center();
                    let s = rect.size();
                    let fill_rgba = bevy_egui::egui::Rgba::from(Color32::BLACK).to_array();
                    let uniform = SdfRectUniform {
                        center: [c.x, c.y],
                        half_size: [s.x * 0.5, s.y * 0.5],
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
                    ui.painter().add(create_sdf_rect_callback(
                        rect.expand(2.0),
                        uniform,
                        frame_id,
                    ));
                }

                // --- Content ---
                if editor.editing_sticky_note_content_id == Some(*note_id) {
                    let mut temp_content = note.content.clone();
                    let text_edit_response = ui.put(
                        content_rect.shrink(4.0),
                        egui::TextEdit::multiline(&mut temp_content)
                            .id_source(("sticky_note_content", note_id))
                            .frame(false)
                            .text_color(Color32::BLACK)
                            .hint_text("Type something..."),
                    );
                    if text_edit_response.changed() {
                        note.content = temp_content;
                    }

                    if text_edit_response.lost_focus() {
                        editor.editing_sticky_note_content_id = None;
                    } else {
                        ui.ctx()
                            .memory_mut(|m| m.request_focus(text_edit_response.id));
                    }
                } else {
                    let content_response = ui.interact(
                        content_rect,
                        ui.make_persistent_id(note_id).with("content"),
                        Sense::click_and_drag(),
                    );
                    if content_response.double_clicked() {
                        editor.editing_sticky_note_content_id = Some(*note_id);
                    }

                    let (text_to_draw, text_color) = if note.content.is_empty() {
                        ("Double-click to type...".to_string(), Color32::GRAY)
                    } else {
                        (note.content.clone(), Color32::BLACK)
                    };

                    // Auto-resize height based on wrapped text (engineering, not AI)
                    let font_px = (14.0 * editor.zoom).clamp(7.0, 18.0);
                    let wrap_w = (note.rect.width() * editor.zoom - 8.0).max(10.0);
                    let galley = ui.fonts_mut(|f| {
                        f.layout(
                            text_to_draw.clone(),
                            egui::FontId::proportional(font_px),
                            text_color,
                            wrap_w,
                        )
                    });
                    let content_h_graph = (galley.size().y / editor.zoom).max(18.0);
                    let min_h_graph = title_bar_height_graph + 80.0;
                    let new_h_graph =
                        (title_bar_height_graph + content_h_graph + 8.0).max(min_h_graph);
                    let new_max_y = note.rect.min.y + new_h_graph;
                    if (note.rect.max.y - new_max_y).abs() > 0.5 {
                        note.rect.max.y = new_max_y;
                    }

                    ui.scope_builder(
                        egui::UiBuilder::new().max_rect(content_rect.shrink(4.0)),
                        |ui| {
                            ui.set_clip_rect(content_rect.shrink(4.0));
                            ui.add(
                                egui::Label::new(
                                    egui::RichText::new(text_to_draw.clone())
                                        .color(text_color)
                                        .font(egui::FontId::proportional(font_px)),
                                )
                                .wrap(),
                            );
                        },
                    );
                    let _ = content_response;
                }

                // --- Resize Handle ---
                let resize_handle_size = Vec2::splat(10.0);
                let resize_handle_rect = Rect::from_min_size(
                    note_rect_screen.right_bottom() - resize_handle_size,
                    resize_handle_size,
                );
                let resize_response = ui.interact(
                    resize_handle_rect,
                    ui.make_persistent_id(note_id).with("resize"),
                    Sense::drag(),
                );

                if resize_response.hovered() || resize_response.dragged() {
                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
                }

                if resize_response.dragged_by(egui::PointerButton::Primary) {
                    let drag_delta_graph = resize_response.drag_delta() / editor.zoom;
                    let min_size = Vec2::new(100.0, 80.0);

                    let new_max = note.rect.max + drag_delta_graph;
                    if new_max.x - note.rect.min.x >= min_size.x {
                        note.rect.max.x = new_max.x;
                    }
                    if new_max.y - note.rect.min.y >= min_size.y {
                        note.rect.max.y = new_max.y;
                    }
                }

                let title_response = ui.interact(
                    title_bar_rect,
                    ui.make_persistent_id(*note_id),
                    Sense::click_and_drag(),
                );
                if title_response.double_clicked() {
                    editor.editing_sticky_note_title_id = Some(*note_id);
                }

                if title_response.drag_started_by(egui::PointerButton::Primary) {
                    editor.selection_start = None; // Cancel marquee while dragging.
                    if !context.ui_state.selected_sticky_notes.contains(note_id) {
                        if !ui.input(|i| i.modifiers.shift) {
                            context.ui_state.selected_nodes.clear();
                            context.ui_state.selected_connections.clear();
                            context.ui_state.selected_network_boxes.clear();
                            context.ui_state.selected_promote_notes.clear();
                            context.ui_state.selected_sticky_notes.clear();
                        }
                        context.ui_state.selected_sticky_notes.insert(*note_id);
                    }
                    editor.box_rect_start.clear();
                    editor.sticky_rect_start.clear();
                    editor.drag_start_positions.clear();
                    let selected_boxes: Vec<_> = context.ui_state.selected_network_boxes.iter().copied().collect();
                    let selected_stickies: Vec<_> = context.ui_state.selected_sticky_notes.iter().copied().collect();
                    let selected_nodes: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
                    for bid in selected_boxes {
                        if let Some(b) = node_graph.network_boxes.get(&bid) {
                            editor.box_rect_start.insert(bid, b.rect);
                        }
                    }
                    for sid in selected_stickies {
                        if sid == *note_id {
                            editor.sticky_rect_start.insert(sid, note.rect);
                        } else if let Some(s) = node_graph.sticky_notes.get(&sid) {
                            editor.sticky_rect_start.insert(sid, s.rect);
                        }
                    }
                    for nid in selected_nodes {
                        if let Some(n) = node_graph.nodes.get(&nid) {
                            editor.drag_start_positions.insert(nid, n.position);
                        }
                    }
                }
                if title_response.dragged() {
                    let drag_delta_graph = title_response.drag_delta() / editor.zoom;
                    if context.ui_state.selected_sticky_notes.contains(note_id) {
                        let selected_boxes: Vec<_> = context.ui_state.selected_network_boxes.iter().copied().collect();
                        let selected_stickies: Vec<_> = context.ui_state.selected_sticky_notes.iter().copied().collect();
                        let selected_nodes: Vec<_> = context.ui_state.selected_nodes.iter().copied().collect();
                        for bid in selected_boxes {
                            if let Some(b) = node_graph.network_boxes.get_mut(&bid) {
                                b.rect.min += drag_delta_graph;
                                b.rect.max += drag_delta_graph;
                            }
                        }
                        for sid in selected_stickies {
                            if sid == *note_id {
                                note.rect.min += drag_delta_graph;
                                note.rect.max += drag_delta_graph;
                            } else if let Some(s) = node_graph.sticky_notes.get_mut(&sid) {
                                s.rect.min += drag_delta_graph;
                                s.rect.max += drag_delta_graph;
                            }
                        }
                        for nid in selected_nodes {
                            if let Some(n) = node_graph.nodes.get_mut(&nid) {
                                n.position += drag_delta_graph;
                            }
                        }
                        for n in editor.cached_nodes.iter_mut() {
                            if context.ui_state.selected_nodes.contains(&n.id) {
                                n.position += drag_delta_graph;
                            }
                        }
                        editor.geometry_rev = editor.geometry_rev.wrapping_add(1);
                    } else {
                        note.rect.min += drag_delta_graph;
                        note.rect.max += drag_delta_graph;
                    }
                }
                if title_response.drag_stopped_by(egui::PointerButton::Primary) {
                    let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                    for (bid, old) in editor.box_rect_start.drain() {
                        if let Some(b) = node_graph.network_boxes.get(&bid) {
                            let new = b.rect;
                            if old != new { cmds.push(Box::new(CmdSetNetworkBoxRect::new(bid, old, new))); }
                        }
                    }
                    for (sid, old) in editor.sticky_rect_start.drain() {
                        let new = if sid == *note_id { note.rect } else { node_graph.sticky_notes.get(&sid).map(|s| s.rect).unwrap_or(old) };
                        if old != new { cmds.push(Box::new(CmdSetStickyNoteRect::new(sid, old, new))); }
                    }
                    let mut moved: Vec<(crate::nodes::NodeId, bevy_egui::egui::Pos2, bevy_egui::egui::Pos2)> = Vec::new();
                    for (nid, old) in editor.drag_start_positions.drain() {
                        if let Some(n) = node_graph.nodes.get(&nid) {
                            let new = n.position;
                            if old != new { moved.push((nid, old, new)); }
                        }
                    }
                    if !moved.is_empty() { cmds.push(Box::new(CmdMoveNodes::new(moved))); }
                    if !cmds.is_empty() {
                        context.node_editor_state.record(Box::new(CmdBatch::new("Move Sticky Note", cmds)));
                        context.graph_changed_writer.write_default();
                    }
                }

                if resize_response.drag_started_by(egui::PointerButton::Primary) {
                    editor.sticky_rect_start.insert(*note_id, note.rect);
                }
                if resize_response.drag_stopped_by(egui::PointerButton::Primary) {
                    if let Some(old) = editor.sticky_rect_start.remove(note_id) {
                        let new = note.rect;
                        if old != new {
                            context
                                .node_editor_state
                                .record(Box::new(CmdSetStickyNoteRect::new(*note_id, old, new)));
                            context.graph_changed_writer.write_default();
                        }
                    }
                }
            if let Some(slot) = node_graph.sticky_notes.get_mut(note_id) {
                *slot = note;
            }
        });
    }
}
