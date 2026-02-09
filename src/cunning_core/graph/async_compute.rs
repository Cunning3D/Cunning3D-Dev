use bevy::prelude::*;
use bevy::tasks::{AsyncComputeTaskPool, Task};
use futures_lite::future;
use std::collections::HashMap;
use std::collections::HashSet;
use std::time::Instant;
use bevy::window::PrimaryWindow;
use bevy_egui::EguiContext;

use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::profiling::ComputeRecord;
use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::mesh::Geometry;
use crate::nodes::structs::NodeId;
use crate::nodes::structs::NodeType;
use crate::nodes::NodeGraph;
use crate::tabs_system::node_editor::cda::navigation as cda_nav;
use crate::ui::NodeEditorState;
use crate::{nodes::NodeGraphResource, GraphChanged, GeometryChanged};
use std::sync::Arc;
use crate::nodes::runtime::cook::{compute_upstream_scope, CookVizShared, NodeCookState};

#[derive(Resource, Default)]
pub struct AsyncComputeState {
    pub task: Option<Task<(NodeGraph, HashMap<NodeId, ComputeRecord>)>>,
    pub is_computing: bool,
    pub computing_nodes: HashSet<NodeId>,
    pub computing_path: Vec<NodeId>,
    pub pending_result: Option<(NodeGraph, HashMap<NodeId, ComputeRecord>)>,
    pub cook_id: u64,
}

/// Throttle state for cook dispatching during UI interaction.
/// Allows low-frequency updates (10-15Hz) during drag to maintain "real-time feel" without stutter.
#[derive(Resource)]
pub struct CookThrottleState {
    /// Last time a cook was dispatched
    pub last_dispatch: Option<Instant>,
    /// Minimum interval between dispatches during interaction (e.g., 66ms for ~15Hz)
    pub throttle_interval_ms: u64,
    /// Whether the last frame was in "interaction mode" (UI dragging)
    pub was_interacting: bool,
}

impl Default for CookThrottleState {
    fn default() -> Self {
        Self {
            last_dispatch: None,
            throttle_interval_ms: 66, // ~15Hz during interaction
            was_interacting: false,
        }
    }
}
// ... (dispatch_compute_tasks logic is fine, just type inference for stats needs to match)

pub fn dispatch_compute_tasks(
    mut node_graph_res: ResMut<NodeGraphResource>,
    node_registry: Res<NodeRegistry>,
    mut async_state: ResMut<AsyncComputeState>,
    mut throttle_state: ResMut<CookThrottleState>,
    perf_monitor: Res<crate::cunning_core::profiling::PerformanceMonitor>, // Check if paused
    node_editor_state: Res<NodeEditorState>,
    nav_input: Res<crate::input::NavigationInput>,
    viewport_interaction: Res<crate::ViewportInteractionState>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut egui_query: Query<&mut EguiContext, With<PrimaryWindow>>,
) {
    puffin::profile_function!();
    if perf_monitor.is_paused {
        return;
    }
    let egui_wants_pointer = if let Ok(mut c) = egui_query.single_mut() {
        c.get_mut().wants_pointer_input()
    } else {
        false
    };
    let ui_dragging = egui_wants_pointer && mouse_buttons.pressed(MouseButton::Left);
    
    // Viewport is actively navigating / dragging
    let viewport_busy = nav_input.active
        || nav_input.zoom_delta != 0.0
        || nav_input.orbit_delta.length_squared() != 0.0
        || nav_input.pan_delta.length_squared() != 0.0
        || nav_input.fly_vector.length_squared() != 0.0
        || viewport_interaction.is_gizmo_dragging
        || viewport_interaction.is_right_button_dragged
        || viewport_interaction.is_middle_button_dragged
        || viewport_interaction.is_alt_left_button_dragged;

    let is_interacting = ui_dragging || viewport_busy;
    let now = Instant::now();
    
    // Throttle logic: during interaction, allow cook at reduced rate (10-15Hz) instead of blocking completely.
    // When interaction ends, immediately allow a cook for responsive "release" update.
    let throttle_passed = if is_interacting {
        if let Some(last) = throttle_state.last_dispatch {
            let elapsed_ms = now.duration_since(last).as_millis() as u64;
            elapsed_ms >= throttle_state.throttle_interval_ms
        } else {
            true // No previous dispatch, allow first one
        }
    } else {
        // Not interacting: if we WERE interacting last frame, immediately allow a cook (release update)
        // Otherwise, full speed.
        true
    };
    
    // Update interaction state for next frame
    throttle_state.was_interacting = is_interacting;

    // 1. Check if task already running / pending result not applied yet
    if async_state.task.is_some() || async_state.pending_result.is_some() {
        return; // Wait for current task to finish
    }
    
    // Throttle check: skip this frame if throttle hasn't passed
    if is_interacting && !throttle_passed {
        puffin::profile_scope!("throttled_skip");
        return;
    }

    let root = &mut node_graph_res.0;
    let mut graph_snapshot: Option<NodeGraph> = None;

    // If we're inside a CDA, compute a preview input set from the parent graph and inject it into the inner graph's
    // CDAInput nodes. This is a preview overlay for editor UX (never written to the CDA definition on disk).
    let mut preview_spec: Option<(NodeGraph, NodeId, Vec<(NodeId, crate::nodes::PortId)>)> = None;
    let cda_path = node_editor_state.cda_path.clone();
    if !cda_path.is_empty() {
        let parent_path = &cda_path[..cda_path.len().saturating_sub(1)];
        let cda_node_id = *cda_path.last().unwrap();
        if let Some(lib) = global_cda_library() {
            let (asset_uuid, parent_graph) = (
                cda_nav::with_graph_by_path(&root, parent_path, |pg| {
                    pg.nodes.get(&cda_node_id).and_then(|n| match &n.node_type {
                        NodeType::CDA(d) => Some(d.asset_ref.uuid),
                        _ => None,
                    })
                }),
                cda_nav::graph_snapshot_by_path(&root, parent_path),
            );
            if let Some(asset_uuid) = asset_uuid {
                if let Some(asset) = lib.get(asset_uuid) {
                    let ifaces: Vec<(NodeId, crate::nodes::PortId)> = asset
                        .inputs
                        .iter()
                        .map(|i| {
                            (
                                i.internal_node,
                                crate::nodes::PortId::from(i.port_key().as_str()),
                            )
                        })
                        .collect();
                    preview_spec = Some((parent_graph, cda_node_id, ifaces));
                }
            }
        }
    }

    let mut cook_viz: Option<Arc<CookVizShared>> = None;
    cda_nav::with_graph_by_path_mut(root, &node_editor_state.cda_path, |graph| {
        // Throttle logic is now handled at the top of the function.
        // We no longer completely block during interaction - just throttle to 10-15Hz.
        // This allows "real-time feel" during drag while avoiding stutter.

        // Force a cook inside CDA so the viewport reflects the subgraph immediately (inputs will be injected into snapshot).
        if preview_spec.is_some() {
            if let Some(id) = graph.display_node {
                graph.dirty_tracker.dirty_nodes.insert(id);
            }
        }

        // If no explicit display node was chosen, pick a deterministic default and force a cook once.
        if graph.ensure_display_node_default() {
            if let Some(id) = graph.display_node {
                graph.dirty_tracker.dirty_nodes.insert(id);
            }
        }
        // Even if nothing is marked dirty, we still must cook when display output cache is missing.
        // This fixes cases like: switching CDA levels / loading projects where display_node exists but was never computed in this graph instance.
        if let Some(id) = graph.display_node {
            if !graph.geometry_cache.contains_key(&id) {
                graph.dirty_tracker.dirty_nodes.insert(id);
            }
        }
        if graph.dirty_tracker.dirty_nodes.is_empty() {
            return;
        }
        // --- Cook viz shared state (authoritative for UI) ---
        let shared: Arc<CookVizShared> = graph
            .cook_viz
            .clone()
            .unwrap_or_else(|| Arc::new(CookVizShared::default()));
        graph.cook_viz = Some(shared.clone());
        async_state.cook_id = async_state.cook_id.wrapping_add(1).max(1);
        shared.begin(async_state.cook_id);
        if let Some(display_id) = graph.display_node {
            let scope = compute_upstream_scope(graph, display_id);
            shared.set_scope(scope.iter().copied());
            for &nid in scope.iter() {
                shared.set_state(nid, NodeCookState::Blocked);
            }
            shared.set_state(display_id, NodeCookState::Queued);
        }
        for &nid in graph.dirty_tracker.dirty_nodes.iter() {
            shared.set_state(nid, NodeCookState::Queued);
        }
        cook_viz = Some(shared);
        async_state.computing_nodes = graph.dirty_tracker.dirty_nodes.clone();
        async_state.computing_path = node_editor_state.cda_path.clone();
        graph_snapshot = Some(graph.clone());
        graph.dirty_tracker.clear();
    });
    let Some(mut graph_snapshot) = graph_snapshot else {
        return;
    };
    if let Some(shared) = cook_viz.clone() {
        graph_snapshot.cook_viz = Some(shared);
    }

    // IMPORTANT: never clear the live graph's last-good geometry while async cooking.
    // Clearing live caches causes the 3D viewport to temporarily see "no geometry" and flicker/disappear.
    let dirty_ids: HashSet<NodeId> = async_state.computing_nodes.clone();
    for dirty_id in dirty_ids.iter().copied().collect::<Vec<_>>() {
        graph_snapshot.geometry_cache.remove(&dirty_id);
    }
    graph_snapshot
        .port_geometry_cache
        .retain(|(nid, _), _| !dirty_ids.contains(nid));

    let registry_clone = node_registry.clone();

    // 4. Spawn Task
    let thread_pool = AsyncComputeTaskPool::get();
    let task = thread_pool.spawn(async move {
        // Enable Puffin for this worker thread
        puffin::set_scopes_on(true);
        puffin::profile_scope!("Async Compute Task");

        // Start visualizing immediately
        puffin::yield_now();

        #[cfg(target_arch = "wasm32")]
        {
            // Ensure GPU runtime is ready before any graph evaluation on WebGPU (no blocking on main thread).
            crate::nodes::gpu::runtime::GpuRuntime::init_async_webgpu().await;
        }

        // Inject parent input preview (CDA internal view).
        if let Some((mut parent_graph, cda_node_id, ifaces)) = preview_spec {
            let empty = Arc::new(Geometry::new());
            let mut injected: Vec<(NodeId, Arc<Geometry>)> = Vec::with_capacity(ifaces.len());
            for (internal_node, port_key) in ifaces {
                let mut srcs: Vec<(uuid::Uuid, NodeId, crate::nodes::PortId)> = parent_graph
                    .connections
                    .values()
                    .filter(|c| c.to_node == cda_node_id && c.to_port == port_key)
                    .map(|c| (c.id, c.from_node, c.from_port.clone()))
                    .collect();
                srcs.sort_by(|a, b| a.0.cmp(&b.0));
                let g = if let Some((_id, from_node, from_port)) = srcs.into_iter().next() {
                    parent_graph.compute_output_simple(from_node, &from_port, &registry_clone)
                } else {
                    empty.clone()
                };
                injected.push((internal_node, g));
            }
            // Ensure cached results never mask preview input changes: full recook (keep injected inputs).
            graph_snapshot.geometry_cache.clear();
            graph_snapshot.port_geometry_cache.clear();
            for (nid, geo) in injected {
                graph_snapshot.geometry_cache.insert(nid, geo);
            }
        }

        // Compute on the snapshot
        let mut targets = HashSet::new();
        if let Some(display_id) = graph_snapshot.display_node {
            targets.insert(display_id);
        }
        for (node_id, node) in &graph_snapshot.nodes {
            if node.is_template {
                targets.insert(*node_id);
            }
        }

        // Collect perf stats in async
        let mut stats = HashMap::new();
        graph_snapshot.compute(&targets, &registry_clone, Some(&mut stats));
        (graph_snapshot, stats)
    });

    async_state.task = Some(task);
    async_state.is_computing = true;
    // Record dispatch time for throttle tracking
    throttle_state.last_dispatch = Some(now);
}

pub fn receive_compute_results(
    mut node_graph_res: ResMut<NodeGraphResource>,
    mut async_state: ResMut<AsyncComputeState>,
    mut graph_changed_writer: MessageWriter<GraphChanged>,
    mut geometry_changed_writer: MessageWriter<GeometryChanged>,
    mut perf_monitor: ResMut<crate::cunning_core::profiling::PerformanceMonitor>,
    nav_input: Res<crate::input::NavigationInput>,
    viewport_interaction: Res<crate::ViewportInteractionState>,
) {
    puffin::profile_function!();
    // If we deferred a result (viewport was interacting), apply it once idle.
    if async_state.pending_result.is_some()
        && !nav_input.active
        && nav_input.zoom_delta == 0.0
        && nav_input.orbit_delta.length_squared() == 0.0
        && nav_input.pan_delta.length_squared() == 0.0
        && nav_input.fly_vector.length_squared() == 0.0
        && !viewport_interaction.is_gizmo_dragging
        && !viewport_interaction.is_right_button_dragged
        && !viewport_interaction.is_middle_button_dragged
        && !viewport_interaction.is_alt_left_button_dragged
    {
        if let Some((computed_graph, stats)) = async_state.pending_result.take() {
            let root = &mut node_graph_res.0;
            let path = async_state.computing_path.clone();
            cda_nav::with_graph_by_path_mut(root, &path, |graph| {
                for node_id in &async_state.computing_nodes {
                    if let Some(geo) = computed_graph.geometry_cache.get(node_id) {
                        graph.geometry_cache.insert(*node_id, geo.clone());
                    }
                }
                for (id, record) in stats {
                    perf_monitor.node_cook_times.insert(id, record);
                }
                if graph.display_node == computed_graph.display_node {
                    graph.final_geometry = computed_graph.final_geometry.clone();
                }
                if let Some(v) = graph.cook_viz.as_ref() {
                    v.end();
                }
            });
            async_state.is_computing = false;
            async_state.computing_nodes.clear();
            async_state.computing_path.clear();
            // Trigger scene update with new GeometryChanged (and legacy GraphChanged for migration)
            geometry_changed_writer.write(GeometryChanged);
            graph_changed_writer.write(GraphChanged);
        }
        return;
    }

    if let Some(mut task) = async_state.task.take() {
        if let Some((computed_graph, stats)) = future::block_on(future::poll_once(&mut task)) {
            // Task finished!
            // If the user is actively navigating the viewport, defer applying results to avoid hitches.
            if nav_input.active
                || nav_input.zoom_delta != 0.0
                || nav_input.orbit_delta.length_squared() != 0.0
                || nav_input.pan_delta.length_squared() != 0.0
                || nav_input.fly_vector.length_squared() != 0.0
                || viewport_interaction.is_gizmo_dragging
                || viewport_interaction.is_right_button_dragged
                || viewport_interaction.is_middle_button_dragged
                || viewport_interaction.is_alt_left_button_dragged
            {
                async_state.pending_result = Some((computed_graph, stats));
                // Cook is complete even if apply is deferred; stop accepting updates from worker.
                let root = &mut node_graph_res.0;
                let path = async_state.computing_path.clone();
                cda_nav::with_graph_by_path_mut(root, &path, |graph| {
                    if let Some(v) = graph.cook_viz.as_ref() {
                        v.end();
                    }
                });
                return;
            }
            let root = &mut node_graph_res.0;
            let path = async_state.computing_path.clone();
            cda_nav::with_graph_by_path_mut(root, &path, |graph| {
                for node_id in &async_state.computing_nodes {
                    if let Some(geo) = computed_graph.geometry_cache.get(node_id) {
                        graph.geometry_cache.insert(*node_id, geo.clone());
                    }
                }
                for (id, record) in stats {
                    perf_monitor.node_cook_times.insert(id, record);
                }
                if graph.display_node == computed_graph.display_node {
                    graph.final_geometry = computed_graph.final_geometry.clone();
                }
                if let Some(v) = graph.cook_viz.as_ref() {
                    v.end();
                }
            });

            async_state.is_computing = false;
            async_state.computing_nodes.clear();
            async_state.computing_path.clear();

            // Trigger scene update with new GeometryChanged (and legacy GraphChanged for migration)
            geometry_changed_writer.write(GeometryChanged);
            graph_changed_writer.write(GraphChanged);
        } else {
            // Task not finished, put it back
            async_state.task = Some(task);
        }
    }
}
