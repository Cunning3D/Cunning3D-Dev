//! Single-writer graph model + snapshot readers (no-hitch foundation).
//!
//! - Authoritative graph lives on main thread as `ResMut<NodeGraphResource>`
//! - Non-ECS callers (AI tools / external threads) mutate via a command queue
//! - Readers can consume an immutable snapshot (`NodeGraphSnapshot`) without touching the live graph

use crate::nodes::{NodeGraph, NodeId, NodeGraphResource};
use crate::{GeometryChanged, GraphChanged};
use bevy::prelude::*;
use once_cell::sync::OnceCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// -----------------------------------------------------------------------------
// Snapshot
// -----------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct NodeGraphSnapshot {
    pub nodes: HashMap<NodeId, crate::nodes::Node>,
    pub connections: HashMap<crate::nodes::ConnectionId, crate::nodes::Connection>,
    pub sticky_notes: HashMap<crate::nodes::StickyNoteId, crate::nodes::StickyNote>,
    pub sticky_note_draw_order: Vec<crate::nodes::StickyNoteId>,
    pub network_boxes: HashMap<crate::nodes::NetworkBoxId, crate::nodes::NetworkBox>,
    pub network_box_draw_order: Vec<crate::nodes::NetworkBoxId>,
    pub promote_notes: HashMap<crate::nodes::PromoteNoteId, crate::nodes::PromoteNote>,
    pub promote_note_draw_order: Vec<crate::nodes::PromoteNoteId>,
    pub display_node: Option<NodeId>,
    pub final_geometry: Arc<crate::mesh::Geometry>,
    pub geometry_cache: HashMap<NodeId, Arc<crate::mesh::Geometry>>,
    pub graph_revision: u64,
    pub param_revision: u64,
}

impl NodeGraphSnapshot {
    pub fn from_live(g: &NodeGraph) -> Self {
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
            final_geometry: g.final_geometry.clone(),
            geometry_cache: g.geometry_cache.clone(),
            graph_revision: g.graph_revision,
            param_revision: g.param_revision,
        }
    }
}

#[derive(Resource, Clone)]
pub struct NodeGraphSnapshotRes(pub Arc<NodeGraphSnapshot>);

impl Default for NodeGraphSnapshotRes {
    fn default() -> Self {
        Self(Arc::new(NodeGraphSnapshot::default()))
    }
}

static GLOBAL_GRAPH_SNAPSHOT: OnceCell<Mutex<Arc<NodeGraphSnapshot>>> = OnceCell::new();

pub fn global_graph_snapshot() -> Option<Arc<NodeGraphSnapshot>> {
    GLOBAL_GRAPH_SNAPSHOT
        .get()
        .and_then(|m| m.lock().ok().map(|g| g.clone()))
}

fn sync_global_graph_snapshot(snapshot: &NodeGraphSnapshotRes) {
    let Some(m) = GLOBAL_GRAPH_SNAPSHOT.get() else {
        return;
    };
    if let Ok(mut g) = m.try_lock() {
        *g = snapshot.0.clone();
    }
}

// -----------------------------------------------------------------------------
// Command queue (single-writer)
// -----------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, Default)]
pub struct GraphCommandEffect {
    pub graph_changed: bool,
    pub geometry_changed: bool,
}

pub type GraphCommand = Box<dyn FnOnce(&mut NodeGraph) -> GraphCommandEffect + Send + 'static>;

#[derive(Resource)]
pub struct GraphCommandQueue {
    rx: crossbeam_channel::Receiver<GraphCommand>,
}

static GLOBAL_GRAPH_CMD_TX: OnceCell<crossbeam_channel::Sender<GraphCommand>> = OnceCell::new();

/// Enqueue a graph mutation from non-ECS callers (AI tools, threads).
pub fn enqueue_graph_command(cmd: GraphCommand) -> Result<(), String> {
    let Some(tx) = GLOBAL_GRAPH_CMD_TX.get() else {
        return Err("GraphCommandQueue not initialized".to_string());
    };
    tx.send(cmd)
        .map_err(|_| "GraphCommandQueue send failed".to_string())
}

fn apply_graph_commands_system(
    q: Res<GraphCommandQueue>,
    mut node_graph: ResMut<NodeGraphResource>,
    mut graph_changed: MessageWriter<GraphChanged>,
    mut geometry_changed: MessageWriter<GeometryChanged>,
) {
    let mut eff = GraphCommandEffect::default();
    for cmd in q.rx.try_iter() {
        let e = (cmd)(&mut node_graph.0);
        eff.graph_changed |= e.graph_changed;
        eff.geometry_changed |= e.geometry_changed;
    }
    if eff.graph_changed {
        graph_changed.write(GraphChanged);
    }
    if eff.geometry_changed {
        geometry_changed.write(GeometryChanged);
    }
}

fn update_graph_snapshot_system(
    node_graph: Res<NodeGraphResource>,
    mut snapshot: ResMut<NodeGraphSnapshotRes>,
    mut graph_changed: MessageReader<GraphChanged>,
    mut geometry_changed: MessageReader<GeometryChanged>,
) {
    // Update snapshot only when graph or geometry changed.
    if graph_changed.read().next().is_none() && geometry_changed.read().next().is_none() {
        return;
    }
    snapshot.0 = Arc::new(NodeGraphSnapshot::from_live(&node_graph.0));
    sync_global_graph_snapshot(&*snapshot);
}

pub struct GraphModelPlugin;

impl Plugin for GraphModelPlugin {
    fn build(&self, app: &mut App) {
        // init global snapshot + command sender
        let _ = GLOBAL_GRAPH_SNAPSHOT.set(Mutex::new(Arc::new(NodeGraphSnapshot::default())));
        let (tx, rx) = crossbeam_channel::unbounded::<GraphCommand>();
        let _ = GLOBAL_GRAPH_CMD_TX.set(tx);
        app.insert_resource(GraphCommandQueue { rx });
        app.init_resource::<NodeGraphSnapshotRes>();

        // Apply external commands early, then snapshot after cook/apply results.
        app.add_systems(Update, apply_graph_commands_system);
        app.add_systems(PostUpdate, update_graph_snapshot_system);
    }
}

