use bevy::prelude::*;
use cunning_viewport::{camera::{CameraController, CameraTransition}, MainCamera, viewport_options::DisplayOptions};
use cunning_kernel::{geometry::attrs, mesh::Geometry};

#[derive(Resource, Default, Clone, Copy)]
pub struct TurntableState {
    pub center: Vec3,
    pub radius: f32,
    pub target_center: Vec3,
    pub target_radius: f32,
    pub dist: f32,
    pub angle_rad: f32,
    pub dirty: u64,
    pub has_bounds: bool,
}

#[inline]
pub fn update_bounds_from_geo(s: &mut TurntableState, g: &Geometry, dirty: u64) {
    let Some(ps) = g.get_point_attribute(attrs::P).and_then(|a| a.as_slice::<Vec3>()) else { s.has_bounds = false; return; };
    let mut min = Vec3::splat(f32::INFINITY);
    let mut max = Vec3::splat(f32::NEG_INFINITY);
    for &p in ps { min = min.min(p); max = max.max(p); }
    if !min.is_finite() || !max.is_finite() { s.has_bounds = false; return; }
    let c = (min + max) * 0.5;
    let r = ((max - min) * 0.5).length().max(0.01);
    if !s.has_bounds { s.center = c; s.radius = r; s.target_center = c; s.target_radius = r; }
    else { s.target_center = c; s.target_radius = r; }
    s.dirty = dirty;
    s.has_bounds = true;
}

pub fn turntable_camera_system(
    mut commands: Commands,
    time: Res<Time>,
    display: Res<DisplayOptions>,
    mut tt: ResMut<TurntableState>,
    mut q: Query<(Entity, &mut Transform, &Projection, &mut CameraController, Option<&CameraTransition>), With<MainCamera>>,
) {
    let Ok((e, mut t, proj, mut ctrl, trans)) = q.single_mut() else { return; };
    let enabled = display.turntable.enabled;
    ctrl.enabled = !enabled;
    if enabled { if trans.is_some() { commands.entity(e).remove::<CameraTransition>(); } }
    if !enabled || !tt.has_bounds { return; }

    let fov = match proj { Projection::Perspective(p) => p.fov, _ => std::f32::consts::FRAC_PI_3 };
    let target_dist = (tt.target_radius / (fov * 0.5).tan()).max(0.1) * display.turntable.distance_factor.max(0.01);
    if tt.dist <= 0.0 { tt.dist = target_dist; }
    let k = 1.0 - (-time.delta_secs() * 10.0).exp();
    tt.center = tt.center.lerp(tt.target_center, k);
    tt.radius = tt.radius + (tt.target_radius - tt.radius) * k;
    tt.dist = tt.dist + (target_dist - tt.dist) * k;
    tt.angle_rad = (tt.angle_rad + time.delta_secs() * display.turntable.speed_deg_per_sec.to_radians()) % (std::f32::consts::TAU);
    let elev = display.turntable.elevation_deg.to_radians().clamp(-1.3, 1.3);
    let ce = elev.cos();
    let dir = Vec3::new(tt.angle_rad.cos() * ce, elev.sin(), tt.angle_rad.sin() * ce);
    t.translation = tt.center + dir * tt.dist;
    t.look_at(tt.center, Vec3::Y);
}

