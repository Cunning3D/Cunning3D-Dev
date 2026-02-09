use bevy::prelude::*;
// use bevy::math::Ray; // Removed
use crate::cunning_core::traits::node_interface::GizmoState;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::apply_translation_ctx;
use crate::nodes::parameter::ParameterValue;
use crate::nodes::spline::tool_state::SplineToolState;
use crate::nodes::NodeGraphResource;
use crate::nodes::NodeId;
use crate::GraphChanged;

pub mod constants;
pub mod control_id;
pub mod input;
pub mod material;
pub mod picking;
pub mod plugin_gizmos;
pub mod renderer; // V5 Retained Renderer
pub mod spline_gizmos;
pub mod standard; // V5 Standard Gizmo

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct TransformGizmoLines;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct SelectedCurveGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct SelectedCurveXrayGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct GridGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct GridMajorGizmos;

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct GridAxisGizmos; // For main axes X=0, Z=0

#[derive(Default, Reflect, GizmoConfigGroup)]
pub struct UvBoundaryGizmos;

pub struct GizmoPlugin;

impl Plugin for GizmoPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<GizmoMovedEvent>()
            .init_resource::<GizmoState>() // V5 State
            .init_resource::<control_id::ControlIdState>()
            .init_resource::<GizmoActionQueue>()
            .init_resource::<renderer::GizmoDrawBuffer>()
            .add_systems(Startup, renderer::setup_gizmo_assets)
            .add_systems(Update, update_gizmo_visuals)
            .add_systems(Update, plugin_gizmos::sync_plugin_gizmos)
            // Curve node needs direct input handling (F/G/H + click-to-add points).
            // NOTE: this system self-guards (no viewport/camera => early return), so it's safe during Splash.
            .add_systems(
                Update,
                input::handle_gizmo_interaction
                    .after(crate::tabs_system::viewport_3d::camera_sync::sync_main_camera_viewport),
            )
            .add_systems(
                Update,
                flush_gizmo_action_queue
                    .after(crate::tabs_system::viewport_3d::gizmo_systems::draw_interactive_gizmos_system)
                    .before(handle_gizmo_moved_events),
            )
            .add_systems(Update, handle_gizmo_moved_events)
            // Spline gizmo pose depends on spline data; run after edits so Auto tangents update live during drag.
            .add_systems(
                Update,
                spline_gizmos::sync_spline_gizmos.after(handle_gizmo_moved_events),
            )
            .add_systems(PostUpdate, renderer::sync_gizmo_entities);
    }
}

// --- Components ---

/// Tag component for all gizmo entities
#[derive(Component)]
pub struct GizmoTag;

/// Component defining how the gizmo looks in different states
#[derive(Component)]
pub struct GizmoColor {
    pub normal: Color,
    pub hover: Color,
    pub active: Color,
}

impl Default for GizmoColor {
    fn default() -> Self {
        Self {
            normal: Color::WHITE,
            hover: Color::srgb(1.0, 1.0, 0.0),
            active: Color::srgb(1.0, 0.5, 0.0),
        }
    }
}

/// Defines what data this gizmo is controlling.
/// This allows the system to be generic.
#[derive(Component, Debug, Clone, PartialEq, Eq, Hash)]
pub enum GizmoBinding {
    SplineKnot {
        node_id: NodeId,
        spline_index: usize,
        knot_index: usize,
    },
    SplineTangent {
        node_id: NodeId,
        spline_index: usize,
        knot_index: usize,
        tangent: crate::libs::algorithms::algorithms_runtime::unity_spline::BezierTangent,
    },
    /// Controls a generic Vec3 parameter on a node (e.g. Transform)
    ParamVec3 {
        node_id: NodeId,
        param_name: String,
    },
    PluginPick {
        node_id: NodeId,
        pick_id: u32,
    },
}

/// Component for interactive state
#[derive(Component, Default)]
pub struct GizmoInteraction {
    pub is_hovered: bool,
    pub is_dragged: bool,
    pub drag_start_ray_origin: Option<Vec3>, // For calculating drag delta
    pub drag_start_ray_dir: Option<Vec3>,
    pub initial_position: Option<Vec3>, // Initial position of the gizmo when drag started
    pub drag_start_mouse: Option<Vec2>,
    pub last_mouse: Option<Vec2>,
}

// --- Events ---

#[derive(Message)]
pub struct GizmoMovedEvent {
    pub binding: GizmoBinding,
    pub new_position: Vec3,
}

#[derive(Resource, Default)]
pub struct GizmoActionQueue(std::sync::Mutex<Vec<GizmoMovedEvent>>);

impl GizmoActionQueue {
    pub fn push(&self, e: GizmoMovedEvent) { self.0.lock().unwrap().push(e); }
    fn drain(&self) -> Vec<GizmoMovedEvent> { std::mem::take(&mut *self.0.lock().unwrap()) }
}

// --- Systems ---

fn flush_gizmo_action_queue(q: Res<GizmoActionQueue>, mut w: MessageWriter<GizmoMovedEvent>) {
    for e in q.drain() { w.write(e); }
}

fn update_gizmo_visuals(
    mut q_std: Query<
        (
            &GizmoInteraction,
            &GizmoColor,
            &mut MeshMaterial3d<StandardMaterial>,
        ),
        With<GizmoTag>,
    >,
    mut std_mats: ResMut<Assets<StandardMaterial>>,
    mut q_ovl: Query<
        (
            &GizmoInteraction,
            &GizmoColor,
            &mut MeshMaterial3d<crate::gizmos::renderer::GizmoMaterial>,
        ),
        With<GizmoTag>,
    >,
    mut ovl_mats: ResMut<Assets<crate::gizmos::renderer::GizmoMaterial>>,
) {
    for (interaction, colors, mat_handle) in q_std.iter_mut() {
        let target_color = if interaction.is_dragged {
            colors.active
        } else if interaction.is_hovered {
            colors.hover
        } else {
            colors.normal
        };

        // Note: In a real optimized system, we wouldn't create a new material every frame.
        // We would swap between pre-created handles or modify a material property.
        // For MVP, we'll assume we are managing handles properly elsewhere or just accepting the overhead for now.
        // Actually, let's NOT create new materials here to avoid leak.
        // A better approach for MVP: The material handle itself should point to a shared material that we swap?
        // Or we just use Wireframe color?
        // Let's skip material swapping for a second and rely on Gizmo lines for now,
        // OR assuming the spawner sets up unique materials we can mutate.

        if let Some(mat) = std_mats.get_mut(mat_handle.0.id()) {
            if mat.base_color != target_color {
                mat.base_color = target_color;
            }
        }
    }

    for (interaction, colors, mat_handle) in q_ovl.iter_mut() {
        let target_color = if interaction.is_dragged {
            colors.active
        } else if interaction.is_hovered {
            colors.hover
        } else {
            colors.normal
        };
        if let Some(mat) = ovl_mats.get_mut(mat_handle.0.id()) {
            if mat.base.base_color != target_color {
                mat.base.base_color = target_color;
            }
        }
    }
}

fn handle_gizmo_moved_events(
    mut events: MessageReader<GizmoMovedEvent>,
    mut node_graph_res: ResMut<NodeGraphResource>,
    mut graph_changed_writer: MessageWriter<GraphChanged>,
    spline_tool_state: Res<SplineToolState>,
    plugin_system: Option<Res<crate::cunning_core::plugin_system::PluginSystem>>,
) {
    if events.is_empty() {
        return;
    }

    let mut graph_modified = false;
    let mut dirty_nodes: std::collections::HashSet<NodeId> = std::collections::HashSet::new();
    // NOTE: Plugin drag callbacks may mutate NodeGraph via host API, so we must not hold a mutable borrow while calling plugins.
    let mut plugin_drags: Vec<(NodeId, u32, Vec3, String)> = Vec::new(); // (node_id, pick_id, world_pos, key)
    {
        let node_graph = &mut node_graph_res.0;
        for event in events.read() {
            match &event.binding {
            GizmoBinding::SplineKnot {
                node_id,
                spline_index,
                knot_index,
            } => {
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            let si = *spline_index;
                            let ki = *knot_index;
                            if si < c.splines.len() && ki < c.splines[si].count() {
                                let old_world = c
                                    .local_to_world
                                    .transform_point3(c.splines[si].knots[ki].position);
                                let delta = event.new_position - old_world;
                                apply_translation_ctx(
                                    c,
                                    &spline_tool_state.selection.selected_elements,
                                    delta,
                                    spline_tool_state.ctx,
                                    None,
                                );
                                graph_modified = true;
                                dirty_nodes.insert(*node_id);
                            }
                        }
                    }
                }
            }
            GizmoBinding::SplineTangent {
                node_id,
                spline_index,
                knot_index,
                tangent,
            } => {
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            let si = *spline_index;
                            let ki = *knot_index;
                            if si < c.splines.len() && ki < c.splines[si].count() {
                                let mode = c.splines[si].meta[ki].mode;
                                if crate::libs::algorithms::algorithms_runtime::unity_spline::are_tangents_modifiable(mode) {
                                    let knot = c.splines[si].knots[ki];
                                    let old_world = c.local_to_world.transform_point3(knot.position + knot.rotation.mul_vec3(if *tangent == crate::libs::algorithms::algorithms_runtime::unity_spline::BezierTangent::In { knot.tangent_in } else { knot.tangent_out }));
                                    let delta = event.new_position - old_world;
                                    apply_translation_ctx(c, &spline_tool_state.selection.selected_elements, delta, spline_tool_state.ctx, None);
                                    graph_modified = true;
                                    dirty_nodes.insert(*node_id);
                                }
                            }
                        }
                    }
                }
            }
            GizmoBinding::ParamVec3 {
                node_id,
                param_name,
            } => {
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == *param_name)
                    {
                        if let ParameterValue::Vec3(ref mut val) = &mut param.value {
                            *val = event.new_position;
                            graph_modified = true;
                            dirty_nodes.insert(*node_id);
                        }
                    }
                }
            }
            GizmoBinding::PluginPick { node_id, pick_id } => {
                if plugin_system.is_some() {
                    let key = node_graph
                        .nodes
                        .get(node_id)
                        .map(|n| n.node_type.name().to_string())
                        .unwrap_or_default();
                    if !key.is_empty() {
                        plugin_drags.push((*node_id, *pick_id, event.new_position, key));
                    }
                }
            }
            }
        }
    } // drop mutable borrow before calling plugins

    if graph_modified {
        for id in dirty_nodes {
            node_graph_res.0.mark_dirty(id);
        }
    }
    let mut plugin_modified = false;
    if let Some(ps) = plugin_system.as_ref() {
        for (node_id, pick_id, pos, key) in plugin_drags {
            if ps.plugin_gizmo_event_drag(&node_graph_res, &key, node_id, pick_id, pos) {
                plugin_modified = true;
            }
        }
    }
    if graph_modified || plugin_modified {
        graph_changed_writer.write_default();
    }
}
