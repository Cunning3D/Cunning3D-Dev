pub mod state;

use crate::nodes::structs::{Connection, NodeGraph, NodeId};
use std::collections::{HashMap, HashSet, VecDeque};

pub use state::{CookVizShared, NodeCookState};

/// Computes upstream cook scope for a root node (includes root).
#[inline]
pub fn compute_upstream_scope(g: &NodeGraph, root: NodeId) -> HashSet<NodeId> {
    let mut ins: HashMap<NodeId, Vec<NodeId>> = HashMap::with_capacity(g.connections.len());
    for Connection { from_node, to_node, .. } in g.connections.values() {
        ins.entry(*to_node).or_default().push(*from_node);
    }
    let mut scope: HashSet<NodeId> = HashSet::new();
    let mut q: VecDeque<NodeId> = VecDeque::new();
    q.push_back(root);
    while let Some(n) = q.pop_front() {
        if !scope.insert(n) {
            continue;
        }
        if let Some(v) = ins.get(&n) {
            for &up in v {
                q.push_back(up);
            }
        }
    }
    scope
}

