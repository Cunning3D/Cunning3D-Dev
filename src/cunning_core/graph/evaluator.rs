//! `graph_traversal.rs` - Contains algorithms for traversing the node graph.

use crate::nodes::{Connection, ConnectionId, Node, NodeId};
use std::collections::HashMap as BevyHashMap;
use std::collections::{HashMap, HashSet, VecDeque};

pub fn find_cook_chain(
    connections: &BevyHashMap<ConnectionId, Connection>,
    start_node_id: NodeId,
) -> HashSet<NodeId> {
    let mut chain = HashSet::new();
    let mut queue = VecDeque::new();

    chain.insert(start_node_id);
    queue.push_back(start_node_id);

    while let Some(node_id) = queue.pop_front() {
        for conn in connections.values() {
            if conn.to_node == node_id {
                if chain.insert(conn.from_node) {
                    queue.push_back(conn.from_node);
                }
            }
        }
    }
    chain
}

pub fn topological_sort(
    _nodes: &BevyHashMap<NodeId, Node>,
    connections: &BevyHashMap<ConnectionId, Connection>,
    nodes_to_sort: &[NodeId],
) -> Vec<NodeId> {
    let mut sorted_order = Vec::new();
    let mut in_degree: HashMap<NodeId, usize> = HashMap::new();
    let mut queue = VecDeque::new();
    let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();

    // Limit the graph to only the nodes that need to be sorted.
    let nodes_to_sort_set: HashSet<_> = nodes_to_sort.iter().cloned().collect();

    for node_id in nodes_to_sort {
        in_degree.insert(*node_id, 0);
        adj.insert(*node_id, vec![]);
    }

    for conn in connections.values() {
        if nodes_to_sort_set.contains(&conn.from_node) && nodes_to_sort_set.contains(&conn.to_node)
        {
            if let Some(degree) = in_degree.get_mut(&conn.to_node) {
                *degree += 1;
            }
            adj.entry(conn.from_node).or_default().push(conn.to_node);
        }
    }

    for node_id in nodes_to_sort {
        if *in_degree.get(node_id).unwrap() == 0 {
            queue.push_back(*node_id);
        }
    }

    while let Some(u) = queue.pop_front() {
        sorted_order.push(u);
        if let Some(neighbors) = adj.get(&u) {
            for &v in neighbors {
                if let Some(degree) = in_degree.get_mut(&v) {
                    *degree -= 1;
                    if *degree == 0 {
                        queue.push_back(v);
                    }
                }
            }
        }
    }

    if sorted_order.len() != nodes_to_sort.len() {
        // This indicates a cycle in the graph, which is an error in a DAG.
        // For now, we'll just print a warning. A more robust solution might
        // involve returning a Result or handling the cycle explicitly.
        println!("Warning: Cycle detected in the graph or disconnected subgraph.");
    }

    sorted_order
}
