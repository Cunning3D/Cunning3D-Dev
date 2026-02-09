//! PromoteNote: voice/text input for Copilot intent injection, 10s auto-destroy after use
use crate::cunning_core::command::basic::CmdRemovePromoteNote;
use crate::gpu_text;
use crate::libs::voice::{VoiceCommand, VoiceEvent};
use crate::nodes::structs::{PromoteNote, PromoteNoteId};
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::{node_editor::NodeEditorTab, EditorTabContext};
use bevy_egui::egui::{self, Color32, CornerRadius, Rect, Sense, Vec2};
use egui_wgpu::sdf::GpuTextUniform;
use egui_wgpu::sdf::{create_sdf_rect_callback, SdfRectUniform};

const PROMOTE_NOTE_DESTROY_SECS: f64 = 10.0;
const PROMOTE_NOTE_COLOR: Color32 = Color32::from_rgb(255, 200, 100);

pub fn draw_promote_notes(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let breadcrumb = editor.cda_state.breadcrumb();
    let note_ids = {
        let root = &context.node_graph_res.0;
        cda::navigation::graph_snapshot_by_path(&root, &breadcrumb)
            .promote_note_draw_order
            .clone()
    };
    let mut to_remove: Vec<PromoteNoteId> = Vec::new();
    let zoom = editor.zoom;
    let pan = editor.pan;
    for note_id in &note_ids {
        let result = {
            let root = &mut context.node_graph_res.0;
            cda::navigation::with_graph_by_path_mut(root, &breadcrumb, |graph| {
            let Some(note) = graph.promote_notes.get_mut(note_id) else {
                return (false, None);
            };
            if let Some(used) = note.used_at {
                if used.elapsed().as_secs_f64() >= PROMOTE_NOTE_DESTROY_SECS && !note.pinned {
                    return (true, None);
                }
            }
            let note_rect_screen = Rect::from_min_max(
                editor_rect.min + note.rect.min.to_vec2() * zoom + pan,
                editor_rect.min + note.rect.max.to_vec2() * zoom + pan,
            );
            let title_h = 24.0 * zoom;
            let title_rect = Rect::from_min_size(
                note_rect_screen.min,
                Vec2::new(note_rect_screen.width(), title_h),
            );
            let content_rect = Rect::from_min_max(title_rect.left_bottom(), note_rect_screen.max);
            (
                false,
                Some((note.clone(), note_rect_screen, title_rect, content_rect)),
            )
            })
        };
        let (should_remove, data) = result;
        if should_remove {
            to_remove.push(*note_id);
            continue;
        }
        let Some((mut note_clone, note_rect_screen, title_rect, content_rect)) = data else {
            continue;
        };
        let select_resp = ui.interact(
            note_rect_screen,
            ui.make_persistent_id(note_id).with("select"),
            Sense::click(),
        );
        if select_resp.clicked() {
            if !ui.input(|i| i.modifiers.shift) {
                context.ui_state.selected_promote_notes.clear();
            }
            context.ui_state.selected_promote_notes.insert(*note_id);
        }
        draw_note_bg(ui, note_rect_screen, note_clone.color);
        draw_title_bar(ui, editor, context, *note_id, &mut note_clone, title_rect);
        draw_content(ui, editor, *note_id, &mut note_clone, content_rect, zoom);
        draw_resize_handle(
            ui,
            editor,
            context,
            *note_id,
            &mut note_clone,
            note_rect_screen,
            zoom,
        );
        handle_drag(
            ui,
            editor,
            context,
            *note_id,
            &mut note_clone,
            title_rect,
            zoom,
        );
        // Write back changes
        let root = &mut context.node_graph_res.0;
        cda::navigation::with_graph_by_path_mut(root, &breadcrumb, |g| {
            if let Some(n) = g.promote_notes.get_mut(note_id) {
                *n = note_clone;
            }
        });
    }
    for id in to_remove {
        context.ui_state.selected_promote_notes.remove(&id); // Clear selection before destroy
        let root = &mut context.node_graph_res.0;
        cda::navigation::with_graph_by_path_mut(root, &breadcrumb, |g| {
            context
                .node_editor_state
                .execute(Box::new(CmdRemovePromoteNote::new(id)), g);
        });
        context.graph_changed_writer.write_default();
    }
}

fn draw_note_bg(ui: &mut egui::Ui, rect: Rect, color: Color32) {
    let screen_size = ui.ctx().screen_rect().size();
    let c = rect.center();
    let s = rect.size();
    let rounding = CornerRadius::from(5.0);
    let fill_rgba = bevy_egui::egui::Rgba::from(color).to_array();
    let border_rgba = bevy_egui::egui::Rgba::from(Color32::from_rgb(180, 120, 60)).to_array();
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
        border_width: 2.0,
        _pad2: [0.0; 3],
        border_color: border_rgba,
        screen_size: [screen_size.x, screen_size.y],
        _pad3: [0.0; 2],
    };
    let frame_id = ui.ctx().cumulative_frame_nr();
    ui.painter().add(create_sdf_rect_callback(
        rect.expand(6.0),
        uniform,
        frame_id,
    ));
}

fn draw_title_bar(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    note_id: PromoteNoteId,
    note: &mut PromoteNote,
    title_rect: Rect,
) {
    let font_px = 12.0 * editor.zoom;
    let frame_id = ui.ctx().cumulative_frame_nr();
    let title_text = if let Some(used) = note.used_at {
        let remain = (PROMOTE_NOTE_DESTROY_SECS - used.elapsed().as_secs_f64()).max(0.0);
        if note.pinned {
            "PROMOTE [pinned]".to_string()
        } else {
            format!("PROMOTE ({:.0}s)", remain)
        }
    } else {
        "PROMOTE".to_string()
    };
    let text_pos = title_rect.left_center() + Vec2::new(5.0 * editor.zoom, 0.0);
    gpu_text::paint(
        ui.painter(),
        GpuTextUniform {
            text: title_text,
            pos: text_pos,
            color: Color32::BLACK,
            font_px,
            bounds: title_rect.size(),
            family: 0,
        },
        frame_id,
    );
    let btn_size = 18.0 * editor.zoom;
    let btn_rect = Rect::from_min_size(
        title_rect.right_top() + Vec2::new(-btn_size - 4.0, 3.0 * editor.zoom),
        Vec2::splat(btn_size),
    );
    let is_recording = editor.promote_note_recording_id == Some(note_id);
    let btn_color = if is_recording {
        Color32::RED
    } else {
        Color32::from_rgb(100, 180, 100)
    };
    ui.painter()
        .circle_filled(btn_rect.center(), btn_size * 0.4, btn_color);
    let btn_resp = ui.interact(
        btn_rect,
        ui.make_persistent_id(note_id).with("voice"),
        Sense::click(),
    );
    if btn_resp.clicked() {
        if is_recording {
            editor.promote_note_recording_id = None;
            if let Some(vs) = &context.voice_service {
                vs.send(VoiceCommand::StopListening);
            }
        } else {
            editor.promote_note_recording_id = Some(note_id);
            if let Some(vs) = &context.voice_service {
                vs.send(VoiceCommand::StartListening);
            }
        }
    }
    if note.used_at.is_some() && !note.pinned {
        let pin_rect = Rect::from_min_size(
            title_rect.min + Vec2::new(4.0, 0.0),
            Vec2::new(title_rect.width() - btn_size - 12.0, title_rect.height()),
        );
        let pin_resp = ui.interact(
            pin_rect,
            ui.make_persistent_id(note_id).with("pin"),
            Sense::click(),
        );
        if pin_resp.clicked() {
            note.pinned = true;
        }
    }
}

fn draw_content(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    note_id: PromoteNoteId,
    note: &mut PromoteNote,
    content_rect: Rect,
    zoom: f32,
) {
    if editor.editing_promote_note_id == Some(note_id) {
        let mut temp = note.content.clone();
        let resp = ui.put(
            content_rect.shrink(4.0),
            egui::TextEdit::multiline(&mut temp)
                .id_source(("promote_note", note_id))
                .frame(false)
                .hint_text("Type your intent..."),
        );
        if resp.changed() {
            note.content = temp;
        }
        if resp.lost_focus() {
            editor.editing_promote_note_id = None;
        } else {
            ui.ctx().memory_mut(|m| m.request_focus(resp.id));
        }
    } else {
        let resp = ui.interact(
            content_rect,
            ui.make_persistent_id(note_id).with("content"),
            Sense::click_and_drag(),
        );
        if resp.double_clicked() {
            editor.editing_promote_note_id = Some(note_id);
        }
        let (text, color) = if note.content.is_empty() {
            ("Double-click to type...".into(), Color32::GRAY)
        } else {
            (note.content.clone(), Color32::BLACK)
        };
        let font_px = 13.0 * zoom;
        let frame_id = ui.ctx().cumulative_frame_nr();
        gpu_text::paint(
            ui.painter(),
            GpuTextUniform {
                text,
                pos: content_rect.shrink(4.0).min,
                color,
                font_px,
                bounds: content_rect.shrink(4.0).size(),
                family: 0,
            },
            frame_id,
        );
    }
}

fn draw_resize_handle(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    note_id: PromoteNoteId,
    note: &mut PromoteNote,
    note_rect_screen: Rect,
    zoom: f32,
) {
    let handle_size = Vec2::splat(10.0);
    let handle_rect =
        Rect::from_min_size(note_rect_screen.right_bottom() - handle_size, handle_size);
    let resp = ui.interact(
        handle_rect,
        ui.make_persistent_id(note_id).with("resize"),
        Sense::drag(),
    );
    if resp.hovered() || resp.dragged() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeNwSe);
    }
    if resp.drag_started_by(egui::PointerButton::Primary) {
        editor.promote_note_rect_start.insert(note_id, note.rect);
    }
    if resp.dragged_by(egui::PointerButton::Primary) {
        let delta = resp.drag_delta() / zoom;
        let min_size = Vec2::new(100.0, 60.0);
        let new_max = note.rect.max + delta;
        if new_max.x - note.rect.min.x >= min_size.x {
            note.rect.max.x = new_max.x;
        }
        if new_max.y - note.rect.min.y >= min_size.y {
            note.rect.max.y = new_max.y;
        }
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary) {
        if let Some(old) = editor.promote_note_rect_start.remove(&note_id) {
            let new = note.rect;
            if old != new {
                context.node_editor_state.record(Box::new(
                    crate::cunning_core::command::basic::CmdSetPromoteNoteRect::new(
                        note_id, old, new,
                    ),
                ));
                context.graph_changed_writer.write_default();
            }
        }
    }
}

fn handle_drag(
    ui: &mut egui::Ui,
    editor: &mut NodeEditorTab,
    context: &mut EditorTabContext,
    note_id: PromoteNoteId,
    note: &mut PromoteNote,
    title_rect: Rect,
    zoom: f32,
) {
    let resp = ui.interact(
        title_rect,
        ui.make_persistent_id(note_id),
        Sense::click_and_drag(),
    );
    if resp.drag_started_by(egui::PointerButton::Primary) {
        editor.promote_note_rect_start.insert(note_id, note.rect);
    }
    if resp.dragged() {
        let delta = resp.drag_delta() / zoom;
        note.rect.min += delta;
        note.rect.max += delta;
    }
    if resp.drag_stopped_by(egui::PointerButton::Primary) {
        if let Some(old) = editor.promote_note_rect_start.remove(&note_id) {
            let new = note.rect;
            if old != new {
                context.node_editor_state.record(Box::new(
                    crate::cunning_core::command::basic::CmdSetPromoteNoteRect::new(
                        note_id, old, new,
                    ),
                ));
                context.graph_changed_writer.write_default();
            }
        }
    }
}

pub fn poll_voice_events(editor: &mut NodeEditorTab, context: &mut EditorTabContext) {
    let Some(vs) = &context.voice_service else {
        return;
    };
    let breadcrumb = editor.cda_state.breadcrumb();
    for ev in vs.poll_events() {
        if let VoiceEvent::TranscriptionReady(text) = ev {
            if let Some(note_id) = editor.promote_note_recording_id {
                let root = &mut context.node_graph_res.0;
                cda::navigation::with_graph_by_path_mut(root, &breadcrumb, |g| {
                    if let Some(note) = g.promote_notes.get_mut(&note_id) {
                        if !note.content.is_empty() {
                            note.content.push(' ');
                        }
                        note.content.push_str(&text);
                    }
                });
            }
            editor.promote_note_recording_id = None;
        }
    }
}

pub fn create_promote_note(pos: egui::Pos2) -> PromoteNote {
    PromoteNote {
        id: uuid::Uuid::new_v4(),
        rect: Rect::from_min_size(pos, Vec2::new(180.0, 80.0)),
        content: String::new(),
        color: PROMOTE_NOTE_COLOR,
        used_at: None,
        pinned: false,
    }
}
