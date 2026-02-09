use bevy::{
    input::mouse::{MouseMotion, MouseWheel},
    input::touch::Touch,
    prelude::*,
    window::{PrimaryWindow, RequestRedraw},
};
use bevy::ecs::message::{MessageReader, MessageWriter};
use bevy_egui::EguiContext;

use crate::{camera::ViewportInteractionState, layout::ViewportLayout, nav_input::NavigationInput};

pub fn input_mapping_system(
    mut nav_input: ResMut<NavigationInput>,
    interaction_state: Res<ViewportInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    mut egui_query: Query<&mut EguiContext, With<PrimaryWindow>>,
    viewport_layout: Res<ViewportLayout>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    *nav_input = NavigationInput::default();

    let (egui_wants_pointer, egui_wants_keyboard) = if let Ok(mut egui_ctx) = egui_query.single_mut() {
        let c = egui_ctx.get_mut();
        (c.wants_pointer_input(), c.wants_keyboard_input())
    } else {
        (false, false)
    };

    let mut raw_mouse_delta = Vec2::ZERO;
    for event in mouse_motion_events.read() { raw_mouse_delta += event.delta; }
    const MAX_MOUSE_DELTA: f32 = 100.0;
    raw_mouse_delta.x = raw_mouse_delta.x.clamp(-MAX_MOUSE_DELTA, MAX_MOUSE_DELTA);
    raw_mouse_delta.y = raw_mouse_delta.y.clamp(-MAX_MOUSE_DELTA, MAX_MOUSE_DELTA);

    let mut raw_scroll = 0.0;
    for event in mouse_wheel_events.read() { raw_scroll += event.y; }

    let active_touches: Vec<&Touch> = touches.iter().collect();
    if !active_touches.is_empty() {
        if egui_wants_pointer || egui_wants_keyboard { return; }
        nav_input.active = true;
        if active_touches.len() == 1 {
            nav_input.orbit_delta += active_touches[0].delta();
        } else if active_touches.len() == 2 {
            let t1 = active_touches[0];
            let t2 = active_touches[1];
            let mid_current = (t1.position() + t2.position()) * 0.5;
            let mid_prev = (t1.previous_position() + t2.previous_position()) * 0.5;
            nav_input.pan_delta += mid_current - mid_prev;
            let dist_current = t1.position().distance(t2.position());
            let dist_prev = t1.previous_position().distance(t2.previous_position());
            nav_input.zoom_delta += (dist_current - dist_prev) * 0.05;
        }
        return;
    }

    let viewport_cursor_over = if viewport_layout.logical_rect.is_some() {
        if let Ok(mut egui_ctx) = egui_query.single_mut() {
            let ctx = egui_ctx.get_mut();
            let p = ctx.pointer_latest_pos().or_else(|| ctx.pointer_hover_pos());
            viewport_layout.logical_rect.map_or(false, |r| p.map_or(false, |p| r.contains(p)))
        } else {
            false
        }
    } else {
        false
    };

    let ongoing_drag = interaction_state.is_right_button_dragged
        || interaction_state.is_middle_button_dragged
        || interaction_state.is_alt_left_button_dragged;
    let buttons_dragging_now =
        mouse_buttons.pressed(MouseButton::Right)
            || mouse_buttons.pressed(MouseButton::Middle)
            || (mouse_buttons.pressed(MouseButton::Left) && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)));
    let dragging_now = ongoing_drag || (buttons_dragging_now && (viewport_cursor_over || interaction_state.is_hovered));

    if egui_wants_pointer && !viewport_cursor_over && !dragging_now { return; }
    if egui_wants_keyboard
        && !interaction_state.is_right_button_dragged
        && !interaction_state.is_middle_button_dragged
        && !interaction_state.is_alt_left_button_dragged
    {
        return;
    }
    if interaction_state.is_gizmo_dragging { return; }

    let right_drag = interaction_state.is_right_button_dragged || (viewport_cursor_over && mouse_buttons.pressed(MouseButton::Right));
    let middle_drag = interaction_state.is_middle_button_dragged || (viewport_cursor_over && mouse_buttons.pressed(MouseButton::Middle));
    let alt_left_drag = interaction_state.is_alt_left_button_dragged
        || (viewport_cursor_over && mouse_buttons.pressed(MouseButton::Left) && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)));
    let is_interacting = interaction_state.is_hovered || right_drag || middle_drag || alt_left_drag;
    if !is_interacting { return; }

    if interaction_state.is_hovered || viewport_cursor_over { nav_input.zoom_delta = raw_scroll; }

    if right_drag {
        nav_input.active = true;
        nav_input.orbit_delta = raw_mouse_delta;
        let mut fly = Vec3::ZERO;
        if keys.pressed(KeyCode::KeyW) { fly.z += 1.0; }
        if keys.pressed(KeyCode::KeyS) { fly.z -= 1.0; }
        if keys.pressed(KeyCode::KeyA) { fly.x -= 1.0; }
        if keys.pressed(KeyCode::KeyD) { fly.x += 1.0; }
        if keys.pressed(KeyCode::KeyQ) { fly.y -= 1.0; }
        if keys.pressed(KeyCode::KeyE) { fly.y += 1.0; }
        nav_input.fly_vector = fly;
    } else if middle_drag {
        nav_input.active = true;
        nav_input.pan_delta = raw_mouse_delta;
    } else if alt_left_drag {
        nav_input.active = true;
        nav_input.orbit_delta = raw_mouse_delta;
    }

    if dragging_now
        || nav_input.active
        || nav_input.zoom_delta != 0.0
        || nav_input.orbit_delta.length_squared() != 0.0
        || nav_input.pan_delta.length_squared() != 0.0
    {
        redraw.write(RequestRedraw);
    }
}

