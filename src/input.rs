use crate::camera::ViewportInteractionState;
use crate::coverlay_bevy_ui::CoverlayUiWantsInput;
use crate::tabs_system::viewport_3d::ViewportLayout;
use crate::timeline_bevy_ui::TimelineUiWantsInput;
use crate::topbar_bevy_ui::TopbarUiWantsInput;
use bevy::{
    input::mouse::{MouseMotion, MouseWheel},
    input::touch::Touch,
    prelude::*,
    window::{PrimaryWindow, RequestRedraw},
};
use bevy_egui::EguiContext;
use bevy_egui::EguiInput;

/// An abstraction of navigation commands.
/// This resource holds the "intent" of the user, regardless of the input method (Mouse vs Touch).
#[derive(Resource, Default, Debug)]
pub struct NavigationInput {
    /// Orbit rotation delta (Yaw, Pitch) in pixels/units
    pub orbit_delta: Vec2,
    /// Pan translation delta (X, Y) in pixels/units
    pub pan_delta: Vec2,
    /// Zoom/Dolly delta
    pub zoom_delta: f32,
    /// Fly movement vector (X=Right, Y=Up, Z=Forward) for WASD-like control
    pub fly_vector: Vec3,
    /// Whether an orbiting/panning interaction is currently active
    pub active: bool,
}

/// Reads raw hardware inputs (Mouse/Keyboard) and maps them to abstract NavigationInput.
/// This logic is specific to Desktop PC interaction.
pub fn input_mapping_system(
    mut nav_input: ResMut<NavigationInput>,
    interaction_state: Res<ViewportInteractionState>,
    keys: Res<ButtonInput<KeyCode>>,
    mouse_buttons: Res<ButtonInput<MouseButton>>,
    mut mouse_motion_events: MessageReader<MouseMotion>,
    mut mouse_wheel_events: MessageReader<MouseWheel>,
    touches: Res<Touches>,
    mut egui_query: Query<(&mut EguiContext, &mut EguiInput), With<PrimaryWindow>>,
    viewport_layout: Res<ViewportLayout>,
    timeline_wants: Res<TimelineUiWantsInput>,
    topbar_wants: Res<TopbarUiWantsInput>,
    coverlay_wants: Res<CoverlayUiWantsInput>,
    mut redraw: MessageWriter<RequestRedraw>,
) {
    // 1. Reset state for this frame
    *nav_input = NavigationInput::default();

    // Check if bevy_ui Timeline is consuming input
    if timeline_wants.0 || topbar_wants.0 || coverlay_wants.0 {
        return;
    }

    // Check if egui is actively consuming input (widgets active / text focused).
    let (egui_wants_pointer, egui_wants_keyboard) =
        if let Ok((mut egui_ctx, _egui_in)) = egui_query.single_mut() {
            let c = egui_ctx.get_mut();
            (c.wants_pointer_input(), c.wants_keyboard_input())
        } else {
            (false, false)
        };

    let mut raw_mouse_delta = Vec2::ZERO;
    for event in mouse_motion_events.read() {
        raw_mouse_delta += event.delta;
    }
    // Remote Desktop / Virtual Machine protection:
    // Clamp the raw delta to prevent massive jumps when cursor position resets or wraps.
    // Even with high sensitivity, single-frame movement rarely exceeds 100px.
    const MAX_MOUSE_DELTA: f32 = 100.0;
    raw_mouse_delta.x = raw_mouse_delta.x.clamp(-MAX_MOUSE_DELTA, MAX_MOUSE_DELTA);
    raw_mouse_delta.y = raw_mouse_delta.y.clamp(-MAX_MOUSE_DELTA, MAX_MOUSE_DELTA);

    let mut raw_scroll = 0.0;
    for event in mouse_wheel_events.read() {
        raw_scroll += event.y;
    }

    // --- Touch Input Handling ---
    // Touch inputs often bypass the "Hover" check because Egui might not claim touch events the same way.
    // Or if we treat the whole screen as viewport on mobile.
    // We explicitly block touch navigation if Egui is interacting.
    let active_touches: Vec<&Touch> = touches.iter().collect();

    if !active_touches.is_empty() {
        if egui_wants_pointer || egui_wants_keyboard {
            return;
        }

        nav_input.active = true;

        if active_touches.len() == 1 {
            // Single Finger -> Orbit
            // We invert Y to match standard "drag up to look up" or "drag down to tilt down"
            // Depending on camera sensitivity. Mouse delta Y up is usually negative or positive?
            // Bevy mouse motion Y is usually positive down? No, Bevy window Y is positive down.
            // MouseMotion event delta: Y is positive moving down.
            // Touch delta: Y is positive moving down.
            // Camera orbit: Pitching up/down.
            // Usually drag down -> look down (pitch decreases? or increases?).
            // Let's stick to passing raw delta and letting camera system handle sign.
            let touch = active_touches[0];
            nav_input.orbit_delta += touch.delta();
        } else if active_touches.len() == 2 {
            // Two Fingers -> Pan + Zoom
            let t1 = active_touches[0];
            let t2 = active_touches[1];

            // Pan: Move based on the average delta of both fingers
            let mid_current = (t1.position() + t2.position()) * 0.5;
            let mid_prev = (t1.previous_position() + t2.previous_position()) * 0.5;
            let pan_delta = mid_current - mid_prev;
            nav_input.pan_delta += pan_delta;

            // Zoom: Pinch detection
            let dist_current = t1.position().distance(t2.position());
            let dist_prev = t1.previous_position().distance(t2.previous_position());
            let zoom_change = dist_current - dist_prev;

            // Scale zoom sensitivity for touch
            // Mouse scroll is usually around 1.0 or -1.0 per step.
            // Pinch pixels can be small.
            nav_input.zoom_delta += zoom_change * 0.05;
        }
        // If using touch, we skip mouse logic to avoid conflict if mouse emulation is on
        return;
    }

    // If egui is actively consuming pointer, don't passthrough to viewport *unless*
    // the interaction started inside the native viewport hole (viewport gets priority).
    let viewport_cursor_over = if viewport_layout.logical_rect.is_some() {
        if let Ok((mut egui_ctx, _egui_in)) = egui_query.single_mut() {
            let ctx = egui_ctx.get_mut();
            let p = ctx.pointer_latest_pos().or_else(|| ctx.pointer_hover_pos());
            viewport_layout
                .logical_rect
                .map_or(false, |r| p.map_or(false, |p| r.contains(p)))
        } else {
            false
        }
    } else {
        false
    };
    // Robust "viewport interaction in progress" detection:
    // - interaction_state is written later in the frame (egui UI), so it can lag by 1 frame.
    // - during MMB/RMB drag some platforms/cursor-lock setups can make pointer pos temporarily unavailable.
    let ongoing_drag = interaction_state.is_right_button_dragged
        || interaction_state.is_middle_button_dragged
        || interaction_state.is_alt_left_button_dragged;
    let buttons_dragging_now = mouse_buttons.pressed(MouseButton::Right)
        || mouse_buttons.pressed(MouseButton::Middle)
        || (mouse_buttons.pressed(MouseButton::Left)
            && (keys.pressed(KeyCode::AltLeft) || keys.pressed(KeyCode::AltRight)));
    let dragging_now = ongoing_drag
        || (buttons_dragging_now && (viewport_cursor_over || interaction_state.is_hovered));

    // Keyboard/Focus isolation:
    // If the cursor is over the 3D viewport and the user starts interacting (keys/buttons),
    // egui text fields must stop receiving characters (e.g. "wwww" while moving).
    let any_key_down = keys.get_pressed().next().is_some();
    let any_mouse_down = mouse_buttons.get_pressed().next().is_some();
    let any_wheel = raw_scroll != 0.0;
    if viewport_cursor_over && (any_key_down || any_mouse_down || any_wheel || dragging_now) {
        if let Ok((mut egui_ctx, mut egui_in)) = egui_query.single_mut() {
            let ctx = egui_ctx.get_mut();
            if ctx.wants_keyboard_input() {
                ctx.memory_mut(|m| {
                    if let Some(id) = m.focused() {
                        m.surrender_focus(id);
                    }
                });
                // Best-effort: prevent queued key/char events from reaching the old focused TextEdit this frame.
                egui_in.events.retain(|e| {
                    !matches!(
                        e,
                        bevy_egui::egui::Event::Text(_)
                            | bevy_egui::egui::Event::Key { .. }
                            | bevy_egui::egui::Event::Copy
                            | bevy_egui::egui::Event::Cut
                            | bevy_egui::egui::Event::Paste(_)
                    )
                });
            }
            // Wheel isolation: when cursor is over the native 3D viewport hole, prevent egui widgets
            // (scroll areas, zoomable canvas, etc.) from also consuming the wheel in the same frame.
            if any_wheel {
                egui_in
                    .events
                    .retain(|e| !matches!(e, bevy_egui::egui::Event::MouseWheel { .. }));
            }
        }
    }

    if egui_wants_pointer && !viewport_cursor_over && !dragging_now {
        return;
    }
    // If egui is consuming keyboard (text edit focused), don't use WASD unless we're actively dragging navigation.
    if egui_wants_keyboard
        && !interaction_state.is_right_button_dragged
        && !interaction_state.is_middle_button_dragged
        && !interaction_state.is_alt_left_button_dragged
    {
        return;
    }

    // If not hovered and not dragging, ignore inputs.
    // We check distinct flags because drag might continue even if cursor leaves viewport (if Egui captured it).
    // Also check if we are interacting with a Gizmo. If so, we suppress camera navigation to prevent conflict.
    if interaction_state.is_gizmo_dragging {
        return;
    }

    // IMPORTANT: `interaction_state` is written by egui later in the frame (after this system),
    // so we can't rely on it for same-frame capture. We instead use the last known viewport rect.
    let right_drag = interaction_state.is_right_button_dragged
        || (viewport_cursor_over && mouse_buttons.pressed(MouseButton::Right));
    let middle_drag = interaction_state.is_middle_button_dragged
        || (viewport_cursor_over && mouse_buttons.pressed(MouseButton::Middle));
    let alt_left_drag = interaction_state.is_alt_left_button_dragged
        || (viewport_cursor_over
            && mouse_buttons.pressed(MouseButton::Left)
            && keys.pressed(KeyCode::AltLeft));
    let is_interacting = interaction_state.is_hovered || right_drag || middle_drag || alt_left_drag;

    if !is_interacting {
        return;
    }

    // 2. Map Inputs to Actions

    // Zoom (Scroll) - Always active if hovered
    if interaction_state.is_hovered || viewport_cursor_over {
        nav_input.zoom_delta = raw_scroll;
    }

    // Interaction Modes
    if right_drag {
        // MODE: Fly + Look
        nav_input.active = true;
        nav_input.orbit_delta = raw_mouse_delta; // Mouse Look

        // WASD Fly Vector
        let mut fly = Vec3::ZERO;
        if keys.pressed(KeyCode::KeyW) {
            fly.z += 1.0;
        } // Forward
        if keys.pressed(KeyCode::KeyS) {
            fly.z -= 1.0;
        } // Backward
        if keys.pressed(KeyCode::KeyA) {
            fly.x -= 1.0;
        } // Left
        if keys.pressed(KeyCode::KeyD) {
            fly.x += 1.0;
        } // Right
        if keys.pressed(KeyCode::KeyQ) {
            fly.y -= 1.0;
        } // Down
        if keys.pressed(KeyCode::KeyE) {
            fly.y += 1.0;
        } // Up
        nav_input.fly_vector = fly;
    } else if middle_drag {
        // MODE: Pan
        nav_input.active = true;
        nav_input.pan_delta = raw_mouse_delta;
    } else if alt_left_drag {
        // MODE: Orbit (Turntable style)
        nav_input.active = true;
        nav_input.orbit_delta = raw_mouse_delta;
    }

    // Reactive winit mode: ensure smooth viewport navigation by forcing redraw while interacting.
    // NOTE: don't depend solely on deltas: during a drag, some platforms coalesce/miss motion events.
    if dragging_now
        || nav_input.active
        || nav_input.zoom_delta != 0.0
        || nav_input.orbit_delta.length_squared() != 0.0
        || nav_input.pan_delta.length_squared() != 0.0
        || nav_input.fly_vector.length_squared() != 0.0
    {
        redraw.write(RequestRedraw);
    }
}
