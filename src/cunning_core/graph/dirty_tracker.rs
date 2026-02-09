use crate::nodes::structs::{Connection, ConnectionId, NodeId};
use bevy::prelude::*;
use std::collections::{HashMap, HashSet};

/// Dirty Tracker: O(N+E) propagation via adjacency cache
#[derive(Resource, Default, Clone, Debug, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct DirtyTracker {
    pub dirty_nodes: HashSet<NodeId>,
}

impl DirtyTracker {
    #[inline]
    pub fn mark_dirty(&mut self, node_id: NodeId) {
        self.dirty_nodes.insert(node_id);
    }
    #[inline]
    pub fn is_dirty(&self, node_id: &NodeId) -> bool {
        self.dirty_nodes.contains(node_id)
    }
    #[inline]
    pub fn clear(&mut self) {
        self.dirty_nodes.clear();
    }

    /// O(N+E) propagation using pre-built adjacency list (from_node -> [to_nodes])
    pub fn propagate_dirty_fast(&mut self, start: NodeId, adj: &HashMap<NodeId, Vec<NodeId>>) {
        let mut queue = vec![start];
        self.mark_dirty(start);
        while let Some(cur) = queue.pop() {
            if let Some(targets) = adj.get(&cur) {
                for &t in targets {
                    if self.dirty_nodes.insert(t) {
                        queue.push(t);
                    }
                }
            }
        }
    }

    /// Legacy O(N*E) fallback - use propagate_dirty_fast instead
    pub fn propagate_dirty(
        &mut self,
        start_node: NodeId,
        connections: &HashMap<ConnectionId, Connection>,
    ) {
        let mut queue = vec![start_node];
        self.mark_dirty(start_node);
        while let Some(current_id) = queue.pop() {
            for conn in connections.values() {
                if conn.from_node == current_id && self.dirty_nodes.insert(conn.to_node) {
                    queue.push(conn.to_node);
                }
            }
        }
    }
}
