use crate::cunning_core::registries::node_registry::NodeRegistry;
use crate::cunning_core::scripting::ScriptNodeState;
use crate::gizmos::GizmoActionQueue;
use crate::nodes::NodeGraphResource;
use crate::tabs_system::{FloatingEditorTabs, TabViewer, Viewport3DTab};
use crate::ui::FloatingTabRegistry;
use crate::{
    camera::ViewportInteractionState,
    cunning_core::traits::node_interface::{
        GizmoContext, GizmoDrawBuffer, GizmoState, ServiceProvider,
    },
    ui::UiState,
    GraphChanged,
};
use bevy::gizmos::config::{DefaultGizmoConfigGroup, GizmoConfigStore};
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use bevy_ecs::system::SystemParam;
use bevy_egui::egui;

#[derive(SystemParam)]
pub struct GizmoSystemParams<'w, 's> {
    buffer: Option<ResMut<'w, GizmoDrawBuffer>>,
    node_graph_res: Option<ResMut<'w, NodeGraphResource>>,
    node_registry: Option<Res<'w, NodeRegistry>>,
    tab_viewer: Option<Res<'w, TabViewer>>,
    floating_tabs: Option<Res<'w, FloatingEditorTabs>>,
    floating_registry: Option<Res<'w, FloatingTabRegistry>>,
    nav_input: Option<Res<'w, crate::input::NavigationInput>>,
    ui_state: Option<Res<'w, UiState>>,
    script_node_state: Option<Res<'w, ScriptNodeState>>,
    gizmo_action_queue: Option<Res<'w, GizmoActionQueue>>,
    gizmo_state: Option<ResMut<'w, GizmoState>>,
    interaction_state: Option<ResMut<'w, ViewportInteractionState>>,
    mouse_button: Option<Res<'w, ButtonInput<MouseButton>>>,
    primary_window_query: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    windows_query: Query<'w, 's, (Entity, &'static Window)>,
    camera_query: Query<
        'w,
        's,
        (
            &'static Camera,
            &'static GlobalTransform,
            &'static Projection,
        ),
        With<crate::MainCamera>,
    >,
    graph_events: MessageWriter<'w, GraphChanged>,
}

pub fn draw_interactive_gizmos_system(mut p: GizmoSystemParams<'_, '_>) {
    let (
        buffer,
        node_graph_res,
        node_registry,
        tab_viewer,
        floating_tabs,
        floating_registry,
        nav_input,
        ui_state,
        script_node_state,
        gizmo_action_queue,
        gizmo_state,
        interaction_state,
        mouse_button,
    ) = match (
        p.buffer.as_mut(),
        p.node_graph_res.as_mut(),
        p.node_registry.as_ref(),
        p.tab_viewer.as_ref(),
        p.floating_tabs.as_ref(),
        p.floating_registry.as_ref(),
        p.nav_input.as_ref(),
        p.ui_state.as_ref(),
        p.script_node_state.as_ref(),
        p.gizmo_action_queue.as_ref(),
        p.gizmo_state.as_mut(),
        p.interaction_state.as_mut(),
        p.mouse_button.as_ref(),
    ) {
        (
            Some(buffer),
            Some(node_graph_res),
            Some(node_registry),
            Some(tab_viewer),
            Some(floating_tabs),
            Some(floating_registry),
            Some(nav_input),
            Some(ui_state),
            Some(script_node_state),
            Some(gizmo_action_queue),
            Some(gizmo_state),
            Some(interaction_state),
            Some(mouse_button),
        ) => (
            buffer,
            node_graph_res,
            node_registry,
            tab_viewer,
            floating_tabs,
            floating_registry,
            nav_input,
            ui_state,
            script_node_state,
            gizmo_action_queue,
            gizmo_state,
            interaction_state,
            mouse_button,
        ),
        _ => return,
    };
    // Determine where the active 3D viewport lives (main dock vs floating window),
    // and obtain its viewport rect + gizmo visibility flag.
    enum ViewportHost {
        Primary,
        Floating(Entity),
    }

    let (host, viewport_rect, show_gizmos) = {
        // 1. Try to find a Viewport3DTab in the main DockState.
        if let Some((rect, show)) =
            tab_viewer
                .dock_state
                .iter_all_tabs()
                .find_map(|((_s, _n), tab)| {
                    let tab = tab.as_any().downcast_ref::<Viewport3DTab>()?;
                    Some((tab.viewport_rect?, tab.show_gizmos))
                })
        {
            (ViewportHost::Primary, rect, show)
        } else {
            // 2. Fallback: look for a floating Viewport3DTab instance.
            let mut found: Option<(ViewportHost, egui::Rect, bool)> = None;

            for (id, tab) in floating_tabs.tabs.iter() {
                if let Some(vp) = tab.as_any().downcast_ref::<Viewport3DTab>() {
                    if let Some(rect) = vp.viewport_rect {
                        // Find which native window hosts this floating tab.
                        if let Some((window_entity, _entry)) = floating_registry
                            .floating_windows
                            .iter()
                            .find(|(_, entry)| &entry.id == id)
                        {
                            found = Some((
                                ViewportHost::Floating(*window_entity),
                                rect,
                                vp.show_gizmos,
                            ));
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

    if !show_gizmos {
        return;
    }

    // Do not hide gizmos during viewport navigation (Orbit, Pan, Zoom).
    // Instead, we disable interaction below.
    let is_navigating = nav_input.active;

    let Ok((camera, camera_transform, projection)) = p.camera_query.single() else {
        return;
    };

    // Select the correct Bevy Window depending on where the viewport is rendered.
    let window: &Window = match host {
        ViewportHost::Primary => match p.primary_window_query.single() {
            Ok(w) => w,
            Err(_) => return,
        },
        ViewportHost::Floating(window_entity) => match p.windows_query.get(window_entity) {
            Ok((_e, w)) => w,
            Err(_) => return,
        },
    };

    // If cursor is outside window, use a dummy position to allow drawing but prevent interaction
    let cursor_pos = window
        .cursor_position()
        .unwrap_or(Vec2::new(-99999.0, -99999.0));

    // FIX: Gizmo Interaction Accuracy
    // - `cursor_pos` is in Window-Space logical coordinates (top-left origin).
    // - `Camera::logical_viewport_rect` returns the camera's actual viewport rect in the
    //   same logical space, after DPI scaling & clamping.
    // - For ray casting we must convert the window-space cursor into *viewport-local*
    //   coordinates by subtracting the viewport's logical min.
    // - For 2D gizmo/HUD logic we still want coordinates relative to the egui viewport rect.
    let ui_viewport_min = Vec2::new(viewport_rect.min.x, viewport_rect.min.y);
    let ui_cursor_viewport_pos = cursor_pos - ui_viewport_min;

    // Bevy 0.18: `viewport_to_world` expects WINDOW coordinates (same as `Window::cursor_position()`),
    // and will internally account for `camera.viewport` if set.
    // If we subtract the viewport min here, the viewport offset is applied twice and picking breaks.
    let cursor_in_view = camera
        .logical_viewport_rect()
        .map(|r| r.contains(cursor_pos))
        .unwrap_or(true);
    let Ok(ray) = camera.viewport_to_world(camera_transform, cursor_pos) else {
        return;
    };

    // Determine Projection State
    let (is_ortho, ortho_scale) = match projection {
        Projection::Orthographic(ortho) => (true, ortho.scale),
        Projection::Perspective(_) => (false, 1.0),
        _ => (false, 1.0),
    };

    let (_, cam_rot, _) = camera_transform.to_scale_rotation_translation();

    // Prepare Context
    let context = GizmoContext {
        ray_origin: ray.origin,
        ray_direction: if is_navigating || !cursor_in_view {
            Vec3::ZERO
        } else {
            *ray.direction
        },
        mouse_left_pressed: if is_navigating || !cursor_in_view {
            false
        } else {
            mouse_button.pressed(MouseButton::Left)
        },
        mouse_left_just_pressed: if is_navigating || !cursor_in_view {
            false
        } else {
            mouse_button.just_pressed(MouseButton::Left)
        },
        mouse_left_just_released: if is_navigating || !cursor_in_view {
            false
        } else {
            mouse_button.just_released(MouseButton::Left)
        },
        cursor_pos: ui_cursor_viewport_pos,
        cam_pos: camera_transform.translation(),
        cam_up: *camera_transform.up(),
        cam_rotation: cam_rot,
        is_orthographic: is_ortho,
        scale_factor: ortho_scale,
    };

    // 2. Dispatch to Nodes
    let registry = node_registry.nodes.read().unwrap();
    let mut interactions = Vec::new();
    let mut nodes_to_mark_dirty = Vec::new(); // Collect dirty nodes

    {
        let node_graph = &node_graph_res.0;
        for node_id in &ui_state.selected_nodes {
            if let Some(node) = node_graph.nodes.get(node_id) {
                let node_type_name = node.node_type.name();
                if let Some(descriptor) = registry.get(node_type_name) {
                    if let Some(factory) = &descriptor.interaction_factory {
                        interactions.push((*node_id, factory.clone()));
                    }
                }
            }
        }
    }

    // For each selected node, draw its gizmo
    {
        // Prepare Service Provider (scope-limited so we can mutably touch graph later)
        let services = DispatchServiceProvider {
            node_graph_res: &**node_graph_res,
            script_node_state: &**script_node_state,
            gizmo_action_queue: &**gizmo_action_queue,
        };

        for (node_id, factory) in interactions {
            let interaction = factory();
            let mut temp_gizmo_state = std::mem::take(&mut **gizmo_state); // Take state to pass down

            interaction.draw_gizmos(buffer, &context, &mut temp_gizmo_state, &services, node_id);

            if temp_gizmo_state.graph_modified {
                nodes_to_mark_dirty.push(node_id);
            }

            **gizmo_state = temp_gizmo_state; // Restore state
        }
    }

    if !nodes_to_mark_dirty.is_empty() {
        for node_id in nodes_to_mark_dirty {
            node_graph_res.0.mark_dirty(node_id);
        }
        gizmo_state.graph_modified = true;
    }

    if gizmo_state.graph_modified {
        gizmo_state.graph_modified = false;
        p.graph_events.write(GraphChanged);
    }

    // Update global interaction state to block camera movement if gizmo is active
    interaction_state.is_gizmo_dragging = gizmo_state.active_node_id.is_some();
}

// Helper struct for ServiceProvider
struct DispatchServiceProvider<'a> {
    node_graph_res: &'a NodeGraphResource,
    script_node_state: &'a ScriptNodeState,
    gizmo_action_queue: &'a GizmoActionQueue,
}

impl<'a> ServiceProvider for DispatchServiceProvider<'a> {
    fn get_service(&self, service_type: std::any::TypeId) -> Option<&dyn std::any::Any> {
        if service_type == std::any::TypeId::of::<NodeGraphResource>() {
            return Some(self.node_graph_res);
        }
        if service_type == std::any::TypeId::of::<ScriptNodeState>() {
            return Some(self.script_node_state);
        }
        if service_type == std::any::TypeId::of::<GizmoActionQueue>() {
            return Some(self.gizmo_action_queue);
        }
        None
    }
}

pub fn configure_gizmos_system(mut config_store: ResMut<GizmoConfigStore>) {
    // Split gizmo config:
    // - Grid should be depth-tested (model covers grid)
    // - Transform gizmo helper lines should be on top
    config_store
        .config_mut::<DefaultGizmoConfigGroup>()
        .0
        .depth_bias = 0.0;

    // Grid Configurations
    let (gc, _) = config_store.config_mut::<crate::gizmos::GridGizmos>(); // Minor
    gc.depth_bias = 0.0;
    gc.line.width = 1.0;

    let (gc, _) = config_store.config_mut::<crate::gizmos::GridMajorGizmos>(); // Major
    gc.depth_bias = 0.0;
    gc.line.width = 2.0;

    let (gc, _) = config_store.config_mut::<crate::gizmos::GridAxisGizmos>(); // Axes
    gc.depth_bias = 0.0;
    gc.line.width = 3.0;

    config_store
        .config_mut::<crate::tabs_system::viewport_3d::UvBoundaryGizmos>()
        .0
        .depth_bias = 0.0;

    let (gc, _) = config_store.config_mut::<crate::gizmos::TransformGizmoLines>();
    gc.depth_bias = -1.0;
    gc.line.width = 2.0;

    let (gc, _) = config_store.config_mut::<crate::gizmos::SelectedCurveGizmos>();
    gc.depth_bias = 0.0;
    gc.line.width = 2.0;

    let (gc, _) = config_store.config_mut::<crate::gizmos::SelectedCurveXrayGizmos>();
    gc.depth_bias = -1.0;
    gc.line.width = 2.0;
}
