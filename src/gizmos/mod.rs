use bevy::prelude::*;
// use bevy::math::Ray; // Removed
use crate::cunning_core::traits::node_interface::GizmoState;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::{
    apply_rotation_ctx, apply_scale_ctx, apply_translation_ctx, HandleOrientation, PivotMode,
    SelectableElement,
};
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
pub mod polyline_gizmos;
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
            .add_systems(Update, polyline_gizmos::draw_polyline_overlay_system)
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
    SplineSelectionTranslate {
        node_id: NodeId,
    },
    SplineSelectionRotate {
        node_id: NodeId,
    },
    SplineSelectionScale {
        node_id: NodeId,
    },
    /// Controls a generic Vec3 parameter on a node (e.g. Transform)
    ParamVec3 {
        node_id: NodeId,
        param_name: String,
    },
    ParamFloat {
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
    mut spline_tool_state: ResMut<SplineToolState>,
    plugin_system: Option<Res<crate::cunning_core::plugin_system::PluginSystem>>,
) {
    if events.is_empty() {
        return;
    }

    let selection = spline_tool_state.selection.clone();
    let base_ctx = spline_tool_state.ctx;

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
                                    &selection.selected_elements,
                                    delta,
                                    base_ctx,
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
                                    apply_translation_ctx(c, &selection.selected_elements, delta, base_ctx, None);
                                    graph_modified = true;
                                    dirty_nodes.insert(*node_id);
                                }
                            }
                        }
                    }
                }
            }
            GizmoBinding::SplineSelectionTranslate { node_id } => {
                spline_tool_state.drag_last_world = None;
                spline_tool_state.drag_last_scalar = None;
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            let ctx = update_spline_transform_ctx(
                                base_ctx,
                                c,
                                &selection,
                            );
                            let delta = event.new_position - ctx.pivot_position_world;
                            if delta.length_squared() > 1e-12 {
                                apply_translation_ctx(
                                    c,
                                    &selection.selected_elements,
                                    delta,
                                    ctx,
                                    None,
                                );
                                graph_modified = true;
                                dirty_nodes.insert(*node_id);
                            }
                        }
                    }
                }
            }
            GizmoBinding::SplineSelectionRotate { node_id } => {
                let total_deg = event.new_position;
                let prev_total = if spline_tool_state.drag_last_scalar == Some(1.0) {
                    spline_tool_state.drag_last_world.unwrap_or(Vec3::ZERO)
                } else {
                    Vec3::ZERO
                };
                let step_deg = total_deg - prev_total;
                spline_tool_state.drag_last_world = Some(total_deg);
                spline_tool_state.drag_last_scalar = Some(1.0);
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            if step_deg.length_squared() > 1e-10 {
                                let q_delta = Quat::from_euler(
                                    bevy::prelude::EulerRot::YXZ,
                                    step_deg.y.to_radians(),
                                    step_deg.x.to_radians(),
                                    step_deg.z.to_radians(),
                                );
                                let ctx = update_spline_transform_ctx(
                                    base_ctx,
                                    c,
                                    &selection,
                                );
                                apply_rotation_ctx(
                                    c,
                                    &selection.selected_elements,
                                    q_delta,
                                    ctx,
                                );
                                graph_modified = true;
                                dirty_nodes.insert(*node_id);
                            }
                        }
                    }
                }
            }
            GizmoBinding::SplineSelectionScale { node_id } => {
                let total = Vec3::new(
                    event.new_position.x.max(0.01),
                    event.new_position.y.max(0.01),
                    event.new_position.z.max(0.01),
                );
                let prev_total = if spline_tool_state.drag_last_scalar == Some(2.0) {
                    spline_tool_state.drag_last_world.unwrap_or(Vec3::ONE)
                } else {
                    Vec3::ONE
                };
                let safe_ratio = |a: f32, b: f32| {
                    if b.abs() > 1e-6 { a / b } else { 1.0 }
                };
                let step = Vec3::new(
                    safe_ratio(total.x, prev_total.x).max(0.01),
                    safe_ratio(total.y, prev_total.y).max(0.01),
                    safe_ratio(total.z, prev_total.z).max(0.01),
                );
                spline_tool_state.drag_last_world = Some(total);
                spline_tool_state.drag_last_scalar = Some(2.0);
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            if (step - Vec3::ONE).length_squared() > 1e-12 {
                                let ctx = update_spline_transform_ctx(
                                    base_ctx,
                                    c,
                                    &selection,
                                );
                                apply_scale_ctx(
                                    c,
                                    &selection.selected_elements,
                                    step,
                                    ctx,
                                );
                                graph_modified = true;
                                dirty_nodes.insert(*node_id);
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
            GizmoBinding::ParamFloat {
                node_id,
                param_name,
            } => {
                if let Some(node) = node_graph.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == *param_name)
                    {
                        if let ParameterValue::Float(ref mut val) = &mut param.value {
                            *val = event.new_position.x;
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

fn update_spline_transform_ctx(
    mut ctx: crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext,
    c: &crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer,
    sel: &crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SplineSelectionState,
) -> crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext {
    let spline_owner_knot = |e: SelectableElement| match e {
        SelectableElement::Knot(k) => Some((k.spline_index, k.knot_index)),
        SelectableElement::Tangent(t) => Some((t.spline_index, t.knot_index)),
    };

    // Handle rotation
    ctx.handle_rotation_world = match ctx.handle_orientation {
        HandleOrientation::Global => Quat::IDENTITY,
        HandleOrientation::Parent => c.local_to_world.to_scale_rotation_translation().1,
        HandleOrientation::Element => {
            let (spline_index, knot_index) = sel
                .active_element
                .and_then(spline_owner_knot)
                .unwrap_or((0, 0));
            if spline_index < c.splines.len() && knot_index < c.splines[spline_index].count() {
                let parent = c.local_to_world.to_scale_rotation_translation().1;
                parent * c.splines[spline_index].knots[knot_index].rotation
            } else {
                Quat::IDENTITY
            }
        }
    };

    // Pivot position
    ctx.pivot_position_world = match ctx.pivot_mode {
        PivotMode::Pivot => {
            if let Some((si, ki)) = sel.active_element.and_then(spline_owner_knot) {
                if si < c.splines.len() && ki < c.splines[si].count() {
                    c.local_to_world
                        .transform_point3(c.splines[si].knots[ki].position)
                } else {
                    ctx.pivot_position_world
                }
            } else {
                ctx.pivot_position_world
            }
        }
        PivotMode::Center => {
            let mut sum = Vec3::ZERO;
            let mut n = 0.0f32;
            let mut seen: std::collections::HashSet<(usize, usize)> =
                std::collections::HashSet::new();
            for &e in sel.selected_elements.iter() {
                if let Some((si, ki)) = spline_owner_knot(e) {
                    if !seen.insert((si, ki)) {
                        continue;
                    }
                    if si < c.splines.len() && ki < c.splines[si].count() {
                        sum += c
                            .local_to_world
                            .transform_point3(c.splines[si].knots[ki].position);
                        n += 1.0;
                    }
                }
            }
            if n > 0.0 {
                sum / n
            } else {
                ctx.pivot_position_world
            }
        }
    };

    ctx
}
