use crate::nodes::{Connection, ConnectionId, NodeId};
use crate::tabs_system::node_editor::state::NodeAnimation;
use bevy_egui::egui::Pos2;
use std::collections::HashMap as BevyHashMap;
use std::collections::HashMap;

pub fn handle_node_insertion(
    node_graph: &mut crate::nodes::NodeGraph,
    target_connection_id: ConnectionId,
    dragged_node_id: NodeId,
    animations: &mut HashMap<NodeId, NodeAnimation>,
    current_time: f64,
) {
    let (source_node_id, source_port_id, dest_node_id, dest_port_id) =
        if let Some(conn) = node_graph.connections.get(&target_connection_id) {
            (
                conn.from_node,
                conn.from_port.clone(),
                conn.to_node,
                conn.to_port.clone(),
            )
        } else {
            return;
        };

    let (dragged_input_port, dragged_output_port) =
        if let Some(dragged_node) = node_graph.nodes.get(&dragged_node_id) {
            (
                dragged_node.inputs.keys().next().cloned(),
                dragged_node.outputs.keys().next().cloned(),
            )
        } else {
            return;
        };

    if let (Some(dragged_input_port), Some(dragged_output_port)) =
        (dragged_input_port, dragged_output_port)
    {
        // NOTE: this function is used from editor interactions. The actual command push happens at call-site.
        // Here we only perform the geometry/animation, but the connection edits must be undoable.
        // So we do NOT mutate connections here anymore.

        let stride_multiplier = 2.5;

        let (u_pos, u_size) = if let Some(node) = node_graph.nodes.get(&source_node_id) {
            (node.position, node.size)
        } else {
            return;
        };

        let i_pos = if let Some(node) = node_graph.nodes.get(&dragged_node_id) {
            node.position
        } else {
            return;
        };

        let desired_i_y = u_pos.y + u_size.y * stride_multiplier;
        let needs_i_move = i_pos.y < desired_i_y;

        let final_i_pos = if needs_i_move {
            Pos2::new(i_pos.x, desired_i_y)
        } else {
            i_pos
        };

        let i_size = if let Some(node) = node_graph.nodes.get(&dragged_node_id) {
            node.size
        } else {
            return;
        };
        let desired_d_y = final_i_pos.y + i_size.y * stride_multiplier;

        if needs_i_move {
            if let Some(i_node) = node_graph.nodes.get_mut(&dragged_node_id) {
                animations.insert(
                    dragged_node_id,
                    NodeAnimation {
                        start_pos: i_node.position,
                        target_pos: final_i_pos,
                        start_time: current_time,
                        duration: 0.2,
                    },
                );
            }
        }

        if let Some(d_node) = node_graph.nodes.get_mut(&dest_node_id) {
            if d_node.position.y < desired_d_y {
                animations.insert(
                    dest_node_id,
                    NodeAnimation {
                        start_pos: d_node.position,
                        target_pos: Pos2::new(d_node.position.x, desired_d_y),
                        start_time: current_time,
                        duration: 0.2,
                    },
                );
            }
        }
    }
}
