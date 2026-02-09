use crate::cunning_core::command::basic::CmdRemoveConnections;
use crate::tabs_system::node_editor::cda;
use crate::tabs_system::node_editor::gestures::does_path_intersect_bezier;
use crate::tabs_system::node_editor::NodeEditorTab;
use crate::tabs_system::EditorTabContext;
use bevy_egui::egui::{self, Color32, Rect, Sense, Stroke};

pub fn handle_cut_tool(
    editor: &mut NodeEditorTab,
    ui: &mut egui::Ui,
    context: &mut EditorTabContext,
    editor_rect: Rect,
) {
    if ui.ctx().wants_keyboard_input() {
        return;
    }

    // Toggle logic via 'Y' key
    let y_key_down = ui.input(|i| i.key_down(egui::Key::Y));

    if y_key_down && !editor.is_cutting {
        editor.is_cutting = true;
        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
    } else if !y_key_down && editor.is_cutting {
        // Finish cutting
        editor.is_cutting = false;

        // Perform cut check
        if !editor.cut_path.is_empty() {
            let root_graph = &mut context.node_graph_res.0;
            let mut connections_to_remove = Vec::new();
            cda::navigation::with_current_graph_mut(
                root_graph,
                &editor.cda_state,
                |node_graph| {
                    // Pre-process to find all connections going into each merge bar (for distributed end points)
                    // This duplicates logic from drawing.rs but ensures cutting accuracy
                    let mut merge_port_connections: std::collections::HashMap<
                        (crate::nodes::NodeId, crate::nodes::PortId),
                        Vec<(i32, crate::nodes::ConnectionId)>,
                    > = std::collections::HashMap::new();

                    for connection in node_graph.connections.values() {
                        if let Some((_, Some(_))) = editor
                            .port_locations
                            .get(&(connection.to_node, connection.to_port.clone()))
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

                    for (conn_id, connection) in &node_graph.connections {
                        // Use new (NodeId, PortId) lookup
                        if let (Some((start_pos, _)), Some((end_center_pos, end_bar_width_opt))) = (
                            editor
                                .port_locations
                                .get(&(connection.from_node, connection.from_port.clone())),
                            editor
                                .port_locations
                                .get(&(connection.to_node, connection.to_port.clone())),
                        ) {
                            let mut final_end_pos = *end_center_pos;

                            // Calculate distributed end position for Bar inputs
                            if let Some(bar_width) = end_bar_width_opt {
                                if let Some(conns) = merge_port_connections
                                    .get(&(connection.to_node, connection.to_port.clone()))
                                {
                                    let count = conns.len();
                                    if count > 1 {
                                        if let Some(index) =
                                            conns.iter().position(|(_, id)| *id == connection.id)
                                        {
                                            let fraction =
                                                (index as f32 + 1.0) / (count as f32 + 1.0);
                                            let offset_x =
                                                (fraction * *bar_width) - (*bar_width / 2.0);
                                            final_end_pos.x = end_center_pos.x + offset_x;
                                        }
                                    }
                                }
                            }

                            if does_path_intersect_bezier(
                                &editor.cut_path,
                                *start_pos,
                                final_end_pos,
                            ) {
                                connections_to_remove.push(*conn_id);
                            }
                        }
                    }

                    let has_cut = !connections_to_remove.is_empty();
                    if has_cut {
                        context.node_editor_state.execute(
                            Box::new(CmdRemoveConnections::new(connections_to_remove)),
                            node_graph,
                        );
                        context.graph_changed_writer.write_default();
                    }
                },
            );
        }

        editor.cut_path.clear();
        ui.ctx().set_cursor_icon(egui::CursorIcon::Default);
    }

    // Recording path
    if editor.is_cutting {
        // Draw a transparent overlay to catch all drag events
        let _response = ui.allocate_rect(editor_rect, Sense::drag());

        if let Some(pointer_pos) = ui.input(|i| i.pointer.interact_pos()) {
            if editor_rect.contains(pointer_pos) {
                if editor.cut_path.is_empty() {
                    editor.cut_path.push(pointer_pos);
                } else if let Some(last) = editor.cut_path.last() {
                    if last.distance(pointer_pos) > 5.0 {
                        editor.cut_path.push(pointer_pos);
                    }
                }
            }
        }

        // Draw path
        if editor.cut_path.len() > 1 {
            ui.painter().add(egui::Shape::line(
                editor.cut_path.clone(),
                Stroke::new(2.0, Color32::RED),
            ));
        }
    }
}
