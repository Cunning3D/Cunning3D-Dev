use crate::invalidator::{RepaintCause, UiInvalidator};
use bevy::prelude::*; // Import added
use bevy::{camera::RenderTarget, window::WindowRef};
use crate::viewport_options::DisplayOptions;

#[derive(Component)]
pub struct CameraController {
    pub enabled: bool,
    pub sensitivity: f32,
    pub speed: f32,
    pub pivot: Vec3,
    pub is_orbiting: bool,
}

impl Default for CameraController {
    fn default() -> Self {
        Self {
            enabled: true,
            sensitivity: 0.2,
            speed: 5.0,
            pivot: Vec3::ZERO,
            is_orbiting: false,
        }
    }
}

#[derive(Resource, Default)]
pub struct ViewportInteractionState {
    pub is_hovered: bool,
    pub is_right_button_dragged: bool,
    pub is_middle_button_dragged: bool,
    pub is_alt_left_button_dragged: bool,
    pub is_gizmo_dragging: bool,
}

use crate::input::NavigationInput;
use crate::viewport_options::TurntableSettings;

pub fn camera_control_system(
    mut query: Query<(&mut Transform, &mut CameraController), With<Camera>>,
    nav_input: Res<NavigationInput>,
    time: Res<Time>,
    mut viewport_perf: ResMut<crate::viewport_perf::ViewportPerfTrace>,
) {
    let _perf = crate::viewport_perf::PerfScope::new(
        &mut viewport_perf,
        crate::viewport_perf::ViewportPerfSection::CameraControl,
    );
    for (mut transform, mut controller) in query.iter_mut() {
        if !controller.enabled {
            controller.is_orbiting = false;
            continue;
        }

        // 1. Zoom / Dolly
        if nav_input.zoom_delta.abs() > 0.0 {
            let move_dir =
                *transform.forward() * nav_input.zoom_delta * controller.speed * time.delta_secs();
            transform.translation += move_dir;
        }

        // 2. Active Navigation (Orbit / Pan / Fly)
        if nav_input.active {
            // Orbit Rotation
            if nav_input.orbit_delta.length_squared() > 0.0 {
                let (mut yaw, mut pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);

                // Apply input with sensitivity
                yaw -= nav_input.orbit_delta.x.to_radians() * controller.sensitivity;
                pitch -= nav_input.orbit_delta.y.to_radians() * controller.sensitivity;

                // Prevent Gimbal Lock: Clamp pitch strictly inside [-90, 90] degrees
                // Using a small epsilon to avoid singularity at exact poles
                const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.001;
                pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);

                // Reconstruct rotation from Euler angles
                // Note: We construct Y rotation first, then X, matching YXZ order
                transform.rotation =
                    Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
            }

            // Pan Translation
            if nav_input.pan_delta.length_squared() > 0.0 {
                let right = *transform.right() * -nav_input.pan_delta.x * 0.01;
                let up = *transform.up() * nav_input.pan_delta.y * 0.01;
                transform.translation += right + up;
            }

            // WASD Fly
            if nav_input.fly_vector.length_squared() > 0.0 {
                let mut translation = Vec3::ZERO;
                translation += *transform.forward() * nav_input.fly_vector.z; // W/S
                translation += *transform.right() * nav_input.fly_vector.x; // A/D
                translation += *transform.up() * nav_input.fly_vector.y; // Q/E

                if translation.length_squared() > 0.0 {
                    transform.translation +=
                        translation.normalize() * controller.speed * time.delta_secs();
                }
            }
        } else {
            controller.is_orbiting = false;
        }
    }
}

#[derive(Resource, Default)]
pub struct TurntableRuntimeState {
    pub dist: f32,
    pub angle_rad: f32,
}

pub fn turntable_camera_system(
    mut commands: Commands,
    time: Res<Time>,
    display: Res<DisplayOptions>,
    mut rt: ResMut<TurntableRuntimeState>,
    mut invalidator: ResMut<UiInvalidator>,
    mut q: Query<(Entity, &mut Transform, &Projection, &mut CameraController, Option<&CameraTransition>), With<crate::MainCamera>>,
) {
    let Ok((e, mut t, proj, mut ctrl, trans)) = q.single_mut() else { return; };
    let TurntableSettings { enabled, speed_deg_per_sec, elevation_deg, distance_factor, .. } = display.turntable;
    ctrl.enabled = !enabled;
    if !enabled { rt.dist = 0.0; return; }
    if trans.is_some() { commands.entity(e).remove::<CameraTransition>(); }
    let fov = match proj { Projection::Perspective(p) => p.fov, _ => std::f32::consts::FRAC_PI_3 };
    let cur_dist = t.translation.length().max(0.1);
    if rt.dist <= 0.0 { rt.dist = cur_dist * distance_factor.max(0.01); }
    rt.angle_rad = (rt.angle_rad + time.delta_secs() * speed_deg_per_sec.to_radians()) % (std::f32::consts::TAU);
    let elev = elevation_deg.to_radians().clamp(-1.3, 1.3);
    let ce = elev.cos();
    let dir = Vec3::new(rt.angle_rad.cos() * ce, elev.sin(), rt.angle_rad.sin() * ce);
    let _ = fov; // reserved for future auto-frame
    let center = Vec3::ZERO;
    t.translation = center + dir * rt.dist;
    t.look_at(center, Vec3::Y);
    invalidator.request_repaint_after_tagged(
        "camera/turntable",
        std::time::Duration::from_secs_f32(1.0 / 60.0),
        RepaintCause::Animation,
    );
}

// --- Transition System ---

#[derive(Component)]
pub struct CameraTransition {
    pub start_pos: Vec3,
    pub target_pos: Vec3,
    pub start_rot: Quat,
    pub target_rot: Quat,
    pub t: f32,
    pub duration: f32,
}

pub fn camera_transition_system(
    mut commands: Commands,
    mut query: Query<(Entity, &mut Transform, &mut CameraTransition)>,
    time: Res<Time>,
    mut invalidator: ResMut<UiInvalidator>, // Added invalidator
) {
    let mut animating = false;
    for (entity, mut transform, mut transition) in query.iter_mut() {
        animating = true;
        transition.t += time.delta_secs();
        let progress = (transition.t / transition.duration).clamp(0.0, 1.0);

        // Cubic Ease In Out
        let ease = if progress < 0.5 {
            4.0 * progress * progress * progress
        } else {
            1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0
        };

        transform.translation = transition.start_pos.lerp(transition.target_pos, ease);
        transform.rotation = transition.start_rot.slerp(transition.target_rot, ease);

        if progress >= 1.0 {
            commands.entity(entity).remove::<CameraTransition>();
        }
    }

    if animating {
        invalidator.request_repaint_after_tagged(
            "camera/transition",
            std::time::Duration::from_secs_f32(1.0 / 60.0),
            RepaintCause::Animation,
        );
    }
}

pub fn update_camera_speed_system(
    display_options: Res<DisplayOptions>,
    mut camera_query: Query<&mut CameraController>,
) {
    if display_options.is_changed() {
        if let Ok(mut controller) = camera_query.single_mut() {
            controller.speed = display_options.camera_speed;
        }
    }
}

pub fn sanitize_camera_window_targets_system(
    windows: Query<(), With<Window>>,
    mut cams: Query<(&mut RenderTarget, &mut Camera)>,
) {
    for (mut target, mut cam) in &mut cams {
        if let RenderTarget::Window(WindowRef::Entity(e)) = *target {
            if windows.get(e).is_err() {
                *target = RenderTarget::Window(WindowRef::Primary);
                cam.is_active = windows.iter().next().is_some();
                cam.viewport = None;
            }
        }
    }
}

/// System to handle camera view change events from the viewport gizmo
pub fn handle_camera_view_events(
    mut commands: Commands,
    mut events: MessageReader<crate::viewport_options::SetCameraViewEvent>,
    mut camera_query: Query<
        (Entity, &Transform, Option<&CameraTransition>),
        (With<Camera>, With<crate::MainCamera>),
    >,
) {
    use crate::viewport_options::CameraViewDirection;

    for event in events.read() {
        if let Ok((entity, transform, transition)) = camera_query.single_mut() {
            // Block input if currently animating
            if transition.is_some() {
                continue;
            }

            let distance = transform.translation.length();
            let target = Vec3::ZERO; // Assuming we're looking at origin

            // Set camera position and rotation based on view direction
            let position = match event.0 {
                CameraViewDirection::Front => Vec3::new(0.0, 0.0, distance),
                CameraViewDirection::Back => Vec3::new(0.0, 0.0, -distance),
                CameraViewDirection::Right => Vec3::new(distance, 0.0, 0.0),
                CameraViewDirection::Left => Vec3::new(-distance, 0.0, 0.0),
                CameraViewDirection::Top => Vec3::new(0.0, distance, 0.0),
                CameraViewDirection::Bottom => Vec3::new(0.0, -distance, 0.0),
                CameraViewDirection::Perspective => {
                    Vec3::new(-2.0, 2.5, 5.0).normalize() * distance
                }
                CameraViewDirection::Custom(dir) => dir.normalize() * distance,
            };

            let rotation = Transform::from_translation(position)
                .looking_at(target, Vec3::Y)
                .rotation;

            // Dynamic duration based on angular distance
            let angle = transform.rotation.angle_between(rotation);
            let duration = (angle / 2.0).clamp(0.4, 1.5);

            commands.entity(entity).insert(CameraTransition {
                start_pos: transform.translation,
                target_pos: position,
                start_rot: transform.rotation,
                target_rot: rotation,
                t: 0.0,
                duration,
            });
        }
    }
}

/// System to handle relative camera rotation events (Orbit/Roll)
pub fn handle_camera_rotate_events(
    mut commands: Commands,
    mut events: MessageReader<crate::viewport_options::CameraRotateEvent>,
    mut camera_query: Query<
        (Entity, &mut Transform, Option<&CameraTransition>),
        (With<Camera>, With<crate::MainCamera>),
    >,
) {
    for event in events.read() {
        if let Ok((entity, mut transform, transition)) = camera_query.single_mut() {
            let local_rot = event.rotation;

            // Immediate mode (Drag)
            if event.immediate {
                if transition.is_some() {
                    commands.entity(entity).remove::<CameraTransition>();
                }

                // w_rot represents the rotation in World Space
                let w_rot = transform.rotation * local_rot * transform.rotation.inverse();

                // Orbit around origin
                transform.translation = w_rot * transform.translation;
                transform.rotation = (w_rot * transform.rotation).normalize();

                continue;
            }

            // Animated mode (Click) - Block input if currently animating
            if transition.is_some() {
                continue;
            }

            // w_rot represents the rotation in World Space
            let w_rot = transform.rotation * local_rot * transform.rotation.inverse();

            let final_pos = w_rot * transform.translation;
            let final_rot = (w_rot * transform.rotation).normalize();

            // Dynamic duration based on angular distance
            let angle = transform.rotation.angle_between(final_rot);
            let duration = (angle / 2.0).clamp(0.4, 1.5);

            commands.entity(entity).insert(CameraTransition {
                start_pos: transform.translation,
                target_pos: final_pos,
                start_rot: transform.rotation,
                target_rot: final_rot,
                t: 0.0,
                duration,
            });
        }
    }
}
