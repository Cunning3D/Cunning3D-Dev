use crate::coverlay_bevy_ui::{
    read_voxel_cmds, resolve_voxel_edit_target, voxel_size_for_target, write_voxel_cmds,
    CoverlayUiWantsInput, VoxelEditTarget, VoxelToolMode, VoxelToolState,
};
use crate::cunning_core::cda::library::global_cda_library;
use crate::cunning_core::plugin_system::c_api as plugin_c_api;
use crate::cunning_core::plugin_system::PluginSystem;
use crate::gizmos::constants::*;
use crate::gizmos::control_id::ControlIdState;
use crate::gizmos::GizmoMovedEvent;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::get_nearest_point_on_curve_ray;
use crate::libs::algorithms::algorithms_runtime::unity_spline::editor::{
    HandleOrientation, PivotMode, SelectableElement,
};
use crate::libs::algorithms::algorithms_runtime::unity_spline::TangentMode;
use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    BezierCurve, SelectableKnot, SelectableTangent,
};
use crate::libs::algorithms::algorithms_runtime::unity_spline::{
    DrawingDirection, SplineKnotIndex,
};
use crate::nodes::parameter::ParameterValue;
use crate::nodes::spline::tool_state::DirectDragKnot;
use crate::nodes::spline::tool_state::SplineDrawState;
use crate::nodes::spline::tool_state::SplineEditMode;
use crate::nodes::spline::tool_state::{HoveredCurve, SplineTransformTool};
use crate::nodes::spline::tool_state::{
    SplineAxisConstraint, SplineToolState,
};
use crate::tabs_system::{FloatingEditorTabs, TabViewer, Viewport3DTab};
use crate::ui::FloatingTabRegistry;
use crate::ui::UiState;
use crate::GraphChanged;
use crate::{
    gizmos::{GizmoBinding, GizmoInteraction, GizmoTag},
    nodes::NodeGraphResource,
    nodes::NodeId,
    nodes::NodeType,
};
use bevy::math::primitives::InfinitePlane3d;
use bevy::math::{Dir3, Ray3d};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_ecs::system::SystemParam;
use bevy_egui::egui;
use cunning_kernel::algorithms::algorithms_editor::voxel::{DiscreteVoxelCmdList, DiscreteVoxelOp};

// Bevy systems have an arity limit for function params; pack Curve tool inputs into a SystemParam.
#[derive(SystemParam)]
pub struct CurveGizmoInputParams<'w, 's> {
    commands: Commands<'w, 's>,
    // Use `Transform` for picking (immediate, no transform-propagate ordering issues).
    q_gizmos: Query<
        'w,
        's,
        (
            Entity,
            &'static Transform,
            &'static mut GizmoInteraction,
            &'static GizmoBinding,
        ),
        With<GizmoTag>,
    >,
    mouse_button: Res<'w, ButtonInput<MouseButton>>,
    keyboard: Res<'w, ButtonInput<KeyCode>>,
    gizmos: Gizmos<'w, 's>,
    _mouse_motion: MessageReader<'w, 's, bevy::input::mouse::MouseMotion>,
    primary_window: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    windows: Query<'w, 's, (Entity, &'static Window)>,
    camera_q: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<crate::MainCamera>>,
    tab_viewer: Res<'w, TabViewer>,
    floating_tabs: Res<'w, FloatingEditorTabs>,
    floating_registry: Res<'w, FloatingTabRegistry>,
    node_graph_res: ResMut<'w, NodeGraphResource>,
    ui_state: ResMut<'w, UiState>,
    spline_tool_state: ResMut<'w, SplineToolState>,
    control_id: ResMut<'w, ControlIdState>,
    gizmo_moved_writer: MessageWriter<'w, GizmoMovedEvent>,
    graph_changed_writer: MessageWriter<'w, GraphChanged>,
    console_log: Res<'w, crate::console::ConsoleLog>,
    plugin_system: Option<Res<'w, PluginSystem>>,
    coverlay_wants_input: Res<'w, CoverlayUiWantsInput>,
    voxel_tool_state: ResMut<'w, VoxelToolState>,
}

pub fn handle_gizmo_interaction(p: CurveGizmoInputParams) {
    let mut commands = p.commands;
    let mut q_gizmos = p.q_gizmos;
    let mouse_button = p.mouse_button;
    let keyboard = p.keyboard;
    let mut gizmos = p.gizmos;
    let primary_window = p.primary_window;
    let windows = p.windows;
    let camera_q = p.camera_q;
    let tab_viewer = p.tab_viewer;
    let floating_tabs = p.floating_tabs;
    let floating_registry = p.floating_registry;
    let mut node_graph_res = p.node_graph_res;
    let mut ui_state = p.ui_state;
    let mut spline_tool_state = p.spline_tool_state;
    let mut control_id = p.control_id;
    let mut gizmo_moved_writer = p.gizmo_moved_writer;
    let mut graph_changed_writer = p.graph_changed_writer;
    let console_log = p.console_log;
    let plugin_system = p.plugin_system;
    let coverlay_wants_input = p.coverlay_wants_input;
    let mut voxel_tool_state = p.voxel_tool_state;
    let log_spline_pick = std::env::var_os("DCC_LOG_SPLINE_PICK").is_some();
    let log_curve_plugin = std::env::var_os("DCC_LOG_CURVE_PLUGIN").is_some();
    // 1. Locate the active viewport (dock or floating) and its host window.
    enum ViewportHost {
        Primary,
        Floating(Entity),
    }

    let (host, viewport_rect) = {
        if let Some(rect) = tab_viewer
            .dock_state
            .iter_all_tabs()
            .find_map(|((_s, _n), tab)| tab.as_any().downcast_ref::<Viewport3DTab>())
            .and_then(|t| t.viewport_rect)
        {
            (ViewportHost::Primary, rect)
        } else {
            let mut found: Option<(ViewportHost, egui::Rect)> = None;

            for (id, tab) in floating_tabs.tabs.iter() {
                if let Some(vp) = tab.as_any().downcast_ref::<Viewport3DTab>() {
                    if let Some(rect) = vp.viewport_rect {
                        if let Some((window_entity, _entry)) = floating_registry
                            .floating_windows
                            .iter()
                            .find(|(_, entry)| &entry.id == id)
                        {
                            found = Some((ViewportHost::Floating(*window_entity), rect));
                            break;
                        }
                    }
                }
            }

            match found {
                Some(info) => info,
                None => return,
            }
        }
    };

    let Ok((camera, camera_transform)) = camera_q.single() else {
        return;
    };

    // Pick the correct window for cursor queries.
    let window: &Window = match host {
        ViewportHost::Primary => match primary_window.single() {
            Ok(w) => w,
            Err(_) => return,
        },
        ViewportHost::Floating(window_entity) => match windows.get(window_entity) {
            Ok((_e, w)) => w,
            Err(_) => return,
        },
    };

    // 2. Cursor + ray
    // IMPORTANT: all 3D viewport-bound node interactions (including mode keys) must be gated to viewport.
    // We gate by egui viewport_rect (available even before camera viewport sync),
    // but keep ray creation in WINDOW coordinates (Bevy 0.18 will apply camera.viewport internally).
    // NOTE: do not hard-return when the cursor is outside `viewport_rect`.
    // Different UI layers may report slightly different coordinate spaces; returning here would
    // disable spline picking entirely. We instead use a dummy cursor so rays naturally miss.
    let cursor_pos_window = window
        .cursor_position()
        .unwrap_or(Vec2::new(-99999.0, -99999.0));
    let cursor_in_viewport =
        viewport_rect.contains(egui::pos2(cursor_pos_window.x, cursor_pos_window.y));

    // Bevy 0.18: `Camera::viewport_to_world` expects WINDOW logical coordinates.
    // It subtracts `camera.logical_viewport_rect().min` internally and flips Y for NDC itself.
    // If we subtract viewport_min here, we double-apply the viewport offset -> consistent click/pick bias.
    let (ray, ray_is_fallback) = match camera.viewport_to_world(camera_transform, cursor_pos_window)
    {
        Ok(r) => (r, false),
        Err(_) => (Ray3d::new(Vec3::splat(1.0e9), Dir3::NEG_Z), true),
    };

    fn spline_owner_knot(e: SelectableElement) -> Option<(usize, usize)> {
        match e {
            SelectableElement::Knot(k) => Some((k.spline_index, k.knot_index)),
            SelectableElement::Tangent(t) => Some((t.spline_index, t.knot_index)),
        }
    }

    fn update_spline_transform_ctx(
        mut ctx: crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext,
        c: &crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer,
        sel: &crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SplineSelectionState,
    ) -> crate::libs::algorithms::algorithms_runtime::unity_spline::editor::TransformContext {
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

    fn resolve_cda_hud_spline_target(
        ui: &UiState,
        g: &crate::nodes::structs::NodeGraph,
    ) -> Option<(NodeId, NodeId)> {
        let inst_id = ui
            .last_selected_node_id
            .or_else(|| ui.selected_nodes.iter().next().copied())?;
        let inst = g.nodes.get(&inst_id)?;
        let NodeType::CDA(data) = &inst.node_type else {
            return None;
        };
        let lib = global_cda_library()?;
        let _ = lib.ensure_loaded(&data.asset_ref);
        let a = lib.get(data.asset_ref.uuid)?;
        let mut hud_id = data.coverlay_hud;
        if hud_id.is_none() {
            let mut huds = a.hud_units.clone();
            huds.sort_by_key(|u| (u.order, u.node_id));
            hud_id = huds
                .iter()
                .find(|u| u.is_default)
                .map(|u| u.node_id)
                .or_else(|| huds.first().map(|u| u.node_id));
        }
        let hud_id = hud_id?;
        let inner = a.inner_graph.nodes.get(&hud_id)?;
        if matches!(inner.node_type, NodeType::Spline) {
            Some((inst_id, hud_id))
        } else {
            None
        }
    }
    fn base_spline_from_cda(
        data: &crate::nodes::structs::CDANodeData,
        internal: NodeId,
    ) -> Option<crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer> {
        if let Some(m) = data
            .inner_param_overrides
            .get(&internal)
            .and_then(|m| m.get("spline"))
        {
            if let ParameterValue::UnitySpline(c) = m {
                return Some(c.clone());
            }
        }
        let lib = global_cda_library()?;
        let _ = lib.ensure_loaded(&data.asset_ref);
        let a = lib.get(data.asset_ref.uuid)?;
        a.inner_graph
            .nodes
            .get(&internal)
            .and_then(|n| n.parameters.iter().find(|p| p.name == "spline"))
            .and_then(|p| {
                if let ParameterValue::UnitySpline(c) = &p.value {
                    Some(c.clone())
                } else {
                    None
                }
            })
    }
    fn store_spline_to_cda(
        data: &mut crate::nodes::structs::CDANodeData,
        internal: NodeId,
        c: crate::libs::algorithms::algorithms_runtime::unity_spline::SplineContainer,
    ) {
        data.inner_param_overrides
            .entry(internal)
            .or_default()
            .insert("spline".to_string(), ParameterValue::UnitySpline(c));
    }

    // 体素编辑目标与参数读写放在 coverlay_bevy_ui 里统一实现，避免 UI / 交互两套逻辑漂移。

    // Selected node kind.
    // If the node panel selection is empty, fall back to display node so viewport interaction still works.
    let (
        is_spline_selected,
        selected_node_id,
        cda_spline_target,
        plugin_node_key,
        selected_type_name,
    ): (
        bool,
        Option<NodeId>,
        Option<(NodeId, NodeId)>,
        Option<String>,
        Option<String>,
    ) = {
        let sel_single = if ui_state.selected_nodes.len() == 1 {
            ui_state.selected_nodes.iter().next().copied()
        } else {
            None
        };
        let sel_primary = sel_single.or(ui_state.last_selected_node_id);
        let sel2 = sel_primary.or_else(|| node_graph_res.0.display_node);
        let (ty, cda_spline): (Option<NodeType>, Option<(NodeId, NodeId)>) = sel2
            .and_then(|id| {
                let ng = &node_graph_res.0;
                let ty = ng.nodes.get(&id).map(|n| n.node_type.clone());
                let cda_spline = resolve_cda_hud_spline_target(&ui_state, ng);
                Some((ty, cda_spline))
            })
            .unwrap_or((None, None));
        let tname = ty.as_ref().map(|t| t.name().to_string());
        let key = sel2
            .and_then(|id| {
                node_graph_res
                    .0
                    .nodes
                    .get(&id)
                    .map(|n| n.node_type.name().to_string())
            })
            .and_then(|k| {
                plugin_system.as_ref().and_then(|ps| {
                    if ps.interaction_shared(&k).is_some() {
                        Some(k)
                    } else {
                        None
                    }
                })
            });
        let (is_spline, node_for_spline, cda_target) = if let Some((inst_id, internal)) = cda_spline
        {
            (true, Some(internal), Some((inst_id, internal)))
        } else {
            (matches!(ty, Some(NodeType::Spline)), sel2, None)
        };
        (is_spline, node_for_spline, cda_target, key, tname)
    };

    let voxel_target: Option<VoxelEditTarget> =
        resolve_voxel_edit_target(&ui_state, &node_graph_res.0);
    let is_voxel_selected =
        matches!(selected_type_name.as_deref(), Some("Voxel Edit")) || voxel_target.is_some();
    if log_curve_plugin
        && selected_type_name.as_deref() == Some("Curve")
        && plugin_node_key.is_none()
    {
        bevy::log::warn!("[CurvePlugin] selected=Curve but no interaction key found (maybe plugin not loaded or name mismatch).");
    }

    // --- Mode Switching (Curve only; viewport-gated) ---
    if plugin_node_key.is_some() {
        if let (Some(ps), Some(node_id), Some(key)) = (
            plugin_system.as_ref(),
            selected_node_id,
            plugin_node_key.as_ref(),
        ) {
            if keyboard.just_pressed(KeyCode::KeyF) {
                let ok = ps.plugin_input_key_down(
                    &node_graph_res,
                    key,
                    node_id,
                    plugin_c_api::CKeyCode::F,
                );
                if log_curve_plugin && key == "Curve" {
                    bevy::log::info!("[CurvePlugin] KeyF -> ok={}", ok);
                }
            }
            if keyboard.just_pressed(KeyCode::KeyG) {
                let ok = ps.plugin_input_key_down(
                    &node_graph_res,
                    key,
                    node_id,
                    plugin_c_api::CKeyCode::G,
                );
                if log_curve_plugin && key == "Curve" {
                    bevy::log::info!("[CurvePlugin] KeyG -> ok={}", ok);
                }
            }
            if keyboard.just_pressed(KeyCode::KeyH) {
                let ok = ps.plugin_input_key_down(
                    &node_graph_res,
                    key,
                    node_id,
                    plugin_c_api::CKeyCode::H,
                );
                if log_curve_plugin && key == "Curve" {
                    bevy::log::info!("[CurvePlugin] KeyH -> ok={}", ok);
                }
            }
            if keyboard.just_pressed(KeyCode::KeyK) {
                let ok = ps.plugin_input_key_down(
                    &node_graph_res,
                    key,
                    node_id,
                    plugin_c_api::CKeyCode::K,
                );
                if log_curve_plugin && key == "Curve" {
                    bevy::log::info!("[CurvePlugin] KeyK -> ok={}", ok);
                }
            }
        }
    }

    // --- Common Interaction State ---
    let just_clicked = mouse_button.just_pressed(MouseButton::Left);
    let is_pressed = mouse_button.pressed(MouseButton::Left);
    let just_released = mouse_button.just_released(MouseButton::Left);
    let is_ctrl_held =
        keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let is_shift_held =
        keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);
    if coverlay_wants_input.0 {
        return;
    }

    // Voxel editing input moved to `voxel_editor` (avoid double-writing cmds_json from two systems).
    if false && is_voxel_selected {
        if !cursor_in_viewport || ray_is_fallback {
            return;
        }
        if keyboard.just_pressed(KeyCode::Digit1) {
            voxel_tool_state.mode = VoxelToolMode::Add;
        }
        if keyboard.just_pressed(KeyCode::Digit2) {
            voxel_tool_state.mode = VoxelToolMode::Select;
        }
        if keyboard.just_pressed(KeyCode::Digit3) {
            voxel_tool_state.mode = VoxelToolMode::Move;
        }
        if keyboard.just_pressed(KeyCode::Digit4) {
            voxel_tool_state.mode = VoxelToolMode::Paint;
        }
        if keyboard.just_pressed(KeyCode::BracketLeft) {
            voxel_tool_state.brush_radius = (voxel_tool_state.brush_radius * 0.9).max(0.01);
        }
        if keyboard.just_pressed(KeyCode::BracketRight) {
            voxel_tool_state.brush_radius = (voxel_tool_state.brush_radius * 1.1).min(1000.0);
        }
        if keyboard.just_pressed(KeyCode::KeyZ) || keyboard.just_pressed(KeyCode::KeyY) {
            let Some(t) = voxel_target else { return; };
            let ng = &mut node_graph_res.0;
            let mut c = read_voxel_cmds(&ng, t);
            if keyboard.just_pressed(KeyCode::KeyZ) { let _ = c.undo(); }
            if keyboard.just_pressed(KeyCode::KeyY) { let _ = c.redo(); }
            if let Some(dirty) = write_voxel_cmds(ng, t, c) { ng.mark_dirty(dirty); }
            graph_changed_writer.write_default();
            return;
        }

        if just_clicked
            && matches!(
                voxel_tool_state.mode,
                VoxelToolMode::Add | VoxelToolMode::Paint
            )
        {
            let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) else {
                return;
            };
            let hit = ray.get_point(dist);
            let palette_index = voxel_tool_state.palette_index.max(1);
            let Some(t) = voxel_target else { return; };
            let voxel_size = voxel_size_for_target(&node_graph_res.0, t).max(0.001);
            let op = if voxel_tool_state.mode == VoxelToolMode::Add {
                let p = (hit / voxel_size).floor().as_ivec3();
                if is_shift_held {
                    DiscreteVoxelOp::RemoveVoxel {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                    }
                } else {
                    DiscreteVoxelOp::SetVoxel {
                        x: p.x,
                        y: p.y,
                        z: p.z,
                        palette_index,
                    }
                }
            } else {
                if is_shift_held {
                    DiscreteVoxelOp::SphereRemove {
                        center: hit,
                        radius: voxel_tool_state.brush_radius,
                    }
                } else {
                    DiscreteVoxelOp::SphereAdd {
                        center: hit,
                        radius: voxel_tool_state.brush_radius,
                        palette_index,
                    }
                }
            };
            let Some(t) = voxel_target else { return; };
            let ng = &mut node_graph_res.0;
            let mut c = read_voxel_cmds(&ng, t);
            c.push(op);
            if let Some(dirty) = write_voxel_cmds(ng, t, c) { ng.mark_dirty(dirty); }
            graph_changed_writer.write_default();
            return;
        }
    }

    #[inline]
    fn is_knot(e: SelectableElement) -> bool {
        matches!(e, SelectableElement::Knot(_))
    }

    // Unity spline selection rules:
    // - Shift: add (never removes), sets active to clicked element
    // - Ctrl: remove only (never adds), but cannot remove active KNOT
    // - No modifier: replace selection with clicked element
    fn apply_spline_select(
        sel: &mut crate::libs::algorithms::algorithms_runtime::unity_spline::editor::SplineSelectionState,
        e: SelectableElement,
        shift: bool,
        ctrl: bool,
    ) {
        if ctrl {
            if is_knot(e) && sel.is_active(e) {
                return;
            }
            if sel.contains(e) {
                sel.remove(e);
            }
            return;
        }
        if shift {
            sel.add(e);
            sel.set_active(Some(e));
            return;
        }
        sel.clear();
        sel.add(e);
        sel.set_active(Some(e));
    }
    if log_spline_pick
        && is_spline_selected
        && (just_clicked
            || keyboard.just_pressed(KeyCode::KeyW)
            || keyboard.just_pressed(KeyCode::KeyE)
            || keyboard.just_pressed(KeyCode::KeyR))
    {
        bevy::log::info!(
            "[SplinePick] click={} cursor_win={:?} ray_fallback={}",
            just_clicked,
            cursor_pos_window,
            ray_is_fallback
        );
    }

    // Safety: if a gizmo drag got "stuck" due to lost mouse-up event, clear it when the button is not pressed.
    if !is_pressed && !just_released {
        for (_e, _t, mut i, _b) in q_gizmos.iter_mut() {
            if i.is_dragged {
                i.is_dragged = false;
                i.drag_start_ray_origin = None;
                i.initial_position = None;
                i.drag_start_mouse = None;
                i.last_mouse = None;
            }
        }
        control_id.hot_entity = None;
        spline_tool_state.direct_drag_knot = None;
    }

    // --- Spline: Ctrl+LMB adds a knot to end (minimal Unity-style edit hook; full tools continue to be ported) ---
    if is_spline_selected && just_clicked && is_ctrl_held {
        if let Some(node_id) = selected_node_id {
            let ng = &mut node_graph_res.0;
            if let Some((inst_id, internal)) = cda_spline_target {
                let Some(inst) = ng.nodes.get_mut(&inst_id) else {
                    return;
                };
                let NodeType::CDA(data) = &mut inst.node_type else {
                    return;
                };
                let Some(mut c) = base_spline_from_cda(data, internal) else {
                    return;
                };
                if c.splines.is_empty() {
                    c.splines.push(Default::default());
                }
                if let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) {
                    let hit = ray.get_point(dist);
                    let _ = c.add_knot_to_end(0, hit, Vec3::Y, Vec3::ZERO, TangentMode::AutoSmooth);
                    store_spline_to_cda(data, internal, c);
                    ng.mark_dirty(inst_id);
                    graph_changed_writer.write_default();
                    return;
                }
            } else if let Some(node) = ng.nodes.get_mut(&node_id) {
                if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                    if let ParameterValue::UnitySpline(c) = &mut param.value {
                        if c.splines.is_empty() {
                            c.splines.push(Default::default());
                        }
                        let mut best: Option<(usize, usize, f32, f32)> = None; // (spline, curve, t, dist_world)
                        for si in 0..c.splines.len() {
                            let count = c.splines[si].count();
                            if count < 2 {
                                continue;
                            }
                            let curves = if c.splines[si].closed {
                                count
                            } else {
                                count - 1
                            };
                            for ci in 0..curves {
                                let local = c.splines[si].get_curve(ci);
                                let world = BezierCurve {
                                    p0: c.local_to_world.transform_point3(local.p0),
                                    p1: c.local_to_world.transform_point3(local.p1),
                                    p2: c.local_to_world.transform_point3(local.p2),
                                    p3: c.local_to_world.transform_point3(local.p3),
                                };
                                let (pos, t, dist) = get_nearest_point_on_curve_ray(
                                    world,
                                    ray.origin,
                                    *ray.direction,
                                    96,
                                );
                                let dist_cam = camera_transform.translation().distance(pos);
                                let size = (dist_cam * SPLINE_HANDLE_SIZE_FACTOR)
                                    .max(SPLINE_HANDLE_SIZE_MIN);
                                let thresh = (size * 0.2).max(0.03);
                                if dist <= thresh && best.map_or(true, |b| dist < b.3) {
                                    best = Some((si, ci, t, dist));
                                }
                            }
                        }
                        if let Some((si, ci, t, _d)) = best {
                            let count = c.splines[si].count();
                            let next = if c.splines[si].closed {
                                (ci + 1) % count
                            } else {
                                ci + 1
                            };
                            c.splines[si].insert_on_curve(next, t);
                            spline_tool_state.selection.clear();
                            let e = SelectableElement::Knot(SelectableKnot {
                                spline_index: si,
                                knot_index: next,
                            });
                            spline_tool_state.selection.add(e);
                            spline_tool_state.selection.set_active(Some(e));
                            ng.mark_dirty(node_id);
                            graph_changed_writer.write_default();
                            console_log.info("Spline: Insert knot on curve (Ctrl+LMB)");
                            return;
                        }
                        if let Some(dist) =
                            ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y))
                        {
                            let hit = ray.get_point(dist);
                            let _ = c.add_knot_to_end(
                                0,
                                hit,
                                Vec3::Y,
                                Vec3::ZERO,
                                TangentMode::AutoSmooth,
                            );
                            ng.mark_dirty(node_id);
                            graph_changed_writer.write_default();
                            return;
                        }
                    }
                }
            }
        }
    }

    // --- Spline: simple hotkeys (WIP) ---
    if is_spline_selected {
        // Edit / Draw mode (match Curve's F/G).
        if keyboard.just_pressed(KeyCode::KeyF) {
            spline_tool_state.mode = SplineEditMode::Edit;
            spline_tool_state.draw_state = None;
            console_log.info("Spline Mode: Edit (F)");
        }
        if keyboard.just_pressed(KeyCode::KeyG) {
            spline_tool_state.mode = SplineEditMode::Draw;
            spline_tool_state.draw_state = None;
            spline_tool_state.selection.clear();
            console_log.info("Spline Mode: Draw (G)");
        }

        // Keep TransformContext updated from actual selection + spline data (Unity-style pivot/orientation).
        // NOTE: pivot_mode / handle_orientation can be wired to UI later; for now we keep defaults stable.
        if keyboard.just_pressed(KeyCode::KeyP) {
            spline_tool_state.ctx.pivot_mode = match spline_tool_state.ctx.pivot_mode {
                PivotMode::Pivot => PivotMode::Center,
                PivotMode::Center => PivotMode::Pivot,
            };
            console_log.info(if spline_tool_state.ctx.pivot_mode == PivotMode::Pivot {
                "Spline PivotMode: Pivot (P)"
            } else {
                "Spline PivotMode: Center (P)"
            });
        }
        if keyboard.just_pressed(KeyCode::KeyO) {
            spline_tool_state.ctx.handle_orientation =
                match spline_tool_state.ctx.handle_orientation {
                    HandleOrientation::Global => HandleOrientation::Parent,
                    HandleOrientation::Parent => HandleOrientation::Element,
                    HandleOrientation::Element => HandleOrientation::Global,
                };
            console_log.info("Spline HandleOrientation: toggled (O)");
        }

        // Axis constraint only affects Rotate/Scale tools.
        if keyboard.just_pressed(KeyCode::KeyX) {
            spline_tool_state.axis = if spline_tool_state.axis == SplineAxisConstraint::X {
                SplineAxisConstraint::None
            } else {
                SplineAxisConstraint::X
            };
        }
        if keyboard.just_pressed(KeyCode::KeyY) {
            spline_tool_state.axis = if spline_tool_state.axis == SplineAxisConstraint::Y {
                SplineAxisConstraint::None
            } else {
                SplineAxisConstraint::Y
            };
        }
        if keyboard.just_pressed(KeyCode::KeyZ) {
            spline_tool_state.axis = if spline_tool_state.axis == SplineAxisConstraint::Z {
                SplineAxisConstraint::None
            } else {
                SplineAxisConstraint::Z
            };
        }

        // Snapping (Unity-style flags). We only wire the flags + move_snap now; actual move snapping logic is in TransformOperation.
        if keyboard.just_pressed(KeyCode::KeyI) {
            spline_tool_state.ctx.snapping.incremental_snap_active =
                !spline_tool_state.ctx.snapping.incremental_snap_active;
            console_log.info(if spline_tool_state.ctx.snapping.incremental_snap_active {
                "Spline Snap: Incremental ON (I)"
            } else {
                "Spline Snap: Incremental OFF (I)"
            });
        }
        // Coarse presets; later we can read actual editor snap settings from global app config.
        if keyboard.just_pressed(KeyCode::Digit1) {
            spline_tool_state.ctx.move_snap = Vec3::splat(0.1);
            console_log.info("Spline MoveSnap: 0.1 (1)");
        }
        if keyboard.just_pressed(KeyCode::Digit2) {
            spline_tool_state.ctx.move_snap = Vec3::splat(1.0);
            console_log.info("Spline MoveSnap: 1.0 (2)");
        }
        if keyboard.just_pressed(KeyCode::Digit3) {
            spline_tool_state.ctx.move_snap = Vec3::splat(10.0);
            console_log.info("Spline MoveSnap: 10.0 (3)");
        }

        if spline_tool_state.ctx.move_snap == Vec3::ZERO {
            spline_tool_state.ctx.move_snap = Vec3::ONE;
        }
        if let Some(node_id) = selected_node_id {
            if let Some((inst_id, internal)) = cda_spline_target {
                let ng = &node_graph_res.0;
                let c = ng.nodes.get(&inst_id).and_then(|n| {
                    if let NodeType::CDA(data) = &n.node_type {
                        base_spline_from_cda(data, internal)
                    } else {
                        None
                    }
                });
                if let Some(c) = c {
                    spline_tool_state.ctx = update_spline_transform_ctx(
                        spline_tool_state.ctx,
                        &c,
                        &spline_tool_state.selection,
                    );
                }
            } else if let Some(c) = {
                let ng = &node_graph_res.0;
                ng.nodes
                    .get(&node_id)
                    .and_then(|n| n.parameters.iter().find(|p| p.name == "spline"))
                    .and_then(|p| match &p.value {
                        ParameterValue::UnitySpline(c) => Some(c.clone()),
                        _ => None,
                    })
            } {
                spline_tool_state.ctx = update_spline_transform_ctx(
                    spline_tool_state.ctx,
                    &c,
                    &spline_tool_state.selection,
                );
            }
        }
        // Spline now follows the global xform hotkeys (Q/W/E/R).
        // Legacy spline-specific rotate/scale handles are removed; keep spline tool state in Move.
        spline_tool_state.tool = SplineTransformTool::Move;
        if keyboard.just_pressed(KeyCode::KeyQ) {
            console_log.info("Spline Xform: Aggregate (Q)");
        }
        if keyboard.just_pressed(KeyCode::KeyW) {
            spline_tool_state.mode = SplineEditMode::Edit; // W exits Draw
            spline_tool_state.draw_state = None;
            console_log.info("Spline Xform: Move (W)");
        }
        if keyboard.just_pressed(KeyCode::KeyE) {
            console_log.info("Spline Xform: Scale (E)");
        }
        if keyboard.just_pressed(KeyCode::KeyR) {
            console_log.info("Spline Xform: All (R)");
        }

        // Clear incremental xform drag cache on mouse release.
        if !is_pressed {
            spline_tool_state.drag_last_world = None;
            spline_tool_state.drag_last_scalar = None;
        }

        // Legacy spline rotate/scale handle gizmo removed.

        if keyboard.just_pressed(KeyCode::KeyC) {
            let Some(node_id) = selected_node_id else {
                return;
            };
            let ng = &mut node_graph_res.0;
            let mut msg: Option<&'static str> = None;
            {
                if let Some(node) = ng.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            if c.splines.is_empty() {
                                c.splines.push(Default::default());
                            }
                            c.splines[0].closed = !c.splines[0].closed;
                            msg = Some(if c.splines[0].closed {
                                "Spline: Close (C)"
                            } else {
                                "Spline: Open (C)"
                            });
                        }
                    }
                }
            }
            if let Some(m) = msg {
                ng.mark_dirty(node_id);
                graph_changed_writer.write_default();
                console_log.info(m);
                return;
            }
        }
        if keyboard.just_pressed(KeyCode::Backspace) {
            let Some(node_id) = selected_node_id else {
                return;
            };
            let ng = &mut node_graph_res.0;
            let mut changed = false;
            {
                if let Some(node) = ng.nodes.get_mut(&node_id) {
                    if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline") {
                        if let ParameterValue::UnitySpline(c) = &mut param.value {
                            if c.splines.is_empty() {
                                return;
                            }
                            let count = c.splines[0].count();
                            if count > 0 {
                                c.splines[0].remove_at(count - 1);
                                changed = true;
                            }
                        }
                    }
                }
            }
            if changed {
                ng.mark_dirty(node_id);
                graph_changed_writer.write_default();
                console_log.info("Spline: Remove last knot (Backspace)");
                return;
            }
        }
    }

    // --- Gizmo Dragging (High Priority in both modes) ---
    let mut dragging_gizmo = false;
    for (entity, transform, mut interaction, binding) in q_gizmos.iter_mut() {
        if interaction.is_dragged {
            // Reset drag if released (and emit deferred plugin click if it was a tap).
            if just_released {
                if let (Some(ps), Some(node_id), Some(key)) = (
                    plugin_system.as_ref(),
                    selected_node_id,
                    plugin_node_key.as_ref(),
                ) {
                    for (_e, tf, inter, b) in q_gizmos.iter() {
                        if !inter.is_dragged {
                            continue;
                        }
                        if let GizmoBinding::PluginPick {
                            node_id: nid,
                            pick_id,
                        } = b
                        {
                            if *nid != node_id {
                                continue;
                            }
                            let drag_px = inter
                                .drag_start_mouse
                                .map(|s| (cursor_pos_window - s).length())
                                .unwrap_or(9999.0);
                            if drag_px <= 3.0 {
                                let _ = ps.plugin_gizmo_event_click(
                                    &node_graph_res,
                                    key,
                                    node_id,
                                    *pick_id,
                                    tf.translation,
                                );
                            }
                            let _ = ps.plugin_gizmo_event_release(
                                &node_graph_res,
                                key,
                                node_id,
                                *pick_id,
                                tf.translation,
                            );
                        }
                    }
                }
                for (_, _, mut interaction, _) in q_gizmos.iter_mut() {
                    interaction.is_dragged = false;
                    interaction.drag_start_ray_origin = None;
                    interaction.initial_position = None;
                    interaction.drag_start_mouse = None;
                    interaction.last_mouse = None;
                }
                control_id.hot_entity = None;
                ui_state.dragged_node_id = None;
                return;
            }

            dragging_gizmo = true;

            let (plane_origin, plane_normal) = {
                let o = interaction.initial_position.unwrap_or(Vec3::ZERO);
                let n0 = match binding {
                    GizmoBinding::PluginPick { .. } => {
                        interaction.drag_start_ray_dir.unwrap_or(*ray.direction)
                    }
                    _ => Vec3::Y,
                };
                let n = n0.normalize_or_zero();
                (
                    o,
                    if n.length_squared() <= 1.0e-8 {
                        Vec3::Y
                    } else {
                        n
                    },
                )
            };

            if let Some(dist) =
                ray.intersect_plane(plane_origin, InfinitePlane3d::new(plane_normal))
            {
                let new_pos = ray.get_point(dist);
                interaction.last_mouse = Some(cursor_pos_window);
                // For spline, only allow direct dragging in Move tool. Rotate/Scale uses transform handles.
                let is_spline_binding = is_spline_selected
                    && matches!(
                        binding,
                        GizmoBinding::SplineKnot { .. } | GizmoBinding::SplineTangent { .. }
                    );
                if !(is_spline_binding && spline_tool_state.tool != SplineTransformTool::Move) {
                    gizmo_moved_writer.write(GizmoMovedEvent {
                        binding: binding.clone(),
                        new_position: new_pos,
                    });
                    // For spline knots/tangents we don't move the gizmo entity directly; spline data drives gizmo pose.
                    // This keeps Auto tangents + knot direction updating live during drag (Unity-like).
                    if !is_spline_binding {
                        // Preserve scale/rotation; only update translation (avoids "gizmo grows when held").
                        let mut t = *transform;
                        t.translation = new_pos;
                        commands.entity(entity).insert(t);
                    }
                }
            }
        }
    }

    if dragging_gizmo {
        // Ensure we don't drag generic gizmo and create points same time
        ui_state.dragged_node_id = None;
        return;
    }

    // --- Plugin: click on empty space (pick_id=0) ---
    if let (Some(ps), Some(node_id), Some(key)) = (
        plugin_system.as_ref(),
        selected_node_id,
        plugin_node_key.as_ref(),
    ) {
        if just_clicked && control_id.nearest_entity.is_none() {
            let hit =
                if let Some(t) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) {
                    ray.get_point(t)
                } else {
                    // Fallback: when the view ray is (near) parallel to world Y plane, pick a stable point along the ray.
                    let t0 = (Vec3::ZERO - ray.origin).dot(*ray.direction);
                    let t = if t0.is_finite() && t0 > 0.1 { t0 } else { 5.0 };
                    ray.get_point(t)
                };
            let ok = ps.plugin_gizmo_event_click(&node_graph_res, key, node_id, 0, hit);
            if log_curve_plugin && key == "Curve" {
                bevy::log::info!(
                    "[CurvePlugin] ClickCanvas pick_id=0 hit={:?} -> ok={}",
                    hit,
                    ok
                );
            }
            node_graph_res.0.mark_dirty(node_id);
            graph_changed_writer.write_default();
            return;
        }
    }

    // --- Gizmo Hover/Pick ---
    // IMPORTANT: this must NOT be globally gated by Curve draw state; otherwise spline knots become unselectable.
    control_id.begin_frame();
    let sphere_radius = 0.2;
    for (entity, transform, _interaction, binding) in q_gizmos.iter() {
        let mut r = sphere_radius;
        if is_spline_selected
            && matches!(
                *binding,
                GizmoBinding::SplineKnot { .. } | GizmoBinding::SplineTangent { .. }
            )
        {
            let dist_cam = camera_transform
                .translation()
                .distance(transform.translation);
            let size = (dist_cam * SPLINE_HANDLE_SIZE_FACTOR).max(SPLINE_HANDLE_SIZE_MIN);
            r = (size * 0.18).max(0.03);
        }
        // Robust hit test: use ray->point closest distance (world-space), then order by ray projection.
        // This is less brittle than analytic ray-sphere and matches the "distance to geometry" style used elsewhere.
        let p = transform.translation;
        let d = p - ray.origin;
        let proj = d.dot(*ray.direction);
        if proj > 0.0 {
            let closest = ray.origin + *ray.direction * proj;
            let dist_sq = (closest - p).length_squared();
            if dist_sq <= r * r {
                control_id.consider(entity, dist_sq, proj);
            }
        }
    }
    if log_spline_pick && is_spline_selected && just_clicked {
        let mut spline_total = 0usize;
        let mut sample: Option<(Vec3, f32, f32, GizmoBinding)> = None; // (pos, proj, r, binding)
        for (_e, tf, _i, b) in q_gizmos.iter() {
            let is_spline = matches!(
                *b,
                GizmoBinding::SplineKnot { .. } | GizmoBinding::SplineTangent { .. }
            );
            if !is_spline {
                continue;
            }
            spline_total += 1;
            if sample.is_none() {
                let p = tf.translation;
                let dist_cam = camera_transform.translation().distance(p);
                let size = (dist_cam * SPLINE_HANDLE_SIZE_FACTOR).max(SPLINE_HANDLE_SIZE_MIN);
                let r = (size * 0.18).max(0.03);
                let d = p - ray.origin;
                let proj = d.dot(*ray.direction);
                sample = Some((p, proj, r, b.clone()));
            }
        }
        if let Some((p, proj, r, b)) = sample {
            let closest = ray.origin + *ray.direction * proj;
            let dist = (closest - p).length();
            bevy::log::info!(
                "[SplinePick] ray_o={:?} ray_d={:?} spline_total={} sample_pos={:?} proj={:.4} dist={:.5} r={:.5} binding={:?}",
                ray.origin, *ray.direction, spline_total, p, proj, dist, r, b
            );
        } else {
            bevy::log::info!("[SplinePick] spline_total=0 (no spline gizmo entities in q_gizmos)");
        }
    }
    if log_spline_pick && is_spline_selected && just_clicked {
        if let Some(e) = control_id.nearest_entity {
            if let Ok((_e, tf, _i, b)) = q_gizmos.get(e) {
                bevy::log::info!("[SplinePick] nearest_entity={:?} dist_sq={:.6} proj={:.4} pos={:?} binding={:?}", e, control_id.nearest_dist_sq, control_id.nearest_proj, tf.translation, b);
            }
        } else {
            bevy::log::info!("[SplinePick] nearest_entity=None");
        }
    }
    for (entity, _t, mut interaction, _binding) in q_gizmos.iter_mut() {
        interaction.is_hovered = control_id.nearest_entity == Some(entity);
    }

    // --- Spline: direct knot pick/drag fallback (bypasses gizmo entities) ---
    // If spline gizmo entities aren't pickable (ordering/transform issues), we still allow selecting/dragging knots.
    if is_spline_selected {
        // End direct drag on mouse release.
        if just_released {
            spline_tool_state.direct_drag_knot = None;
        }

        // Apply direct drag (Move tool only).
        if is_pressed && matches!(spline_tool_state.tool, SplineTransformTool::Move) {
            if let Some(drag) = spline_tool_state.direct_drag_knot {
                if let Some(dist) =
                    ray.intersect_plane(drag.plane_origin_world, InfinitePlane3d::new(Vec3::Y))
                {
                    let new_pos = ray.get_point(dist);
                    gizmo_moved_writer.write(GizmoMovedEvent {
                        binding: GizmoBinding::SplineKnot {
                            node_id: drag.node_id,
                            spline_index: drag.spline_index,
                            knot_index: drag.knot_index,
                        },
                        new_position: new_pos,
                    });
                    return;
                }
            }
        }

        // Draw mode: click on empty space to add a knot (Unity-like: can create spline or continue a drawing operation).
        if spline_tool_state.mode == SplineEditMode::Draw
            && just_clicked
            && control_id.nearest_entity.is_none()
        {
            if let Some(node_id) = selected_node_id {
                if spline_tool_state
                    .draw_state
                    .as_ref()
                    .map_or(false, |d| d.node_id != node_id)
                {
                    spline_tool_state.draw_state = None;
                }
                if let Some(dist) = ray.intersect_plane(Vec3::ZERO, InfinitePlane3d::new(Vec3::Y)) {
                    let hit = ray.get_point(dist);
                    let ng = &mut node_graph_res.0;
                    if let Some((inst_id, internal)) = cda_spline_target {
                        let Some(inst) = ng.nodes.get_mut(&inst_id) else {
                            return;
                        };
                        let NodeType::CDA(data) = &mut inst.node_type else {
                            return;
                        };
                        let Some(mut c) = base_spline_from_cda(data, internal) else {
                            return;
                        };
                        if c.splines.is_empty() {
                            c.splines.push(Default::default());
                        }
                        let (si, dir, allow_delete) = if let Some(d) = spline_tool_state.draw_state
                        {
                            (d.spline_index, d.dir, d.allow_delete_if_no_curves)
                        } else {
                            let si = if c.splines.len() == 1 && c.splines[0].count() == 0 {
                                0
                            } else {
                                c.splines.len()
                            };
                            if si == c.splines.len() {
                                c.splines.push(Default::default());
                            }
                            spline_tool_state.draw_state = Some(SplineDrawState {
                                node_id,
                                spline_index: si,
                                dir: DrawingDirection::End,
                                allow_delete_if_no_curves: true,
                            });
                            (si, DrawingDirection::End, true)
                        };
                        let _ = c.create_knot_on_surface(si, dir, hit, Vec3::Y, Vec3::ZERO);
                        let ki = if dir == DrawingDirection::End {
                            c.splines[si].count().saturating_sub(1)
                        } else {
                            0
                        };
                        spline_tool_state.selection.clear();
                        let e = SelectableElement::Knot(SelectableKnot {
                            spline_index: si,
                            knot_index: ki,
                        });
                        spline_tool_state.selection.add(e);
                        spline_tool_state.selection.set_active(Some(e));
                        spline_tool_state.draw_state = Some(SplineDrawState {
                            node_id,
                            spline_index: si,
                            dir,
                            allow_delete_if_no_curves: allow_delete,
                        });
                        store_spline_to_cda(data, internal, c);
                        ng.mark_dirty(inst_id);
                        graph_changed_writer.write_default();
                        return;
                    } else if let Some(node) = ng.nodes.get_mut(&node_id) {
                        if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline")
                        {
                            if let ParameterValue::UnitySpline(c) = &mut param.value {
                                if c.splines.is_empty() {
                                    c.splines.push(Default::default());
                                }
                                let (si, dir, allow_delete) = if let Some(d) =
                                    spline_tool_state.draw_state
                                {
                                    (d.spline_index, d.dir, d.allow_delete_if_no_curves)
                                } else {
                                    let si = if c.splines.len() == 1 && c.splines[0].count() == 0 {
                                        0
                                    } else {
                                        c.splines.len()
                                    };
                                    if si == c.splines.len() {
                                        c.splines.push(Default::default());
                                    }
                                    spline_tool_state.draw_state = Some(SplineDrawState {
                                        node_id,
                                        spline_index: si,
                                        dir: DrawingDirection::End,
                                        allow_delete_if_no_curves: true,
                                    });
                                    (si, DrawingDirection::End, true)
                                };
                                let _ = c.create_knot_on_surface(si, dir, hit, Vec3::Y, Vec3::ZERO);
                                let ki = if dir == DrawingDirection::End {
                                    c.splines[si].count().saturating_sub(1)
                                } else {
                                    0
                                };
                                spline_tool_state.selection.clear();
                                let e = SelectableElement::Knot(SelectableKnot {
                                    spline_index: si,
                                    knot_index: ki,
                                });
                                spline_tool_state.selection.add(e);
                                spline_tool_state.selection.set_active(Some(e));
                                spline_tool_state.draw_state = Some(SplineDrawState {
                                    node_id,
                                    spline_index: si,
                                    dir,
                                    allow_delete_if_no_curves: allow_delete,
                                });
                                ng.mark_dirty(node_id);
                                graph_changed_writer.write_default();
                                return;
                            }
                        }
                    }
                }
            }
        }

        // Start direct pick when click misses all gizmo entities (Edit mode).
        if spline_tool_state.mode == SplineEditMode::Edit
            && just_clicked
            && control_id.nearest_entity.is_none()
            && spline_tool_state.direct_drag_knot.is_none()
        {
            if let Some(node_id) = selected_node_id {
                let c = {
                    let ng = &node_graph_res.0;
                    ng.nodes
                        .get(&node_id)
                        .and_then(|n| n.parameters.iter().find(|p| p.name == "spline"))
                        .and_then(|p| match &p.value {
                            ParameterValue::UnitySpline(c) => Some(c.clone()),
                            _ => None,
                        })
                };
                if let Some(c) = c {
                    let mut best: Option<(usize, usize, f32, Vec3)> = None; // (si, ki, dist, world_pos)
                    for si in 0..c.splines.len() {
                        for ki in 0..c.splines[si].count() {
                            let wp = c
                                .local_to_world
                                .transform_point3(c.splines[si].knots[ki].position);
                            let d = wp - ray.origin;
                            let proj = d.dot(*ray.direction);
                            if proj <= 0.0 {
                                continue;
                            }
                            let closest = ray.origin + *ray.direction * proj;
                            let dist = (closest - wp).length();
                            if best.as_ref().map_or(true, |b| dist < b.2) {
                                best = Some((si, ki, dist, wp));
                            }
                        }
                    }
                    if let Some((si, ki, dist, wp)) = best {
                        let dist_cam = camera_transform.translation().distance(wp);
                        let size =
                            (dist_cam * SPLINE_HANDLE_SIZE_FACTOR).max(SPLINE_HANDLE_SIZE_MIN);
                        let thresh = (size * 0.25).max(0.06);
                        if dist <= thresh {
                            let e = SelectableElement::Knot(SelectableKnot {
                                spline_index: si,
                                knot_index: ki,
                            });
                            apply_spline_select(
                                &mut spline_tool_state.selection,
                                e,
                                is_shift_held,
                                is_ctrl_held,
                            );
                            spline_tool_state.direct_drag_knot = Some(DirectDragKnot {
                                node_id,
                                spline_index: si,
                                knot_index: ki,
                                plane_origin_world: wp,
                            });
                            return;
                        }
                    }
                }
            }
        }
    }

    if let Some(entity) = control_id.nearest_entity {
        if let Ok((_, transform, mut interaction, binding)) = q_gizmos.get_mut(entity) {
            // --- Spline: Draw mode click on knot (Unity-like append/prepend/branch + link) ---
            if is_spline_selected && spline_tool_state.mode == SplineEditMode::Draw && just_clicked
            {
                if let GizmoBinding::SplineKnot {
                    node_id,
                    spline_index,
                    knot_index,
                } = *binding
                {
                    let ng = &mut node_graph_res.0;
                    if let Some(node) = ng.nodes.get_mut(&node_id) {
                        if let Some(param) = node.parameters.iter_mut().find(|p| p.name == "spline")
                        {
                            if let ParameterValue::UnitySpline(c) = &mut param.value {
                                if spline_index >= c.splines.len()
                                    || knot_index >= c.splines[spline_index].count()
                                {
                                    return;
                                }

                                // If we're already drawing on this node, continue that operation.
                                if spline_tool_state
                                    .draw_state
                                    .as_ref()
                                    .map_or(false, |d| d.node_id != node_id)
                                {
                                    spline_tool_state.draw_state = None;
                                }

                                let _clicked = SelectableKnot {
                                    spline_index,
                                    knot_index,
                                };
                                let links = c.links.get_knot_links(SplineKnotIndex::new(
                                    spline_index as i32,
                                    knot_index as i32,
                                ));
                                let is_unlinked = links.len() == 1;
                                let is_closed = c.splines[spline_index].closed;
                                let count = c.splines[spline_index].count();
                                let is_end = knot_index + 1 == count;
                                let is_start = knot_index == 0;

                                // Unity: if startFrom is an extremity and not linked and not closed -> draw on same spline.
                                // Otherwise: create a new spline and link (branch).
                                let (start_spline, dir, allow_delete_if_no_curves) =
                                    if is_unlinked && !is_closed && (is_end || is_start) {
                                        (
                                            spline_index,
                                            if is_end {
                                                DrawingDirection::End
                                            } else {
                                                DrawingDirection::Start
                                            },
                                            false,
                                        )
                                    } else {
                                        // Branch: create a new spline with one knot at clicked, then link.
                                        let new_si = c.splines.len();
                                        c.splines.push(Default::default());
                                        let k = c.splines[spline_index].knots[knot_index];
                                        let parent_rot =
                                            c.local_to_world.to_scale_rotation_translation().1;
                                        let knot_rot_world = parent_rot * k.rotation;
                                        let normal_world = knot_rot_world.mul_vec3(Vec3::Y);
                                        let pos_world =
                                            c.local_to_world.transform_point3(k.position);
                                        let _ = c.add_knot_to_end(
                                            new_si,
                                            pos_world,
                                            normal_world,
                                            Vec3::ZERO,
                                            TangentMode::AutoSmooth,
                                        );
                                        c.link_knots(
                                            SplineKnotIndex::new(
                                                spline_index as i32,
                                                knot_index as i32,
                                            ),
                                            SplineKnotIndex::new(new_si as i32, 0),
                                        );
                                        c.set_linked_knot_position(SplineKnotIndex::new(
                                            new_si as i32,
                                            0,
                                        ));
                                        (new_si, DrawingDirection::End, true)
                                    };

                                spline_tool_state.draw_state = Some(SplineDrawState {
                                    node_id,
                                    spline_index: start_spline,
                                    dir,
                                    allow_delete_if_no_curves,
                                });
                                spline_tool_state.selection.clear();
                                let e = SelectableElement::Knot(SelectableKnot {
                                    spline_index: start_spline,
                                    knot_index: if dir == DrawingDirection::End {
                                        c.splines[start_spline].count().saturating_sub(1)
                                    } else {
                                        0
                                    },
                                });
                                spline_tool_state.selection.add(e);
                                spline_tool_state.selection.set_active(Some(e));
                                ng.mark_dirty(node_id);
                                graph_changed_writer.write_default();
                                return;
                            }
                        }
                    }
                }
            }

            // --- Spline: while drawing, clicking a knot continues the drawing op (create_knot_on_knot) ---
            if is_spline_selected && spline_tool_state.mode == SplineEditMode::Draw && just_clicked
            {
                if let GizmoBinding::SplineKnot {
                    node_id,
                    spline_index: clicked_si,
                    knot_index: clicked_ki,
                } = *binding
                {
                    if let Some(d) = spline_tool_state.draw_state {
                        if d.node_id == node_id {
                            let ng = &mut node_graph_res.0;
                            if let Some(node) = ng.nodes.get_mut(&node_id) {
                                if let Some(param) =
                                    node.parameters.iter_mut().find(|p| p.name == "spline")
                                {
                                    if let ParameterValue::UnitySpline(c) = &mut param.value {
                                        c.create_knot_on_knot(
                                            d.spline_index,
                                            d.dir,
                                            SelectableKnot {
                                                spline_index: clicked_si,
                                                knot_index: clicked_ki,
                                            },
                                            Vec3::ZERO,
                                        );
                                        let ki = if d.dir == DrawingDirection::End {
                                            c.splines[d.spline_index].count().saturating_sub(1)
                                        } else {
                                            0
                                        };
                                        spline_tool_state.selection.clear();
                                        let e = SelectableElement::Knot(SelectableKnot {
                                            spline_index: d.spline_index,
                                            knot_index: ki,
                                        });
                                        spline_tool_state.selection.add(e);
                                        spline_tool_state.selection.set_active(Some(e));
                                        ng.mark_dirty(node_id);
                                        graph_changed_writer.write_default();
                                        return;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if just_clicked {
                if is_spline_selected {
                    console_log.info(format!(
                        "SplineClick: hit={:?} binding={:?}",
                        entity, binding
                    ));
                }
                // Spline selection must be updated here because we early-return on gizmo hit.
                // Otherwise knots/tangents appear "unclickable" even though they are rendered.
                if is_spline_selected {
                    let mut elem: Option<SelectableElement> = None;
                    match *binding {
                        GizmoBinding::SplineKnot {
                            spline_index,
                            knot_index,
                            ..
                        } => {
                            elem = Some(SelectableElement::Knot(SelectableKnot {
                                spline_index,
                                knot_index,
                            }));
                        }
                        GizmoBinding::SplineTangent {
                            spline_index,
                            knot_index,
                            tangent,
                            ..
                        } => {
                            elem = Some(SelectableElement::Tangent(SelectableTangent {
                                spline_index,
                                knot_index,
                                tangent,
                            }));
                        }
                        _ => {}
                    }
                    if let Some(e) = elem {
                        apply_spline_select(
                            &mut spline_tool_state.selection,
                            e,
                            is_shift_held,
                            is_ctrl_held,
                        );
                    }
                }

                // Start drag-candidate on mouse-down; plugin "click" is deferred to mouse-up if drag distance is tiny.
                // This avoids click-vs-drag conflicts (e.g. Draw mode: press+hold should drag immediately).
                let is_spline_binding = matches!(
                    binding,
                    GizmoBinding::SplineKnot { .. } | GizmoBinding::SplineTangent { .. }
                );
                if !(is_spline_selected
                    && is_spline_binding
                    && spline_tool_state.tool != SplineTransformTool::Move)
                {
                    interaction.is_dragged = true;
                    control_id.hot_entity = Some(entity);
                    interaction.drag_start_ray_origin = Some(ray.origin);
                    interaction.drag_start_ray_dir = Some(*ray.direction);
                    interaction.initial_position = Some(transform.translation);
                    interaction.drag_start_mouse = Some(cursor_pos_window);
                    interaction.last_mouse = Some(cursor_pos_window);
                }
            }
        }
        return; // Hit gizmo
    }

    // --- Spline: pick knot gizmo to update selection (knot-only for now) ---
    if is_spline_selected && just_clicked {
        let mut hit: Option<(SelectableElement, bool)> = None; // (element, is_dragged/hovered)
        for (_entity, _t, interaction, binding) in q_gizmos.iter() {
            if !interaction.is_hovered && !interaction.is_dragged {
                continue;
            }
            match *binding {
                GizmoBinding::SplineKnot {
                    spline_index,
                    knot_index,
                    ..
                } => {
                    hit = Some((
                        SelectableElement::Knot(SelectableKnot {
                            spline_index,
                            knot_index,
                        }),
                        interaction.is_dragged,
                    ));
                    break;
                }
                GizmoBinding::SplineTangent {
                    spline_index,
                    knot_index,
                    tangent,
                    ..
                } => {
                    hit = Some((
                        SelectableElement::Tangent(SelectableTangent {
                            spline_index,
                            knot_index,
                            tangent,
                        }),
                        interaction.is_dragged,
                    ));
                    break;
                }
                _ => {}
            }
        }
        if let Some((elem, _)) = hit {
            apply_spline_select(
                &mut spline_tool_state.selection,
                elem,
                is_shift_held,
                is_ctrl_held,
            );
        }
    }

    // --- Spline: curve hover (ray distance; no screen-space) ---
    if is_spline_selected {
        spline_tool_state.hovered_curve = None;
        if control_id.nearest_entity.is_none() {
            if let Some(node_id) = selected_node_id {
                let c = {
                    let ng = &node_graph_res.0;
                    ng.nodes
                        .get(&node_id)
                        .and_then(|n| n.parameters.iter().find(|p| p.name == "spline"))
                        .and_then(|p| match &p.value {
                            ParameterValue::UnitySpline(c) => Some(c.clone()),
                            _ => None,
                        })
                };
                if let Some(c) = c {
                    let mut best: Option<HoveredCurve> = None;
                    for si in 0..c.splines.len() {
                        let count = c.splines[si].count();
                        if count < 2 {
                            continue;
                        }
                        let curves = if c.splines[si].closed {
                            count
                        } else {
                            count - 1
                        };
                        for ci in 0..curves {
                            let local = c.splines[si].get_curve(ci);
                            let world = BezierCurve {
                                p0: c.local_to_world.transform_point3(local.p0),
                                p1: c.local_to_world.transform_point3(local.p1),
                                p2: c.local_to_world.transform_point3(local.p2),
                                p3: c.local_to_world.transform_point3(local.p3),
                            };
                            let (pos, t, dist) = get_nearest_point_on_curve_ray(
                                world,
                                ray.origin,
                                *ray.direction,
                                96,
                            );
                            let dist_cam = camera_transform.translation().distance(pos);
                            let size =
                                (dist_cam * SPLINE_HANDLE_SIZE_FACTOR).max(SPLINE_HANDLE_SIZE_MIN);
                            let thresh = (size * 0.2).max(0.03);
                            if dist <= thresh && best.as_ref().map_or(true, |b| dist < b.dist) {
                                best = Some(HoveredCurve {
                                    spline_index: si,
                                    curve_index: ci,
                                    t,
                                    world_pos: pos,
                                    dist,
                                });
                            }
                        }
                    }
                    spline_tool_state.hovered_curve = best;
                }
            }
        }
    }

    // NOTE: Old built-in Curve edit logic removed (Curve is plugin-only).
}
