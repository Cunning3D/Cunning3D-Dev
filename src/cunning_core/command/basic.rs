use super::Command;
use crate::nodes::parameter::ParameterValue as UiParameterValue;
use crate::nodes::{
    structs::{
        Connection, ConnectionId, NetworkBox, NetworkBoxId, Node, NodeId, PromoteNote,
        PromoteNoteId, StickyNote, StickyNoteId,
    },
    NodeGraph, PortId,
};
use bevy_egui::egui::Pos2;
use bevy::prelude::Vec2;
use std::collections::HashSet;

// --- Add Node ---
#[derive(Debug)]
pub struct CmdAddNode {
    pub node: Node,
}

impl Command for CmdAddNode {
    fn apply(&mut self, graph: &mut NodeGraph) {
        graph.nodes.insert(self.node.id, self.node.clone());
        graph.invalidate_adjacency();
        graph.mark_dirty(self.node.id);
        graph.rebuild_block_id_index();
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.nodes.remove(&self.node.id);
        graph.invalidate_adjacency();
        graph.mark_dirty(self.node.id);
        graph.rebuild_block_id_index();
    }
    fn name(&self) -> &str {
        "Add Node"
    }
}

#[derive(Debug)]
pub struct CmdAddStickyNote {
    pub note: StickyNote,
}

impl Command for CmdAddStickyNote {
    fn apply(&mut self, graph: &mut NodeGraph) {
        graph.sticky_notes.insert(self.note.id, self.note.clone());
        if !graph.sticky_note_draw_order.contains(&self.note.id) {
            graph.sticky_note_draw_order.push(self.note.id);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.sticky_notes.remove(&self.note.id);
        graph
            .sticky_note_draw_order
            .retain(|id| *id != self.note.id);
    }
    fn name(&self) -> &str {
        "Add Sticky Note"
    }
}

#[derive(Debug)]
pub struct CmdRemoveStickyNote {
    pub id: StickyNoteId,
    removed: Option<StickyNote>,
}

impl CmdRemoveStickyNote {
    pub fn new(id: StickyNoteId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for CmdRemoveStickyNote {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_none() {
            self.removed = graph.sticky_notes.remove(&self.id);
        } else {
            graph.sticky_notes.remove(&self.id);
        }
        graph.sticky_note_draw_order.retain(|id| *id != self.id);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(n) = &self.removed {
            graph.sticky_notes.insert(self.id, n.clone());
            if !graph.sticky_note_draw_order.contains(&self.id) {
                graph.sticky_note_draw_order.push(self.id);
            }
        }
    }
    fn name(&self) -> &str {
        "Remove Sticky Note"
    }
}

#[derive(Debug)]
pub struct CmdAddNetworkBox {
    pub box_: NetworkBox,
}

impl Command for CmdAddNetworkBox {
    fn apply(&mut self, graph: &mut NodeGraph) {
        graph.network_boxes.insert(self.box_.id, self.box_.clone());
        if !graph.network_box_draw_order.contains(&self.box_.id) {
            graph.network_box_draw_order.push(self.box_.id);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.network_boxes.remove(&self.box_.id);
        graph
            .network_box_draw_order
            .retain(|id| *id != self.box_.id);
    }
    fn name(&self) -> &str {
        "Add Network Box"
    }
}

#[derive(Debug)]
pub struct CmdRemoveNetworkBox {
    pub id: NetworkBoxId,
    removed: Option<NetworkBox>,
}

impl CmdRemoveNetworkBox {
    pub fn new(id: NetworkBoxId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for CmdRemoveNetworkBox {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_none() {
            self.removed = graph.network_boxes.remove(&self.id);
        } else {
            graph.network_boxes.remove(&self.id);
        }
        graph.network_box_draw_order.retain(|id| *id != self.id);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(b) = &self.removed {
            graph.network_boxes.insert(self.id, b.clone());
            if !graph.network_box_draw_order.contains(&self.id) {
                graph.network_box_draw_order.push(self.id);
            }
        }
    }
    fn name(&self) -> &str {
        "Remove Network Box"
    }
}

#[derive(Debug)]
pub struct CmdAddPromoteNote {
    pub note: PromoteNote,
}

impl Command for CmdAddPromoteNote {
    fn apply(&mut self, graph: &mut NodeGraph) {
        graph.promote_notes.insert(self.note.id, self.note.clone());
        if !graph.promote_note_draw_order.contains(&self.note.id) {
            graph.promote_note_draw_order.push(self.note.id);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.promote_notes.remove(&self.note.id);
        graph
            .promote_note_draw_order
            .retain(|id| *id != self.note.id);
    }
    fn name(&self) -> &str {
        "Add Promote Note"
    }
}

#[derive(Debug)]
pub struct CmdRemovePromoteNote {
    pub id: PromoteNoteId,
    removed: Option<PromoteNote>,
}

impl CmdRemovePromoteNote {
    pub fn new(id: PromoteNoteId) -> Self {
        Self { id, removed: None }
    }
}

impl Command for CmdRemovePromoteNote {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_none() {
            self.removed = graph.promote_notes.remove(&self.id);
        } else {
            graph.promote_notes.remove(&self.id);
        }
        graph.promote_note_draw_order.retain(|id| *id != self.id);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(n) = &self.removed {
            graph.promote_notes.insert(self.id, n.clone());
            if !graph.promote_note_draw_order.contains(&self.id) {
                graph.promote_note_draw_order.push(self.id);
            }
        }
    }
    fn name(&self) -> &str {
        "Remove Promote Note"
    }
}

// --- Remove Node ---
#[derive(Debug)]
pub struct CmdRemoveNode {
    pub node_id: NodeId,
    // State capture
    removed_node: Option<Node>,
    removed_connections: Vec<Connection>,
}

impl CmdRemoveNode {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            removed_node: None,
            removed_connections: Vec::new(),
        }
    }
}

impl Command for CmdRemoveNode {
    fn apply(&mut self, graph: &mut NodeGraph) {
        // Capture state if first time
        if self.removed_node.is_none() {
            if let Some(node) = graph.nodes.remove(&self.node_id) {
                self.removed_node = Some(node);
                // Find and remove associated connections
                let mut to_remove = Vec::new();
                for (id, conn) in &graph.connections {
                    if conn.from_node == self.node_id || conn.to_node == self.node_id {
                        to_remove.push(*id);
                    }
                }
                for id in to_remove {
                    if let Some(conn) = graph.connections.remove(&id) {
                        self.removed_connections.push(conn);
                    }
                }
            }
        } else {
            // Re-apply: just remove
            graph.nodes.remove(&self.node_id);
            for conn in &self.removed_connections {
                graph.connections.remove(&conn.id);
            }
        }
        graph.mark_dirty(self.node_id);
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }

    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(node) = &self.removed_node {
            graph.nodes.insert(self.node_id, node.clone());
            for conn in &self.removed_connections {
                graph.connections.insert(conn.id, conn.clone());
            }
            graph.mark_dirty(self.node_id);
        }
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }
    fn name(&self) -> &str {
        "Remove Node"
    }
}

// --- Move Node ---
#[derive(Debug)]
pub struct CmdMoveNode {
    pub node_id: NodeId,
    pub old_pos: Vec2,
    pub new_pos: Vec2,
}

impl Command for CmdMoveNode {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if let Some(node) = graph.nodes.get_mut(&self.node_id) {
            node.position.x = self.new_pos.x;
            node.position.y = self.new_pos.y;
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(node) = graph.nodes.get_mut(&self.node_id) {
            node.position.x = self.old_pos.x;
            node.position.y = self.old_pos.y;
        }
    }
    fn merge(&mut self, other: &dyn Command) -> bool {
        if let Some(other_cmd) = other.as_any().downcast_ref::<CmdMoveNode>() {
            if self.node_id == other_cmd.node_id {
                self.new_pos = other_cmd.new_pos;
                return true;
            }
        }
        false
    }
    fn name(&self) -> &str {
        "Move Node"
    }
}

// --- Connect ---
#[derive(Debug)]
pub struct CmdConnect {
    pub connection: Connection,
}

impl Command for CmdConnect {
    fn apply(&mut self, graph: &mut NodeGraph) {
        graph
            .connections
            .insert(self.connection.id, self.connection.clone());
        graph.invalidate_adjacency();
        graph.mark_dirty(self.connection.to_node);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.connections.remove(&self.connection.id);
        graph.invalidate_adjacency();
        graph.mark_dirty(self.connection.to_node);
    }
    fn name(&self) -> &str {
        "Connect"
    }
}

// --- Disconnect ---
#[derive(Debug)]
pub struct CmdDisconnect {
    pub connection_id: ConnectionId,
    removed_connection: Option<Connection>,
}

impl CmdDisconnect {
    pub fn new(connection_id: ConnectionId) -> Self {
        Self {
            connection_id,
            removed_connection: None,
        }
    }
}

impl Command for CmdDisconnect {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if let Some(conn) = graph.connections.remove(&self.connection_id) {
            self.removed_connection = Some(conn.clone());
            graph.invalidate_adjacency();
            graph.mark_dirty(conn.to_node);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(conn) = &self.removed_connection {
            graph.connections.insert(self.connection_id, conn.clone());
            graph.invalidate_adjacency();
            graph.mark_dirty(conn.to_node);
        }
    }
    fn name(&self) -> &str {
        "Disconnect"
    }
}

// --- Set Connection (replace existing single-input connection) ---
#[derive(Debug)]
pub struct CmdSetConnection {
    pub connection: Connection,
    pub replace: bool,
    removed: Vec<Connection>,
}

impl CmdSetConnection {
    pub fn new(connection: Connection, replace: bool) -> Self {
        Self {
            connection,
            replace,
            removed: Vec::new(),
        }
    }
}

impl Command for CmdSetConnection {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_empty() && self.replace {
            let mut ids: Vec<_> = graph
                .connections
                .iter()
                .filter(|(_, c)| {
                    c.to_node == self.connection.to_node && c.to_port == self.connection.to_port
                })
                .map(|(id, _)| *id)
                .collect();
            ids.sort();
            for id in ids {
                if let Some(c) = graph.connections.remove(&id) {
                    self.removed.push(c);
                }
            }
        } else if self.replace {
            for c in &self.removed {
                graph.connections.remove(&c.id);
            }
        }
        graph
            .connections
            .insert(self.connection.id, self.connection.clone());
        graph.invalidate_adjacency();
        graph.mark_dirty(self.connection.to_node);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        graph.connections.remove(&self.connection.id);
        for c in &self.removed {
            graph.connections.insert(c.id, c.clone());
        }
        graph.invalidate_adjacency();
        graph.mark_dirty(self.connection.to_node);
    }
    fn name(&self) -> &str {
        "Set Connection"
    }
}

#[derive(Debug)]
pub struct CmdSetConnectionOrders {
    pub to_node: NodeId,
    pub to_port: PortId,
    pub new_ids: Vec<ConnectionId>,
    old: Vec<(ConnectionId, i32)>,
}

impl CmdSetConnectionOrders {
    pub fn new(to_node: NodeId, to_port: PortId, new_ids: Vec<ConnectionId>) -> Self {
        Self {
            to_node,
            to_port,
            new_ids,
            old: Vec::new(),
        }
    }
}

impl Command for CmdSetConnectionOrders {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.old.is_empty() {
            let mut olds: Vec<(ConnectionId, i32)> = graph
                .connections
                .values()
                .filter(|c| c.to_node == self.to_node && c.to_port == self.to_port)
                .map(|c| (c.id, c.order))
                .collect();
            olds.sort_by(|a, b| a.0.cmp(&b.0));
            self.old = olds;
        }
        for (i, id) in self.new_ids.iter().copied().enumerate() {
            if let Some(c) = graph.connections.get_mut(&id) {
                c.order = i as i32;
            }
        }
        graph.mark_dirty(self.to_node);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for (id, ord) in &self.old {
            if let Some(c) = graph.connections.get_mut(id) {
                c.order = *ord;
            }
        }
        graph.mark_dirty(self.to_node);
    }
    fn merge(&mut self, next: &dyn Command) -> bool {
        let Some(n) = next.as_any().downcast_ref::<CmdSetConnectionOrders>() else {
            return false;
        };
        if self.to_node != n.to_node || self.to_port != n.to_port {
            return false;
        }
        self.new_ids = n.new_ids.clone();
        true
    }
    fn name(&self) -> &str {
        "Reorder Inputs"
    }
}

// --- Set Connection Waypoints ---
#[derive(Debug)]
pub struct CmdSetConnectionWaypoints {
    pub id: ConnectionId,
    pub new: Vec<Pos2>,
    old: Vec<Pos2>,
}

impl CmdSetConnectionWaypoints {
    pub fn new(id: ConnectionId, old: Vec<Pos2>, new: Vec<Pos2>) -> Self {
        Self { id, new, old }
    }
}

impl Command for CmdSetConnectionWaypoints {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if let Some(c) = graph.connections.get_mut(&self.id) {
            c.waypoints = self.new.clone();
            let to = c.to_node;
            graph.mark_dirty(to);
            graph.invalidate_adjacency();
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(c) = graph.connections.get_mut(&self.id) {
            c.waypoints = self.old.clone();
            let to = c.to_node;
            graph.mark_dirty(to);
            graph.invalidate_adjacency();
        }
    }
    fn merge(&mut self, next: &dyn Command) -> bool {
        let Some(n) = next.as_any().downcast_ref::<CmdSetConnectionWaypoints>() else {
            return false;
        };
        if self.id != n.id {
            return false;
        }
        self.new = n.new.clone();
        true
    }
    fn name(&self) -> &str {
        "Set Connection Waypoints"
    }
}

// --- Delete Nodes (composite) ---
#[derive(Debug)]
pub struct CmdDeleteNodes {
    pub node_ids: Vec<NodeId>,
    removed_nodes: Vec<Node>,
    removed_connections: Vec<Connection>,
    removed_network_boxes: Vec<(NetworkBox, usize)>,
    prev_display_node: Option<NodeId>,
}

impl CmdDeleteNodes {
    pub fn new(node_ids: Vec<NodeId>) -> Self {
        Self {
            node_ids,
            removed_nodes: Vec::new(),
            removed_connections: Vec::new(),
            removed_network_boxes: Vec::new(),
            prev_display_node: None,
        }
    }
}

impl Command for CmdDeleteNodes {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.prev_display_node.is_none() {
            self.prev_display_node = graph.display_node;
        }
        if self.removed_nodes.is_empty() {
            // Expand ForEach deletes to block pairs and remove their network box.
            let mut blocks: HashSet<String> = HashSet::new();
            for id in &self.node_ids {
                let Some(n) = graph.nodes.get(id) else {
                    continue;
                };
                let is_foreach = matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End");
                if !is_foreach {
                    continue;
                }
                if let Some(p) = n.parameters.iter().find(|p| p.name == "block_id") {
                    if let UiParameterValue::String(s) = &p.value {
                        if !s.trim().is_empty() {
                            blocks.insert(s.trim().to_string());
                        }
                    }
                }
            }
            if !blocks.is_empty() {
                let mut extra: Vec<NodeId> = Vec::new();
                for (id, n) in &graph.nodes {
                    let is_foreach = matches!(&n.node_type, crate::nodes::structs::NodeType::Generic(s) if s == "ForEach Begin" || s == "ForEach End" || s == "ForEach Meta");
                    if !is_foreach {
                        continue;
                    }
                    let bid = n
                        .parameters
                        .iter()
                        .find(|p| p.name == "block_id")
                        .and_then(|p| {
                            if let UiParameterValue::String(s) = &p.value {
                                Some(s.trim())
                            } else {
                                None
                            }
                        })
                        .unwrap_or("");
                    if blocks.contains(bid) {
                        extra.push(*id);
                    }
                }
                self.node_ids.extend(extra);
                self.node_ids.sort();
                self.node_ids.dedup();

                let mut to_remove: Vec<(usize, crate::nodes::structs::NetworkBoxId)> = Vec::new();
                for (i, bid) in graph.network_box_draw_order.iter().copied().enumerate() {
                    if let Some(b) = graph.network_boxes.get(&bid) {
                        if let Some(t) = b.title.strip_prefix("ForEach ").map(str::trim) {
                            if blocks.contains(t) {
                                to_remove.push((i, bid));
                            }
                        }
                    }
                }
                to_remove.sort_by(|a, b| b.0.cmp(&a.0)); // remove from back to keep indices valid
                for (idx, bid) in to_remove {
                    if let Some(b) = graph.network_boxes.remove(&bid) {
                        self.removed_network_boxes.push((b, idx));
                    }
                    graph.network_box_draw_order.remove(idx);
                }
            }

            let set: HashSet<NodeId> = self.node_ids.iter().copied().collect();
            for id in &self.node_ids {
                if let Some(n) = graph.nodes.get(id) {
                    self.removed_nodes.push(n.clone());
                }
            }
            let mut cids: Vec<_> = graph
                .connections
                .iter()
                .filter(|(_, c)| set.contains(&c.from_node) || set.contains(&c.to_node))
                .map(|(id, _)| *id)
                .collect();
            cids.sort();
            for id in cids {
                if let Some(c) = graph.connections.remove(&id) {
                    self.removed_connections.push(c);
                }
            }
        } else {
            for c in &self.removed_connections {
                graph.connections.remove(&c.id);
            }
            for (b, idx) in &self.removed_network_boxes {
                graph.network_boxes.remove(&b.id);
                if *idx < graph.network_box_draw_order.len() {
                    graph.network_box_draw_order.remove(*idx);
                } else {
                    graph.network_box_draw_order.retain(|id| id != &b.id);
                }
            }
        }
        let set: HashSet<NodeId> = self.node_ids.iter().copied().collect();
        for id in &self.node_ids {
            graph.nodes.remove(id);
            graph.geometry_cache.remove(id);
            graph.port_geometry_cache.retain(|(nid, _), _| nid != id);
            graph.port_ref_cache.retain(|(nid, _), _| nid != id);
            graph.foreach_piece_cache.remove(id);
            graph.foreach_block_cache.remove(id);
            graph.mark_dirty(*id);
        }
        if let Some(d) = graph.display_node {
            if set.contains(&d) {
                graph.display_node = None;
            }
        }
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for (b, idx) in self.removed_network_boxes.iter().rev() {
            graph.network_boxes.insert(b.id, b.clone());
            let i = (*idx).min(graph.network_box_draw_order.len());
            graph.network_box_draw_order.insert(i, b.id);
        }
        for n in &self.removed_nodes {
            graph.nodes.insert(n.id, n.clone());
            graph.mark_dirty(n.id);
        }
        for c in &self.removed_connections {
            graph.connections.insert(c.id, c.clone());
            graph.mark_dirty(c.to_node);
        }
        graph.display_node = self.prev_display_node;
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }
    fn name(&self) -> &str {
        "Delete Nodes"
    }
}

// --- Paste Nodes (composite add) ---
#[derive(Debug)]
pub struct CmdPasteNodes {
    pub nodes: Vec<Node>,
    added_ids: Vec<NodeId>,
}

impl CmdPasteNodes {
    pub fn new(nodes: Vec<Node>) -> Self {
        Self {
            nodes,
            added_ids: Vec::new(),
        }
    }
    pub fn added(&self) -> &[NodeId] {
        &self.added_ids
    }
}

impl Command for CmdPasteNodes {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.added_ids.is_empty() {
            self.added_ids = self.nodes.iter().map(|n| n.id).collect();
        }
        for n in &self.nodes {
            graph.nodes.insert(n.id, n.clone());
            graph.mark_dirty(n.id);
        }
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for id in &self.added_ids {
            graph.nodes.remove(id);
            graph.geometry_cache.remove(id);
            graph.port_geometry_cache.retain(|(nid, _), _| nid != id);
            graph.mark_dirty(*id);
        }
        graph.invalidate_adjacency();
        graph.rebuild_block_id_index();
    }
    fn name(&self) -> &str {
        "Paste Nodes"
    }
}

// --- Remove Connections (composite) ---
#[derive(Debug)]
pub struct CmdRemoveConnections {
    pub ids: Vec<ConnectionId>,
    removed: Vec<Connection>,
}

impl CmdRemoveConnections {
    pub fn new(ids: Vec<ConnectionId>) -> Self {
        Self {
            ids,
            removed: Vec::new(),
        }
    }
}

impl Command for CmdRemoveConnections {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_empty() {
            let mut ids = self.ids.clone();
            ids.sort();
            ids.dedup();
            for id in ids {
                if let Some(c) = graph.connections.remove(&id) {
                    graph.mark_dirty(c.to_node);
                    self.removed.push(c);
                }
            }
        } else {
            for c in &self.removed {
                graph.connections.remove(&c.id);
                graph.mark_dirty(c.to_node);
            }
        }
        graph.invalidate_adjacency();
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for c in &self.removed {
            graph.connections.insert(c.id, c.clone());
            graph.mark_dirty(c.to_node);
        }
        graph.invalidate_adjacency();
    }
    fn name(&self) -> &str {
        "Remove Connections"
    }
}

// --- Insert Node on Connection (remove 1 connection, add 2) ---
#[derive(Debug)]
pub struct CmdInsertNodeOnConnection {
    pub target_connection_id: ConnectionId,
    pub node_id: NodeId,
    pub in_port: crate::nodes::PortId,
    pub out_port: crate::nodes::PortId,
    removed: Option<Connection>,
    added: Vec<Connection>,
}

impl CmdInsertNodeOnConnection {
    pub fn new(
        target_connection_id: ConnectionId,
        node_id: NodeId,
        in_port: crate::nodes::PortId,
        out_port: crate::nodes::PortId,
    ) -> Self {
        Self {
            target_connection_id,
            node_id,
            in_port,
            out_port,
            removed: None,
            added: Vec::new(),
        }
    }
}

impl Command for CmdInsertNodeOnConnection {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if self.removed.is_none() {
            self.removed = graph.connections.get(&self.target_connection_id).cloned();
        }
        let Some(src) = self.removed.clone() else {
            return;
        };
        graph.connections.remove(&self.target_connection_id);
        if self.added.is_empty() {
            let id1 = ConnectionId::new_v4();
            let id2 = ConnectionId::new_v4();
            self.added = vec![
                Connection {
                    id: id1,
                    from_node: src.from_node,
                    from_port: src.from_port.clone(),
                    to_node: self.node_id,
                    to_port: self.in_port.clone(),
                    order: 0,
                    waypoints: src.waypoints.clone(),
                },
                Connection {
                    id: id2,
                    from_node: self.node_id,
                    from_port: self.out_port.clone(),
                    to_node: src.to_node,
                    to_port: src.to_port.clone(),
                    order: src.order,
                    waypoints: Vec::new(),
                },
            ];
        }
        for c in &self.added {
            graph.connections.insert(c.id, c.clone());
            graph.mark_dirty(c.to_node);
        }
        graph.mark_dirty(src.to_node);
        graph.invalidate_adjacency();
    }

    fn revert(&mut self, graph: &mut NodeGraph) {
        for c in &self.added {
            graph.connections.remove(&c.id);
            graph.mark_dirty(c.to_node);
        }
        if let Some(src) = &self.removed {
            graph
                .connections
                .insert(self.target_connection_id, src.clone());
            graph.mark_dirty(src.to_node);
        }
        graph.invalidate_adjacency();
    }
    fn name(&self) -> &str {
        "Insert Node on Connection"
    }
}

// --- Replace whole graph (snapshot) ---
#[derive(Debug, Clone)]
pub struct GraphSnapshot {
    pub nodes: std::collections::HashMap<NodeId, Node>,
    pub connections: std::collections::HashMap<ConnectionId, Connection>,
    pub sticky_notes: std::collections::HashMap<
        crate::nodes::structs::StickyNoteId,
        crate::nodes::structs::StickyNote,
    >,
    pub sticky_note_draw_order: Vec<crate::nodes::structs::StickyNoteId>,
    pub network_boxes: std::collections::HashMap<
        crate::nodes::structs::NetworkBoxId,
        crate::nodes::structs::NetworkBox,
    >,
    pub network_box_draw_order: Vec<crate::nodes::structs::NetworkBoxId>,
    pub promote_notes: std::collections::HashMap<
        crate::nodes::structs::PromoteNoteId,
        crate::nodes::structs::PromoteNote,
    >,
    pub promote_note_draw_order: Vec<crate::nodes::structs::PromoteNoteId>,
    pub display_node: Option<NodeId>,
}

impl GraphSnapshot {
    pub fn capture(g: &NodeGraph) -> Self {
        Self {
            nodes: g.nodes.clone(),
            connections: g.connections.clone(),
            sticky_notes: g.sticky_notes.clone(),
            sticky_note_draw_order: g.sticky_note_draw_order.clone(),
            network_boxes: g.network_boxes.clone(),
            network_box_draw_order: g.network_box_draw_order.clone(),
            promote_notes: g.promote_notes.clone(),
            promote_note_draw_order: g.promote_note_draw_order.clone(),
            display_node: g.display_node,
        }
    }
    pub fn restore(&self, g: &mut NodeGraph) {
        g.nodes = self.nodes.clone();
        g.connections = self.connections.clone();
        g.invalidate_adjacency();
        g.rebuild_block_id_index();
        g.sticky_notes = self.sticky_notes.clone();
        g.sticky_note_draw_order = self.sticky_note_draw_order.clone();
        g.network_boxes = self.network_boxes.clone();
        g.network_box_draw_order = self.network_box_draw_order.clone();
        g.promote_notes = self.promote_notes.clone();
        g.promote_note_draw_order = self.promote_note_draw_order.clone();
        g.display_node = self.display_node;
        g.final_geometry = std::sync::Arc::new(crate::mesh::Geometry::new());
        g.geometry_cache.clear();
        g.port_geometry_cache.clear();
        g.dirty_tracker = Default::default();
    }
}

#[derive(Debug)]
pub struct CmdReplaceGraph {
    pub before: GraphSnapshot,
    pub after: GraphSnapshot,
}

impl CmdReplaceGraph {
    pub fn new(before: GraphSnapshot, after: GraphSnapshot) -> Self {
        Self { before, after }
    }
}

impl Command for CmdReplaceGraph {
    fn apply(&mut self, graph: &mut NodeGraph) {
        self.after.restore(graph);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        self.before.restore(graph);
    }
    fn name(&self) -> &str {
        "Replace Graph"
    }
}

// --- Toggle node flags ---
#[derive(Debug)]
pub struct CmdToggleFlag {
    pub node_id: NodeId,
    pub kind: u8,
    pub old: bool,
    pub new: bool,
}

impl CmdToggleFlag {
    pub const BYPASS: u8 = 1;
    pub const TEMPLATE: u8 = 2;
    pub const LOCK: u8 = 3;
    pub fn new(node_id: NodeId, kind: u8, old: bool, new: bool) -> Self {
        Self {
            node_id,
            kind,
            old,
            new,
        }
    }
}

impl Command for CmdToggleFlag {
    fn apply(&mut self, graph: &mut NodeGraph) {
        if let Some(n) = graph.nodes.get_mut(&self.node_id) {
            match self.kind {
                Self::BYPASS => n.is_bypassed = self.new,
                Self::TEMPLATE => n.is_template = self.new,
                Self::LOCK => n.is_locked = self.new,
                _ => {}
            }
            graph.mark_dirty(self.node_id);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        if let Some(n) = graph.nodes.get_mut(&self.node_id) {
            match self.kind {
                Self::BYPASS => n.is_bypassed = self.old,
                Self::TEMPLATE => n.is_template = self.old,
                Self::LOCK => n.is_locked = self.old,
                _ => {}
            }
            graph.mark_dirty(self.node_id);
        }
    }
    fn name(&self) -> &str {
        "Toggle Flag"
    }
}

#[derive(Debug)]
pub struct CmdSetDisplayNode {
    pub old: Option<NodeId>,
    pub new: Option<NodeId>,
}

impl CmdSetDisplayNode {
    pub fn new(old: Option<NodeId>, new: Option<NodeId>) -> Self {
        Self { old, new }
    }
    fn set(graph: &mut NodeGraph, v: Option<NodeId>) {
        graph.display_node = v;
        for (id, n) in graph.nodes.iter_mut() {
            n.is_display_node = Some(*id) == v;
        }
        if let Some(id) = v {
            graph.mark_dirty(id);
        }
        if v.is_none() {
            graph.final_geometry = std::sync::Arc::new(crate::mesh::Geometry::new());
        }
    }
}

impl Command for CmdSetDisplayNode {
    fn apply(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.new);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.old);
    }
    fn name(&self) -> &str {
        "Set Display Node"
    }
}

#[derive(Debug)]
pub struct CmdSetParam {
    pub node_id: NodeId,
    pub param_id: uuid::Uuid,
    pub old: UiParameterValue,
    pub new: UiParameterValue,
}

impl CmdSetParam {
    pub fn new(
        node_id: NodeId,
        param_id: uuid::Uuid,
        old: UiParameterValue,
        new: UiParameterValue,
    ) -> Self {
        Self {
            node_id,
            param_id,
            old,
            new,
        }
    }
    fn set(graph: &mut NodeGraph, node_id: NodeId, param_id: uuid::Uuid, v: &UiParameterValue) {
        if let Some(n) = graph.nodes.get_mut(&node_id) {
            if let Some(p) = n.parameters.iter_mut().find(|p| p.id == param_id) {
                let is_block = p.name == "block_id" || p.name == "block_uid";
                p.value = v.clone();
                if is_block {
                    graph.rebuild_block_id_index();
                }
            }
        }
        graph.bump_param_revision();
        graph.mark_dirty(node_id);
    }
}

impl Command for CmdSetParam {
    fn apply(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.node_id, self.param_id, &self.new);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.node_id, self.param_id, &self.old);
    }
    fn merge(&mut self, other: &dyn Command) -> bool {
        if let Some(o) = other.as_any().downcast_ref::<CmdSetParam>() {
            if self.node_id == o.node_id && self.param_id == o.param_id {
                self.new = o.new.clone();
                return true;
            }
        }
        false
    }
    fn name(&self) -> &str {
        "Set Param"
    }
}

#[derive(Debug)]
pub struct CmdMoveNodes {
    pub items: Vec<(NodeId, bevy_egui::egui::Pos2, bevy_egui::egui::Pos2)>,
}

impl CmdMoveNodes {
    pub fn new(items: Vec<(NodeId, bevy_egui::egui::Pos2, bevy_egui::egui::Pos2)>) -> Self {
        Self { items }
    }
}

impl Command for CmdMoveNodes {
    fn apply(&mut self, graph: &mut NodeGraph) {
        for (id, _old, new) in &self.items {
            if let Some(n) = graph.nodes.get_mut(id) {
                n.position = *new;
            }
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for (id, old, _new) in &self.items {
            if let Some(n) = graph.nodes.get_mut(id) {
                n.position = *old;
            }
        }
    }
    fn name(&self) -> &str {
        "Move Nodes"
    }
}

#[derive(Debug)]
pub struct CmdBatch {
    pub name: &'static str,
    pub cmds: Vec<Box<dyn Command>>,
}

impl CmdBatch {
    pub fn new(name: &'static str, cmds: Vec<Box<dyn Command>>) -> Self {
        Self { name, cmds }
    }
}

impl Command for CmdBatch {
    fn apply(&mut self, graph: &mut NodeGraph) {
        for c in &mut self.cmds {
            c.apply(graph);
        }
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        for c in self.cmds.iter_mut().rev() {
            c.revert(graph);
        }
    }
    fn name(&self) -> &str {
        self.name
    }
}

#[derive(Debug)]
pub struct CmdSetStickyNoteRect {
    pub id: crate::nodes::structs::StickyNoteId,
    pub old: bevy_egui::egui::Rect,
    pub new: bevy_egui::egui::Rect,
}

impl CmdSetStickyNoteRect {
    pub fn new(
        id: crate::nodes::structs::StickyNoteId,
        old: bevy_egui::egui::Rect,
        new: bevy_egui::egui::Rect,
    ) -> Self {
        Self { id, old, new }
    }
    fn set(
        graph: &mut NodeGraph,
        id: crate::nodes::structs::StickyNoteId,
        r: bevy_egui::egui::Rect,
    ) {
        if let Some(n) = graph.sticky_notes.get_mut(&id) {
            n.rect = r;
        }
    }
}

impl Command for CmdSetStickyNoteRect {
    fn apply(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.new);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.old);
    }
    fn name(&self) -> &str {
        "Set Sticky Rect"
    }
}

#[derive(Debug)]
pub struct CmdSetNetworkBoxRect {
    pub id: crate::nodes::structs::NetworkBoxId,
    pub old: bevy_egui::egui::Rect,
    pub new: bevy_egui::egui::Rect,
}

impl CmdSetNetworkBoxRect {
    pub fn new(
        id: crate::nodes::structs::NetworkBoxId,
        old: bevy_egui::egui::Rect,
        new: bevy_egui::egui::Rect,
    ) -> Self {
        Self { id, old, new }
    }
    fn set(
        graph: &mut NodeGraph,
        id: crate::nodes::structs::NetworkBoxId,
        r: bevy_egui::egui::Rect,
    ) {
        if let Some(b) = graph.network_boxes.get_mut(&id) {
            b.rect = r;
        }
    }
}

impl Command for CmdSetNetworkBoxRect {
    fn apply(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.new);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.old);
    }
    fn name(&self) -> &str {
        "Set Box Rect"
    }
}

#[derive(Debug)]
pub struct CmdSetPromoteNoteRect {
    pub id: PromoteNoteId,
    pub old: bevy_egui::egui::Rect,
    pub new: bevy_egui::egui::Rect,
}

impl CmdSetPromoteNoteRect {
    pub fn new(id: PromoteNoteId, old: bevy_egui::egui::Rect, new: bevy_egui::egui::Rect) -> Self {
        Self { id, old, new }
    }
    fn set(graph: &mut NodeGraph, id: PromoteNoteId, r: bevy_egui::egui::Rect) {
        if let Some(n) = graph.promote_notes.get_mut(&id) {
            n.rect = r;
        }
    }
}

impl Command for CmdSetPromoteNoteRect {
    fn apply(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.new);
    }
    fn revert(&mut self, graph: &mut NodeGraph) {
        Self::set(graph, self.id, self.old);
    }
    fn name(&self) -> &str {
        "Set Promote Note Rect"
    }
}
