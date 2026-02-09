//! CDA Tools (Currently only keeping unpack, context menu moved to `menus/context.rs`)
use crate::nodes::structs::{Connection, NodeGraph, NodeId, NodeType};
use std::collections::{HashMap, HashSet};

/// Unpack CDA: Replace CDA node with its internal nodes (skip internal IO nodes)
pub fn unpack_cda(graph: &mut NodeGraph, cda_node_id: NodeId) {
    let incoming: Vec<Connection> = graph
        .connections
        .values()
        .filter(|c| c.to_node == cda_node_id)
        .cloned()
        .collect();
    let outgoing: Vec<Connection> = graph
        .connections
        .values()
        .filter(|c| c.from_node == cda_node_id)
        .cloned()
        .collect();
    if graph.display_node == Some(cda_node_id) {
        graph.display_node = None;
    }
    graph.geometry_cache.remove(&cda_node_id);
    graph
        .port_geometry_cache
        .retain(|(nid, _), _| *nid != cda_node_id);

    let Some(cda_node) = graph.nodes.remove(&cda_node_id) else {
        return;
    };
    let NodeType::CDA(data) = cda_node.node_type else {
        graph.nodes.insert(cda_node_id, cda_node);
        return;
    };

    let offset = cda_node.position.to_vec2();

    let Some(lib) = crate::cunning_core::cda::library::global_cda_library() else {
        return;
    };
    let Ok(_) = lib.ensure_loaded(&data.asset_ref) else {
        return;
    };
    let Some(def) = lib.get(data.asset_ref.uuid) else {
        return;
    };

    let mut io_nodes: HashSet<NodeId> = HashSet::new();
    for i in &def.inputs {
        io_nodes.insert(i.internal_node);
    }
    for o in &def.outputs {
        io_nodes.insert(o.internal_node);
    }
    for (id, n) in &def.inner_graph.nodes {
        if matches!(n.node_type, NodeType::CDAInput(_) | NodeType::CDAOutput(_)) {
            io_nodes.insert(*id);
        }
    }

    let input_by_name: HashMap<String, NodeId> = def
        .inputs
        .iter()
        .map(|i| (i.name.clone(), i.internal_node))
        .collect();
    let output_by_name: HashMap<String, NodeId> = def
        .outputs
        .iter()
        .map(|o| (o.name.clone(), o.internal_node))
        .collect();

    // Cache inner connections involving IO before we move graphs
    let mut io_outgoing: HashMap<NodeId, Vec<(NodeId, crate::nodes::PortId)>> = HashMap::new(); // input_io -> [(to_node,to_port)]
    let mut io_incoming: HashMap<NodeId, Vec<(NodeId, crate::nodes::PortId)>> = HashMap::new(); // output_io -> [(from_node,from_port)]
    for c in def.inner_graph.connections.values() {
        if io_nodes.contains(&c.from_node) && !io_nodes.contains(&c.to_node) {
            io_outgoing
                .entry(c.from_node)
                .or_default()
                .push((c.to_node, c.to_port));
        }
        if !io_nodes.contains(&c.from_node) && io_nodes.contains(&c.to_node) {
            io_incoming
                .entry(c.to_node)
                .or_default()
                .push((c.from_node, c.from_port));
        }
    }

    // Remap all inner ids to avoid collisions when unpacking multiple times / multiple instances
    let mut node_remap: HashMap<NodeId, NodeId> = HashMap::new();
    for (id, n) in &def.inner_graph.nodes {
        if io_nodes.contains(id) {
            continue;
        }
        if graph.nodes.contains_key(id) {
            node_remap.insert(*id, uuid::Uuid::new_v4());
        } else {
            node_remap.insert(*id, uuid::Uuid::new_v4());
        }
        let _ = n;
    }

    // Copy inner nodes (skip IO) with new ids
    for (id, mut node) in def.inner_graph.nodes.clone() {
        if io_nodes.contains(&id) {
            continue;
        }
        let new_id = *node_remap.get(&id).unwrap();
        node.id = new_id;
        node.position = (node.position.to_vec2() + offset).to_pos2();
        graph.nodes.insert(new_id, node);
    }

    // Copy inner connections (exclude IO) with new ids and remapped endpoints
    for (_id, mut conn) in def.inner_graph.connections.clone() {
        if io_nodes.contains(&conn.from_node) || io_nodes.contains(&conn.to_node) {
            continue;
        }
        let Some(&from) = node_remap.get(&conn.from_node) else {
            continue;
        };
        let Some(&to) = node_remap.get(&conn.to_node) else {
            continue;
        };
        let id = uuid::Uuid::new_v4();
        conn.id = id;
        conn.from_node = from;
        conn.to_node = to;
        graph.connections.insert(id, conn);
    }

    // Rewire: External -> Internal (inputs)
    for ext in incoming {
        if let Some(&io) = input_by_name.get(ext.to_port.as_str()) {
            if let Some(targets) = io_outgoing.get(&io) {
                for (to_node, to_port) in targets {
                    let id = uuid::Uuid::new_v4();
                    let Some(&to) = node_remap.get(to_node) else {
                        continue;
                    };
                    graph.connections.insert(
                        id,
                        Connection {
                            id,
                            from_node: ext.from_node,
                            from_port: ext.from_port,
                            to_node: to,
                            to_port: *to_port,
                            order: 0,
                            waypoints: Vec::new(),
                        },
                    );
                    graph.mark_dirty(to);
                }
            }
        }
    }

    // Rewire: Internal -> External (outputs)
    for ext in outgoing {
        if let Some(&io) = output_by_name.get(ext.from_port.as_str()) {
            if let Some(sources) = io_incoming.get(&io) {
                for (from_node, from_port) in sources {
                    let id = uuid::Uuid::new_v4();
                    let Some(&from) = node_remap.get(from_node) else {
                        continue;
                    };
                    graph.connections.insert(
                        id,
                        Connection {
                            id,
                            from_node: from,
                            from_port: *from_port,
                            to_node: ext.to_node,
                            to_port: ext.to_port,
                            order: ext.order,
                            waypoints: Vec::new(),
                        },
                    );
                    graph.mark_dirty(ext.to_node);
                }
            }
        }
    }

    // Delete connections to CDA node
    graph
        .connections
        .retain(|_, c| c.from_node != cda_node_id && c.to_node != cda_node_id);
    graph.final_geometry = std::sync::Arc::new(crate::mesh::Geometry::new());
}
