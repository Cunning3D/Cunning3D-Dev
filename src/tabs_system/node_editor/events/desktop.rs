use crate::cunning_core::command::basic::{
    CmdAddNode, CmdAddStickyNote, CmdBatch, CmdDeleteNodes, CmdPasteNodes, CmdRemoveNetworkBox,
    CmdRemovePromoteNote, CmdRemoveStickyNote, CmdSetConnection, CmdSetDisplayNode,
};
use crate::cunning_core::command::basic::CmdRemoveConnections;
use crate::cunning_core::command::basic::CmdSetConnectionWaypoints;
use crate::nodes::NodeId;
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::node_editor::NodeEditorTab;
use crate::tabs_system::EditorTabContext;
use crate::ui::prepare_generic_node;
use bevy::log::info;
use bevy_egui::egui::{self, Color32, Rect, Vec2};
use egui_wgpu::sdf::{create_sdf_rect_callback, SdfRectUniform};
use std::collections::HashSet;

pub fn handle_box_selection(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    response: &egui::Response,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    // Prevent box selection if we are interacting with a connection
    if editor.did_start_connection_this_frame
        || editor.pending_connection_from.is_some()
        || !editor.pending_connections_from.is_empty()
    {
        return;
    }

    if response.drag_started_by(egui::PointerButton::Primary) {
        editor.selection_start = response
            .interact_pointer_pos()
            .or_else(|| ui.ctx().pointer_interact_pos())
            .map(|p| {
                egui::pos2(
                    p.x.clamp(editor_rect.min.x, editor_rect.max.x),
                    p.y.clamp(editor_rect.min.y, editor_rect.max.y),
                )
            });
    }

    if let Some(start_pos) = editor.selection_start {
        if let Some(current_pos) = ui
            .input(|i| i.pointer.hover_pos())
            .or_else(|| ui.ctx().pointer_interact_pos())
            .map(|p| {
            egui::pos2(
                p.x.clamp(editor_rect.min.x, editor_rect.max.x),
                p.y.clamp(editor_rect.min.y, editor_rect.max.y),
            )
        }) {
            let selection_rect_screen =
                Rect::from_two_pos(start_pos, current_pos).intersect(editor_rect);
            let screen_size = ui.ctx().screen_rect().size();
            let c = selection_rect_screen.center();
            let s = selection_rect_screen.size();
            let fill_rgba =
                bevy_egui::egui::Rgba::from(Color32::from_rgba_unmultiplied(100, 100, 255, 64))
                    .to_array();
            let uniform = SdfRectUniform {
                center: [c.x, c.y],
                half_size: [s.x * 0.5, s.y * 0.5],
                corner_radii: [0.0; 4],
                fill_color: fill_rgba,
                shadow_color: [0.0; 4],
                shadow_blur: 0.0,
                _pad1: 0.0,
                shadow_offset: [0.0, 0.0],
                border_width: 1.0,
                _pad2: [0.0; 3],
                border_color: [0.39, 0.39, 1.0, 0.85],
                screen_size: [screen_size.x, screen_size.y],
                _pad3: [0.0; 2],
            };
            let frame_id = ui.ctx().cumulative_frame_nr();
            // Draw selection overlay in a foreground layer so it's not occluded by grid/nodes.
            let layer_id = egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("node_editor_box_select_overlay"),
            );
            let painter = ui.ctx().layer_painter(layer_id);
            painter.add(create_sdf_rect_callback(
                selection_rect_screen.expand(4.0),
                uniform,
                frame_id,
            ));
        }
    }

    let stop = response.drag_stopped_by(egui::PointerButton::Primary)
        || (editor.selection_start.is_some() && ui.input(|i| i.pointer.primary_released()));
    if stop {
        if let Some(start_pos) = editor.selection_start.take() {
            if let Some(end_pos) = ui
                .input(|i| i.pointer.hover_pos())
                .or_else(|| ui.ctx().pointer_interact_pos())
            {
                let selection_rect_screen = Rect::from_two_pos(start_pos, end_pos);
                let start_g = ((start_pos - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                let end_g = ((end_pos - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                let selection_rect_graph = Rect::from_two_pos(start_g, end_g);

                if !ui.input(|i| i.modifiers.shift) {
                    context.ui_state.selected_nodes.clear();
                    context.ui_state.selected_connections.clear();
                    context.ui_state.selected_network_boxes.clear();
                    context.ui_state.selected_promote_notes.clear();
                    context.ui_state.selected_sticky_notes.clear();
                }

                // Use retained hit-cache (bucket grid) to avoid locking the graph and scanning all nodes.
                let bs = editor.hit_cache.bucket_size.max(1.0);
                let min_x = (selection_rect_graph.min.x / bs).floor() as i32;
                let max_x = (selection_rect_graph.max.x / bs).floor() as i32;
                let min_y = (selection_rect_graph.min.y / bs).floor() as i32;
                let max_y = (selection_rect_graph.max.y / bs).floor() as i32;
                let mut cand: HashSet<usize> = HashSet::new();
                for x in min_x..=max_x {
                    for y in min_y..=max_y {
                        if let Some(v) = editor.hit_cache.buckets.get(&(x, y)) {
                            cand.extend(v.iter().copied());
                        }
                    }
                }
                for i in cand {
                    if let Some(n) = editor.hit_cache.nodes.get(i) {
                        if selection_rect_graph.intersects(n.logical_rect) {
                            context.ui_state.selected_nodes.insert(n.id);
                        }
                    }
                }
                // Promote notes selection (screen space)
                let root = &context.node_graph_res.0;
                let g =
                    cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb());
                for (id, note) in g.promote_notes.iter() {
                    let rect_screen = Rect::from_min_max(
                        editor_rect.min + note.rect.min.to_vec2() * editor.zoom + editor.pan,
                        editor_rect.min + note.rect.max.to_vec2() * editor.zoom + editor.pan,
                    );
                    if selection_rect_screen.intersects(rect_screen) {
                        context.ui_state.selected_promote_notes.insert(*id);
                    }
                }
                for (id, b) in g.network_boxes.iter() {
                    let rect_screen = Rect::from_min_max(
                        editor_rect.min + b.rect.min.to_vec2() * editor.zoom + editor.pan,
                        editor_rect.min + b.rect.max.to_vec2() * editor.zoom + editor.pan,
                    );
                    if selection_rect_screen.intersects(rect_screen) {
                        context.ui_state.selected_network_boxes.insert(*id);
                    }
                }
                for (id, note) in g.sticky_notes.iter() {
                    let rect_screen = Rect::from_min_max(
                        editor_rect.min + note.rect.min.to_vec2() * editor.zoom + editor.pan,
                        editor_rect.min + note.rect.max.to_vec2() * editor.zoom + editor.pan,
                    );
                    if selection_rect_screen.intersects(rect_screen) {
                        context.ui_state.selected_sticky_notes.insert(*id);
                    }
                }
            }
        }
    }
}

pub fn handle_shortcuts(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    let allow_ghost_keys = editor.pending_connection_from.is_some()
        || !editor.pending_connections_from.is_empty()
        || editor.ghost_request_id.is_some()
        || editor.ghost_graph.is_some();
    if ui.ctx().wants_keyboard_input() && !allow_ghost_keys {
        return;
    }

    // Ghost completion: only active while dragging a wire (primary down + pending_connection_from).
    // Note: Double-tap semantics (100ms window):
    // - Tab while NO ghost: start inference (does NOT participate in double-tap window).
    // - Tab while IN-FLIGHT: ignore (don't reroll; user will double-tap after results appear).
    // - Tab when ghost is visible: first tap arms "apply"; second tap within 100ms applies; otherwise (timeout) rerolls.
    let dragging = (editor.pending_connection_from.is_some()
        || !editor.pending_connections_from.is_empty())
        && ui.input(|i| i.pointer.button_down(egui::PointerButton::Primary));
    let inter_tap = crate::libs::ai_service::ai_defaults::GHOST_TAB_INTER_TAP_S;
    let tab_down = ui.input(|i| i.key_down(egui::Key::Tab));
    let alt_down = ui.input(|i| i.modifiers.alt);

    // Houdini-like relay point while dragging a SINGLE wire: Alt adds a waypoint.
    if dragging && editor.pending_connection_from.is_some() && editor.pending_connections_from.is_empty() {
        if alt_down && !editor.single_alt_was_down && editor.snapped_to_port.is_none() {
            if let Some(p) = ui.ctx().pointer_interact_pos() {
                let wp = ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2();
                editor.pending_wire_waypoints.push(wp);
            }
        }
        editor.single_alt_was_down = alt_down;
    } else if !alt_down {
        editor.single_alt_was_down = false;
    }

    // Copilot relay shortcuts (only when NOT dragging).
    if !dragging {
        if let Some(sid) = editor.copilot_relay_selected {
            if ui.input(|i| i.key_pressed(egui::Key::Backtick)) {
                editor.copilot_relay_actions.push(crate::tabs_system::node_editor::state::CopilotRelayAction {
                    session_id: sid,
                    kind: crate::tabs_system::node_editor::state::CopilotRelayActionKind::Apply,
                });
                return;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
                editor.copilot_relay_actions.push(crate::tabs_system::node_editor::state::CopilotRelayAction {
                    session_id: sid,
                    kind: crate::tabs_system::node_editor::state::CopilotRelayActionKind::Reroll,
                });
                return;
            }
            if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
                editor.copilot_relay_actions.push(crate::tabs_system::node_editor::state::CopilotRelayAction {
                    session_id: sid,
                    kind: crate::tabs_system::node_editor::state::CopilotRelayActionKind::Cancel,
                });
                return;
            }
        }
    }

    // Box selection annotation (Tab while NOT dragging).
    if ui.input(|i| i.key_pressed(egui::Key::Tab)) && !dragging {
        if editor.box_note_request_id.is_none() && editor.ghost_request_id.is_none() {
            if !context.ui_state.selected_nodes.is_empty()
                || !context.ui_state.selected_sticky_notes.is_empty()
            {
                crate::tabs_system::node_editor::events::common::request_box_note(
                    editor,
                    context,
                    editor_rect,
                );
                return;
            }
        }
    }

    // Backtick key: Apply ghost (when dragging) OR Graph Explain (when not dragging with selection)
    if ui.input(|i| i.key_pressed(egui::Key::Backtick)) {
        if !dragging {
            // Not dragging: trigger graph explain if nodes selected
            if !context.ui_state.selected_nodes.is_empty() && editor.explain_request_id.is_none() {
                let screen_pos = ui.ctx().pointer_interact_pos().unwrap_or(editor_rect.center());
                crate::tabs_system::node_editor::events::common::request_graph_explain(editor, context, screen_pos);
            }
            return;
        }
        if editor.ghost_request_id.is_some() {
            return;
        }
        if let Some(ghost) = editor.ghost_graph.take() {
            info!("GhostApply: backtick");
            let reason_title = editor
                .ghost_reason_title
                .clone()
                .unwrap_or_else(|| "Why".to_string());
            let reason = editor.ghost_reason.clone().unwrap_or_default();
            let root_graph = &mut context.node_graph_res.0;
            cda::navigation::with_current_graph_mut(
                root_graph,
                &editor.cda_state,
                |node_graph| {
                    let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                    let mut created: Vec<crate::nodes::Node> = Vec::new();
                    let ghost_params = editor.ghost_params.clone();
                    for snap in &ghost.nodes {
                        let mut node = prepare_generic_node(
                            context.node_registry,
                            context.node_editor_settings,
                            snap.position,
                            &snap.name,
                        );
                        // Apply AI suggested params for this node
                        for (key, val) in &ghost_params {
                            if let Some((node_name, param_name)) = key.split_once('.') {
                                if node_name == snap.name {
                                    if let Some(p) = node.parameters.iter_mut().find(|p| p.name == param_name) {
                                        use crate::cunning_core::traits::parameter::{ParameterUIType, ParameterValue};
                                        match (&mut p.value, &p.ui_type) {
                                            (ParameterValue::Bool(_), _) => {
                                                p.value = ParameterValue::Bool(val.parse().unwrap_or(false));
                                            }
                                            (ParameterValue::String(_), _) => {
                                                p.value = ParameterValue::String(val.clone());
                                            }
                                            // Dropdown values are stored as Int; UI type carries the choices mapping.
                                            (ParameterValue::Int(_), ParameterUIType::Dropdown { choices }) => {
                                                if let Some((_, out_v)) = choices
                                                    .iter()
                                                    .find(|(label, _)| label.eq_ignore_ascii_case(val))
                                                {
                                                    p.value = ParameterValue::Int(*out_v);
                                                }
                                            }
                                            (ParameterValue::Int(_), _) => {
                                                if let Ok(i) = val.parse() {
                                                    p.value = ParameterValue::Int(i);
                                                }
                                            }
                                            _ => {} // Skip numeric params as per AI rules
                                        }
                                    }
                                }
                            }
                        }
                        created.push(node);
                    }
                    for node in created.iter().cloned() {
                        cmds.push(Box::new(CmdAddNode { node }));
                    }
                    if let Some(first) = created.first() {
                        if let Some(in_port) = first.inputs.keys().next().cloned() {
                            if let Some((from_node, from_port)) = editor.ghost_from.clone() {
                                let conn = crate::nodes::Connection {
                                    id: crate::nodes::ConnectionId::new_v4(),
                                    from_node,
                                    from_port: from_port.clone(),
                                    to_node: first.id,
                                    to_port: in_port.clone(),
                                    order: 0,
                                    waypoints: Vec::new(),
                                };
                                cmds.push(Box::new(CmdSetConnection::new(conn, true)));
                            } else if !editor.pending_connections_from.is_empty() {
                                for (i, (from_node, from_port)) in
                                    editor.pending_connections_from.iter().cloned().enumerate()
                                {
                                    let conn = crate::nodes::Connection {
                                        id: crate::nodes::ConnectionId::new_v4(),
                                        from_node,
                                        from_port,
                                        to_node: first.id,
                                        to_port: in_port.clone(),
                                        order: i as i32,
                                        waypoints: Vec::new(),
                                    };
                                    cmds.push(Box::new(CmdSetConnection::new(conn, false)));
                                }
                            }
                        }
                    }
                    for w in created.windows(2) {
                        let a = &w[0];
                        let b = &w[1];
                        let out_port = a
                            .outputs
                            .keys()
                            .next()
                            .cloned()
                            .unwrap_or_else(|| crate::nodes::PortId::from("Output"));
                        let in_port = b
                            .inputs
                            .keys()
                            .next()
                            .cloned()
                            .unwrap_or_else(|| crate::nodes::PortId::from("Input"));
                        let conn = crate::nodes::Connection {
                            id: crate::nodes::ConnectionId::new_v4(),
                            from_node: a.id,
                            from_port: out_port,
                            to_node: b.id,
                            to_port: in_port,
                            order: 0,
                            waypoints: Vec::new(),
                        };
                        cmds.push(Box::new(CmdSetConnection::new(conn, true)));
                    }
                    if let Some(last) = created.last() {
                        cmds.push(Box::new(CmdSetDisplayNode::new(
                            node_graph.display_node,
                            Some(last.id),
                        )));
                    }
                    if !reason.trim().is_empty() {
                        let first = ghost.nodes.first().cloned();
                        if let Some(first) = first {
                            let size = Vec2::new(260.0, 130.0);
                            let pos = egui::pos2(
                                first.position.x + first.size.x + 20.0,
                                first.position.y,
                            );
                            let rect = Rect::from_min_size(pos, size);
                            let note = crate::nodes::StickyNote {
                                id: crate::nodes::StickyNoteId::new_v4(),
                                rect,
                                title: reason_title.clone(),
                                content: reason.clone(),
                                color: Color32::from_rgb(255, 243, 138),
                            };
                            cmds.push(Box::new(CmdAddStickyNote { note }));
                        }
                    }
                    if !cmds.is_empty() {
                        context
                            .node_editor_state
                            .execute(Box::new(CmdBatch::new("AI Ghost Apply", cmds)), node_graph);
                        context.graph_changed_writer.write_default();
                    }
                },
            );
            editor.ghost_graph = None;
            editor.ghost_request_id = None;
            editor.ghost_anchor_graph_pos = None;
            editor.ghost_from = None;
            editor.ghost_multi_agg_name = None;
            editor.ghost_dialog.clear();
            editor.ghost_turns = 0;
            editor.ghost_pending_user = None;
            editor.ghost_reason_title = None;
            editor.ghost_reason = None;
            editor.ghost_tab_last_time = None;
            editor.ghost_tab_burst = 0;
            editor.ghost_target_nodes = None;
            editor.pending_connection_from = None;
            editor.pending_connections_from.clear();
            editor.snapped_to_port = None;
            // Reset deep mode
            editor.deep_mode = false;
            editor.deep_skill_turns = 0;
            editor.deep_history.clear();
        }
        return;
    }

    // Tab burst: count presses; after 100ms inactivity, (re)request with target length hint.
    if ui.input(|i| i.key_pressed(egui::Key::Tab)) {
        if !dragging {
            return;
        }
        if editor.ghost_request_id.is_some() {
            return;
        }
        editor.ghost_tab_burst = editor.ghost_tab_burst.saturating_add(1);
        editor.ghost_tab_last_time = Some(std::time::Instant::now());
        info!(
            "GhostTab: burst={} (window={:.3}s)",
            editor.ghost_tab_burst, inter_tap
        );
        return;
    }
    if dragging && editor.ghost_tab_last_time.is_some() && editor.ghost_request_id.is_none() {
        if let Some(t) = editor.ghost_tab_last_time {
            let dt = t.elapsed().as_secs_f64();
            if !tab_down && dt > inter_tap {
                let n = editor.ghost_tab_burst.max(1).min(32) as usize;
                editor.ghost_tab_last_time = None;
                editor.ghost_tab_burst = 0;
                info!("GhostTab: fire (dt={:.3}s) target_nodes={}", dt, n);
                // AI relay session: for wire drags (single or multi). Always detaches from mouse so user can keep working.
                // Multi-wire drag keeps Alt=Merge behavior (handled elsewhere); Tab burst can still trigger AI relay.
                let sources: Vec<(crate::nodes::NodeId, crate::nodes::PortId)> =
                    if let Some((from_node, from_port)) = editor.pending_connection_from.clone() {
                        vec![(from_node, from_port)]
                    } else if !editor.pending_connections_from.is_empty() {
                        editor.pending_connections_from.clone()
                    } else {
                        Vec::new()
                    };
                if !sources.is_empty() {
                    let anchor = editor
                        .ghost_anchor_graph_pos
                        .or_else(|| {
                            ui.ctx().pointer_interact_pos().map(|p| {
                                ((p - editor_rect.min - editor.pan) / editor.zoom).to_pos2()
                            })
                        })
                        .unwrap_or_default();
                    let sid = uuid::Uuid::new_v4();
                    let req_id = format!("relay_{}", uuid::Uuid::new_v4());
                    let mut session = crate::tabs_system::node_editor::state::CopilotRelaySession {
                        session_id: sid,
                        backend: editor.copilot_backend,
                        sources,
                        anchor_graph_pos: anchor,
                        request_id: req_id,
                        status: crate::tabs_system::node_editor::state::CopilotRelayStatus::Generating,
                        target_nodes: n,
                        created_at: std::time::Instant::now(),
                        ghost: None,
                        ghost_params: std::collections::HashMap::new(),
                        reason_title: None,
                        reason: None,
                        error: None,
                    };
                    // Detach from mouse so user can start another wire immediately.
                    editor.pending_connection_from = None;
                    editor.pending_connections_from.clear();
                    editor.pending_wire_waypoints.clear();
                    editor.pending_wire_parked = false;
                    editor.single_alt_was_down = false;
                    editor.multi_alt_was_down = false;
                    editor.snapped_to_port = None;
                    editor.did_start_connection_this_frame = false;
                    editor.copilot_relay_selected = Some(sid);
                    crate::tabs_system::node_editor::events::common::request_relay_session(
                        &mut session,
                        editor,
                        context,
                    );
                    editor.copilot_relays.insert(sid, session);
                    return;
                }
                editor.ghost_target_nodes = Some(n);
                crate::tabs_system::node_editor::events::common::request_ghost_path_or_multi(
                    editor,
                    ui,
                    context,
                    editor_rect,
                );
            }
        }
    }

    // Undo
    if ui.input(|i| i.modifiers.command_only() && i.key_pressed(egui::Key::Z)) {
        let root_graph = &mut context.node_graph_res.0;
        context.node_editor_state.undo(root_graph);
        context.graph_changed_writer.write_default();
    }
    // Redo
    if ui.input(|i| i.modifiers.command_only() && i.key_pressed(egui::Key::Y)) {
        let root_graph = &mut context.node_graph_res.0;
        context.node_editor_state.redo(root_graph);
        context.graph_changed_writer.write_default();
    }

    // Copy
    if ui.input(|i| i.modifiers.command_only() && i.key_pressed(egui::Key::C)) {
        if !context.ui_state.selected_nodes.is_empty() {
            let root_graph = &context.node_graph_res.0;
            let node_graph = cda::navigation::graph_snapshot_by_path(
                &root_graph,
                &editor.cda_state.breadcrumb(),
            );
            editor.copied_nodes.clear();
            for node_id in &context.ui_state.selected_nodes {
                if let Some(node) = node_graph.nodes.get(node_id) {
                    editor.copied_nodes.push(node.clone());
                }
            }
        }
    }
    // Paste
    if ui.input(|i| i.modifiers.command_only() && i.key_pressed(egui::Key::V)) {
        if !editor.copied_nodes.is_empty() {
            let root_graph = &mut context.node_graph_res.0;
            let paste_pos_screen = ui
                .ctx()
                .input(|i| i.pointer.hover_pos())
                .unwrap_or(editor_rect.center());
            let graph_paste_pos = (paste_pos_screen - editor_rect.min - editor.pan) / editor.zoom;

            let mut centroid = Vec2::ZERO;
            for node in &editor.copied_nodes {
                centroid += node.position.to_vec2();
            }
            if !editor.copied_nodes.is_empty() {
                centroid /= editor.copied_nodes.len() as f32;
            }

            let mut new_node_ids = HashSet::new();
            let mut new_nodes = Vec::new();
            for copied_node in &editor.copied_nodes {
                let mut new_node = copied_node.clone();
                new_node.id = NodeId::new_v4();
                new_node.rebuild_ports();
                let offset = new_node.position.to_vec2() - centroid;
                new_node.position = (graph_paste_pos + offset).to_pos2();
                new_node_ids.insert(new_node.id);
                new_nodes.push(new_node);
            }
            cda::navigation::with_current_graph_mut(
                root_graph,
                &editor.cda_state,
                |node_graph| {
                    context
                        .node_editor_state
                        .execute(Box::new(CmdPasteNodes::new(new_nodes)), node_graph);
                    context.ui_state.selected_nodes = new_node_ids;
                    context.graph_changed_writer.write_default();
                },
            );
        }
    }
    // Delete
    if ui.input(|i| i.key_pressed(egui::Key::Delete) || i.key_pressed(egui::Key::Backspace)) {
        if let Some((cid, wi)) = editor.selected_waypoint {
            let root = &context.node_graph_res.0;
            let g = cda::navigation::graph_snapshot_by_path(&root, &editor.cda_state.breadcrumb());
            if let Some(c) = g.connections.get(&cid) {
                if wi < c.waypoints.len() {
                    let old = c.waypoints.clone();
                    let mut new = old.clone();
                    new.remove(wi);
                    let root_graph = &mut context.node_graph_res.0;
                    cda::navigation::with_current_graph_mut(
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
            editor.selected_waypoint = None;
            editor.waypoint_drag_old = None;
            return;
        }
        let selected_nodes = context
            .ui_state
            .selected_nodes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let selected_stickies = context
            .ui_state
            .selected_sticky_notes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let selected_boxes = context
            .ui_state
            .selected_network_boxes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let selected_promote = context
            .ui_state
            .selected_promote_notes
            .iter()
            .copied()
            .collect::<Vec<_>>();
        let selected_connections = context
            .ui_state
            .selected_connections
            .iter()
            .copied()
            .collect::<Vec<_>>();
        if !selected_nodes.is_empty()
            || !selected_stickies.is_empty()
            || !selected_boxes.is_empty()
            || !selected_promote.is_empty()
            || !selected_connections.is_empty()
        {
            let root_graph = &mut context.node_graph_res.0;
            cda::navigation::with_current_graph_mut(
                root_graph,
                &editor.cda_state,
                |node_graph| {
                    let mut cmds: Vec<Box<dyn crate::cunning_core::command::Command>> = Vec::new();
                    if !selected_connections.is_empty() {
                        cmds.push(Box::new(CmdRemoveConnections::new(selected_connections)));
                    }
                    if !selected_nodes.is_empty() {
                        cmds.push(Box::new(CmdDeleteNodes::new(selected_nodes.clone())));
                    }
                    for id in selected_stickies {
                        cmds.push(Box::new(CmdRemoveStickyNote::new(id)));
                    }
                    for id in selected_boxes {
                        cmds.push(Box::new(CmdRemoveNetworkBox::new(id)));
                    }
                    for id in selected_promote {
                        cmds.push(Box::new(CmdRemovePromoteNote::new(id)));
                    }
                    if !cmds.is_empty() {
                        context.node_editor_state.execute(
                            Box::new(CmdBatch::new("Delete Selection", cmds)),
                            node_graph,
                        );
                        context.graph_changed_writer.write_default();
                    }
                },
            );
            context.ui_state.selected_nodes.clear();
            context.ui_state.selected_connections.clear();
            context.ui_state.selected_network_boxes.clear();
            context.ui_state.selected_promote_notes.clear();
            context.ui_state.selected_sticky_notes.clear();
        }
    }
}
