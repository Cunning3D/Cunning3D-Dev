use crate::nodes::parameter::ParameterValue;
use crate::nodes::structs::NodeType;
use crate::nodes::runtime::cook::{compute_upstream_scope, NodeCookState};
use crate::nodes::{Connection, Node, NodeGraph, NodeId};
use std::collections::{HashMap, HashSet, VecDeque};

#[derive(Clone)]
pub struct GraphSnapshot {
    pub nodes: HashMap<NodeId, Node>,
    pub connections: Vec<Connection>,
    pub out: HashMap<NodeId, Vec<NodeId>>,
    pub display_node: Option<NodeId>,
    pub cook_scope_nodes: HashSet<NodeId>,
    pub node_cook_states: HashMap<NodeId, NodeCookState>,
    pub node_cook_errors: HashMap<NodeId, String>,
}

impl GraphSnapshot {
    pub fn from_graph(g: &NodeGraph) -> Self {
        let mut connections: Vec<Connection> = g.connections.values().cloned().collect();
        connections.sort_by(|a, b| a.id.cmp(&b.id));
        let mut out: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        for c in &connections {
            out.entry(c.from_node).or_default().push(c.to_node);
        }
        let display_node = g.display_node;
        let cook_scope_nodes = display_node.map(|id| compute_upstream_scope(g, id)).unwrap_or_default();
        let mut node_cook_states: HashMap<NodeId, NodeCookState> = HashMap::new();
        let mut node_cook_errors: HashMap<NodeId, String> = HashMap::new();
        if let Some(v) = g.cook_viz.as_ref() {
            for it in v.states.iter() {
                node_cook_states.insert(*it.key(), *it.value());
            }
            for it in v.errors.iter() {
                node_cook_errors.insert(*it.key(), it.value().clone());
            }
        }
        Self {
            nodes: g.nodes.clone(),
            connections,
            out,
            display_node,
            cook_scope_nodes,
            node_cook_states,
            node_cook_errors,
        }
    }

    #[inline]
    pub fn foreach_block_nodes(
        &self,
        block_key: &str,
    ) -> (Option<NodeId>, Option<NodeId>, Option<NodeId>) {
        let mut begin = None;
        let mut end = None;
        let mut meta = None;
        for (id, n) in &self.nodes {
            let uid = n
                .parameters
                .iter()
                .find(|p| p.name == "block_uid")
                .and_then(|p| {
                    if let ParameterValue::String(s) = &p.value {
                        Some(s.as_str())
                    } else {
                        None
                    }
                });
            let bid = n
                .parameters
                .iter()
                .find(|p| p.name == "block_id")
                .and_then(|p| {
                    if let ParameterValue::String(s) = &p.value {
                        Some(s.as_str())
                    } else {
                        None
                    }
                });
            let ok = uid == Some(block_key) || bid == Some(block_key);
            if !ok {
                continue;
            }
            match &n.node_type {
                NodeType::Generic(s) if s == "ForEach Begin" => {
                    if begin.is_none() {
                        begin = Some(*id);
                    }
                }
                NodeType::Generic(s) if s == "ForEach End" => {
                    if end.is_none() {
                        end = Some(*id);
                    }
                }
                NodeType::Generic(s) if s == "ForEach Meta" => {
                    if meta.is_none() {
                        meta = Some(*id);
                    }
                }
                _ => {}
            }
        }
        (begin, end, meta)
    }

    pub fn foreach_reachable(
        &self,
        begin: NodeId,
        end: NodeId,
        meta: Option<NodeId>,
    ) -> HashSet<NodeId> {
        let mut reach: HashSet<NodeId> = HashSet::new();
        let mut q: VecDeque<NodeId> = VecDeque::new();
        q.push_back(begin);
        while let Some(nid) = q.pop_front() {
            if !reach.insert(nid) {
                continue;
            }
            if nid == end || Some(nid) == meta {
                continue;
            }
            if let Some(v) = self.out.get(&nid) {
                for &to in v {
                    q.push_back(to);
                }
            }
        }
        reach.insert(begin);
        reach.insert(end);
        if let Some(m) = meta {
            reach.insert(m);
        }
        reach
    }
}
