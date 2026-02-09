use bevy::prelude::*;
use bevy::ecs::message::MessageReader;

use crate::{nav_input::NavigationInput, viewport_options::{CameraRotateEvent, CameraViewDirection, DisplayOptions, SetCameraViewEvent}, MainCamera};

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
        Self { enabled: true, sensitivity: 0.2, speed: 5.0, pivot: Vec3::ZERO, is_orbiting: false }
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

pub fn camera_control_system(
    mut query: Query<(&mut Transform, &mut CameraController), (With<Camera>, With<MainCamera>)>,
    nav_input: Res<NavigationInput>,
    display_options: Res<DisplayOptions>,
    time: Res<Time>,
) {
    for (mut transform, mut controller) in query.iter_mut() {
        if !controller.enabled { controller.is_orbiting = false; continue; }
        controller.speed = display_options.camera_speed.max(0.01);

        if nav_input.zoom_delta.abs() > 0.0 {
            let move_dir = *transform.forward() * nav_input.zoom_delta * controller.speed * time.delta_secs();
            transform.translation += move_dir;
        }

        if nav_input.active {
            if nav_input.orbit_delta.length_squared() > 0.0 {
                let (mut yaw, mut pitch, _roll) = transform.rotation.to_euler(EulerRot::YXZ);
                yaw -= nav_input.orbit_delta.x.to_radians() * controller.sensitivity;
                pitch -= nav_input.orbit_delta.y.to_radians() * controller.sensitivity;
                const PITCH_LIMIT: f32 = std::f32::consts::FRAC_PI_2 - 0.001;
                pitch = pitch.clamp(-PITCH_LIMIT, PITCH_LIMIT);
                transform.rotation = Quat::from_axis_angle(Vec3::Y, yaw) * Quat::from_axis_angle(Vec3::X, pitch);
            }
            if nav_input.pan_delta.length_squared() > 0.0 {
                let right = *transform.right() * -nav_input.pan_delta.x * 0.01;
                let up = *transform.up() * nav_input.pan_delta.y * 0.01;
                transform.translation += right + up;
            }
            if nav_input.fly_vector.length_squared() > 0.0 {
                let mut translation = Vec3::ZERO;
                translation += *transform.forward() * nav_input.fly_vector.z;
                translation += *transform.right() * nav_input.fly_vector.x;
                translation += *transform.up() * nav_input.fly_vector.y;
                if translation.length_squared() > 0.0 {
                    transform.translation += translation.normalize() * controller.speed * time.delta_secs();
                }
            }
        } else {
            controller.is_orbiting = false;
        }
    }
}

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
    mut query: Query<(Entity, &mut Transform, &mut CameraTransition), With<MainCamera>>,
    time: Res<Time>,
) {
    for (entity, mut transform, mut transition) in query.iter_mut() {
        transition.t += time.delta_secs();
        let progress = (transition.t / transition.duration).clamp(0.0, 1.0);
        let ease = if progress < 0.5 { 4.0 * progress * progress * progress } else { 1.0 - (-2.0 * progress + 2.0).powi(3) / 2.0 };
        transform.translation = transition.start_pos.lerp(transition.target_pos, ease);
        transform.rotation = transition.start_rot.slerp(transition.target_rot, ease);
        if progress >= 1.0 { commands.entity(entity).remove::<CameraTransition>(); }
    }
}

pub fn handle_camera_view_events(
    mut commands: Commands,
    mut events: MessageReader<SetCameraViewEvent>,
    mut camera_query: Query<(Entity, &Transform, Option<&CameraTransition>), (With<Camera>, With<MainCamera>)>,
) {
    for event in events.read() {
        if let Ok((entity, transform, transition)) = camera_query.single_mut() {
            if transition.is_some() { continue; }
            let distance = transform.translation.length();
            let target = Vec3::ZERO;
            let position = match event.0 {
                CameraViewDirection::Front => Vec3::new(0.0, 0.0, distance),
                CameraViewDirection::Back => Vec3::new(0.0, 0.0, -distance),
                CameraViewDirection::Right => Vec3::new(distance, 0.0, 0.0),
                CameraViewDirection::Left => Vec3::new(-distance, 0.0, 0.0),
                CameraViewDirection::Top => Vec3::new(0.0, distance, 0.0),
                CameraViewDirection::Bottom => Vec3::new(0.0, -distance, 0.0),
                CameraViewDirection::Perspective => Vec3::new(-2.0, 2.5, 5.0).normalize() * distance,
                CameraViewDirection::Custom(dir) => dir.normalize() * distance,
            };
            let rotation = Transform::from_translation(position).looking_at(target, Vec3::Y).rotation;
            let angle = transform.rotation.angle_between(rotation);
            let duration = (angle / 2.0).clamp(0.4, 1.5);
            commands.entity(entity).insert(CameraTransition { start_pos: transform.translation, target_pos: position, start_rot: transform.rotation, target_rot: rotation, t: 0.0, duration });
        }
    }
}

pub fn handle_camera_rotate_events(
    mut commands: Commands,
    mut events: MessageReader<CameraRotateEvent>,
    mut camera_query: Query<(Entity, &mut Transform, Option<&CameraTransition>), (With<Camera>, With<MainCamera>)>,
) {
    for event in events.read() {
        if let Ok((entity, mut transform, transition)) = camera_query.single_mut() {
            let local_rot = event.rotation;
            if event.immediate {
                if transition.is_some() { commands.entity(entity).remove::<CameraTransition>(); }
                let w_rot = transform.rotation * local_rot * transform.rotation.inverse();
                transform.translation = w_rot * transform.translation;
                transform.rotation = (w_rot * transform.rotation).normalize();
                continue;
            }
            if transition.is_some() { continue; }
            let w_rot = transform.rotation * local_rot * transform.rotation.inverse();
            let final_pos = w_rot * transform.translation;
            let final_rot = (w_rot * transform.rotation).normalize();
            let angle = transform.rotation.angle_between(final_rot);
            let duration = (angle / 2.0).clamp(0.4, 1.5);
            commands.entity(entity).insert(CameraTransition { start_pos: transform.translation, target_pos: final_pos, start_rot: transform.rotation, target_rot: final_rot, t: 0.0, duration });
        }
    }
}

