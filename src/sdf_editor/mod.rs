//! SDF Clay Edit: viewport brush -> RenderApp GPU brush queue.
//!
//! Current implementation goals:
//! - Zero CPU meshing (handled by `render::sdf_surface`).
//! - Push lightweight brush strokes from main world input -> render world compute.
//! - Keep logic minimal; picking uses ground plane fallback for now.

use bevy::prelude::*;
use bevy::ecs::system::SystemParam;

use crate::camera::ViewportInteractionState;
use crate::coverlay_bevy_ui::CoverlayUiWantsInput;
use crate::input::NavigationInput;
use crate::nodes::{NodeGraphResource, NodeType};
use crate::render::sdf_surface::{SdfBrushStroke, SdfBrushStrokeQueueShared};
use crate::tabs_system::viewport_3d::ViewportLayout;
use crate::ui::UiState;

pub struct SdfEditorPlugin;

impl Plugin for SdfEditorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SdfBrushStrokeQueueShared>();
        app.init_resource::<SdfClayStrokeState>();
        app.add_systems(Update, sdf_clay_input_system);
    }
}

#[derive(Resource, Default)]
struct SdfClayStrokeState {
    // (target_ptr, last_hit_world)
    last_add: Option<(usize, Vec3)>,
    last_sub: Option<(usize, Vec3)>,
}

#[derive(SystemParam)]
struct SdfEditorParams<'w, 's> {
    app_state: Res<'w, State<crate::launcher::plugin::AppState>>,
    mouse: Res<'w, ButtonInput<MouseButton>>,
    keys: Res<'w, ButtonInput<KeyCode>>,
    nav: Res<'w, NavigationInput>,
    viewport_layout: Res<'w, ViewportLayout>,
    windows: Query<'w, 's, (Entity, &'static Window)>,
    cam_q: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<crate::MainCamera>>,
    interaction: Res<'w, ViewportInteractionState>,
    coverlay_wants: Res<'w, CoverlayUiWantsInput>,
    ui_state: Res<'w, UiState>,
    node_graph_res: Res<'w, NodeGraphResource>,
    strokes: Res<'w, SdfBrushStrokeQueueShared>,
    stroke_state: ResMut<'w, SdfClayStrokeState>,
}

fn resolve_sdf_clay_edit_node_id(ui: &UiState, g: &crate::nodes::NodeGraph) -> Option<crate::nodes::NodeId> {
    let id = ui
        .last_selected_node_id
        .or_else(|| ui.selected_nodes.iter().next().copied())?;
    let n = g.nodes.get(&id)?;
    match &n.node_type {
        NodeType::Generic(s) if s.trim().eq_ignore_ascii_case("SDF Clay Edit") => Some(id),
        _ => None,
    }
}

fn read_node_param_bool(n: &crate::nodes::Node, key: &str, d: bool) -> bool {
    n.parameters
        .iter()
        .find(|p| p.name == key)
        .and_then(|p| if let crate::nodes::parameter::ParameterValue::Bool(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn read_node_param_f32(n: &crate::nodes::Node, key: &str, d: f32) -> f32 {
    n.parameters
        .iter()
        .find(|p| p.name == key)
        .and_then(|p| if let crate::nodes::parameter::ParameterValue::Float(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn read_node_param_i32(n: &crate::nodes::Node, key: &str, d: i32) -> i32 {
    n.parameters
        .iter()
        .find(|p| p.name == key)
        .and_then(|p| if let crate::nodes::parameter::ParameterValue::Int(v) = &p.value { Some(*v) } else { None })
        .unwrap_or(d)
}

fn sdf_clay_input_system(mut p: SdfEditorParams) {
    puffin::profile_function!();

    if *p.app_state.get() != crate::launcher::plugin::AppState::Editor {
        p.stroke_state.last_add = None;
        p.stroke_state.last_sub = None;
        return;
    }
    if p.coverlay_wants.0 || p.nav.active || !p.interaction.is_hovered {
        p.stroke_state.last_add = None;
        p.stroke_state.last_sub = None;
        return;
    }

    let Ok((camera, camera_tfm)) = p.cam_q.single() else { return; };
    let cursor = p
        .viewport_layout
        .window_entity
        .and_then(|e| p.windows.get(e).ok().and_then(|(_, w)| w.cursor_position()))
        .unwrap_or(Vec2::new(-99999.0, -99999.0));
    let Ok(ray) = camera.viewport_to_world(camera_tfm, cursor) else { return; };

    let Some(node_id) = resolve_sdf_clay_edit_node_id(&p.ui_state, &p.node_graph_res.0) else {
        return;
    };
    let Some(node) = p.node_graph_res.0.nodes.get(&node_id) else { return; };

    if !read_node_param_bool(node, "enabled", true) {
        return;
    }

    // Target handle: prefer geometry_cache (fresh), fallback to prev cache, then final geometry.
    let target_handle = p
        .node_graph_res
        .0
        .geometry_cache
        .get(&node_id)
        .or_else(|| p.node_graph_res.0.prev_geometry_cache.get(&node_id))
        .and_then(|g| g.sdfs.first().cloned())
        .or_else(|| p.node_graph_res.0.final_geometry.sdfs.first().cloned());

    let Some(handle) = target_handle else { return; };

    let radius = read_node_param_f32(node, "radius", 0.5).max(0.0);
    let smooth_k = read_node_param_f32(node, "smooth_k", 0.05).max(0.0);
    let mut mode = read_node_param_i32(node, "mode", 0).clamp(0, 2) as u32;

    let down_l = p.mouse.pressed(MouseButton::Left);
    let down_r = p.mouse.pressed(MouseButton::Right);
    if !down_l && !down_r {
        p.stroke_state.last_add = None;
        p.stroke_state.last_sub = None;
        return;
    }

    // Shift inverts add/subtract quickly.
    let shift = p.keys.pressed(KeyCode::ShiftLeft) || p.keys.pressed(KeyCode::ShiftRight);
    let is_subtract = down_r ^ shift;
    if is_subtract {
        mode = 1;
    }

    // Picking MVP: ground plane intersection (y=0). Replace with SDF raymarch later.
    let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else { return; };
    let hit_world = ray.get_point(dist);
    let target_ptr = std::sync::Arc::as_ptr(&handle.grid) as usize;

    // Continuous stroke: turn per-frame samples into capsules (prev -> current).
    let (a_world, b_world) = if is_subtract {
        match p.stroke_state.last_sub {
            Some((ptr, last)) if ptr == target_ptr => (last, hit_world),
            _ => (hit_world, hit_world),
        }
    } else {
        match p.stroke_state.last_add {
            Some((ptr, last)) if ptr == target_ptr => (last, hit_world),
            _ => (hit_world, hit_world),
        }
    };

    if is_subtract {
        p.stroke_state.last_sub = Some((target_ptr, hit_world));
    } else {
        p.stroke_state.last_add = Some((target_ptr, hit_world));
    }

    let stroke = SdfBrushStroke {
        target_ptr,
        a_world,
        b_world,
        radius_world: radius,
        smooth_k_world: smooth_k,
        mode,
    };

    if let Ok(mut q) = p.strokes.0.try_lock() {
        q.push(stroke);
    }
}
